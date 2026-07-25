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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableState {
    transaction_id: TransactionId,
    note_id: NoteId,
    path: WorkspacePath,
    state: String,
    expected_hash: ContentHash,
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
            let materialized = apply_operations(&source, &operations)?;
            if ContentHash::digest(source.as_bytes()) == marker.expected_hash {
                marker.state = "MATERIALIZING".to_owned();
                persist_marker(&marker_path, &marker)?;
                self.workspace
                    .atomic_write(&marker.path, materialized.as_bytes())?;
            } else if materialized != source {
                // The file is neither the recorded pre-state nor an idempotent
                // post-state. Preserve it rather than overwriting newer data.
                marker.state = "ABORTED".to_owned();
                persist_marker(&marker_path, &marker)?;
                continue;
            }

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
        let marker_path = self
            .workspace
            .canonical_path()
            .join(".secondbrain/transactions")
            .join(format!("{transaction_id}.json"));
        let mut marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
        marker.state = "ABORTED".to_owned();
        persist_marker(&marker_path, &marker)
    }

    /// Every transaction marker in the workspace, in deterministic order.
    ///
    /// A marker is named after its transaction, so anything else in the
    /// directory — a `<id>.conflict.json` review descriptor, for instance — is
    /// not a marker and must not be parsed as one.
    fn marker_paths(&self) -> Result<Vec<PathBuf>, TransactionError> {
        let directory = self
            .workspace
            .canonical_path()
            .join(".secondbrain/transactions");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut markers = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let is_marker = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.parse::<TransactionId>().is_ok());
            if is_marker {
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

fn persist_marker(path: &Path, marker: &DurableState) -> Result<(), TransactionError> {
    let bytes = serde_json::to_vec_pretty(marker)?;
    secondbrain_vault::atomic_write::atomic_write(path, &bytes)?;
    Ok(())
}
