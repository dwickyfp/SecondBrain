//! Deterministic recovery for interrupted single-note transactions.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, TransactionId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::apply::apply_operations;
use secondbrain_vault::base_snapshot::BaseSnapshotStore;

use crate::engine::{TransactionEngine, TransactionError};
use crate::marker::DurableState;
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
    ///
    /// `path` is the note, as it is for every other variant, so a formatter can
    /// answer "which note did this happen to" without special-casing. The
    /// preserved journal suffix lives inside `.secondbrain/` and is not a note
    /// path at all, so it is named for what it is.
    Quarantined {
        transaction_id: TransactionId,
        note_id: NoteId,
        path: WorkspacePath,
        quarantine_path: PathBuf,
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

impl fmt::Display for AbandonedReason {
    /// Says what happened and what it means for the operator's data, because
    /// this is what a `recovery check` prints to a human.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OperationsDoNotAnchor => {
                "the text this edit was written against is no longer in the note, \
                 so the edit could not be replayed; the file on disk is untouched \
                 and the edit is lost"
            }
            Self::UnrecognizedFileState => {
                "the note holds neither the content this edit started from nor the \
                 content it would produce, so replaying it would have overwritten \
                 changes nothing accounts for; the file on disk is untouched and \
                 the edit is lost"
            }
        })
    }
}

/// A read-only view of one durable transaction marker.
///
/// Enough to answer "what is this workspace's transaction state" without
/// exposing the marker schema itself, which only [`crate::engine`] and
/// [`crate::recovery`] may write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionSummary {
    /// The transaction this marker belongs to.
    pub transaction_id: TransactionId,
    /// The note the transaction edits.
    pub note_id: NoteId,
    /// The note's path at the time the marker was written.
    pub path: WorkspacePath,
    /// The durable state label exactly as the marker holds it, one of
    /// `PREPARED`, `OPERATIONS_DURABLE`, `MATERIALIZING`, `COMMITTED`, or
    /// `ABORTED` — carried for diagnostics that display it verbatim.
    ///
    /// Callers deciding anything ask the predicates below instead. The labels
    /// are this crate's vocabulary, and a caller comparing the string would be
    /// keeping a second copy of it that nothing stops from drifting.
    pub state: String,
    /// Whether the derived index has been refreshed for this transaction.
    pub index_repaired: bool,
}

/// One change still waiting on a person to decide what it meant.
///
/// Named by the transaction it is filed under rather than by the file it lives
/// in, for the reason [`TransactionSummary`] exists: where these records sit
/// and what they hold is this crate's to know, and handing a caller a
/// filesystem path would make the layout — and the descriptor schema behind it
/// — part of the API that only the coordinator may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingReview {
    /// The transaction the review is filed under.
    pub transaction_id: TransactionId,
}

impl TransactionSummary {
    /// Whether this transaction reached its commit marker.
    #[must_use]
    pub fn committed(&self) -> bool {
        self.state == "COMMITTED"
    }

    /// Whether this transaction was aborted.
    #[must_use]
    pub fn aborted(&self) -> bool {
        self.state == "ABORTED"
    }

    /// Whether this transaction has reached a state recovery will not revisit.
    ///
    /// A label this build does not recognize is deliberately not terminal:
    /// unfinished work is the safe reading, and it keeps the transaction
    /// visible to whoever has to look at it.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.committed() || self.aborted()
    }

    /// Whether this transaction is committed but its index repair is still owed.
    #[must_use]
    pub fn awaits_index_repair(&self) -> bool {
        self.committed() && !self.index_repaired
    }
}

