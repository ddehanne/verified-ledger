//! Pure reducer: LedgerState × EventRecord → LedgerState.
//!
//! LAW-01: no clock, no RNG, no IO. Same event sequence in, same state
//! out — replay determinism (INV-2) follows from purity.
//!
//! INV-1: an ExecutionId is applied AT MOST ONCE. A duplicate execution
//! report is recorded as seen but has ZERO economic effect.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::event::{EventPayload, EventRecord, Side};
use crate::ids::{ExecutionId, OrderId, SequenceNumber};
use crate::money::UsdMoney;
use crate::order::{Order, OrderEvent, TransitionClass, transition_order};
use crate::quantity::OrderQuantity;

/// Whole-ledger state. BTreeMap/BTreeSet for deterministic iteration
/// order (replay produces byte-identical serialized state).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LedgerState {
    pub orders: BTreeMap<OrderId, OrderAggregate>,
    pub seen_executions: BTreeSet<ExecutionId>,
    pub last_sequence: Option<SequenceNumber>,
    /// Signed net position in units (Buy +, Sell −) per symbol-less V1.
    pub net_position_units: i64,
    /// Cash impact of executions and commissions (signed).
    pub cash_delta: UsdMoney,
    pub anomalies: Vec<AnomalyRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderAggregate {
    pub order: Order,
    pub side: Side,
    pub limit_price: Option<UsdMoney>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyRecord {
    pub sequence: SequenceNumber,
    pub order_id: Option<OrderId>,
    pub description: String,
}

/// Outcome classification for audit trails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    /// Duplicate ExecutionId — recorded, zero economic effect (INV-1).
    DuplicateIgnored,
    AnomalyQuarantined,
}

/// Apply one event. Errors leave the input state UNTOUCHED (caller
/// keeps the old value — we take &LedgerState and return a new one).
pub fn apply(
    state: &LedgerState,
    record: &EventRecord,
) -> Result<(LedgerState, ApplyOutcome), DomainError> {
    // Sequence contiguity: the journal's law, enforced here too.
    if let Some(prev) = state.last_sequence {
        if !record.sequence.is_successor_of(prev) {
            return Err(DomainError::SequenceGap {
                expected: prev.value() + 1,
                got: record.sequence.value(),
            });
        }
    } else if record.sequence != SequenceNumber::FIRST {
        return Err(DomainError::SequenceGap {
            expected: SequenceNumber::FIRST.value(),
            got: record.sequence.value(),
        });
    }

    let mut next = state.clone();
    next.last_sequence = Some(record.sequence);

    let outcome = match &record.payload {
        EventPayload::OrderCreated {
            order_id,
            side,
            quantity,
            limit_price,
        } => {
            if next.orders.contains_key(order_id) {
                return Err(DomainError::DuplicateOrderId {
                    order_id: order_id.as_str().to_owned(),
                });
            }
            next.orders.insert(
                order_id.clone(),
                OrderAggregate {
                    order: Order::new(order_id.clone(), *quantity),
                    side: *side,
                    limit_price: *limit_price,
                },
            );
            ApplyOutcome::Applied
        }

        EventPayload::OrderSubmitted { order_id } => {
            step(&mut next, record.sequence, order_id, OrderEvent::Submitted)?
        }
        EventPayload::OrderAcknowledged { order_id } => step(
            &mut next,
            record.sequence,
            order_id,
            OrderEvent::Acknowledged,
        )?,
        EventPayload::OrderRejected { order_id, .. } => {
            step(&mut next, record.sequence, order_id, OrderEvent::Rejected)?
        }
        EventPayload::CancelRequested { order_id } => step(
            &mut next,
            record.sequence,
            order_id,
            OrderEvent::CancelRequested,
        )?,
        EventPayload::CancelConfirmed { order_id } => step(
            &mut next,
            record.sequence,
            order_id,
            OrderEvent::CancelConfirmed,
        )?,

        EventPayload::ExecutionReceived {
            order_id,
            execution_id,
            quantity,
            price,
        } => {
            if next.seen_executions.contains(execution_id) {
                // INV-1: duplicate proof — seen, zero economic effect.
                ApplyOutcome::DuplicateIgnored
            } else {
                next.seen_executions.insert(execution_id.clone());
                let outcome = step(
                    &mut next,
                    record.sequence,
                    order_id,
                    OrderEvent::Execution {
                        quantity: *quantity,
                    },
                )?;
                // Economic effects (position, cash) apply exactly once.
                let side = next.orders.get(order_id).map(|a| a.side).ok_or_else(|| {
                    DomainError::UnknownOrderId {
                        order_id: order_id.as_str().to_owned(),
                    }
                })?;
                let signed_units = match side {
                    Side::Buy => quantity.as_units(),
                    Side::Sell => -quantity.as_units(),
                };
                next.net_position_units = next.net_position_units.checked_add(signed_units).ok_or(
                    DomainError::ArithmeticOverflow {
                        operation: "LedgerState::net_position_units",
                    },
                )?;
                let notional = price.checked_mul(quantity.as_units())?;
                // Buy consumes cash, sell produces it.
                next.cash_delta = match side {
                    Side::Buy => next.cash_delta.checked_sub(notional)?,
                    Side::Sell => next.cash_delta.checked_add(notional)?,
                };
                outcome
            }
        }

        EventPayload::CommissionReceived {
            execution_id,
            amount,
        } => {
            if !next.seen_executions.contains(execution_id) {
                // Proof referencing an unknown execution: preserved as
                // anomaly (corrected I4), not rejected.
                next.anomalies.push(AnomalyRecord {
                    sequence: record.sequence,
                    order_id: None,
                    description: format!(
                        "commission for unknown execution {}",
                        execution_id.as_str()
                    ),
                });
                ApplyOutcome::AnomalyQuarantined
            } else {
                next.cash_delta = next.cash_delta.checked_sub(*amount)?;
                ApplyOutcome::Applied
            }
        }
    };

    Ok((next, outcome))
}

/// Route an order-scoped event through the transition table, recording
/// anomalies in the ledger-level audit list.
fn step(
    state: &mut LedgerState,
    sequence: SequenceNumber,
    order_id: &OrderId,
    event: OrderEvent,
) -> Result<ApplyOutcome, DomainError> {
    let agg = state
        .orders
        .get(order_id)
        .ok_or_else(|| DomainError::UnknownOrderId {
            order_id: order_id.as_str().to_owned(),
        })?;
    let (new_order, class) = transition_order(&agg.order, event)?;
    let outcome = match class {
        TransitionClass::Ordinary => ApplyOutcome::Applied,
        TransitionClass::Anomalous(kind) => {
            state.anomalies.push(AnomalyRecord {
                sequence,
                order_id: Some(order_id.clone()),
                description: format!("{kind:?}"),
            });
            ApplyOutcome::AnomalyQuarantined
        }
    };
    if let Some(a) = state.orders.get_mut(order_id) {
        a.order = new_order;
    }
    Ok(outcome)
}

/// Fold a whole journal. Purity ⇒ determinism (INV-2).
pub fn replay<'a>(
    events: impl IntoIterator<Item = &'a EventRecord>,
) -> Result<LedgerState, DomainError> {
    let mut state = LedgerState::default();
    for record in events {
        let (next, _) = apply(&state, record)?;
        state = next;
    }
    Ok(state)
}

#[allow(clippy::used_underscore_binding)]
const _: fn(OrderQuantity) -> i64 = OrderQuantity::as_units; // keep import honest
