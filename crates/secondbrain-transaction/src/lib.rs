#![forbid(unsafe_code)]

//! SecondBrain Transaction: versioned local mutation journal records.
//!
//! Phase 0 durability layer with corruption detection. Provides deterministic
//! encoding of local operation records for the pre-CRDT recovery plumbing.
//! This format is **not** a network envelope — it is for local persistence only.

pub mod record;

pub use record::{
    FORMAT_VERSION_1, LocalOperationRecord, RecordDecodeError, RecordEncodeError, RecordFormat,
};
