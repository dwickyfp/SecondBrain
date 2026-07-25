//! `secondbrain recovery check` — finish what a crash interrupted, and say what
//! that cost.
//!
//! Recovery reports three kinds of outcome and this command surfaces all three,
//! because two of them are how an operator learns their data needed attention:
//!
//! * an index repair is ordinary work, and this command performs it;
//! * a quarantined journal suffix means damaged records were set aside;
//! * an abandoned edit means a durably journaled change could not be replayed
//!   and is gone. "Confirmed edits survive process and power failure" is the
//!   invariant the whole journal exists for, so an abandonment that nobody was
//!   told about would be that invariant broken silently — the exact outcome
//!   reporting it was designed to prevent.

use std::path::Path;

use secondbrain_core::id::{NoteId, TransactionId};
use secondbrain_transaction::{RecoveryAction, TransactionEngine};
use serde::Serialize;

use crate::exit::{CliError, DIAGNOSTICS, OK};
use crate::output::{Format, Report, emit, plural};
use crate::workspace::Workspace;

/// One durable repair recovery performed, in the form the `--json` contract
/// promises.
///
/// The variants are flattened into a tagged object rather than nested, so that
/// a caller can filter on `action` without knowing the shape of every kind.
#[derive(Serialize)]
struct Action {
    action: &'static str,
    transaction_id: TransactionId,
    note_id: NoteId,
    path: String,
    /// Set for a quarantine: where the damaged journal suffix was preserved.
    #[serde(skip_serializing_if = "Option::is_none")]
    quarantine_path: Option<String>,
    /// Set for an abandonment: the machine-readable reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    /// Set for an abandonment: what it means for the operator's data.
    #[serde(skip_serializing_if = "Option::is_none")]
    explanation: Option<String>,
}

impl From<&RecoveryAction> for Action {
    fn from(action: &RecoveryAction) -> Self {
        match action {
            RecoveryAction::IndexRepair {
                transaction_id,
                note_id,
                path,
            } => Self {
                action: "index_repair",
                transaction_id: *transaction_id,
                note_id: *note_id,
                path: path.to_string(),
                quarantine_path: None,
                reason: None,
                explanation: None,
            },
            RecoveryAction::Quarantined {
                transaction_id,
                note_id,
                path,
                quarantine_path,
            } => Self {
                action: "quarantined",
                transaction_id: *transaction_id,
                note_id: *note_id,
                path: path.to_string(),
                quarantine_path: Some(quarantine_path.display().to_string()),
                reason: None,
                explanation: None,
            },
            RecoveryAction::Abandoned {
                transaction_id,
                note_id,
                path,
                reason,
            } => Self {
                action: "abandoned",
                transaction_id: *transaction_id,
                note_id: *note_id,
                path: path.to_string(),
                quarantine_path: None,
                reason: Some(match reason {
                    secondbrain_transaction::AbandonedReason::OperationsDoNotAnchor => {
                        "operations_do_not_anchor"
                    }
                    secondbrain_transaction::AbandonedReason::UnrecognizedFileState => {
                        "unrecognized_file_state"
                    }
                }),
                // `AbandonedReason` carries a `Display` written for exactly this
                // line: it says what happened to the operator's data, not which
                // Rust variant matched.
                explanation: Some(reason.to_string()),
            },
        }
    }
}

/// What a recovery pass did.
#[derive(Serialize)]
struct RecoveryReport {
    workspace: String,
    actions: Vec<Action>,
    index_repairs: usize,
    quarantined: usize,
    abandoned: usize,
    index_refreshed: bool,
}

impl Report for RecoveryReport {
    fn render(&self) -> String {
        if self.actions.is_empty() {
            return format!("Checked {}\n  nothing to recover", self.workspace);
        }
        let mut text = format!(
            "Recovered {}\n  {}",
            self.workspace,
            plural(self.actions.len(), "action", "actions")
        );
        for action in &self.actions {
            text.push_str(&format!("\n    {} {}", action.action, action.path));
            if let Some(quarantine) = &action.quarantine_path {
                text.push_str(&format!("\n      journal suffix preserved at {quarantine}"));
            }
            if let Some(explanation) = &action.explanation {
                text.push_str(&format!("\n      {explanation}"));
            }
        }
        if self.index_refreshed {
            text.push_str("\n  the index was rebuilt for the recovered notes");
        }
        if self.abandoned > 0 || self.quarantined > 0 {
            text.push_str(&format!(
                "\n  {} lost, {} set aside — these need a person",
                plural(self.abandoned, "edit", "edits"),
                plural(self.quarantined, "journal", "journals")
            ));
        }
        text
    }
}

/// Runs recovery over `workspace` and reports every action it took.
pub fn check(format: Format, workspace: &Path) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let engine =
        TransactionEngine::new(workspace.root().clone(), workspace.manifest().workspace_id);
    let actions = engine.recover()?;

    let repaired: Vec<NoteId> = actions
        .iter()
        .filter_map(|action| match action {
            RecoveryAction::IndexRepair { note_id, .. } => Some(*note_id),
            _ => None,
        })
        .collect();
    let index_repairs = repaired.len();
    let quarantined = count(&actions, |action| {
        matches!(action, RecoveryAction::Quarantined { .. })
    });
    let abandoned = count(&actions, |action| {
        matches!(action, RecoveryAction::Abandoned { .. })
    });

    // Recovery asks for the repair; performing it is this command's job, and
    // nothing else in the system would. One rebuild covers every note recovery
    // touched, because a rebuild is the only refresh the index crate offers.
    //
    // Recording it is a separate step on purpose. If the rebuild below fails,
    // this returns before any marker is told the repair happened, so the next
    // `recovery check` asks for it again instead of a marker claiming work
    // nobody did.
    let index_refreshed = index_repairs > 0;
    if index_refreshed {
        workspace.index().rebuild()?;
        for note_id in repaired {
            engine.record_index_refreshed(note_id)?;
        }
    }

    let code = if abandoned > 0 || quarantined > 0 {
        DIAGNOSTICS
    } else {
        OK
    };
    emit(
        format,
        &RecoveryReport {
            workspace: workspace.path().display().to_string(),
            actions: actions.iter().map(Action::from).collect(),
            index_repairs,
            quarantined,
            abandoned,
            index_refreshed,
        },
    )?;
    Ok(code)
}

fn count(actions: &[RecoveryAction], predicate: impl Fn(&RecoveryAction) -> bool) -> usize {
    actions.iter().filter(|action| predicate(action)).count()
}
