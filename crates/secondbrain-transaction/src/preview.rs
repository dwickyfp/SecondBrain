//! Reusable preview and apply orchestration for full-source note edits.

use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId, WorkspaceId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_index::{IndexConfig, IndexDatabase, index_path, rebuild};
use secondbrain_markdown::SourceDocument;
use secondbrain_markdown::apply::apply_operations;
use secondbrain_markdown::diff::diff_documents;
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_vault::base_snapshot::{BaseSnapshotStore, GENESIS_VERSION};
use secondbrain_vault::{WorkspaceRoot, load_manifest};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{TransactionEngine, TransactionError, TransactionRequest};

/// The adapter-facing preview format understood by this version of the crate.
pub const TRANSACTION_PREVIEW_FORMAT_V1: &str = "sb-transaction-preview-v1";

/// A validated, read-only description of one proposed full-source edit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionPreview {
    pub format: String,
    pub workspace_id: WorkspaceId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub actor: ActorId,
    pub device: DeviceId,
    pub expected_hash: ContentHash,
    pub expected_version: NoteVersion,
    pub proposed_source_hash: ContentHash,
    pub review_required: bool,
    pub summary: String,
    pub operations: Vec<SemanticOperation>,
}

/// The fields carried by the established CLI plan contract.
///
/// Its missing proposed-source hash and attribution are supplied and validated
/// by [`apply_legacy_preview`], keeping compatibility logic out of adapters.
#[derive(Clone, Debug)]
pub struct LegacyTransactionPreview {
    pub workspace_id: WorkspaceId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub expected_hash: ContentHash,
    pub expected_version: NoteVersion,
    pub review_required: bool,
    pub operations: Vec<SemanticOperation>,
}

/// Result of applying a preview and refreshing derived state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplyPreviewOutcome {
    pub transaction_id: TransactionId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub changed: bool,
    pub version: NoteVersion,
    pub index_refreshed: bool,
}

