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
use secondbrain_transaction::TransactionEngine;
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

    if plan.workspace_id != workspace.manifest().workspace_id {
        return Err(CliError::PlanWorkspace {
            plan: plan.workspace_id,
            workspace: workspace.manifest().workspace_id,
        });
    }
    if plan.review_required {
        return Err(CliError::ReviewRequired(format!(
            "the plan for {} was derived from an ambiguous change and records no decision",
            plan.path
        )));
    }

    let engine =
        TransactionEngine::new(workspace.root().clone(), workspace.manifest().workspace_id);
    let request = plan.request(ActorId::new(CLI_ACTOR)?, DeviceId::new(CLI_DEVICE)?);
    let transaction_id = request.id;
    let note_id = request.note_id;
    let outcome = engine.commit(request)?;

    // The commit left the marker saying the index repair is still owed, because
    // at that point it was. Refreshing the index and then recording it is what
    // makes the marker honest — and if the refresh below fails, the flag stays
    // clear and `recovery check` asks for the repair instead.
    let index_refreshed = outcome.changed;
    if index_refreshed {
        workspace.index().rebuild()?;
        engine.record_index_refreshed(note_id)?;
    }

    emit(
        format,
        &ApplyReport {
            transaction_id,
            note_id,
            path: plan.path.to_string(),
            changed: outcome.changed,
            version: outcome.version,
            index_refreshed,
        },
    )?;
    Ok(OK)
}
