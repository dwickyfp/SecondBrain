//! `secondbrain doctor` — one report on the health of a workspace.
//!
//! The distinction the report draws is between facts about the *content* — how
//! many links go nowhere, how many notes nothing links to — and facts about the
//! *workspace state*. Only the second kind sets the diagnostics exit code. A
//! vault with broken links is a vault, not a broken workspace, and a `doctor`
//! that failed CI over an author's unwritten note would teach people to ignore
//! it.

use std::path::Path;

use secondbrain_core::id::WorkspaceId;
use secondbrain_transaction::TransactionEngine;
use serde::Serialize;

use crate::exit::{CliError, DIAGNOSTICS, OK};
use crate::output::{Format, Report, emit, plural};
use crate::workspace::Workspace;

/// Something about the workspace that needs a person.
#[derive(Serialize)]
struct Problem {
    code: &'static str,
    message: String,
}

/// What the derived index holds.
#[derive(Serialize)]
struct IndexHealth {
    present: bool,
    path: String,
    notes: usize,
    links: usize,
    broken_links: usize,
    orphans: usize,
}

/// What the transaction journal holds.
#[derive(Serialize)]
struct TransactionHealth {
    total: usize,
    committed: usize,
    aborted: usize,
    pending: usize,
    index_repairs_outstanding: usize,
}

/// The whole diagnostic.
#[derive(Serialize)]
struct DoctorReport {
    workspace: String,
    workspace_id: WorkspaceId,
    format_version: u32,
    index: IndexHealth,
    transactions: TransactionHealth,
    reviews_pending: usize,
    problems: Vec<Problem>,
}

impl Report for DoctorReport {
    fn render(&self) -> String {
        let mut text = format!(
            "Workspace {}\n  workspace id:   {}\n  format version: {}",
            self.workspace, self.workspace_id, self.format_version
        );
        text.push_str(&if self.index.present {
            format!(
                "\n  index:          {}, {}, {} broken, {} orphaned",
                plural(self.index.notes, "note", "notes"),
                plural(self.index.links, "link", "links"),
                self.index.broken_links,
                self.index.orphans
            )
        } else {
            "\n  index:          absent".to_owned()
        });
        text.push_str(&format!(
            "\n  transactions:   {} total, {} committed, {} aborted, {} pending",
            self.transactions.total,
            self.transactions.committed,
            self.transactions.aborted,
            self.transactions.pending
        ));
        text.push_str(&format!(
            "\n  index repairs:  {} outstanding",
            self.transactions.index_repairs_outstanding
        ));
        text.push_str(&format!(
            "\n  reviews:        {} pending",
            self.reviews_pending
        ));
        if self.problems.is_empty() {
            text.push_str("\n  no problems found");
            return text;
        }
        text.push_str(&format!(
            "\n  {} found",
            plural(self.problems.len(), "problem", "problems")
        ));
        for problem in &self.problems {
            text.push_str(&format!("\n    [{}] {}", problem.code, problem.message));
        }
        text
    }
}

/// Reports on the health of `workspace`.
pub fn run(format: Format, workspace: &Path) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let mut problems = Vec::new();
    let index = index_health(&workspace, &mut problems)?;
    let engine =
        TransactionEngine::new(workspace.root().clone(), workspace.manifest().workspace_id);
    let transactions = transaction_health(&engine, &mut problems)?;

    let reviews = engine.pending_reviews()?;
    if !reviews.is_empty() {
        problems.push(Problem {
            code: "SB-TXN-STALE-PRECONDITION",
            message: format!(
                "{} awaiting a decision; see .secondbrain/transactions/",
                plural(reviews.len(), "review", "reviews")
            ),
        });
    }

    let code = if problems.is_empty() { OK } else { DIAGNOSTICS };
    emit(
        format,
        &DoctorReport {
            workspace: workspace.path().display().to_string(),
            workspace_id: workspace.manifest().workspace_id,
            format_version: workspace.manifest().format_version,
            index,
            transactions,
            reviews_pending: reviews.len(),
            problems,
        },
    )?;
    Ok(code)
}

fn index_health(
    workspace: &Workspace,
    problems: &mut Vec<Problem>,
) -> Result<IndexHealth, CliError> {
    let path = secondbrain_index::index_path(workspace.path());
    if !path.exists() {
        problems.push(Problem {
            code: "SB-INDEX-MISSING",
            message: "no derived index; run `secondbrain index rebuild`".to_owned(),
        });
        return Ok(IndexHealth {
            present: false,
            path: path.display().to_string(),
            notes: 0,
            links: 0,
            broken_links: 0,
            orphans: 0,
        });
    }
    let dump = secondbrain_index::logical_dump(&path)?;
    let database = workspace.open_index()?;
    Ok(IndexHealth {
        present: true,
        path: path.display().to_string(),
        notes: dump.notes.len(),
        links: dump.links.len(),
        broken_links: database.broken_links()?.len(),
        orphans: database.orphans()?.len(),
    })
}

fn transaction_health(
    engine: &TransactionEngine,
    problems: &mut Vec<Problem>,
) -> Result<TransactionHealth, CliError> {
    let summaries = engine.transactions()?;
    let pending: Vec<_> = summaries
        .iter()
        .filter(|summary| !summary.is_terminal())
        .collect();
    let outstanding = summaries
        .iter()
        .filter(|summary| summary.awaits_index_repair())
        .count();

    if !pending.is_empty() {
        problems.push(Problem {
            code: "SB-TXN-STATE",
            message: format!(
                "{} interrupted before completing; run `secondbrain recovery check`",
                plural(pending.len(), "transaction", "transactions")
            ),
        });
    }
    if outstanding > 0 {
        problems.push(Problem {
            code: "SB-INDEX-STALE",
            message: format!(
                "{} committed without the index being refreshed; run `secondbrain recovery check`",
                plural(outstanding, "transaction", "transactions")
            ),
        });
    }

    Ok(TransactionHealth {
        total: summaries.len(),
        committed: summaries
            .iter()
            .filter(|summary| summary.state == "COMMITTED")
            .count(),
        aborted: summaries
            .iter()
            .filter(|summary| summary.state == "ABORTED")
            .count(),
        pending: pending.len(),
        index_repairs_outstanding: outstanding,
    })
}
