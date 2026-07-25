//! Deterministic recovery for interrupted single-note transactions.

use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::apply::apply_operations;
use serde::{Deserialize, Serialize};

use crate::base_snapshot::BaseSnapshotStore;
use crate::engine::{TransactionEngine, TransactionError};
use crate::oplog::LocalMutationLog;
use crate::paths;

/// A durable repair performed while recovering interrupted transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// The derived index must be refreshed for a committed note.
    IndexRepair {
        transaction_id: TransactionId,
        note_id: NoteId,
        path: WorkspacePath,
    },
    /// An invalid journal suffix was preserved and the transaction was aborted.
    Quarantined {
        transaction_id: TransactionId,
        note_id: NoteId,
        path: PathBuf,
    },
    /// A durably journaled edit could not be replayed and was abandoned.
    ///
    /// The file on disk is preserved and the marker aborted, so the edit those
    /// operations describe is lost. "Confirmed edits survive process and power
    /// failure" is the invariant this layer exists for, so the loss is
    /// reported rather than left to be inferred from a marker nobody reads.
    /// The journal records themselves stay intact for whoever investigates.
    Abandoned {
        transaction_id: TransactionId,
        note_id: NoteId,
        path: WorkspacePath,
        reason: AbandonedReason,
    },
}

/// Why recovery could not replay a durably journaled edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbandonedReason {
    /// The operations no longer anchor in the file: the text they name was
    /// rewritten by an external editor or carried forward by another
    /// transaction.
    OperationsDoNotAnchor,
    /// The operations still anchor, but the file is neither the state they
    /// were derived from nor the state they produce, so replaying them would
    /// overwrite content no marker accounts for.
    UnrecognizedFileState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableState {
    transaction_id: TransactionId,
    note_id: NoteId,
    path: WorkspacePath,
    state: String,
    expected_hash: ContentHash,
    /// Hash of the content this transaction's operations produce, recorded
    /// before the Markdown was written.
    ///
    /// Absent only on markers written before the engine recorded it, which are
    /// recovered by the pre-state and idempotence rules alone.
    #[serde(default)]
    materialized_hash: Option<ContentHash>,
    expected_version: NoteVersion,
    committed_version: NoteVersion,
    #[serde(default)]
    index_repaired: bool,
}

impl TransactionEngine {
    /// Recover all durable transaction markers in deterministic filename order.
    pub fn recover(&self) -> Result<Vec<RecoveryAction>, TransactionError> {
        let mut actions = Vec::new();
        for marker_path in self.marker_paths()? {
            let mut marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
            if marker.state == "ABORTED" || (marker.state == "COMMITTED" && marker.index_repaired) {
                continue;
            }

            if marker.state == "COMMITTED" {
                actions.push(index_repair(&marker));
                marker.index_repaired = true;
                persist_marker(&marker_path, &marker)?;
                continue;
            }

            let log = LocalMutationLog::open(self.workspace.canonical_path(), marker.note_id)?;
            let replay = log.replay()?;
            if let Some(corruption) = replay.corruption {
                marker.state = "ABORTED".to_owned();
                persist_marker(&marker_path, &marker)?;
                actions.push(RecoveryAction::Quarantined {
                    transaction_id: marker.transaction_id,
                    note_id: marker.note_id,
                    path: corruption.quarantine_path,
                });
                continue;
            }

            let operations = replay
                .records
                .iter()
                .filter(|record| record.transaction_id == marker.transaction_id)
                .map(|record| record.operation.clone())
                .collect::<Vec<_>>();
            if operations.is_empty() {
                marker.state = "ABORTED".to_owned();
                persist_marker(&marker_path, &marker)?;
                continue;
            }

            let note_path = self.workspace.resolve(&marker.path)?;
            let source = fs::read_to_string(&note_path)?;
            let source_hash = ContentHash::digest(source.as_bytes());
            let materialized = if marker.materialized_hash == Some(source_hash) {
                // The file is this transaction's own result, identified by the
                // hash its marker recorded before the Markdown write. That is
                // a positive test: re-applying the operations to decide the
                // same thing only works for operations that happen to be
                // idempotent, and a `ReplaceNode` anchors on the text it
                // replaces, which its own post-state no longer contains.
                source.clone()
            } else if source_hash == marker.expected_hash {
                // The file is the recorded pre-state: finish the write.
                let Ok(materialized) = apply_operations(&source, &operations) else {
                    actions.push(abandon(
                        &mut marker,
                        &marker_path,
                        AbandonedReason::OperationsDoNotAnchor,
                    )?);
                    continue;
                };
                marker.state = "MATERIALIZING".to_owned();
                persist_marker(&marker_path, &marker)?;
                self.workspace
                    .atomic_write(&marker.path, materialized.as_bytes())?;
                materialized
            } else {
                // Neither state the marker names. The operations may still be
                // idempotent against the file — a marker predating
                // `materialized_hash` is recovered that way — but anything
                // else is newer data, preserved rather than overwritten.
                let Ok(materialized) = apply_operations(&source, &operations) else {
                    // Their anchors are gone, because another transaction
                    // carried them forward or rewrote the same text. That is
                    // one transaction's problem: it is aborted, the file is
                    // preserved, and the remaining notes are still recovered.
                    actions.push(abandon(
                        &mut marker,
                        &marker_path,
                        AbandonedReason::OperationsDoNotAnchor,
                    )?);
                    continue;
                };
                if materialized != source {
                    actions.push(abandon(
                        &mut marker,
                        &marker_path,
                        AbandonedReason::UnrecognizedFileState,
                    )?);
                    continue;
                }
                materialized
            };

            marker.state = "COMMITTED".to_owned();
            marker.index_repaired = false;
            persist_marker(&marker_path, &marker)?;
            // Recovery converged the note, so it owes the converged base the
            // same update a successful commit performs.
            BaseSnapshotStore::new(&self.workspace).save(
                marker.note_id,
                &marker.path,
                marker.committed_version,
                &materialized,
            )?;
            actions.push(index_repair(&marker));
            marker.index_repaired = true;
            persist_marker(&marker_path, &marker)?;
        }
        Ok(actions)
    }

