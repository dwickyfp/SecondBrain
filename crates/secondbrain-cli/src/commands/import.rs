use std::path::Path;

use secondbrain_index::{ImportApplyOutcome, ImportPreview, apply_import, preview_import};

use crate::exit::{CliError, OK, read_file};
use crate::output::{Format, Report, emit};

impl Report for ImportPreview {
    fn render(&self) -> String {
        format!(
            "Import preview for {}\n  Markdown: {}\n  attachments: {}\n  Obsidian config: {}\n  source/config/attachment writes: 0/0/0\n  can apply: {}",
            self.root,
            self.inventory.markdown.len(),
            self.inventory.attachments.len(),
            self.inventory.obsidian_config.len(),
            self.can_apply
        )
    }
}

impl Report for ImportApplyOutcome {
    fn render(&self) -> String {
        format!(
            "Imported {} ({})\n  workspace id: {}\n  indexed: {}\n  source/config/attachment writes: 0/0/0",
            self.root, self.status, self.workspace_id, self.index.indexed
        )
    }
}

pub fn preview(format: Format, workspace: &Path, out: Option<&Path>) -> Result<u8, CliError> {
    let preview = preview_import(workspace)?;
    if let Some(out) = out {
        std::fs::write(out, serde_json::to_string_pretty(&preview)?).map_err(|source| {
            CliError::Io {
                operation: "write import preview to",
                path: out.to_path_buf(),
                source,
            }
        })?;
    } else {
        emit(format, &preview)?;
    }
    Ok(OK)
}

pub fn apply(format: Format, workspace: &Path, preview_file: &Path) -> Result<u8, CliError> {
    let preview: ImportPreview =
        serde_json::from_str(&read_file("read import preview", preview_file)?)
            .map_err(|source| CliError::PlanUnreadable { source })?;
    let outcome = apply_import(workspace, &preview)?;
    emit(format, &outcome)?;
    Ok(OK)
}
