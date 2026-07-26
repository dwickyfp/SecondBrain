//! `secondbrain diff` — derive the transaction plan an incoming file implies.
//!
//! This command writes nothing to the workspace, ever. It reads the note, reads
//! the incoming file, asks the Markdown layer what semantic operations turn one
//! into the other, and reports them as a plan. Applying that plan is a separate
//! command by design: the operator sees what will happen, and the thing they
//! saw is the thing that runs.

use std::path::Path;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_transaction::preview_transaction;
use serde::Serialize;

use crate::commands::{CLI_ACTOR, CLI_DEVICE};
use crate::exit::{CliError, OK, REVIEW_REQUIRED, read_file};
use crate::output::{Format, Report, emit, plural};
use crate::plan::{PLAN_FORMAT, TransactionPlan};
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
    let incoming_source = read_file("read incoming file", incoming)?;
    let preview = preview_transaction(
        workspace.path(),
        path,
        &incoming_source,
        ActorId::new(CLI_ACTOR)?,
        DeviceId::new(CLI_DEVICE)?,
    )?;
    let plan = TransactionPlan {
        format: PLAN_FORMAT.to_owned(),
        workspace_id: preview.workspace_id,
        note_id: preview.note_id,
        path: preview.path,
        expected_hash: preview.expected_hash,
        expected_version: preview.expected_version,
        review_required: preview.review_required,
        operations: preview.operations,
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
