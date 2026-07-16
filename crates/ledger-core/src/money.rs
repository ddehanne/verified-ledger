//! USD-only exact money representation (frozen V1 decision).
//!
//! - minor units (cents) as `i64` — exact, no floats anywhere (INV-5)
//! - all arithmetic is checked: overflow returns a typed error, never
//!   panics and never wraps (INV-8, INV-10)
//! - no `Deref` to raw i64: amounts leave the type only through
//!   explicit, named accessors

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// An exact USD amount in minor units (cents). May be negative
/// (e.g. commissions, corrections, short proceeds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UsdMoney {
    minor_units: i64,
}

impl UsdMoney {
    pub const ZERO: UsdMoney = UsdMoney { minor_units: 0 };

    /// Construct from minor units (cents). Total function — every i64
    /// is a representable amount.
    pub const fn from_minor_units(minor_units: i64) -> Self {
        Self { minor_units }
    }

    /// Explicit, named exit point — no silent coercion to i64.
    pub const fn as_minor_units(self) -> i64 {
        self.minor_units
    }

    pub fn checked_add(self, rhs: UsdMoney) -> Result<UsdMoney, DomainError> {
        self.minor_units
            .checked_add(rhs.minor_units)
            .map(UsdMoney::from_minor_units)
            .ok_or(DomainError::ArithmeticOverflow {
                operation: "UsdMoney::checked_add",
            })
    }

    pub fn checked_sub(self, rhs: UsdMoney) -> Result<UsdMoney, DomainError> {
        self.minor_units
            .checked_sub(rhs.minor_units)
            .map(UsdMoney::from_minor_units)
            .ok_or(DomainError::ArithmeticOverflow {
                operation: "UsdMoney::checked_sub",
            })
    }

    /// Scale by an integer factor (e.g. price-per-unit × units).
    pub fn checked_mul(self, factor: i64) -> Result<UsdMoney, DomainError> {
        self.minor_units
            .checked_mul(factor)
            .map(UsdMoney::from_minor_units)
            .ok_or(DomainError::ArithmeticOverflow {
                operation: "UsdMoney::checked_mul",
            })
    }

    pub fn checked_neg(self) -> Result<UsdMoney, DomainError> {
        self.minor_units
            .checked_neg()
            .map(UsdMoney::from_minor_units)
            .ok_or(DomainError::ArithmeticOverflow {
                operation: "UsdMoney::checked_neg",
            })
    }

    pub const fn is_negative(self) -> bool {
        self.minor_units < 0
    }
}

impl core::fmt::Display for UsdMoney {
    /// Render as dollars with sign and two decimals: `-12.34 USD`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let sign = if self.minor_units < 0 { "-" } else { "" };
        let abs = self.minor_units.unsigned_abs();
        write!(f, "{sign}{}.{:02} USD", abs / 100, abs % 100)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // tests must panic on broken setup
    use super::*;

    #[test]
    fn add_is_exact() {
        let a = UsdMoney::from_minor_units(10_050); // 100.50
        let b = UsdMoney::from_minor_units(25); //   0.25
        assert_eq!(a.checked_add(b), Ok(UsdMoney::from_minor_units(10_075)));
    }

    #[test]
    fn overflow_is_typed_error_not_panic() {
        let max = UsdMoney::from_minor_units(i64::MAX);
        let one = UsdMoney::from_minor_units(1);
        assert_eq!(
            max.checked_add(one),
            Err(DomainError::ArithmeticOverflow {
                operation: "UsdMoney::checked_add"
            })
        );
    }

    #[test]
    fn neg_of_i64_min_is_error() {
        let m = UsdMoney::from_minor_units(i64::MIN);
        assert!(m.checked_neg().is_err());
    }

    #[test]
    fn display_formats_cents_and_sign() {
        assert_eq!(UsdMoney::from_minor_units(-12_34).to_string(), "-12.34 USD");
        assert_eq!(UsdMoney::from_minor_units(5).to_string(), "0.05 USD");
    }
}