impl TransactionEngine {
    /// Recover all durable transaction markers in deterministic filename order.
    ///
    /// A returned [`RecoveryAction::IndexRepair`] is a request, not a receipt:
    /// this engine holds no index and cannot perform one. The caller that does
    /// perform it calls [`Self::record_index_refreshed`] afterwards, and a
    /// caller whose refresh failed calls nothing — so the repair is asked for
    /// again on the next pass instead of being marked done by a marker that
    /// lies. Recovery is therefore idempotent in that form, and only that form:
    /// re-running it before the caller records the repair asks again, which is
    /// the correct answer while the index is still stale.
    pub fn recover(&self) -> Result<Vec<RecoveryAction>, TransactionError> {
        let mut actions = Vec::new();
        for marker_path in self.marker_paths()? {
            let mut marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
            if marker.state == "ABORTED" || (marker.state == "COMMITTED" && marker.index_repaired) {
                continue;
            }

            if marker.state == "COMMITTED" {
                // Nothing to persist: the marker already says what is true, and
                // it keeps saying it until whoever refreshed the index says
                // otherwise.
                actions.push(index_repair(&marker));
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
                    path: marker.path.clone(),
                    quarantine_path: corruption.quarantine_path,
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
            let materialized = if marker.materialized_hash == source_hash {
                // The file is this transaction's own result, identified by the
                // hash its marker recorded before the Markdown write. That is
                // the *only* positive test: re-applying the operations to
                // decide the same thing only works for operations that happen
                // to be idempotent, and a `ReplaceNode` anchors on the text it
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
                // Neither this transaction's own result nor the pre-state it
                // recorded. Re-applying the operations here would decide the
                // question by idempotence, which can agree with content this
                // transaction never produced — committing over a third party
                // and adopting their bytes as this note's converged base. The
                // recorded hashes are the only evidence about what this
                // transaction wrote, and they both say no.
                //
                // Replaying is still worth doing to tell the operator *why*
                // the edit was abandoned — whether its anchors are gone or the
                // file is simply in a state recovery cannot place — but the
                // result is never written.
                let reason = if apply_operations(&source, &operations).is_ok() {
                    AbandonedReason::UnrecognizedFileState
                } else {
                    AbandonedReason::OperationsDoNotAnchor
                };
                actions.push(abandon(&mut marker, &marker_path, reason)?);
                continue;
            };

            marker.state = "COMMITTED".to_owned();
            // The repair stays owed, for the reason [`Self::recover`] gives:
            // this pass converged the Markdown, and nothing here touched an
            // index.
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
        }
        Ok(actions)
    }

    /// Records that the derived index now reflects every committed transaction
    /// of `note_id`.
    ///
    /// The engine never sets `index_repaired` itself: it holds no index, so a
    /// marker it wrote claiming the repair had happened would be a marker that
    /// lies, and [`Self::recover`] skips committed markers that claim it — so
    /// the repair would be owed by nobody and performed by nobody. The caller
    /// that actually refreshed the index says so here, and a caller that could
    /// not leaves the flag clear so recovery asks for the repair later.
    ///
    /// Markers that are not committed are left alone: the flag only means
    /// anything once there is a committed version for the index to reflect.
    ///
    /// # Errors
    ///
    /// Returns an error if a marker cannot be read, parsed, or rewritten.
    pub fn record_index_refreshed(&self, note_id: NoteId) -> Result<(), TransactionError> {
        for marker_path in self.marker_paths()? {
            let mut marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
            if marker.note_id != note_id || marker.state != "COMMITTED" || marker.index_repaired {
                continue;
            }
            marker.index_repaired = true;
            persist_marker(&marker_path, &marker)?;
        }
        Ok(())
    }

    /// Every durable transaction marker in the workspace, in deterministic
    /// order, as a read-only summary.
    ///
    /// # Errors
    ///
    /// Returns an error if a marker cannot be read or parsed.
    pub fn transactions(&self) -> Result<Vec<TransactionSummary>, TransactionError> {
        let mut summaries = Vec::new();
        for marker_path in self.marker_paths()? {
            let marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
            summaries.push(TransactionSummary {
                transaction_id: marker.transaction_id,
                note_id: marker.note_id,
                path: marker.path,
                state: marker.state,
                index_repaired: marker.index_repaired,
            });
        }
        Ok(summaries)
    }

    /// Every review still awaiting a human, in deterministic order.
    ///
    /// A descriptor is filed when a change cannot be integrated without someone
    /// deciding what it meant, and it stays on disk until they do; nothing
    /// clears it automatically. Counting them is therefore how a diagnostic
    /// reports that the workspace is waiting on a person.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction directory cannot be read.
    pub fn pending_reviews(&self) -> Result<Vec<PendingReview>, TransactionError> {
        let directory = paths::transactions_dir(self.workspace.canonical_path());
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut reviews = Vec::new();
        for entry in fs::read_dir(&directory)? {
            if let Some(transaction_id) = paths::review_descriptor_transaction(&entry?.path()) {
                reviews.push(PendingReview { transaction_id });
            }
        }
        reviews.sort_by_key(|review| review.transaction_id);
        Ok(reviews)
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

/// Rewrites a marker in place.
///
/// This re-serializes the whole struct, which is why the schema it round-trips
/// through is [`crate::marker::DurableState`] — the same one the engine wrote.
fn persist_marker(path: &Path, marker: &DurableState) -> Result<(), TransactionError> {
    marker.persist(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(state: &str, index_repaired: bool) -> TransactionSummary {
        TransactionSummary {
            transaction_id: TransactionId::new(),
            note_id: NoteId::new(),
            path: WorkspacePath::new("notes/a.md").expect("workspace path"),
            state: state.to_owned(),
            index_repaired,
        }
    }

    #[test]
    fn a_summary_answers_every_question_about_a_durable_state_itself() {
        // The labels are this crate's vocabulary: the engine writes them and
        // recovery rewrites them. A caller comparing the string would be
        // holding a second copy, free to drift from the one on disk.
        assert!(summary("COMMITTED", true).committed());
        assert!(!summary("COMMITTED", true).aborted());
        assert!(summary("ABORTED", false).aborted());
        assert!(!summary("ABORTED", false).committed());
        assert!(summary("COMMITTED", true).is_terminal());
        assert!(summary("ABORTED", false).is_terminal());

        for state in ["PREPARED", "OPERATIONS_DURABLE", "MATERIALIZING"] {
            assert!(!summary(state, false).committed(), "{state}");
            assert!(!summary(state, false).aborted(), "{state}");
            assert!(!summary(state, false).is_terminal(), "{state}");
        }

        assert!(summary("COMMITTED", false).awaits_index_repair());
        assert!(!summary("COMMITTED", true).awaits_index_repair());
        assert!(
            !summary("ABORTED", false).awaits_index_repair(),
            "an aborted transaction has no committed version for an index to reflect"
        );
    }

    #[test]
    fn pending_reviews_name_the_transactions_waiting_and_not_the_files() {
        use secondbrain_core::id::WorkspaceId;
        use secondbrain_vault::WorkspaceRoot;

        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path();
        let transactions = paths::transactions_dir(root);
        fs::create_dir_all(&transactions).expect("create the transaction directory");
        let mut filed = vec![TransactionId::new(), TransactionId::new()];
        for transaction_id in &filed {
            fs::write(paths::review_descriptor_path(root, *transaction_id), "{}")
                .expect("file a review descriptor");
        }
        // A marker shares the directory and is not a review.
        fs::write(
            paths::marker_path(root, TransactionId::new()),
            "not a descriptor",
        )
        .expect("write a marker");

        let engine =
            TransactionEngine::new(WorkspaceRoot::open(root).expect("root"), WorkspaceId::new());
        let reviews = engine.pending_reviews().expect("read pending reviews");

        filed.sort();
        assert_eq!(
            reviews
                .iter()
                .map(|review| review.transaction_id)
                .collect::<Vec<_>>(),
            filed,
            "a marker counted as a review would report a person is needed when none is"
        );
    }

    #[test]
    fn an_unrecognized_durable_state_is_neither_committed_nor_aborted() {
        // A marker holding a label this build does not know is not finished
        // work, and must keep being reported as a transaction that never
        // completed. Folding an unknown label into a known one — which is what
        // typing this field would force, since the state machine models no
        // aborted phase — would hide exactly that.
        let unknown = summary("QUIESCED", false);

        assert!(!unknown.committed());
        assert!(!unknown.aborted());
        assert!(!unknown.is_terminal());
    }
}
