//! Typed property preview/apply contracts bound to the shared transaction preview.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::{PropertyEdit, PropertyValue, edit_property, read_properties};
use secondbrain_vault::{WorkspaceRoot, load_manifest};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyPreviewOutcome, TransactionError, TransactionPreview, TransactionPreviewError,
    apply_preview, preview_transaction,
};

/// The external JSON format for a typed property preview.
pub const PROPERTY_PREVIEW_FORMAT_V1: &str = "sb-property-preview-v1";

/// Typed intent and resulting properties bound to a shared transaction preview.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PropertyPreview {
    pub format: String,
    pub edit: PropertyEdit,
    pub properties: BTreeMap<String, PropertyValue>,
    pub transaction: TransactionPreview,
}

/// Read a note's editable, JSON-compatible properties through workspace confinement.
pub fn read_note_properties(
    workspace_root: &Path,
    path: &str,
) -> Result<BTreeMap<String, PropertyValue>, TransactionPreviewError> {
    let root = WorkspaceRoot::open(workspace_root)?;
    load_manifest(root.canonical_path())?;
    let path = WorkspacePath::new(path).map_err(secondbrain_core::Error::from)?;
    let source = fs::read_to_string(root.resolve(&path)?)?;
    Ok(read_properties(&source)?)
}

/// Build a read-only typed property preview using the shared transaction engine.
pub fn preview_property(
    workspace_root: &Path,
    path: &str,
    edit: PropertyEdit,
    actor: ActorId,
    device: DeviceId,
) -> Result<PropertyPreview, TransactionPreviewError> {
    let root = WorkspaceRoot::open(workspace_root)?;
    load_manifest(root.canonical_path())?;
    let confined_path = WorkspacePath::new(path).map_err(secondbrain_core::Error::from)?;
    let source = fs::read_to_string(root.resolve(&confined_path)?)?;
    let patch = edit_property(&source, &edit)?;
    let properties = read_properties(&patch.source)?;
    let transaction = preview_transaction(
        root.canonical_path(),
        confined_path.as_str(),
        &patch.source,
        actor,
        device,
    )?;
    Ok(PropertyPreview {
        format: PROPERTY_PREVIEW_FORMAT_V1.to_owned(),
        edit,
        properties,
        transaction,
    })
}

/// Validate a typed intent against current bytes and apply its shared preview.
pub fn apply_property_preview(
    workspace_root: &Path,
    preview: &PropertyPreview,
) -> Result<ApplyPreviewOutcome, TransactionPreviewError> {
    if preview.format != PROPERTY_PREVIEW_FORMAT_V1 {
        return Err(TransactionPreviewError::UnsupportedPropertyFormat {
            expected: PROPERTY_PREVIEW_FORMAT_V1,
            found: preview.format.clone(),
        });
    }
    let root = WorkspaceRoot::open(workspace_root)?;
    let source = fs::read_to_string(root.resolve(&preview.transaction.path)?)?;
    let actual_hash = ContentHash::digest(source.as_bytes());
    if actual_hash != preview.transaction.expected_hash {
        return Err(TransactionError::StaleHash {
            expected: preview.transaction.expected_hash,
            actual: actual_hash,
        }
        .into());
    }
    let patch = edit_property(&source, &preview.edit)?;
    if ContentHash::digest(patch.source.as_bytes()) != preview.transaction.proposed_source_hash {
        return Err(TransactionPreviewError::PreviewModified {
            field: "property_edit",
        });
    }
    if read_properties(&patch.source)? != preview.properties {
        return Err(TransactionPreviewError::PreviewModified {
            field: "properties",
        });
    }
    apply_preview(workspace_root, &preview.transaction)
}
