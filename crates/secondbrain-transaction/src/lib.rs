#![forbid(unsafe_code)]

//! SecondBrain Transaction: versioned local mutation journal records.
//!
//! Phase 0 durability layer with corruption detection. Provides deterministic
//! encoding of local operation records for the pre-CRDT recovery plumbing.
//! This format is **not** a network envelope — it is for local persistence only.

pub mod base_snapshot;
pub mod engine;
pub mod external_edit;
pub mod failpoint;
pub mod oplog;
mod paths;
pub mod record;
pub mod recovery;
pub mod state;

pub use base_snapshot::{BaseSnapshot, BaseSnapshotStore, SnapshotError};
pub use engine::{CommitOutcome, TransactionEngine, TransactionError, TransactionRequest};
pub use external_edit::{
    ExternalEditCoordinator, ExternalEditError, ExternalEditOutcome, IndexRefresh,
};
pub use recovery::RecoveryAction;
pub use state::{StateTransitionError, TransactionState};

pub use record::{
    FORMAT_VERSION_1, LocalOperationRecord, RecordDecodeError, RecordEncodeError, RecordFormat,
};
