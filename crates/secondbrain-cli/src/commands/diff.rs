//! `secondbrain diff` — derive the transaction plan an incoming file implies.
//!
//! This command writes nothing to the workspace, ever. It reads the note, reads
//! the incoming file, asks the Markdown layer what semantic operations turn one
//! into the other, and reports them as a plan. Applying that plan is a separate
//! command by design: the operator sees what will happen, and the thing they
//! saw is the thing that runs.

use std::path::Path;

use secondbrain_core::hash::ContentHash;
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::SourceDocument;
use secondbrain_markdown::diff::diff_documents;
use secondbrain_vault::base_snapshot::{BaseSnapshotStore, GENESIS_VERSION};
use serde::Serialize;

use crate::exit::{CliError, OK, REVIEW_REQUIRED, read_file};
use crate::output::{Format, Report, emit, plural};
use crate::plan::{PLAN_FORMAT, TransactionPlan, needs_review};
use crate::workspace::Workspace;

/// Where a written plan ended up.
#[derive(Serialize)]
struct WrittenPlan {
    plan: String,
    path: String,
    operations: usize,
    review_required: bool,
    summary: String,
}

impl Report for WrittenPlan {
    fn render(&self) -> String {
        let mut text = format!(
            "Wrote a plan for {} to {}\n  {}: {}",
            self.path,
            self.plan,
            plural(self.operations, "operation", "operations"),
            self.summary
        );
        if self.review_required {
            text.push_str(
                "\n  review required: this change is ambiguous and will not be applied \
                 automatically",
            );
        }
        text
    }
}

/// Derives a plan turning the note at `path` into the content of `incoming`.
///
/// With `out`, the plan is written there and stdout carries a summary; without
/// it, the plan itself is the output — so `--json` on its own produces exactly
/// the bytes `transaction apply` expects.
pub fn run(
    format: Format,
    workspace: &Path,
    path: &str,
    incoming: &Path,
    out: Option<&Path>,
) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let note_path = WorkspacePath::new(path)?;
    let database = workspace.open_index()?;
    let summary = database.note_by_path(note_path.as_str())?.ok_or_else(|| {
        CliError::Core(secondbrain_core::Error::NoteNotIndexed {
            path: note_path.as_path().to_path_buf(),
        })
    })?;

    // The base is the note as it is on disk, which is exactly the precondition
    // the transaction engine checks when the plan is applied. Diffing against
    // anything else would produce a plan whose operations are anchored in text
    // the apply would never see.
    let base = read_file("read note", &workspace.root().resolve(&note_path)?)?;
    let base_hash = ContentHash::digest(base.as_bytes());

    // The version, though, comes from the converged base, and the engine
    // validates only the hash. If the two disagree, the file holds an edit made
    // outside the workspace that nothing has journaled: applying a plan built
    // here would bump the version from a state the note no longer holds and
    // record a new converged base over it, leaving an oplog that can never
    // replay to the file on disk. "External edits are semantic operations,
    // never silent overwrites" is the invariant that breaks, so this refuses
    // instead of planning across the gap.
    let snapshot = BaseSnapshotStore::new(workspace.root()).load(summary.note_id)?;
    let expected_version = match &snapshot {
        Some(snapshot) if !snapshot.describes(base_hash) => {
            return Err(CliError::Core(secondbrain_core::Error::NoteDiverged {
                path: note_path.as_path().to_path_buf(),
                version: snapshot.version,
            }));
        }
        Some(snapshot) => snapshot.version,
        // No base was ever recorded, so no transaction has ever touched this
        // note and there is no earlier state for it to have diverged from.
        None => GENESIS_VERSION,
    };

    let incoming_source = read_file("read incoming file", incoming)?;
    let operations = diff_documents(
        &SourceDocument::parse(&base)?,
        &SourceDocument::parse(&incoming_source)?,
    );
    let plan = TransactionPlan {
        format: PLAN_FORMAT.to_owned(),
        workspace_id: workspace.manifest().workspace_id,
        note_id: summary.note_id,
        path: note_path,
        expected_hash: base_hash,
        expected_version,
        review_required: needs_review(&operations),
        operations,
    };

    let code = if plan.review_required {
        REVIEW_REQUIRED
    } else {
        OK
    };
    match out {
        Some(target) => {
            let serialized = serde_json::to_string_pretty(&plan)?;
            std::fs::write(target, serialized).map_err(|source| CliError::Io {
                operation: "write plan to",
                path: target.to_path_buf(),
                source,
            })?;
            emit(
                format,
                &WrittenPlan {
                    plan: target.display().to_string(),
                    path: plan.path.to_string(),
                    operations: plan.operations.len(),
                    review_required: plan.review_required,
                    summary: plan.summary(),
                },
            )?;
        }
        None => emit(format, &plan)?,
    }
    Ok(code)
}
