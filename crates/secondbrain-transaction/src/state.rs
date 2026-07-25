//! Validated durable transaction states.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Durable phases of a single-note transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransactionState {
    /// Preconditions were validated and the transaction was recorded.
    Prepared,
    /// All semantic operations are durable in the note oplog.
    OperationsDurable,
    /// Markdown materialization is in progress.
    Materializing,
    /// Markdown and the final transaction marker are durable.
    Committed,
}

impl TransactionState {
    /// Advance to the next legal state.
    pub fn transition_to(&mut self, next: Self) -> Result<(), StateTransitionError> {
        let valid = matches!(
            (*self, next),
            (Self::Prepared, Self::OperationsDurable)
                | (Self::OperationsDurable, Self::Materializing)
                | (Self::Materializing, Self::Committed)
        );
        if !valid {
            return Err(StateTransitionError {
                current: *self,
                requested: next,
            });
        }
        *self = next;
        Ok(())
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Prepared => "PREPARED",
            Self::OperationsDurable => "OPERATIONS_DURABLE",
            Self::Materializing => "MATERIALIZING",
            Self::Committed => "COMMITTED",
        }
    }
}

/// An attempted transaction state transition was not legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error(
    "illegal transaction transition from {} to {}",
    current.label(),
    requested.label()
)]
pub struct StateTransitionError {
    /// Current state.
    pub current: TransactionState,
    /// Requested state.
    pub requested: TransactionState,
}
