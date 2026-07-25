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
use serde::Serialize;
use thiserror::Error;

use crate::oplog::{LocalMutationLog, OplogError};
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
}

/// Commits transactions within one workspace.
pub struct TransactionEngine {
    workspace: WorkspaceRoot,
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

        let mut state = TransactionState::Prepared;
        self.persist_state(&request, state, version)?;

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

        state.transition_to(TransactionState::OperationsDurable)?;
        self.persist_state(&request, state, version)?;
        state.transition_to(TransactionState::Materializing)?;
        self.persist_state(&request, state, version)?;
        self.workspace
            .atomic_write(&request.path, materialized.as_bytes())?;
        state.transition_to(TransactionState::Committed)?;
        self.persist_state(&request, state, version)?;

        Ok(CommitOutcome {
            changed: true,
            version,
        })
    }

    fn persist_state(
        &self,
        request: &TransactionRequest,
        state: TransactionState,
        version: NoteVersion,
    ) -> Result<(), TransactionError> {
        #[derive(Serialize)]
        struct DurableState<'a> {
            transaction_id: TransactionId,
            note_id: NoteId,
            path: &'a WorkspacePath,
            state: &'static str,
            expected_hash: ContentHash,
            expected_version: NoteVersion,
            committed_version: NoteVersion,
        }
        let bytes = serde_json::to_vec_pretty(&DurableState {
            transaction_id: request.id,
            note_id: request.note_id,
            path: &request.path,
            state: state.label(),
            expected_hash: request.expected_hash,
            expected_version: request.expected_version,
            committed_version: version,
        })?;
        let directory = self
            .workspace
            .canonical_path()
            .join(".secondbrain/transactions");
        fs::create_dir_all(&directory)?;
        secondbrain_vault::atomic_write::atomic_write(
            &directory.join(format!("{}.json", request.id)),
            &bytes,
        )?;
        Ok(())
    }
}
