//! Append-only, hash-chained journal (V2 integrity model).
//!
//! One line = one `JournalLine`:
//!   { record, record_hash, prev_hash }
//! - record_hash = SHA-256(canonical JSON of record)   → detects bit rot
//! - prev_hash   = record_hash of the previous line     → detects
//!   reordering/removal (genesis uses 64 zeros)
//! - sequence contiguity is re-verified at read time    → the ordering law
//!
//! Recovery contract (frozen; FAULT-01 formalized):
//! - a corrupted/truncated LAST line = crash artifact → recover to the
//!   longest valid prefix, report `recovered_truncated_tail = true`
//! - corruption BEFORE the last line = evidence of tampering or disk
//!   failure → fatal typed error; a ledger does not guess.
//!
//! Durability: `append` writes line + newline, then `sync_data`. The
//! fsync-before-acknowledge discipline is the WAL contract (INV-6).

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use ledger_core::event::EventRecord;
use ledger_core::ids::SequenceNumber;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const GENESIS_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("io failure during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("serialization failure: {0}")]
    Serde(#[from] serde_json::Error),

    /// Corruption strictly before the last line — NOT a crash artifact.
    #[error("integrity violation at line {line}: {reason}")]
    IntegrityViolation { line: usize, reason: String },

    #[error("sequence violation at line {line}: expected {expected}, got {got}")]
    SequenceViolation { line: usize, expected: u64, got: u64 },
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalLine {
    record: EventRecord,
    record_hash: String,
    prev_hash: String,
}

fn hash_record(record: &EventRecord) -> Result<String, JournalError> {
    let bytes = serde_json::to_vec(record)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// Result of opening a journal file.
#[derive(Debug)]
pub struct RecoveredJournal {
    pub records: Vec<EventRecord>,
    pub last_hash: String,
    /// True iff a corrupted/truncated final line was discarded.
    pub recovered_truncated_tail: bool,
}

/// Read + verify a journal file. See recovery contract above.
pub fn read_journal(path: &Path) -> Result<RecoveredJournal, JournalError> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveredJournal {
                records: Vec::new(),
                last_hash: GENESIS_HASH.to_owned(),
                recovered_truncated_tail: false,
            });
        }
        Err(e) => return Err(JournalError::Io { operation: "open", source: e }),
    };

    let mut lines: Vec<String> = Vec::new();
    for l in BufReader::new(file).lines() {
        lines.push(l.map_err(|e| JournalError::Io { operation: "read_line", source: e })?);
    }
    // Trailing empty line (file ends with \n) is not a record.
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }

    let mut records = Vec::with_capacity(lines.len());
    let mut prev_hash = GENESIS_HASH.to_owned();
    let mut recovered = false;

    for (i, raw) in lines.iter().enumerate() {
        let is_last = i + 1 == lines.len();
        let line_no = i + 1;

        let parsed: Result<JournalLine, _> = serde_json::from_str(raw);
        let line = match parsed {
            Ok(l) => l,
            Err(e) if is_last => {
                // Crash artifact: torn final write. Recover.
                let _ = e;
                recovered = true;
                break;
            }
            Err(e) => {
                return Err(JournalError::IntegrityViolation {
                    line: line_no,
                    reason: format!("unparseable interior line: {e}"),
                })
            }
        };

        // Hash chain checks — a broken chain is tampering wherever it is,
        // EXCEPT a wrong record_hash on the very last line (torn write).
        let computed = hash_record(&line.record)?;
        if line.record_hash != computed {
            if is_last {
                recovered = true;
                break;
            }
            return Err(JournalError::IntegrityViolation {
                line: line_no,
                reason: "record_hash mismatch".to_owned(),
            });
        }
        if line.prev_hash != prev_hash {
            return Err(JournalError::IntegrityViolation {
                line: line_no,
                reason: "prev_hash chain broken".to_owned(),
            });
        }

        // Sequence law re-verified at read time.
        let expected = records
            .last()
            .map(|r: &EventRecord| r.sequence.value() + 1)
            .unwrap_or(SequenceNumber::FIRST.value());
        if line.record.sequence.value() != expected {
            return Err(JournalError::SequenceViolation {
                line: line_no,
                expected,
                got: line.record.sequence.value(),
            });
        }

        prev_hash = line.record_hash.clone();
        records.push(line.record);
    }

    Ok(RecoveredJournal { records, last_hash: prev_hash, recovered_truncated_tail: recovered })
}

/// Append-only writer holding the chain head.
pub struct JournalWriter {
    path: PathBuf,
    file: File,
    last_hash: String,
}

impl JournalWriter {
    /// Open for append. Runs full read-side verification first — a
    /// writer never appends to a journal it has not verified.
    pub fn open(path: &Path) -> Result<(Self, RecoveredJournal), JournalError> {
        let recovered = read_journal(path)?;
        if recovered.recovered_truncated_tail {
            truncate_to_valid_prefix(path, recovered.records.len())?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| JournalError::Io { operation: "open_append", source: e })?;
        Ok((
            Self {
                path: path.to_owned(),
                file,
                last_hash: recovered.last_hash.clone(),
            },
            recovered,
        ))
    }

    /// Durably append one record: write line, newline, fsync (INV-6).
    pub fn append(&mut self, record: &EventRecord) -> Result<(), JournalError> {
        let record_hash = hash_record(record)?;
        let line = JournalLine {
            record: record.clone(),
            record_hash: record_hash.clone(),
            prev_hash: self.last_hash.clone(),
        };
        let mut bytes = serde_json::to_vec(&line)?;
        bytes.push(b'\n');
        self.file
            .write_all(&bytes)
            .map_err(|e| JournalError::Io { operation: "write", source: e })?;
        self.file
            .sync_data()
            .map_err(|e| JournalError::Io { operation: "sync_data", source: e })?;
        self.last_hash = record_hash;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Rewrite the file keeping only the first `keep` lines (used once at
/// open when a torn tail was detected). Atomic: tmp + rename.
fn truncate_to_valid_prefix(path: &Path, keep: usize) -> Result<(), JournalError> {
    let file = File::open(path)
        .map_err(|e| JournalError::Io { operation: "open_for_truncate", source: e })?;
    let kept: Vec<String> = BufReader::new(file)
        .lines()
        .take(keep)
        .collect::<Result<_, _>>()
        .map_err(|e| JournalError::Io { operation: "read_for_truncate", source: e })?;

    let tmp = path.with_extension("tmp_recover");
    {
        let mut out = File::create(&tmp)
            .map_err(|e| JournalError::Io { operation: "create_tmp", source: e })?;
        for l in &kept {
            out.write_all(l.as_bytes())
                .and_then(|_| out.write_all(b"\n"))
                .map_err(|e| JournalError::Io { operation: "write_tmp", source: e })?;
        }
        out.sync_data()
            .map_err(|e| JournalError::Io { operation: "sync_tmp", source: e })?;
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| JournalError::Io { operation: "rename_tmp", source: e })?;
    Ok(())
}
