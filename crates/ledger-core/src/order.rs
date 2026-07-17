//! Authoritative order state — runtime enum + exhaustive transition table.
//!
//! Frozen arbitration: the ledger records EXTERNAL EVIDENCE. Late,
//! duplicated, out-of-order or contradictory proofs must be
//! REPRESENTABLE and classified — never made impossible by types,
//! never dropped. Typestate is NOT used for authoritative state (V1).
//!
//! INV-17: any transition absent from the declared table is rejected
//! with a typed error and NO state mutation.

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::ids::OrderId;
use crate::quantity::{FillStatus, FilledQuantity, OrderQuantity, fill_status};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderState {
    IntentCreated,
    Submitted,
    Acknowledged,
    PartiallyFilled,
    CancelPending,
    Filled,
    Cancelled,
    Rejected,
    /// External evidence contradicts local knowledge (late fill after
    /// terminal, overfill…). Preserved, quarantined, human-resolved.
    IntegrityConflict,
}

impl OrderState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            OrderState::Filled
                | OrderState::Cancelled
                | OrderState::Rejected
                | OrderState::IntegrityConflict
        )
    }
}

/// How the applied transition is classified for audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionClass {
    Ordinary,
    Anomalous(AnomalyKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnomalyKind {
    FillBeforeAck,
    FillDuringCancelPending,
    LateFillAfterTerminal,
    Overfill,
}

/// Order aggregate as tracked by the ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub ordered: OrderQuantity,
    pub filled: FilledQuantity,
    pub state: OrderState,
}

impl Order {
    pub fn new(id: OrderId, ordered: OrderQuantity) -> Self {
        Self {
            id,
            ordered,
            filled: FilledQuantity::ZERO,
            state: OrderState::IntentCreated,
        }
    }
}

/// Order-scoped commands the reducer derives from event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderEvent {
    Submitted,
    Acknowledged,
    Rejected,
    Execution { quantity: FilledQuantity },
    CancelRequested,
    CancelConfirmed,
}

/// Pure, exhaustive transition function (INV-17). On error the input
/// order is untouched (callers keep the old value; we never mutate).
pub fn transition_order(
    order: &Order,
    event: OrderEvent,
) -> Result<(Order, TransitionClass), DomainError> {
    use OrderEvent as E;
    use OrderState as S;

    let invalid = |from: OrderState| {
        Err(DomainError::InvalidTransition {
            from,
            event: event_name(event),
        })
    };

    match (order.state, event) {
        // ── Ordinary lifecycle ────────────────────────────────────────
        (S::IntentCreated, E::Submitted) => ok(order, S::Submitted, TransitionClass::Ordinary),
        (S::Submitted, E::Acknowledged) => ok(order, S::Acknowledged, TransitionClass::Ordinary),
        (S::IntentCreated | S::Submitted | S::Acknowledged, E::Rejected) => {
            ok(order, S::Rejected, TransitionClass::Ordinary)
        }
        (S::Submitted | S::Acknowledged | S::PartiallyFilled, E::CancelRequested) => {
            ok(order, S::CancelPending, TransitionClass::Ordinary)
        }
        (S::CancelPending, E::CancelConfirmed) => {
            ok(order, S::Cancelled, TransitionClass::Ordinary)
        }

        // ── Executions: ordinary and anomalous-but-representable ─────
        (S::Acknowledged | S::PartiallyFilled, E::Execution { quantity }) => {
            apply_fill(order, quantity, None)
        }
        (S::Submitted, E::Execution { quantity }) => {
            apply_fill(order, quantity, Some(AnomalyKind::FillBeforeAck))
        }
        (S::CancelPending, E::Execution { quantity }) => {
            apply_fill(order, quantity, Some(AnomalyKind::FillDuringCancelPending))
        }
        // Late fill after terminal: evidence PRESERVED, order quarantined.
        (S::Filled | S::Cancelled | S::Rejected, E::Execution { quantity }) => {
            let filled = order.filled.checked_add(quantity)?;
            Ok((
                Order {
                    filled,
                    state: S::IntegrityConflict,
                    ..order.clone()
                },
                TransitionClass::Anomalous(AnomalyKind::LateFillAfterTerminal),
            ))
        }

        // ── Everything else: rejected, untouched (INV-17) ────────────
        (from, _) => invalid(from),
    }
}

fn ok(
    order: &Order,
    state: OrderState,
    class: TransitionClass,
) -> Result<(Order, TransitionClass), DomainError> {
    Ok((
        Order {
            state,
            ..order.clone()
        },
        class,
    ))
}

