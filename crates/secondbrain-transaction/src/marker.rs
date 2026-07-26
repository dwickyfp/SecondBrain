//! The durable per-transaction marker, stated once.
//!
//! A marker under `.secondbrain/transactions/<id>.json` is written by
//! [`crate::engine`] and read — and rewritten — by [`crate::recovery`]. Both
//! sides must agree on every field, because recovery rewrites a marker by
//! re-serializing the value it deserialized: a field the writer records and the
//! reader does not know about would be *erased* from the marker the first time
//! recovery touched it, silently discarding the very information the field was
//! added to preserve.
//!
//! Stating the schema twice made that a matter of remembering to edit both
//! sides in lockstep. It is therefore stated once, here, for the same reason
//! [`crate::paths`] states the marker filename convention once.

use std::path::Path;

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId};
use secondbrain_core::path::WorkspacePath;
use serde::{Deserialize, Serialize};

use crate::engine::TransactionError;

/// The durable state of one transaction.
///
/// Field order is the marker's on-disk key order; serde follows declaration
/// order, so reordering these rewrites every marker's bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DurableState {
    pub(crate) transaction_id: TransactionId,
    pub(crate) note_id: NoteId,
    pub(crate) path: WorkspacePath,
    pub(crate) state: String,
    pub(crate) expected_hash: ContentHash,
    /// Hash of the content this transaction's operations produce, recorded
    /// before the Markdown was written, so recovery can recognize its own
    /// result on disk by comparison rather than by re-applying operations that
    /// may not be idempotent.
    ///
    /// Required. Phase 0 has released no workspaces and offers no migration
    /// path, so a marker without this field is from a build that no longer
    /// exists; refusing it loudly is honest, whereas defaulting it to "absent"
    /// would half-support a format nobody can produce.
    pub(crate) materialized_hash: ContentHash,
    pub(crate) expected_version: NoteVersion,
    pub(crate) committed_version: NoteVersion,
    pub(crate) index_repaired: bool,
    #[serde(default)]
    pub(crate) creates_note: bool,
}

impl DurableState {
    /// Writes this marker atomically over `path`.
    pub(crate) fn persist(&self, path: &Path) -> Result<(), TransactionError> {
        let bytes = serde_json::to_vec_pretty(self)?;
        secondbrain_vault::atomic_write::atomic_write(path, &bytes)?;
        Ok(())
    }
}
