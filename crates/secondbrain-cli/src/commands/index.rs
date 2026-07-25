//! `secondbrain index rebuild` — rebuild the derived SQLite index.

use std::path::Path;

use serde::Serialize;

use crate::exit::{CliError, OK};
use crate::output::{Format, Report, emit, plural};
use crate::workspace::Workspace;

/// What rebuilding the index found in the workspace.
#[derive(Serialize)]
struct RebuildReport {
    workspace: String,
    index: String,
    indexed: usize,
    skipped: usize,
    warnings: usize,
    errors: usize,
    orphans: usize,
    broken_links: usize,
}

impl Report for RebuildReport {
    fn render(&self) -> String {
        format!(
            "Rebuilt the index of {}\n  {} indexed, {} skipped\n  {} warnings, {} errors\n  {}, {}",
            self.workspace,
            plural(self.indexed, "note", "notes"),
            self.skipped,
            self.warnings,
            self.errors,
            plural(self.orphans, "orphan", "orphans"),
            plural(self.broken_links, "broken link", "broken links"),
        )
    }
}

/// Rebuilds the index of `workspace` from the Markdown on disk.
///
/// The index is derived state: it is thrown away and rebuilt rather than
/// migrated, and the Markdown remains the only source of truth. Nothing here
/// writes to a note.
pub fn rebuild(format: Format, workspace: &Path) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let report = workspace.index().rebuild()?;
    emit(
        format,
        &RebuildReport {
            workspace: workspace.path().display().to_string(),
            index: secondbrain_index::index_path(workspace.path())
                .display()
                .to_string(),
            indexed: report.indexed,
            skipped: report.skipped,
            warnings: report.warnings,
            errors: report.errors,
            orphans: report.orphans,
            broken_links: report.broken_links,
        },
    )?;
    Ok(OK)
}
