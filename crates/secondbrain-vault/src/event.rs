//! Deterministic workspace-level filesystem events.

use secondbrain_core::hash::ContentHash;
use secondbrain_core::path::WorkspacePath;

/// A filesystem change confined to a workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceEvent {
    /// File bytes changed (including creation and recreation).
    ContentChanged {
        /// Changed path.
        path: WorkspacePath,
        /// Hash of the bytes observed after the event burst.
        hash: ContentHash,
    },
    /// A file was removed.
    Deleted {
        /// Removed path.
        path: WorkspacePath,
    },
    /// A file moved within the workspace.
    Renamed {
        /// Previous path.
        from: WorkspacePath,
        /// New path.
        to: WorkspacePath,
    },
}
