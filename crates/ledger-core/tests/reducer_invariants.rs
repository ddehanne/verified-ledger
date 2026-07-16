//! Integration tests for the reducer's core guarantees:
//! INV-1 (execution dedup), sequence authority, INV-2 (replay determinism).

#![allow(clippy::unwrap_used)] // tests must panic on broken setup

use ledger_core::error::DomainError;
use ledger_core::event::{EventPayload, EventRecord, Side, TimestampMicros, SCHEMA_VERSION_V1};
use ledger_core::ids::{EventId, ExecutionId, OrderId, SequenceNumber};
use ledger_core::money::UsdMoney;
use ledger_core::quantity::{FilledQuantity, OrderQuantity};
use ledger_core::reducer::{apply, replay, ApplyOutcome, LedgerState};

fn rec(seq: u64, payload: EventPayload) -> EventRecord {
    EventRecord {
        event_id: EventId::new(format!("evt-{seq}")).unwrap(),
        sequence: SequenceNumber::new(seq),
        schema_version: SCHEMA_VERSION_V1,
        event_time: TimestampMicros(1_000_000 + seq as i64),
        received_time: TimestampMicros(1_000_100 + seq as i64),
        payload,
    }
}

fn oid() -> OrderId {
    OrderId::new("ord-1").unwrap()
}

fn base_journal() -> Vec<EventRecord> {
    vec![
        rec(1, EventPayload::OrderCreated {
            order_id: oid(),
            side: Side::Buy,
            quantity: OrderQuantity::new(10).unwrap(),
            limit_price: None,
        }),
        rec(2, EventPayload::OrderSubmitted { order_id: oid() }),
        rec(3, EventPayload::OrderAcknowledged { order_id: oid() }),
        rec(4, EventPayload::ExecutionReceived {
            order_id: oid(),
            execution_id: ExecutionId::new("exec-A").unwrap(),
            quantity: FilledQuantity::new(4).unwrap(),
            price: UsdMoney::from_minor_units(10_000), // $100.00
        }),
    ]
}

#[test]
fn duplicate_execution_id_has_zero_economic_effect() {
    let mut journal = base_journal();
    let state_once = replay(journal.iter()).unwrap();

    // Same ExecutionId reported again (different event, next sequence).
    journal.push(rec(5, EventPayload::ExecutionReceived {
        order_id: oid(),
        execution_id: ExecutionId::new("exec-A").unwrap(),
        quantity: FilledQuantity::new(4).unwrap(),
        price: UsdMoney::from_minor_units(10_000),
    }));
    let (state_dup, outcome) =
        apply(&state_once, journal.last().unwrap()).unwrap();

    assert_eq!(outcome, ApplyOutcome::DuplicateIgnored);
    // Economic quantities UNCHANGED (INV-1):
    assert_eq!(state_dup.net_position_units, state_once.net_position_units);
    assert_eq!(state_dup.cash_delta, state_once.cash_delta);
    assert_eq!(
        state_dup.orders.get(&oid()).unwrap().order.filled,
        state_once.orders.get(&oid()).unwrap().order.filled,
    );
    // Only the cursor moved:
    assert_eq!(state_dup.last_sequence, Some(SequenceNumber::new(5)));
}

#[test]
fn sequence_gap_and_duplicate_are_rejected_untouched() {
    let journal = base_journal();
    let state = replay(journal.iter()).unwrap();

    // Gap: 4 -> 6
    let gap = rec(6, EventPayload::CancelRequested { order_id: oid() });
    let err = apply(&state, &gap).unwrap_err();
    assert!(matches!(err, DomainError::SequenceGap { expected: 5, got: 6 }));

    // Duplicate sequence: 4 again
    let dup = rec(4, EventPayload::CancelRequested { order_id: oid() });
    assert!(matches!(
        apply(&state, &dup).unwrap_err(),
        DomainError::SequenceGap { expected: 5, got: 4 }
    ));
}

#[test]
fn replay_is_deterministic_byte_identical() {
    let journal = base_journal();
    let a = replay(journal.iter()).unwrap();
    let b = replay(journal.iter()).unwrap();
    assert_eq!(a, b);
    // Byte-identical serialized state (BTreeMap ordering guarantee):
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn economic_accounting_buy_consumes_cash() {
    let state = replay(base_journal().iter()).unwrap();
    assert_eq!(state.net_position_units, 4); // Buy 4
    // 4 × $100.00 = $400.00 consumed
    assert_eq!(state.cash_delta, UsdMoney::from_minor_units(-40_000));
}

#[test]
fn commission_for_unknown_execution_is_quarantined_not_rejected() {
    let state = replay(base_journal().iter()).unwrap();
    let orphan = rec(5, EventPayload::CommissionReceived {
        execution_id: ExecutionId::new("exec-GHOST").unwrap(),
        amount: UsdMoney::from_minor_units(150),
    });
    let (next, outcome) = apply(&state, &orphan).unwrap();
    assert_eq!(outcome, ApplyOutcome::AnomalyQuarantined);
    assert_eq!(next.cash_delta, state.cash_delta); // no economic effect
    assert_eq!(next.anomalies.len(), 1); // evidence preserved
}

#[test]
fn empty_journal_replays_to_default() {
    let state = replay([].iter()).unwrap();
    assert_eq!(state, LedgerState::default());
}
