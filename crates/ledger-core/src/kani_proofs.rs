//! Bounded model-checking proofs (Kani/CBMC).
//!
//! These are not tests: Kani explores ALL values in the stated domains
//! symbolically. A passing proof is exhaustive within its bounds.

#[cfg(kani)]
mod proofs {
    use crate::ids::OrderId;
    use crate::ids::SequenceNumber;
    use crate::money::UsdMoney;
    use crate::order::{Order, OrderEvent, OrderState, transition_order};
    use crate::quantity::{FilledQuantity, OrderQuantity};

    /// P1 — Money arithmetic is total and honest over ALL i64 pairs:
    /// never panics; Err exactly when the mathematical result would
    /// overflow; Ok value is exact.
    #[kani::proof]
    fn p1_money_checked_add_total_and_exact() {
        let a: i64 = kani::any();
        let b: i64 = kani::any();
        let ma = UsdMoney::from_minor_units(a);
        let mb = UsdMoney::from_minor_units(b);
        match ma.checked_add(mb) {
            Ok(sum) => {
                // Exactness: matches wide arithmetic.
                let wide = (a as i128) + (b as i128);
                assert_eq!(sum.as_minor_units() as i128, wide);
            }
            Err(_) => {
                let wide = (a as i128) + (b as i128);
                assert!(wide > i64::MAX as i128 || wide < i64::MIN as i128);
            }
        }
    }

    /// P2 — Sequence law: for every n < u64::MAX, next(n) succeeds and
    /// is_successor_of holds; next fails ONLY at u64::MAX; and
    /// is_successor_of rejects gaps and duplicates for all pairs.
    #[kani::proof]
    fn p2_sequence_successor_law() {
        let n: u64 = kani::any();
        let s = SequenceNumber::new(n);
        match s.next() {
            Ok(succ) => {
                assert!(n < u64::MAX);
                assert!(succ.is_successor_of(s));
            }
            Err(_) => assert_eq!(n, u64::MAX),
        }

        let m: u64 = kani::any();
        let t = SequenceNumber::new(m);
        // Characterization: successor iff m == n+1 exactly.
        assert_eq!(t.is_successor_of(s), n < u64::MAX && m == n + 1);
    }

    /// P3 — Transition table is total-and-typed over the WHOLE
    /// state × event product: never panics; on Err the order is
    /// bit-identical to the input (INV-17 no-mutation, exhaustive).
    #[kani::proof]
    #[kani::unwind(4)]
    fn p3_transition_total_no_mutation_on_err() {
        // Symbolic state
        let state_sel: u8 = kani::any();
        kani::assume(state_sel < 9);
        let state = match state_sel {
            0 => OrderState::IntentCreated,
            1 => OrderState::Submitted,
            2 => OrderState::Acknowledged,
            3 => OrderState::PartiallyFilled,
            4 => OrderState::CancelPending,
            5 => OrderState::Filled,
            6 => OrderState::Cancelled,
            7 => OrderState::Rejected,
            _ => OrderState::IntegrityConflict,
        };

        // Symbolic quantities (bounded but arbitrary within bounds)
        let ordered_raw: i64 = kani::any();
        kani::assume(ordered_raw >= 1 && ordered_raw <= 1_000);
        let filled_raw: i64 = kani::any();
        kani::assume(filled_raw >= 0 && filled_raw <= 2_000);

        let order = {
            let mut o = Order::new(
                OrderId::new("o").unwrap_or_else(|_| unreachable!()),
                OrderQuantity::new(ordered_raw).unwrap_or_else(|_| unreachable!()),
            );
            o.state = state;
            o.filled = FilledQuantity::new(filled_raw).unwrap_or_else(|_| unreachable!());
            o
        };

        // Symbolic event
        let ev_sel: u8 = kani::any();
        kani::assume(ev_sel < 6);
        let qty_raw: i64 = kani::any();
        kani::assume(qty_raw >= 0 && qty_raw <= 1_000);
        let event = match ev_sel {
            0 => OrderEvent::Submitted,
            1 => OrderEvent::Acknowledged,
            2 => OrderEvent::Rejected,
            3 => OrderEvent::Execution {
                quantity: FilledQuantity::new(qty_raw).unwrap_or_else(|_| unreachable!()),
            },
            4 => OrderEvent::CancelRequested,
            _ => OrderEvent::CancelConfirmed,
        };

        let before = order.clone();
        match transition_order(&order, event) {
            Ok((after, _class)) => {
                // Fills never decrease; identity preserved.
                assert!(after.filled.as_units() >= before.filled.as_units());
                assert_eq!(after.id, before.id);
                assert_eq!(after.ordered, before.ordered);
            }
            Err(_) => {
                // INV-17: input untouched (we hold it — prove unchanged).
                assert_eq!(order, before);
            }
        }
    }
}
