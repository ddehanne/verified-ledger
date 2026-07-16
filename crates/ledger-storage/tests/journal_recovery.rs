//! Journal contract tests: round-trip, torn-tail recovery (crash),
//! interior corruption (tampering = fatal), chain break, and the
//! end-to-end crash-replay equivalence.

#![allow(clippy::unwrap_used)]

use std::fs::OpenOptions;
use std::io::Write;

use ledger_core::event::{EventPayload, EventRecord, Side, TimestampMicros, SCHEMA_VERSION_V1};
use ledger_core::ids::{EventId, ExecutionId, OrderId, SequenceNumber};
use ledger_core::money::UsdMoney;
use ledger_core::quantity::{FilledQuantity, OrderQuantity};
use ledger_core::reducer::replay;
use ledger_storage::journal::{read_journal, JournalError, JournalWriter};

fn rec(seq: u64, payload: EventPayload) -> EventRecord {
    EventRecord {
        event_id: EventId::new(format!("evt-{seq}")).unwrap(),
        sequence: SequenceNumber::new(seq),
        schema_version: SCHEMA_VERSION_V1,
        event_time: TimestampMicros(seq as i64),
        received_time: TimestampMicros(seq as i64),
        payload,
    }
}

fn sample_records(n: u64) -> Vec<EventRecord> {
    let oid = OrderId::new("ord-1").unwrap();
    let mut v = vec![
        rec(1, EventPayload::OrderCreated {
            order_id: oid.clone(),
            side: Side::Buy,
            quantity: OrderQuantity::new(100).unwrap(),
            limit_price: None,
        }),
        rec(2, EventPayload::OrderSubmitted { order_id: oid.clone() }),
        rec(3, EventPayload::OrderAcknowledged { order_id: oid.clone() }),
    ];
    for s in 4..=n {
        v.push(rec(s, EventPayload::ExecutionReceived {
            order_id: oid.clone(),
            execution_id: ExecutionId::new(format!("ex-{s}")).unwrap(),
            quantity: FilledQuantity::new(1).unwrap(),
            price: UsdMoney::from_minor_units(10_000),
        }));
    }
    v
}

#[test]
fn round_trip_write_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.ndjson");
    let records = sample_records(8);

    let (mut w, initial) = JournalWriter::open(&path).unwrap();
    assert!(initial.records.is_empty());
    for r in &records {
        w.append(r).unwrap();
    }

    let back = read_journal(&path).unwrap();
    assert!(!back.recovered_truncated_tail);
    assert_eq!(back.records, records);
}

#[test]
fn torn_final_line_is_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.ndjson");
    let records = sample_records(6);
    let (mut w, _) = JournalWriter::open(&path).unwrap();
    for r in &records {
        w.append(r).unwrap();
    }
    drop(w);

    // Simulate SIGKILL mid-write: append garbage without newline.
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(br#"{"record":{"event_id":"evt-99","seq"#).unwrap();
    drop(f);

    let back = read_journal(&path).unwrap();
    assert!(back.recovered_truncated_tail);
    assert_eq!(back.records, records); // longest valid prefix

    // Re-open heals the file and appending works again.
    let (mut w2, healed) = JournalWriter::open(&path).unwrap();
    assert!(healed.recovered_truncated_tail);
    let next = rec(7, EventPayload::CancelRequested {
        order_id: OrderId::new("ord-1").unwrap(),
    });
    w2.append(&next).unwrap();
    let final_read = read_journal(&path).unwrap();
    assert!(!final_read.recovered_truncated_tail);
    assert_eq!(final_read.records.len(), 7);
}

#[test]
fn interior_corruption_is_fatal_not_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.ndjson");
    let (mut w, _) = JournalWriter::open(&path).unwrap();
    for r in sample_records(6) {
        w.append(&r).unwrap();
    }
    drop(w);

    // Flip bytes in line 3 (interior) — tampering, not a crash.
    let content = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = content.lines().collect();
    let corrupted = lines[2].replace("ord-1", "ord-X");
    lines[2] = &corrupted;
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();

    match read_journal(&path) {
        Err(JournalError::IntegrityViolation { line: 3, .. }) => {}
        other => panic!("expected IntegrityViolation at line 3, got {other:?}"),
    }
}

#[test]
fn removed_interior_line_breaks_chain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.ndjson");
    let (mut w, _) = JournalWriter::open(&path).unwrap();
    for r in sample_records(6) {
        w.append(&r).unwrap();
    }
    drop(w);

    // Delete line 2 — prev_hash chain (and sequence) must object.
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    let pruned: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, l)| *l)
        .collect();
    std::fs::write(&path, pruned.join("\n") + "\n").unwrap();

    assert!(read_journal(&path).is_err());
}

/// The end-to-end guarantee the whole project exists for:
/// journal → crash (torn tail) → reopen → replay ⇒ exactly the state
/// of the durable prefix. FAULT-01 as a theorem, not a hope.
#[test]
fn crash_replay_equals_prefix_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.ndjson");
    let records = sample_records(10);
    let (mut w, _) = JournalWriter::open(&path).unwrap();
    for r in &records {
        w.append(r).unwrap();
    }
    drop(w);

    // Crash: torn tail garbage.
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"{torn").unwrap();
    drop(f);

    let (_, recovered) = JournalWriter::open(&path).unwrap();
    let state_recovered = replay(recovered.records.iter()).unwrap();
    let state_expected = replay(records.iter()).unwrap();
    assert_eq!(state_recovered, state_expected);
}
