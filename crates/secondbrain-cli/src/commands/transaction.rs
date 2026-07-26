//! `secondbrain transaction apply` — apply a plan an operator has seen.
//!
//! The plan is explicit and its preconditions are checked before anything is
//! written: the workspace it names must be this one, the note must still hash
//! to what the plan recorded, and the change must not be one the diff flagged
//! as needing a person. There is deliberately no path from an incoming file
//! straight to a write; a change nobody looked at is a change nobody approved.

use std::path::Path;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId};
use secondbrain_transaction::{LegacyTransactionPreview, apply_legacy_preview};
use serde::Serialize;

use crate::commands::{CLI_ACTOR, CLI_DEVICE};
use crate::exit::{CliError, OK, read_file};
use crate::output::{Format, Report, emit};
use crate::plan::TransactionPlan;
use crate::workspace::Workspace;

/// What applying a plan did.
#[derive(Serialize)]
struct ApplyReport {
    transaction_id: TransactionId,
    note_id: NoteId,
    path: String,
    changed: bool,
    version: NoteVersion,
    index_refreshed: bool,
}

impl Report for ApplyReport {
    fn render(&self) -> String {
        format!(
            "{} {}\n  transaction:      {}\n  note id:          {}\n  version:          {}\n  index refreshed:  {}",
            if self.changed {
                "Applied a plan to"
            } else {
                "Plan changed nothing in"
            },
            self.path,
            self.transaction_id,
            self.note_id,
            self.version.get(),
            if self.index_refreshed { "yes" } else { "no" }
        )
    }
}

/// Applies the plan in `plan_file` to `workspace`.
pub fn apply(format: Format, workspace: &Path, plan_file: &Path) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let plan = TransactionPlan::parse(&read_file("read plan", plan_file)?)?;

    let outcome = apply_legacy_preview(
        workspace.path(),
        LegacyTransactionPreview {
            workspace_id: plan.workspace_id,
            note_id: plan.note_id,
            path: plan.path.clone(),
            expected_hash: plan.expected_hash,
            expected_version: plan.expected_version,
            review_required: plan.review_required,
            operations: plan.operations.clone(),
        },
        ActorId::new(CLI_ACTOR)?,
        DeviceId::new(CLI_DEVICE)?,
    )
    .map_err(|error| match error {
        secondbrain_transaction::TransactionPreviewError::WorkspaceMismatch {
            preview,
            workspace,
        } => CliError::PlanWorkspace {
            plan: preview,
            workspace,
        },
        secondbrain_transaction::TransactionPreviewError::NeedsReview(path) => {
            CliError::ReviewRequired(format!(
                "the plan for {path} was derived from an ambiguous change and records no decision"
            ))
        }
        error => CliError::Preview(error),
    })?;

    emit(
        format,
        &ApplyReport {
            transaction_id: outcome.transaction_id,
            note_id: outcome.note_id,
            path: outcome.path.to_string(),
            changed: outcome.changed,
            version: outcome.version,
            index_refreshed: outcome.index_refreshed,
        },
    )?;
    Ok(OK)
}