    /// Transactions of one note whose operations are journaled but whose
    /// Markdown was never materialized, in deterministic marker order.
    pub(crate) fn pending_transactions(
        &self,
        note_id: NoteId,
    ) -> Result<Vec<TransactionId>, TransactionError> {
        let mut pending = Vec::new();
        for marker_path in self.marker_paths()? {
            let marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
            if marker.note_id == note_id && marker.state != "COMMITTED" && marker.state != "ABORTED"
            {
                pending.push(marker.transaction_id);
            }
        }
        Ok(pending)
    }

    /// Closes out a journaled-but-unmaterialized transaction whose operations
    /// another transaction has carried forward.
    ///
    /// The marker is aborted so recovery never replays operations that are
    /// already reflected in the file; the journal records themselves stay
    /// intact for audit.
    pub(crate) fn supersede_transaction(
        &self,
        transaction_id: TransactionId,
    ) -> Result<(), TransactionError> {
        let marker_path = paths::marker_path(self.workspace.canonical_path(), transaction_id);
        let mut marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
        marker.state = "ABORTED".to_owned();
        persist_marker(&marker_path, &marker)
    }

    /// Every transaction marker in the workspace, in deterministic order.
    ///
    /// The directory also holds review descriptors, which are not markers and
    /// must not be parsed as one; [`crate::paths::is_marker`] owns that rule.
    fn marker_paths(&self) -> Result<Vec<PathBuf>, TransactionError> {
        let directory = paths::transactions_dir(self.workspace.canonical_path());
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut markers = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if paths::is_marker(&path) {
                markers.push(path);
            }
        }
        markers.sort();
        Ok(markers)
    }
}

fn index_repair(marker: &DurableState) -> RecoveryAction {
    RecoveryAction::IndexRepair {
        transaction_id: marker.transaction_id,
        note_id: marker.note_id,
        path: marker.path.clone(),
    }
}

/// Aborts a marker whose journaled operations recovery cannot replay, and
/// reports the edit that was given up on.
fn abandon(
    marker: &mut DurableState,
    marker_path: &Path,
    reason: AbandonedReason,
) -> Result<RecoveryAction, TransactionError> {
    marker.state = "ABORTED".to_owned();
    persist_marker(marker_path, marker)?;
    Ok(RecoveryAction::Abandoned {
        transaction_id: marker.transaction_id,
        note_id: marker.note_id,
        path: marker.path.clone(),
        reason,
    })
}

fn persist_marker(path: &Path, marker: &DurableState) -> Result<(), TransactionError> {
    let bytes = serde_json::to_vec_pretty(marker)?;
    secondbrain_vault::atomic_write::atomic_write(path, &bytes)?;
    Ok(())
}
