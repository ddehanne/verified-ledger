//! Identity newtypes — VALIDATED DATA, never generated here.
//!
//! LAW-01: the core has no RNG, no clock, no ambient identity source.
//! IDs arrive from outside (broker, caller) and are validated at the
//! boundary. Empty or oversized identifiers are rejected, not fixed.

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

const MAX_ID_LEN: usize = 64;

macro_rules! string_id {
    ($name:ident, $what:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(raw: impl Into<String>) -> Result<Self, DomainError> {
                let s: String = raw.into();
                if s.is_empty() {
                    return Err(DomainError::InvalidId {
                        kind: $what,
                        reason: "empty",
                    });
                }
                if s.len() > MAX_ID_LEN {
                    return Err(DomainError::InvalidId {
                        kind: $what,
                        reason: "exceeds max length",
                    });
                }
                Ok(Self(s))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(OrderId, "order_id");
string_id!(ExecutionId, "execution_id");
string_id!(EventId, "event_id");

/// Journal sequence number — THE authority for event ordering.
/// Timestamps are external evidence; the contiguous sequence is law
/// (corrected I3): `current == previous + 1`, verified at read time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const FIRST: SequenceNumber = SequenceNumber(1);

    pub const fn new(n: u64) -> Self {
        Self(n)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    /// The only legal successor. Checked — u64 exhaustion is an error,
    /// not a wrap.
    pub fn next(self) -> Result<SequenceNumber, DomainError> {
        self.0
            .checked_add(1)
            .map(SequenceNumber)
            .ok_or(DomainError::ArithmeticOverflow {
                operation: "SequenceNumber::next",
            })
    }

    pub const fn is_successor_of(self, prev: SequenceNumber) -> bool {
        // Written without +1 on prev to avoid overflow on u64::MAX.
        self.0 > prev.0 && self.0 - prev.0 == 1
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // tests must panic on broken setup
    use super::*;

    #[test]
    fn ids_reject_empty_and_oversized() {
        assert!(OrderId::new("").is_err());
        assert!(ExecutionId::new("x".repeat(65)).is_err());
        assert!(EventId::new("evt-1").is_ok());
    }

    #[test]
    fn sequence_contiguity_is_the_law() {
        let a = SequenceNumber::new(41);
        let b = a.next().unwrap();
        assert!(b.is_successor_of(a));
        assert!(!SequenceNumber::new(43).is_successor_of(a)); // gap
        assert!(!SequenceNumber::new(41).is_successor_of(a)); // duplicate
    }

    #[test]
    fn sequence_exhaustion_is_typed_error() {
        assert!(SequenceNumber::new(u64::MAX).next().is_err());
    }
}