fn apply_fill(
    order: &Order,
    quantity: FilledQuantity,
    anomaly: Option<AnomalyKind>,
) -> Result<(Order, TransitionClass), DomainError> {
    let filled = order.filled.checked_add(quantity)?;
    let (state, class) = match fill_status(order.ordered, filled) {
        FillStatus::Partial => (
            OrderState::PartiallyFilled,
            anomaly.map_or(TransitionClass::Ordinary, TransitionClass::Anomalous),
        ),
        FillStatus::Exact => (
            OrderState::Filled,
            anomaly.map_or(TransitionClass::Ordinary, TransitionClass::Anomalous),
        ),
        // Overfill dominates any other anomaly classification.
        FillStatus::Overfill => (
            OrderState::IntegrityConflict,
            TransitionClass::Anomalous(AnomalyKind::Overfill),
        ),
    };
    Ok((
        Order {
            filled,
            state,
            ..order.clone()
        },
        class,
    ))
}

const fn event_name(e: OrderEvent) -> &'static str {
    match e {
        OrderEvent::Submitted => "Submitted",
        OrderEvent::Acknowledged => "Acknowledged",
        OrderEvent::Rejected => "Rejected",
        OrderEvent::Execution { .. } => "Execution",
        OrderEvent::CancelRequested => "CancelRequested",
        OrderEvent::CancelConfirmed => "CancelConfirmed",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn order(qty: i64) -> Order {
        Order::new(
            OrderId::new("ord-1").unwrap(),
            OrderQuantity::new(qty).unwrap(),
        )
    }

    fn fq(n: i64) -> FilledQuantity {
        FilledQuantity::new(n).unwrap()
    }

    #[test]
    fn ordinary_lifecycle_to_filled() {
        let o = order(10);
        let (o, c) = transition_order(&o, OrderEvent::Submitted).unwrap();
        assert_eq!(c, TransitionClass::Ordinary);
        let (o, _) = transition_order(&o, OrderEvent::Acknowledged).unwrap();
        let (o, c) = transition_order(&o, OrderEvent::Execution { quantity: fq(4) }).unwrap();
        assert_eq!(
            (o.state, c),
            (OrderState::PartiallyFilled, TransitionClass::Ordinary)
        );
        let (o, _) = transition_order(&o, OrderEvent::Execution { quantity: fq(6) }).unwrap();
        assert_eq!(o.state, OrderState::Filled);
    }

    #[test]
    fn fill_before_ack_is_anomalous_but_applied() {
        let o = order(5);
        let (o, _) = transition_order(&o, OrderEvent::Submitted).unwrap();
        let (o, c) = transition_order(&o, OrderEvent::Execution { quantity: fq(5) }).unwrap();
        assert_eq!(o.state, OrderState::Filled);
        assert_eq!(c, TransitionClass::Anomalous(AnomalyKind::FillBeforeAck));
    }

    #[test]
    fn late_fill_after_cancel_preserved_and_quarantined() {
        let o = order(10);
        let (o, _) = transition_order(&o, OrderEvent::Submitted).unwrap();
        let (o, _) = transition_order(&o, OrderEvent::CancelRequested).unwrap();
        let (o, _) = transition_order(&o, OrderEvent::CancelConfirmed).unwrap();
        assert_eq!(o.state, OrderState::Cancelled);
        let (o, c) = transition_order(&o, OrderEvent::Execution { quantity: fq(3) }).unwrap();
        assert_eq!(o.state, OrderState::IntegrityConflict);
        assert_eq!(o.filled, fq(3)); // evidence preserved
        assert_eq!(
            c,
            TransitionClass::Anomalous(AnomalyKind::LateFillAfterTerminal)
        );
    }

    #[test]
    fn overfill_quarantines_with_evidence() {
        let o = order(10);
        let (o, _) = transition_order(&o, OrderEvent::Submitted).unwrap();
        let (o, _) = transition_order(&o, OrderEvent::Acknowledged).unwrap();
        let (o, c) = transition_order(&o, OrderEvent::Execution { quantity: fq(15) }).unwrap();
        assert_eq!(o.state, OrderState::IntegrityConflict);
        assert_eq!(o.filled, fq(15));
        assert_eq!(c, TransitionClass::Anomalous(AnomalyKind::Overfill));
    }

    #[test]
    fn undeclared_transition_rejected_no_mutation() {
        let o = order(10); // IntentCreated
        let before = o.clone();
        let err = transition_order(&o, OrderEvent::CancelConfirmed).unwrap_err();
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
        assert_eq!(o, before); // untouched (INV-17)
    }
}
