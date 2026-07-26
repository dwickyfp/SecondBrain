//! Single-note semantic transaction commit pipeline.

use std::fs;
use std::io;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId, WorkspaceId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::apply::apply_operations;
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_vault::WorkspaceRoot;
use secondbrain_vault::base_snapshot::{BaseSnapshotStore, SnapshotError};
use thiserror::Error;

use crate::failpoint;
use crate::marker::DurableState;
use crate::oplog::{LocalMutationLog, OplogError};
use crate::paths;
use crate::record::{FORMAT_VERSION_1, LocalOperationRecord, RecordEncodeError};
use crate::state::{StateTransitionError, TransactionState};

/// Inputs required to commit semantic operations to one note.
#[derive(Debug, Clone)]
pub struct TransactionRequest {
    pub id: TransactionId,
    pub actor: ActorId,
    pub device: DeviceId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub expected_hash: ContentHash,
    pub expected_version: NoteVersion,
    pub operations: Vec<SemanticOperation>,
}

/// Inputs required to durably create one note.
#[derive(Debug, Clone)]
pub struct CreateTransactionRequest {
    pub id: TransactionId,
    pub actor: ActorId,
    pub device: DeviceId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub source: String,
}

/// Result of a successful transaction attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    pub changed: bool,
    pub version: NoteVersion,
}

