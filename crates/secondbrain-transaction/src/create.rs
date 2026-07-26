//! Read-only note-creation previews and transaction-engine application.

use std::path::{Path, PathBuf};

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId, WorkspaceId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_index::{IndexConfig, IndexDatabase, index_path, logical_dump, rebuild};
use secondbrain_markdown::{SourceDocument, ensure_note_id, parse_metadata};
use secondbrain_vault::{WorkspaceRoot, load_manifest};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{CreateTransactionRequest, TransactionEngine, TransactionError};

pub const NOTE_CREATE_PREVIEW_FORMAT_V1: &str = "sb-note-create-preview-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteCreatePreview {
    pub format: String,
    pub transaction_id: TransactionId,
    pub workspace_id: WorkspaceId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub actor: ActorId,
    pub device: DeviceId,
    pub source: String,
    pub source_hash: ContentHash,
    pub expected_absent: bool,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NoteCreateOutcome {
    pub transaction_id: TransactionId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub created: bool,
    pub version: NoteVersion,
    pub index_refreshed: bool,
}

#[derive(Debug, Error)]
pub enum NoteCreateError {
    #[error("note creation I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Core(#[from] secondbrain_core::Error),
    #[error("{0}")]
    Index(#[from] secondbrain_index::IndexError),
    #[error("{0}")]
    IndexQuery(#[from] secondbrain_index::Error),
    #[error("{0}")]
    Markdown(#[from] secondbrain_markdown::parse::ParseError),
    #[error("{0}")]
    Transaction(#[from] TransactionError),
    #[error("no index at {}; rebuild it before creating notes", .0.display())]
    IndexMissing(PathBuf),
    #[error("creation preview declares format {found}, not {expected}")]
    UnsupportedFormat {
        expected: &'static str,
        found: String,
    },
    #[error("creation preview belongs to workspace {preview}, but this workspace is {workspace}")]
    WorkspaceMismatch {
        preview: WorkspaceId,
        workspace: WorkspaceId,
    },
    #[error("note creation target already exists: {0}")]
    TargetExists(WorkspacePath),
    #[error("portable path collision between {first} and {second}")]
    PathCollision { first: String, second: String },
    #[error("creation preview was modified: {0}")]
    PreviewModified(&'static str),
}

pub fn preview_note_creation(
    workspace_root: &Path,
    path: &str,
    source: &str,
    actor: ActorId,
    device: DeviceId,
) -> Result<NoteCreatePreview, NoteCreateError> {
    let (root, workspace_id, database) = open_workspace(workspace_root)?;
    let path = WorkspacePath::new(path).map_err(secondbrain_core::Error::from)?;
    let target = root.resolve_read_only(&path)?;
    if target.exists() || database.note_by_path(path.as_str())?.is_some() {
        return Err(NoteCreateError::TargetExists(path));
    }
    reject_portable_collision(root.canonical_path(), &path)?;
    SourceDocument::parse(source)?;
    let note_id = NoteId::new();
    let source = ensure_note_id(source, note_id)?.source;
    let source_hash = ContentHash::digest(source.as_bytes());
    Ok(NoteCreatePreview {
        format: NOTE_CREATE_PREVIEW_FORMAT_V1.into(),
        transaction_id: TransactionId::new(),
        workspace_id,
        note_id,
        path: path.clone(),
        actor,
        device,
        source,
        source_hash,
        expected_absent: true,
        summary: format!("Create note at {path}"),
    })
}

pub fn apply_note_creation(
    workspace_root: &Path,
    preview: &NoteCreatePreview,
) -> Result<NoteCreateOutcome, NoteCreateError> {
    validate_preview(preview)?;
    let (root, workspace_id, database) = open_workspace(workspace_root)?;
    if preview.workspace_id != workspace_id {
        return Err(NoteCreateError::WorkspaceMismatch {
            preview: preview.workspace_id,
            workspace: workspace_id,
        });
    }
    let target = root.resolve_read_only(&preview.path)?;
    reject_portable_collision(root.canonical_path(), &preview.path)?;
    if !target.exists() && database.note_by_path(preview.path.as_str())?.is_some() {
        return Err(NoteCreateError::TargetExists(preview.path.clone()));
    }
    let engine = TransactionEngine::new(root.clone(), workspace_id);
    let outcome = engine.create(CreateTransactionRequest {
        id: preview.transaction_id,
        actor: preview.actor.clone(),
        device: preview.device.clone(),
        note_id: preview.note_id,
        path: preview.path.clone(),
        source: preview.source.clone(),
    })?;
    let indexed = database.note_by_path(preview.path.as_str())?;
    let index_stale = indexed.is_none_or(|note| note.note_id != preview.note_id);
    drop(database);
    if outcome.changed || index_stale {
        rebuild(root.canonical_path(), &IndexConfig::default())?;
        engine.record_index_refreshed(preview.note_id)?;
    }
    Ok(NoteCreateOutcome {
        transaction_id: preview.transaction_id,
        note_id: preview.note_id,
        path: preview.path.clone(),
        created: outcome.changed,
        version: outcome.version,
        index_refreshed: outcome.changed || index_stale,
    })
}

fn validate_preview(preview: &NoteCreatePreview) -> Result<(), NoteCreateError> {
    if preview.format != NOTE_CREATE_PREVIEW_FORMAT_V1 {
        return Err(NoteCreateError::UnsupportedFormat {
            expected: NOTE_CREATE_PREVIEW_FORMAT_V1,
            found: preview.format.clone(),
        });
    }
    if !preview.expected_absent {
        return Err(NoteCreateError::PreviewModified("expected_absent"));
    }
    if ContentHash::digest(preview.source.as_bytes()) != preview.source_hash {
        return Err(NoteCreateError::PreviewModified("source_hash"));
    }
    if parse_metadata(&preview.source)?.id != Some(preview.note_id) {
        return Err(NoteCreateError::PreviewModified("note_id"));
    }
    if preview.summary != format!("Create note at {}", preview.path) {
        return Err(NoteCreateError::PreviewModified("summary"));
    }
    SourceDocument::parse(&preview.source)?;
    Ok(())
}

fn open_workspace(
    root: &Path,
) -> Result<(WorkspaceRoot, WorkspaceId, IndexDatabase), NoteCreateError> {
    let root = WorkspaceRoot::open(root)?;
    let workspace_id = load_manifest(root.canonical_path())?.workspace_id;
    let index = index_path(root.canonical_path());
    if !index.exists() {
        return Err(NoteCreateError::IndexMissing(index));
    }
    Ok((root, workspace_id, IndexDatabase::open(index)?))
}

fn reject_portable_collision(root: &Path, path: &WorkspacePath) -> Result<(), NoteCreateError> {
    let key = path.as_str().to_lowercase();
    if let Some(existing) = logical_dump(index_path(root))?
        .notes
        .into_iter()
        .map(|note| note.path)
        .find(|existing| existing.to_lowercase() == key && existing != path.as_str())
    {
        return Err(NoteCreateError::PathCollision {
            first: existing.to_string(),
            second: path.to_string(),
        });
    }
    Ok(())
}