/// Failure while validating, previewing, or applying a full-source edit.
#[derive(Debug, Error)]
pub enum TransactionPreviewError {
    #[error("transaction preview I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Core(#[from] secondbrain_core::Error),
    #[error("{0}")]
    Index(#[from] secondbrain_index::IndexError),
    #[error("{0}")]
    IndexQuery(#[from] secondbrain_index::Error),
    #[error("{0}")]
    Snapshot(#[from] secondbrain_vault::SnapshotError),
    #[error("{0}")]
    Markdown(#[from] secondbrain_markdown::parse::ParseError),
    #[error("{0}")]
    Transaction(#[from] TransactionError),
    #[error("no index at {}; rebuild it before previewing or applying", .0.display())]
    IndexMissing(PathBuf),
    #[error("preview declares format {found}, not {expected}")]
    UnsupportedFormat {
        expected: &'static str,
        found: String,
    },
    #[error("preview belongs to workspace {preview}, but this workspace is {workspace}")]
    WorkspaceMismatch {
        preview: WorkspaceId,
        workspace: WorkspaceId,
    },
    #[error("preview note {preview} does not match indexed note {indexed} at {path}")]
    IdentityMismatch {
        preview: NoteId,
        indexed: NoteId,
        path: WorkspacePath,
    },
    #[error("preview for {0} requires human review")]
    NeedsReview(WorkspacePath),
    #[error("preview metadata was modified: {field}")]
    PreviewModified { field: &'static str },
}

/// Validates a workspace and derives a versioned preview without writing it.
pub fn preview_transaction(
    workspace_root: &Path,
    path: &str,
    proposed_source: &str,
    actor: ActorId,
    device: DeviceId,
) -> Result<TransactionPreview, TransactionPreviewError> {
    let (root, manifest, database) = open_workspace(workspace_root)?;
    let path = WorkspacePath::new(path).map_err(secondbrain_core::Error::from)?;
    let indexed = database.note_by_path(path.as_str())?.ok_or_else(|| {
        secondbrain_core::Error::NoteNotIndexed {
            path: path.as_path().to_path_buf(),
        }
    })?;
    let source = fs::read_to_string(root.resolve(&path)?)?;
    let expected_hash = ContentHash::digest(source.as_bytes());
    let expected_version = converged_version(&root, indexed.note_id, &path, expected_hash)?;
    let operations = diff_documents(
        &SourceDocument::parse(&source)?,
        &SourceDocument::parse(proposed_source)?,
    );
    let review_required = needs_review(&operations);

    Ok(TransactionPreview {
        format: TRANSACTION_PREVIEW_FORMAT_V1.to_owned(),
        workspace_id: manifest.workspace_id,
        note_id: indexed.note_id,
        path,
        actor,
        device,
        expected_hash,
        expected_version,
        proposed_source_hash: ContentHash::digest(proposed_source.as_bytes()),
        review_required,
        summary: stable_summary(&operations),
        operations,
    })
}

/// Applies a current preview, then rebuilds and durably acknowledges the index.
pub fn apply_preview(
    workspace_root: &Path,
    preview: &TransactionPreview,
) -> Result<ApplyPreviewOutcome, TransactionPreviewError> {
    apply_validated(workspace_root, preview)
}

/// Applies the established CLI plan shape through the same validation pipeline.
pub fn apply_legacy_preview(
    workspace_root: &Path,
    legacy: LegacyTransactionPreview,
    actor: ActorId,
    device: DeviceId,
) -> Result<ApplyPreviewOutcome, TransactionPreviewError> {
    let root = WorkspaceRoot::open(workspace_root)?;
    let base = match BaseSnapshotStore::new(&root).load(legacy.note_id)? {
        Some(snapshot) => snapshot.source,
        None => fs::read_to_string(root.resolve(&legacy.path)?)?,
    };
    let proposed = apply_operations(&base, &legacy.operations)?;
    let preview = TransactionPreview {
        format: TRANSACTION_PREVIEW_FORMAT_V1.to_owned(),
        workspace_id: legacy.workspace_id,
        note_id: legacy.note_id,
        path: legacy.path,
        actor,
        device,
        expected_hash: legacy.expected_hash,
        expected_version: legacy.expected_version,
        proposed_source_hash: ContentHash::digest(proposed.as_bytes()),
        review_required: legacy.review_required,
        summary: stable_summary(&legacy.operations),
        operations: legacy.operations,
    };
    apply_validated(workspace_root, &preview)
}

fn apply_validated(
    workspace_root: &Path,
    preview: &TransactionPreview,
) -> Result<ApplyPreviewOutcome, TransactionPreviewError> {
    if preview.format != TRANSACTION_PREVIEW_FORMAT_V1 {
        return Err(TransactionPreviewError::UnsupportedFormat {
            expected: TRANSACTION_PREVIEW_FORMAT_V1,
            found: preview.format.clone(),
        });
    }
    let (root, manifest, database) = open_workspace(workspace_root)?;
    if preview.workspace_id != manifest.workspace_id {
        return Err(TransactionPreviewError::WorkspaceMismatch {
            preview: preview.workspace_id,
            workspace: manifest.workspace_id,
        });
    }
    let indexed = database
        .note_by_path(preview.path.as_str())?
        .ok_or_else(|| secondbrain_core::Error::NoteNotIndexed {
            path: preview.path.as_path().to_path_buf(),
        })?;
    if indexed.note_id != preview.note_id {
        return Err(TransactionPreviewError::IdentityMismatch {
            preview: preview.note_id,
            indexed: indexed.note_id,
            path: preview.path.clone(),
        });
    }
    let derived_review = needs_review(&preview.operations);
    if preview.review_required != derived_review {
        return Err(TransactionPreviewError::PreviewModified {
            field: "review_required",
        });
    }
    if preview.summary != stable_summary(&preview.operations) {
        return Err(TransactionPreviewError::PreviewModified { field: "summary" });
    }
    if derived_review {
        return Err(TransactionPreviewError::NeedsReview(preview.path.clone()));
    }

    let source = fs::read_to_string(root.resolve(&preview.path)?)?;
    let actual_hash = ContentHash::digest(source.as_bytes());
    if actual_hash != preview.expected_hash {
        return Err(TransactionError::StaleHash {
            expected: preview.expected_hash,
            actual: actual_hash,
        }
        .into());
    }
    let actual_version = converged_version(&root, preview.note_id, &preview.path, actual_hash)?;
    if actual_version != preview.expected_version {
        return Err(TransactionPreviewError::PreviewModified {
            field: "expected_version",
        });
    }
    let materialized = apply_operations(&source, &preview.operations)?;
    if ContentHash::digest(materialized.as_bytes()) != preview.proposed_source_hash {
        return Err(TransactionPreviewError::PreviewModified {
            field: "proposed_source_hash",
        });
    }

    let transaction_id = TransactionId::new();
    let engine = TransactionEngine::new(root.clone(), manifest.workspace_id);
    let outcome = engine.commit(TransactionRequest {
        id: transaction_id,
        actor: preview.actor.clone(),
        device: preview.device.clone(),
        note_id: preview.note_id,
        path: preview.path.clone(),
        expected_hash: preview.expected_hash,
        expected_version: preview.expected_version,
        operations: preview.operations.clone(),
    })?;
    if outcome.changed {
        rebuild(root.canonical_path(), &IndexConfig::default())?;
        engine.record_index_refreshed(preview.note_id)?;
    }
    Ok(ApplyPreviewOutcome {
        transaction_id,
        note_id: preview.note_id,
        path: preview.path.clone(),
        changed: outcome.changed,
        version: outcome.version,
        index_refreshed: outcome.changed,
    })
}

fn open_workspace(
    workspace_root: &Path,
) -> Result<
    (
        WorkspaceRoot,
        secondbrain_vault::WorkspaceManifest,
        IndexDatabase,
    ),
    TransactionPreviewError,
> {
    let root = WorkspaceRoot::open(workspace_root)?;
    let manifest = load_manifest(root.canonical_path())?;
    let index = index_path(root.canonical_path());
    if !index.exists() {
        return Err(TransactionPreviewError::IndexMissing(index));
    }
    let database = IndexDatabase::open(index)?;
    Ok((root, manifest, database))
}

fn converged_version(
    root: &WorkspaceRoot,
    note_id: NoteId,
    path: &WorkspacePath,
    source_hash: ContentHash,
) -> Result<NoteVersion, TransactionPreviewError> {
    match BaseSnapshotStore::new(root).load(note_id)? {
        Some(snapshot) if !snapshot.describes(source_hash) => {
            Err(secondbrain_core::Error::NoteDiverged {
                path: path.as_path().to_path_buf(),
                version: snapshot.version,
            }
            .into())
        }
        Some(snapshot) => Ok(snapshot.version),
        None => Ok(GENESIS_VERSION),
    }
}

fn needs_review(operations: &[SemanticOperation]) -> bool {
    operations
        .iter()
        .any(|operation| matches!(operation, SemanticOperation::NeedsReview { .. }))
}

fn stable_summary(operations: &[SemanticOperation]) -> String {
    if operations.is_empty() {
        return "no semantic change".to_owned();
    }
    let mut kinds: Vec<_> = operations
        .iter()
        .map(SemanticOperation::kind_name)
        .collect();
    kinds.sort_unstable();
    kinds
        .chunk_by(|left, right| left == right)
        .map(|kind| format!("{} x{}", kind[0], kind.len()))
        .collect::<Vec<_>>()
        .join(", ")
}
