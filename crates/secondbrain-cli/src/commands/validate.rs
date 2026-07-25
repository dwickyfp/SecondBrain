//! `secondbrain validate` — check that every note in a workspace is one this
//! system can handle without losing anything.
//!
//! The checks are the guarantees the Markdown layer makes, turned into
//! questions: does the note parse, does the source model reconstruct it byte
//! for byte, and does any note claim an identity another note already claims.
//! A workspace that passes is one the index and the transaction engine can be
//! trusted on; a workspace that fails would have failed later, less visibly.

use std::collections::BTreeMap;
use std::path::Path;

use secondbrain_core::id::{NoteId, WorkspaceId};
use secondbrain_index::{IndexConfig, note_paths};
use secondbrain_markdown::{SourceDocument, parse_metadata};
use serde::Serialize;

use crate::exit::{CliError, DIAGNOSTICS, OK, read_file};
use crate::output::{Format, Report, emit, plural};
use crate::workspace::Workspace;

/// One thing wrong with one note.
#[derive(Serialize)]
struct Problem {
    path: String,
    code: &'static str,
    message: String,
}

/// What validating a workspace found.
#[derive(Serialize)]
struct ValidateReport {
    workspace: String,
    workspace_id: WorkspaceId,
    format_version: u32,
    notes_checked: usize,
    problems: Vec<Problem>,
}

impl Report for ValidateReport {
    fn render(&self) -> String {
        let mut text = format!(
            "Validated {}\n  {} checked",
            self.workspace,
            plural(self.notes_checked, "note", "notes")
        );
        if self.problems.is_empty() {
            text.push_str("\n  no problems found");
            return text;
        }
        text.push_str(&format!(
            "\n  {} found",
            plural(self.problems.len(), "problem", "problems")
        ));
        for problem in &self.problems {
            text.push_str(&format!(
                "\n    {} [{}]: {}",
                problem.path, problem.code, problem.message
            ));
        }
        text
    }
}

/// Validates every note in `workspace`.
pub fn run(format: Format, workspace: &Path) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let paths = note_paths(workspace.path(), &IndexConfig::default())?;
    let mut problems = Vec::new();
    let mut declared: BTreeMap<NoteId, String> = BTreeMap::new();

    for path in &paths {
        let absolute = workspace.root().resolve(path)?;
        let source = match read_file("read note", &absolute) {
            Ok(source) => source,
            Err(error) => {
                problems.push(Problem {
                    path: path.to_string(),
                    code: "SB-IO",
                    message: error.to_string(),
                });
                continue;
            }
        };

        match parse_metadata(&source) {
            Ok(metadata) => {
                if let Some(id) = metadata.id
                    && let Some(first) = declared.insert(id, path.to_string())
                {
                    problems.push(Problem {
                        path: path.to_string(),
                        code: "SB-NOTE-DUPLICATE-ID",
                        message: format!("note ID {id} is also declared by {first}"),
                    });
                }
            }
            Err(error) => problems.push(Problem {
                path: path.to_string(),
                code: "SB-MD-INVALID",
                message: format!("frontmatter is unreadable: {error}"),
            }),
        }

        match SourceDocument::parse(&source) {
            // The source model is loss-aware by contract: whatever it could not
            // model it keeps as an exact source slice. A note that does not come
            // back byte for byte is one an edit would silently rewrite.
            Ok(document) if document.reconstruct() != source => problems.push(Problem {
                path: path.to_string(),
                code: "SB-MD-INVALID",
                message: "the source model does not reconstruct this note byte for byte".to_owned(),
            }),
            Ok(_) => {}
            Err(error) => problems.push(Problem {
                path: path.to_string(),
                code: "SB-MD-INVALID",
                message: error.to_string(),
            }),
        }
    }

    let code = if problems.is_empty() { OK } else { DIAGNOSTICS };
    emit(
        format,
        &ValidateReport {
            workspace: workspace.path().display().to_string(),
            workspace_id: workspace.manifest().workspace_id,
            format_version: workspace.manifest().format_version,
            notes_checked: paths.len(),
            problems,
        },
    )?;
    Ok(code)
}
