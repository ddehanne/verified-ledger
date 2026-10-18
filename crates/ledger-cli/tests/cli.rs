#![allow(clippy::unwrap_used)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::Command;

use ledger_core::event::{EventPayload, EventRecord, SCHEMA_VERSION_V1, Side, TimestampMicros};
use ledger_core::ids::{EventId, OrderId, SequenceNumber};
use ledger_core::quantity::OrderQuantity;
use ledger_storage::journal::JournalWriter;

fn invoke(command: &str, path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ledger-cli"))
        .arg(command)
        .arg(path)
        .output()
        .unwrap()
}

fn create_valid_journal(path: &std::path::Path) {
    let record = EventRecord {
        event_id: EventId::new("event-1").unwrap(),
        sequence: SequenceNumber::FIRST,
        schema_version: SCHEMA_VERSION_V1,
        event_time: TimestampMicros(1),
        received_time: TimestampMicros(2),
        payload: EventPayload::OrderCreated {
            order_id: OrderId::new("order-1").unwrap(),
            side: Side::Buy,
            quantity: OrderQuantity::new(10).unwrap(),
            limit_price: None,
        },
    };
    let (mut writer, _) = JournalWriter::open(path).unwrap();
    writer.append(&record).unwrap();
}

#[test]
fn missing_journal_fails_without_creation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.ndjson");
    assert!(!invoke("verify", &path).status.success());
    assert!(!invoke("replay", &path).status.success());
    assert!(!path.exists());
}

#[test]
fn verified_and_replayed_journal_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("valid.ndjson");
    create_valid_journal(&path);
    let before = fs::read(&path).unwrap();
    let verify = invoke("verify", &path);
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(String::from_utf8_lossy(&verify.stdout).contains("verified 1 records"));
    let replay = invoke("replay", &path);
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert!(String::from_utf8_lossy(&replay.stdout).contains("order-1"));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn damaged_tail_is_rejected_without_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("torn.ndjson");
    create_valid_journal(&path);
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"{\"incomplete\":").unwrap();
    drop(file);
    let before = fs::read(&path).unwrap();
    assert!(!invoke("verify", &path).status.success());
    assert!(!invoke("replay", &path).status.success());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn invalid_command_fails() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        !invoke("repair", &dir.path().join("irrelevant"))
            .status
            .success()
    );
}