/// Transaction pipeline failure.
#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("transaction I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("vault operation failed: {0}")]
    Vault(#[from] secondbrain_core::Error),
    #[error("stale note hash: expected {expected}, found {actual}")]
    StaleHash {
        expected: ContentHash,
        actual: ContentHash,
    },
    #[error("NeedsReview operations cannot be committed automatically")]
    NeedsReview,
    #[error("adopted post-state {expected} is not the content on disk {actual}")]
    PostStateMismatch {
        expected: ContentHash,
        actual: ContentHash,
    },
    #[error("converged base failed: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("note version overflow")]
    VersionOverflow,
    #[error("operation log failed: {0}")]
    Oplog(#[from] OplogError),
    #[error("record encoding failed: {0}")]
    Record(#[from] RecordEncodeError),
    #[error("state transition failed: {0}")]
    State(#[from] StateTransitionError),
    #[error("transaction state serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("note creation target already exists: {0}")]
    TargetExists(WorkspacePath),
}

/// Commits transactions within one workspace.
pub struct TransactionEngine {
    pub(crate) workspace: WorkspaceRoot,
    workspace_id: WorkspaceId,
}

impl TransactionEngine {
    #[must_use]
    pub const fn new(workspace: WorkspaceRoot, workspace_id: WorkspaceId) -> Self {
        Self {
            workspace,
            workspace_id,
        }
    }

    /// Validate, journal, materialize, and durably mark a transaction committed.
    pub fn commit(&self, request: TransactionRequest) -> Result<CommitOutcome, TransactionError> {
        let note_path = self.workspace.resolve(&request.path)?;
        let source = fs::read_to_string(note_path)?;
        let actual_hash = ContentHash::digest(source.as_bytes());
        if actual_hash != request.expected_hash {
            return Err(TransactionError::StaleHash {
                expected: request.expected_hash,
                actual: actual_hash,
            });
        }
        if request
            .operations
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::NeedsReview { .. }))
        {
            return Err(TransactionError::NeedsReview);
        }

        let materialized = apply_operations(&source, &request.operations)?;
        if materialized == source {
            return Ok(CommitOutcome {
                changed: false,
                version: request.expected_version,
            });
        }
        let version = request
            .expected_version
            .checked_increment()
            .ok_or(TransactionError::VersionOverflow)?;
        // Every marker this transaction writes names the content it
        // materializes, so a crash anywhere after the Markdown write leaves
        // recovery able to recognize the file as this transaction's own result
        // by comparing hashes. Recovery must not have to deduce that by
        // re-applying the operations: an operation that anchors on the text it
        // replaces cannot be applied to its own post-state.
        let materialized_hash = ContentHash::digest(materialized.as_bytes());

        let mut state = TransactionState::Prepared;
        self.persist_state(&request, state, version, materialized_hash, false, false)?;

        failpoint::hit("before_append")?;
        self.journal_operations(&request)?;
        failpoint::hit("after_append_before_state")?;

        state.transition_to(TransactionState::OperationsDurable)?;
        self.persist_state(&request, state, version, materialized_hash, false, false)?;
        failpoint::hit("after_operations_durable")?;
        state.transition_to(TransactionState::Materializing)?;
        self.persist_state(&request, state, version, materialized_hash, false, false)?;
        failpoint::hit("during_temp_markdown_write")?;
        self.workspace
            .atomic_write(&request.path, materialized.as_bytes())?;
        failpoint::hit("after_rename_before_commit")?;
        // The converged base is recorded *before* the transaction is marked
        // committed. A crash in between then leaves a `MATERIALIZING` marker
        // whose recorded materialized hash matches the file, which recovery
        // commits and records the base for — whereas a crash after a
        // `COMMITTED` marker but before the base would leave a committed
        // version whose pre-state nothing holds, and the next external edit
        // would diff against the stale base and journal this transaction's own
        // operations a second time.
        self.record_converged_base(&request, version, &materialized)?;
        state.transition_to(TransactionState::Committed)?;
        // `index_repaired` stays clear. The engine has no index — see
        // [`Self::record_index_refreshed`] — so setting the flag here would
        // claim work that has not happened, and recovery skips committed
        // markers that claim it, which is exactly how the repair would be lost.
        self.persist_state(&request, state, version, materialized_hash, false, false)?;
        failpoint::hit("after_commit_before_index")?;

        Ok(CommitOutcome {
            changed: true,
            version,
        })
    }

    /// Journals and atomically creates a note whose target was absent at preview time.
    pub fn create(
        &self,
        request: CreateTransactionRequest,
    ) -> Result<CommitOutcome, TransactionError> {
        let target = self.workspace.resolve_read_only(&request.path)?;
        let materialized_hash = ContentHash::digest(request.source.as_bytes());
        let marker_path = paths::marker_path(self.workspace.canonical_path(), request.id);
        if marker_path.exists() && !target.exists() {
            let marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
            if marker.creates_note
                && marker.transaction_id == request.id
                && marker.note_id == request.note_id
                && marker.path == request.path
                && marker.materialized_hash == materialized_hash
                && marker.state != "ABORTED"
            {
                self.recover()?;
                if target.exists() && ContentHash::digest(&fs::read(&target)?) == materialized_hash
                {
                    return Ok(CommitOutcome {
                        changed: false,
                        version: NoteVersion::new(0),
                    });
                }
            }
        }
        if target.exists() {
            if marker_path.exists() {
                let marker: DurableState = serde_json::from_slice(&fs::read(&marker_path)?)?;
                let actual = ContentHash::digest(&fs::read(&target)?);
                if marker.creates_note
                    && marker.transaction_id == request.id
                    && marker.note_id == request.note_id
                    && marker.path == request.path
                    && marker.materialized_hash == materialized_hash
                    && actual == materialized_hash
                {
                    if marker.state != "COMMITTED" {
                        self.recover()?;
                    }
                    return Ok(CommitOutcome {
                        changed: false,
                        version: NoteVersion::new(0),
                    });
                }
            }
            return Err(TransactionError::TargetExists(request.path));
        }
        let semantic = SemanticOperation::InsertNode {
            anchor: None,
            content: request.source.clone(),
        };
        let journal_request = TransactionRequest {
            id: request.id,
            actor: request.actor,
            device: request.device,
            note_id: request.note_id,
            path: request.path,
            expected_hash: ContentHash::digest(b""),
            expected_version: NoteVersion::new(0),
            operations: vec![semantic],
        };
        let version = NoteVersion::new(0);
        self.persist_state(
            &journal_request,
            TransactionState::Prepared,
            version,
            materialized_hash,
            false,
            true,
        )?;
        failpoint::hit("create_before_append")?;
        self.journal_operations(&journal_request)?;
        self.persist_state(
            &journal_request,
            TransactionState::OperationsDurable,
            version,
            materialized_hash,
            false,
            true,
        )?;
        failpoint::hit("create_after_operations_durable")?;
        self.persist_state(
            &journal_request,
            TransactionState::Materializing,
            version,
            materialized_hash,
            false,
            true,
        )?;
        self.workspace
            .atomic_create(&journal_request.path, request.source.as_bytes())?;
        failpoint::hit("create_after_rename_before_commit")?;
        self.record_converged_base(&journal_request, version, &request.source)?;
        self.persist_state(
            &journal_request,
            TransactionState::Committed,
            version,
            materialized_hash,
            false,
            true,
        )?;
        failpoint::hit("create_after_commit_before_index")?;
        Ok(CommitOutcome {
            changed: true,
            version,
        })
    }

    /// Journals an external edit whose result is already materialized on disk.
    ///
    /// An external editor rewrites the whole file before the workspace ever
    /// sees the change, so the operations that describe such an edit can never
    /// be applied to the file again — their anchors describe the base the
    /// editor started from, and that base is gone. This entry point therefore
    /// shares [`Self::commit`]'s journal, marker format and converged base, and
    /// skips only the Markdown write, after proving that the file already holds
    /// the operations' post-state. That is the rule recovery already applies
    /// when it finds Markdown matching a transaction's post-state: mark the
    /// transaction committed idempotently rather than rewriting the file.
    ///
    /// `base` is the converged source the operations were derived from, and
    /// `request.expected_hash` is the on-disk state the caller observed.
    ///
    /// Unlike [`Self::commit`], no `PREPARED` or `MATERIALIZING` marker is
    /// written: there is no materialization to resume, and a marker for one
    /// would make recovery replay operations against a file that already
    /// contains them. A crash before the single `COMMITTED` marker therefore
    /// leaves journal records that recovery ignores, with the file already
    /// holding the edit.
    ///
    /// The returned outcome reports `changed: false` because no Markdown was
    /// written; `version` is the version the journaled edit converged at.
    pub fn adopt_external(
        &self,
        request: TransactionRequest,
        base: &str,
    ) -> Result<CommitOutcome, TransactionError> {
        let note_path = self.workspace.resolve(&request.path)?;
        let source = fs::read_to_string(note_path)?;
        let actual_hash = ContentHash::digest(source.as_bytes());
        if actual_hash != request.expected_hash {
            return Err(TransactionError::StaleHash {
                expected: request.expected_hash,
                actual: actual_hash,
            });
        }
        if request
            .operations
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::NeedsReview { .. }))
        {
            return Err(TransactionError::NeedsReview);
        }
        // Checked before the post-state comparison, which an empty operation
        // set would otherwise decide: there is nothing to journal either way,
        // and no version to bump for having journaled it.
        if request.operations.is_empty() {
            return Ok(CommitOutcome {
                changed: false,
                version: request.expected_version,
            });
        }
        let post_state = apply_operations(base, &request.operations)?;
        if post_state != source {
            return Err(TransactionError::PostStateMismatch {
                expected: ContentHash::digest(post_state.as_bytes()),
                actual: actual_hash,
            });
        }
        let version = request
            .expected_version
            .checked_increment()
            .ok_or(TransactionError::VersionOverflow)?;

        self.journal_operations(&request)?;
        // Base first, marker last, for the reason [`Self::commit`] gives. Here
        // a crash in between leaves journal records no marker claims — which
        // recovery ignores — and a base equal to what is already on disk, so
        // the next event for this note correctly reports nothing to do.
        self.record_converged_base(&request, version, &source)?;
        failpoint::hit("during_adopt_convergence")?;
        // The materialized content of an adoption is the file itself: the
        // editor wrote it before the workspace ever saw the change.
        self.persist_state(
            &request,
            TransactionState::Committed,
            version,
            actual_hash,
            false,
            false,
        )?;

        Ok(CommitOutcome {
            changed: false,
            version,
        })
    }

    /// Appends every operation of one transaction to the note's oplog,
    /// continuing the sequence and hash chain of the records already there.
    fn journal_operations(&self, request: &TransactionRequest) -> Result<(), TransactionError> {
        let mut log = LocalMutationLog::open(self.workspace.canonical_path(), request.note_id)?;
        let replay = log.replay()?;
        let mut sequence = replay
            .records
            .last()
            .map_or(1, |record| record.sequence + 1);
        let mut previous_record_hash = replay
            .records
            .last()
            .map(|record| record.encode().map(ContentHash::digest))
            .transpose()?;
        for operation in &request.operations {
            let record = LocalOperationRecord {
                format_version: FORMAT_VERSION_1,
                transaction_id: request.id,
                workspace_id: self.workspace_id,
                note_id: request.note_id,
                actor_id: request.actor.clone(),
                device_id: request.device.clone(),
                sequence,
                previous_record_hash,
                operation: operation.clone(),
                crc32: 0,
            };
            log.append(&record)?;
            previous_record_hash = Some(ContentHash::digest(record.encode()?));
            sequence += 1;
        }
        Ok(())
    }

    fn record_converged_base(
        &self,
        request: &TransactionRequest,
        version: NoteVersion,
        source: &str,
    ) -> Result<(), TransactionError> {
        BaseSnapshotStore::new(&self.workspace).save(
            request.note_id,
            &request.path,
            version,
            source,
        )?;
        Ok(())
    }

    fn persist_state(
        &self,
        request: &TransactionRequest,
        state: TransactionState,
        version: NoteVersion,
        materialized_hash: ContentHash,
        index_repaired: bool,
        creates_note: bool,
    ) -> Result<(), TransactionError> {
        let marker = DurableState {
            transaction_id: request.id,
            note_id: request.note_id,
            path: request.path.clone(),
            state: state.label().to_owned(),
            expected_hash: request.expected_hash,
            materialized_hash,
            expected_version: request.expected_version,
            committed_version: version,
            index_repaired,
            creates_note,
        };
        let root = self.workspace.canonical_path();
        fs::create_dir_all(paths::transactions_dir(root))?;
        marker.persist(&paths::marker_path(root, request.id))
    }
}
