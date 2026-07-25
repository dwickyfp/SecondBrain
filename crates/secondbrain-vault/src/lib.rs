//! SecondBrain vault: on-disk workspace layout and versioned manifest.
//!
//! This crate owns the `.secondbrain/` internal directory and the
//! `manifest.toml` file that describes a workspace. It deliberately never
//! touches user Markdown content; initialization only creates internal state.

#![forbid(unsafe_code)]

pub mod atomic_write;
pub mod manifest;
pub mod root;

pub use manifest::{
    SUPPORTED_FORMAT_VERSION, WorkspaceManifest, initialize_workspace, load_manifest,
};
pub use root::WorkspaceRoot;
