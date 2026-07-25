//! `secondbrain init` — create a workspace's internal state.

use std::path::Path;

use secondbrain_core::id::WorkspaceId;
use secondbrain_vault::{WorkspaceRoot, initialize_workspace};
use serde::Serialize;

use crate::exit::{CliError, OK};
use crate::output::{Format, Report, emit};

/// What initializing a workspace produced.
#[derive(Serialize)]
struct InitReport {
    workspace: String,
    workspace_id: WorkspaceId,
    format_version: u32,
    created_at: String,
    required_features: Vec<String>,
}

impl Report for InitReport {
    fn render(&self) -> String {
        format!(
            "Initialized workspace {}\n  workspace id:      {}\n  format version:    {}\n  created at:        {}\n  required features: {}",
            self.workspace,
            self.workspace_id,
            self.format_version,
            self.created_at,
            self.required_features.join(", ")
        )
    }
}

/// Initializes `workspace`, or reports the identity it already has.
///
/// Initialization only ever creates internal state under `.secondbrain/`; no
/// Markdown is read, written, or normalized, which is why running this on an
/// existing vault is safe.
pub fn run(format: Format, workspace: &Path) -> Result<u8, CliError> {
    // Canonicalized through the vault's own root type rather than by calling
    // the filesystem here, so that the path this binary reports is the same
    // path every later write is confined to.
    let root = WorkspaceRoot::open(workspace)?;
    let manifest = initialize_workspace(root.canonical_path())?;
    emit(
        format,
        &InitReport {
            workspace: root.canonical_path().display().to_string(),
            workspace_id: manifest.workspace_id,
            format_version: manifest.format_version,
            created_at: manifest.created_at,
            required_features: manifest.required_features,
        },
    )?;
    Ok(OK)
}
