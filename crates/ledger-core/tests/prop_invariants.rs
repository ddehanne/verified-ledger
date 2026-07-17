//! Property tests: invariants must hold on ARBITRARY event journals,
//! not just on the paths a human thought of.

#![allow(clippy::unwrap_used)]

use ledger_core::event::{EventPayload, EventRecord, SCHEMA_VERSION_V1, Side, TimestampMicros};
use ledger_core::ids::{EventId, ExecutionId, OrderId, SequenceNumber};
use ledger_core::money::UsdMoney;
use ledger_core::quantity::{FilledQuantity, OrderQuantity};
use ledger_core::reducer::{LedgerState, apply, replay};
use proptest::prelude::*;

const ORDER_POOL: [&str; 3] = ["ord-A", "ord-B", "ord-C"];
const EXEC_POOL: [&str; 5] = ["ex-1", "ex-2", "ex-3", "ex-4", "ex-5"];

/// Arbitrary payload drawing from small pools — collisions (duplicate
/// executions, unknown orders, illegal transitions) are FREQUENT by design.
fn arb_payload() -> impl Strategy<Value = EventPayload> {
    let oid = prop::sample::select(&ORDER_POOL[..]).prop_map(|s| OrderId::new(s).unwrap());
    let eid = prop::sample::select(&EXEC_POOL[..]).prop_map(|s| ExecutionId::new(s).unwrap());
    prop_oneof![
        (oid.clone(), prop::bool::ANY, 1i64..50).prop_map(|(o, buy, q)| {
            EventPayload::OrderCreated {
                order_id: o,
                side: if buy { Side::Buy } else { Side::Sell },
                quantity: OrderQuantity::new(q).unwrap(),
                limit_price: None,
            }
        }),
        oid.clone()
            .prop_map(|o| EventPayload::OrderSubmitted { order_id: o }),
        oid.clone()
            .prop_map(|o| EventPayload::OrderAcknowledged { order_id: o }),
        oid.clone().prop_map(|o| EventPayload::OrderRejected {
            order_id: o,
            reason: "r".into()
        }),
        (oid.clone(), eid.clone(), 1i64..30, 1i64..100_000).prop_map(|(o, e, q, p)| {
            EventPayload::ExecutionReceived {
                order_id: o,
                execution_id: e,
                quantity: FilledQuantity::new(q).unwrap(),
                price: UsdMoney::from_minor_units(p),
            }
        }),
        (eid, 1i64..1_000).prop_map(|(e, c)| EventPayload::CommissionReceived {
            execution_id: e,
            amount: UsdMoney::from_minor_units(c),
        }),
        oid.clone()
            .prop_map(|o| EventPayload::CancelRequested { order_id: o }),
        oid.prop_map(|o| EventPayload::CancelConfirmed { order_id: o }),
    ]
}

/// A journal: contiguous sequence 1..=n, arbitrary payloads. Events the
/// reducer REJECTS (unknown order, illegal transition) are simply not
/// appended — mirroring a writer that only journals accepted events.
fn arb_journal(max_len: usize) -> impl Strategy<Value = Vec<EventRecord>> {
    prop::collection::vec(arb_payload(), 0..max_len).prop_map(|payloads| {
        let mut state = LedgerState::default();
        let mut journal = Vec::new();
        let mut seq = 1u64;
        for payload in payloads {
            let record = EventRecord {
                event_id: EventId::new(format!("e{seq}")).unwrap(),
                sequence: SequenceNumber::new(seq),
                schema_version: SCHEMA_VERSION_V1,
                event_time: TimestampMicros(seq as i64),
                received_time: TimestampMicros(seq as i64),
                payload,
            };
            if let Ok((next, _)) = apply(&state, &record) {
                state = next;
                journal.push(record);
                seq += 1;
            }
        }
        journal
    })
}

proptest! {
    /// INV-2: replay is a pure function of the journal.
    #[test]
    fn replay_deterministic(journal in arb_journal(60)) {
        let a = replay(journal.iter()).unwrap();
        let b = replay(journal.iter()).unwrap();
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    /// INV-1 (global form): position = Σ signed units of DISTINCT
    /// executions actually applied — recomputed independently here.
    #[test]
    fn position_equals_sum_of_distinct_applied_executions(journal in arb_journal(60)) {
        let final_state = replay(journal.iter()).unwrap();

        // Independent recomputation: fold, tracking which executions
        // took effect (first occurrence AND order transition accepted).
        let mut state = LedgerState::default();
        let mut expected_units: i64 = 0;
        for r in &journal {
            let before_filled: Option<i64> = if let EventPayload::ExecutionReceived { order_id, .. } = &r.payload {
                state.orders.get(order_id).map(|a| a.order.filled.as_units())
            } else { None };
            let (next, _) = apply(&state, r).unwrap();
            if let EventPayload::ExecutionReceived { order_id, quantity, .. } = &r.payload {
                let after = next.orders.get(order_id).map(|a| a.order.filled.as_units());
                if after != before_filled {
                    // fill actually applied
                    let side = next.orders.get(order_id).unwrap().side;
                    expected_units += match side {
                        Side::Buy => quantity.as_units(),
                        Side::Sell => -quantity.as_units(),
                    };
                }
            }
            state = next;
        }
        prop_assert_eq!(final_state.net_position_units, expected_units);
    }

    /// Prefix property: replaying any prefix then applying the suffix
    /// equals replaying the whole journal (fold associativity — the
    /// property that makes crash-recovery-by-replay sound).
    #[test]
    fn prefix_replay_plus_suffix_equals_full_replay(
        journal in arb_journal(60),
        cut in 0usize..60
    ) {
        let cut = cut.min(journal.len());
        let full = replay(journal.iter()).unwrap();
        let mut state = replay(journal[..cut].iter()).unwrap();
        for r in &journal[cut..] {
            let (next, _) = apply(&state, r).unwrap();
            state = next;
        }
        prop_assert_eq!(state, full);
    }
}
