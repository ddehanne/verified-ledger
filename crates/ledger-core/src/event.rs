//! Canonical events — the ONLY inputs the reducer accepts.
//!
//! Envelope vs payload:
//! - the envelope carries identity, ordering authority (sequence),
//!   schema version and both timestamps (event_time = when the world
//!   says it happened; received_time = when we learned it). Both are
//!   EVIDENCE — ordering authority is the sequence alone.
//! - the payload is a closed enum: unknown event types cannot exist
//!   in memory (they fail at deserialization, at the boundary).

use serde::{Deserialize, Serialize};

use crate::ids::{EventId, ExecutionId, OrderId, SequenceNumber};
use crate::money::UsdMoney;
use crate::quantity::{FilledQuantity, OrderQuantity};

/// Unix microseconds, UTC. External evidence, not ordering authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimestampMicros(pub i64);

/// Side of an order. Closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

/// Schema version of the payload encoding (versioned enum planned; V1 only).
pub const SCHEMA_VERSION_V1: u16 = 1;

/// The event envelope — one journal record = one `EventRecord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub event_id: EventId,
    pub sequence: SequenceNumber,
    pub schema_version: u16,
    pub event_time: TimestampMicros,
    pub received_time: TimestampMicros,
    pub payload: EventPayload,
}

/// Canonical payloads (frozen V1 minimal set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventPayload {
    OrderCreated {
        order_id: OrderId,
        side: Side,
        quantity: OrderQuantity,
        limit_price: Option<UsdMoney>,
    },
    OrderSubmitted {
        order_id: OrderId,
    },
    OrderAcknowledged {
        order_id: OrderId,
    },
    OrderRejected {
        order_id: OrderId,
        reason: String,
    },
    /// Partial or full execution evidence from the broker.
    /// Dedup key: `execution_id` (INV-1 — no double economic effect).
    ExecutionReceived {
        order_id: OrderId,
        execution_id: ExecutionId,
        quantity: FilledQuantity,
        price: UsdMoney,
    },
    CommissionReceived {
        execution_id: ExecutionId,
        amount: UsdMoney,
    },
    CancelRequested {
        order_id: OrderId,
    },
    CancelConfirmed {
        order_id: OrderId,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn sample() -> EventRecord {
        EventRecord {
            event_id: EventId::new("evt-0001").unwrap(),
            sequence: SequenceNumber::FIRST,
            schema_version: SCHEMA_VERSION_V1,
            event_time: TimestampMicros(1_752_650_000_000_000),
            received_time: TimestampMicros(1_752_650_000_150_000),
            payload: EventPayload::ExecutionReceived {
                order_id: OrderId::new("ord-7").unwrap(),
                execution_id: ExecutionId::new("exec-42").unwrap(),
                quantity: FilledQuantity::new(3).unwrap(),
                price: UsdMoney::from_minor_units(15_000),
            },
        }
    }

    #[test]
    fn serde_round_trip_is_identity() {
        let e = sample();
        let json = serde_json::to_string(&e).unwrap();
        let back: EventRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn unknown_event_type_fails_at_boundary() {
        let bad = r#"{"event_id":"e","sequence":1,"schema_version":1,
            "event_time":0,"received_time":0,
            "payload":{"TotallyUnknownEvent":{}}}"#;
        assert!(serde_json::from_str::<EventRecord>(bad).is_err());
    }
}
