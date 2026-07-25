//! SecondBrain vault: on-disk workspace layout and versioned manifest.
//!
//! This crate owns the `.secondbrain/` internal directory and the
//! `manifest.toml` file that describes a workspace. It deliberately never
//! touches user Markdown content; initialization only creates internal state.

#![forbid(unsafe_code)]

pub mod atomic_write;
pub mod base_snapshot;
pub mod event;
pub mod identity_map;
pub mod manifest;
pub mod root;
pub mod watcher;

pub use base_snapshot::{BaseSnapshot, BaseSnapshotStore, SnapshotError};
pub use identity_map::{FingerprintRecord, IdentityMap, IdentityRecord, RecoveryOutcome};
pub use manifest::{
    SUPPORTED_FORMAT_VERSION, WorkspaceManifest, initialize_workspace, load_manifest,
};
pub use root::WorkspaceRoot;
