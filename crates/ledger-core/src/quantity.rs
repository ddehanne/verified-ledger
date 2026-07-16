//! Quantity newtypes — ordered vs filled are DISTINCT types.
//!
//! Frozen-spec rationale: an overfill (filled > ordered) must be
//! REPRESENTABLE (it is an external fact reported by the broker) and
//! classified as an integrity conflict downstream — never made
//! impossible by construction, never a panic (I1 corrected).
//!
//! Keeping the two quantities as distinct types forces every
//! comparison between them through one explicit, named function.

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Quantity the operator asked for. Always strictly positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OrderQuantity(i64);

impl OrderQuantity {
    /// Validated constructor — an order for zero or negative units is
    /// not a valid intent.
    pub fn new(units: i64) -> Result<Self, DomainError> {
        if units <= 0 {
            return Err(DomainError::InvalidQuantity {
                reason: "order quantity must be strictly positive",
            });
        }
        Ok(Self(units))
    }

    pub const fn as_units(self) -> i64 {
        self.0
    }
}

/// Quantity actually executed, as evidenced by broker reports.
/// Non-negative; may legitimately EXCEED the ordered quantity
/// (overfill — an external fact we must be able to record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FilledQuantity(i64);

impl FilledQuantity {
    pub const ZERO: FilledQuantity = FilledQuantity(0);

    /// Validated constructor — a negative fill is not a fill.
    pub fn new(units: i64) -> Result<Self, DomainError> {
        if units < 0 {
            return Err(DomainError::InvalidQuantity {
                reason: "filled quantity cannot be negative",
            });
        }
        Ok(Self(units))
    }

    pub const fn as_units(self) -> i64 {
        self.0
    }

    /// Accumulate a new partial fill. Checked — overflow is a typed error.
    pub fn checked_add(self, more: FilledQuantity) -> Result<FilledQuantity, DomainError> {
        self.0
            .checked_add(more.0)
            .map(FilledQuantity)
            .ok_or(DomainError::ArithmeticOverflow {
                operation: "FilledQuantity::checked_add",
            })
    }
}

/// The ONLY sanctioned comparison between the two quantity kinds.
/// Names the three economically distinct outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillStatus {
    Partial,
    Exact,
    /// filled > ordered — representable, flagged for integrity review.
    Overfill,
}

pub fn fill_status(ordered: OrderQuantity, filled: FilledQuantity) -> FillStatus {
    match filled.as_units().cmp(&ordered.as_units()) {
        core::cmp::Ordering::Less => FillStatus::Partial,
        core::cmp::Ordering::Equal => FillStatus::Exact,
        core::cmp::Ordering::Greater => FillStatus::Overfill,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // tests must panic on broken setup
    use super::*;

    #[test]
    fn order_quantity_rejects_non_positive() {
        assert!(OrderQuantity::new(0).is_err());
        assert!(OrderQuantity::new(-5).is_err());
        assert!(OrderQuantity::new(1).is_ok());
    }

    #[test]
    fn filled_quantity_rejects_negative_allows_zero() {
        assert!(FilledQuantity::new(-1).is_err());
        assert_eq!(FilledQuantity::new(0), Ok(FilledQuantity::ZERO));
    }

    #[test]
    fn overfill_is_representable_and_named() {
        let ordered = OrderQuantity::new(100).unwrap();
        let filled = FilledQuantity::new(150).unwrap();
        assert_eq!(fill_status(ordered, filled), FillStatus::Overfill);
    }

    #[test]
    fn fill_accumulation_is_checked() {
        let a = FilledQuantity::new(i64::MAX).unwrap();
        let b = FilledQuantity::new(1).unwrap();
        assert!(a.checked_add(b).is_err());
    }
}
