//! Typed error taxonomy for the ledger core.
//!
//! Contract rules:
//! - external/invalid input must NEVER panic (INV-8)
//! - every meaningful failure is a typed variant (INV-10)
//! - no `unwrap`/`expect` in library code (workspace clippy deny)

use thiserror::Error;

/// Domain-level failures produced by core types and the reducer.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum DomainError {
    /// Checked arithmetic on money or quantities overflowed i64.
    #[error("arithmetic overflow in {operation}")]
    ArithmeticOverflow {
        /// Which operation overflowed (e.g. "UsdMoney::checked_add").
        operation: &'static str,
    },

    /// V1 is USD-only by frozen spec; any other currency is rejected.
    #[error("currency unsupported in V1: {currency}")]
    CurrencyUnsupported { currency: String },

    /// A quantity failed validation (e.g. non-positive order quantity).
    #[error("invalid quantity: {reason}")]
    InvalidQuantity { reason: &'static str },

    /// A monetary amount failed validation.
    #[error("invalid money amount: {reason}")]
    InvalidMoney { reason: &'static str },
}
