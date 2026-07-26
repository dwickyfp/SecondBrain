#![forbid(unsafe_code)]

//! Stable workspace diagnostics and recovery orchestration for every frontend.

use std::path::{Path, PathBuf};

use secondbrain_index::{
    IndexConfig, IndexDatabase, IndexHealth as DerivedIndexHealth, ensure_index, index_health,
    index_path, logical_dump,
};
use secondbrain_transaction::{AbandonedReason, RecoveryAction, TransactionEngine};
use secondbrain_vault::{WorkspaceRoot, load_manifest};
use serde::Serialize;
use thiserror::Error;

pub const WORKSPACE_REPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum Error {
    #[error("workspace path failed: {0}")]
    Root(#[from] secondbrain_core::Error),
    #[error("index operation failed: {0}")]
    Index(#[from] secondbrain_index::IndexError),
    #[error("index query failed: {0}")]
    IndexQuery(#[from] secondbrain_index::Error),
    #[error("transaction recovery failed: {0}")]
    Transaction(#[from] secondbrain_transaction::TransactionError),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticIssue {
    pub code: &'static str,
    pub message: String,
    pub action: &'static str,
    pub path: String,
    pub retryable: bool,
    pub review_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexHealth {
    pub status: &'static str,
    pub path: String,
    pub notes: usize,
    pub links: usize,
    pub broken_links: usize,
    pub orphans: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionHealth {
    pub total: usize,
    pub committed: usize,
    pub aborted: usize,
    pub pending: usize,
    pub index_repairs_outstanding: usize,
    pub reviews_pending: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceReport {
    pub format_version: u32,
    pub workspace: String,
    pub workspace_id: String,
    pub status: &'static str,
    pub index: IndexHealth,
    pub transactions: TransactionHealth,
    pub issues: Vec<DiagnosticIssue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
    pub format_version: u32,
    pub workspace: String,
    pub status: &'static str,
    pub actions: Vec<RecoveryReportAction>,
    pub repaired: usize,
    pub quarantined: usize,
    pub abandoned: usize,
    pub review_required: usize,
    pub index_refreshed: bool,
    pub diagnostics: WorkspaceReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReportAction {
    pub code: &'static str,
    pub action: &'static str,
    pub transaction_id: String,
    pub note_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantine_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    pub message: String,
    pub retryable: bool,
    pub review_required: bool,
}

pub fn inspect_workspace(path: &Path) -> Result<WorkspaceReport> {
    let root = WorkspaceRoot::open(path)?;
    let manifest = load_manifest(root.canonical_path())?;
    let engine = TransactionEngine::new(root, manifest.workspace_id);
    let summaries = engine.transactions()?;
    let reviews = engine.pending_reviews()?;
    let pending = summaries.iter().filter(|item| !item.is_terminal()).count();
    let outstanding = summaries
        .iter()
        .filter(|item| item.awaits_index_repair())
        .count();
    let mut issues = Vec::new();
    let index_file = index_path(path);
    let derived_health = index_health(path, &IndexConfig::default())?;
    let index = if derived_health == DerivedIndexHealth::Valid {
        let dump = logical_dump(&index_file)?;
        let database = IndexDatabase::open(&index_file)?;
        IndexHealth {
            status: "ready",
            path: index_file.display().to_string(),
            notes: dump.notes.len(),
            links: dump.links.len(),
            broken_links: database.broken_links()?.len(),
            orphans: database.orphans()?.len(),
        }
    } else {
        let (code, status, message) = match derived_health {
            DerivedIndexHealth::Missing => (
                "SB-INDEX-MISSING",
                "missing",
                "The disposable workspace index is absent.",
            ),
            DerivedIndexHealth::Stale => (
                "SB-INDEX-STALE",
                "stale",
                "The disposable workspace index does not match current Markdown.",
            ),
            DerivedIndexHealth::Invalid => (
                "SB-INDEX-INVALID",
                "invalid",
                "The disposable workspace index is corrupt or uses an unsupported schema.",
            ),
            DerivedIndexHealth::Valid => unreachable!(),
        };
        issues.push(DiagnosticIssue {
            code,
            message: message.into(),
            action: "rebuild_index",
            path: index_file.display().to_string(),
            retryable: true,
            review_required: false,
        });
        IndexHealth {
            status,
            path: index_file.display().to_string(),
            notes: 0,
            links: 0,
            broken_links: 0,
            orphans: 0,
        }
    };
    if pending > 0 {
        issues.push(DiagnosticIssue {
            code: "SB-TXN-STATE",
            message: format!("{pending} transaction(s) were interrupted before completion."),
            action: "run_recovery",
            path: ".secondbrain/transactions".into(),
            retryable: true,
            review_required: false,
        });
    }
    if outstanding > 0 {
        issues.push(DiagnosticIssue {
            code: "SB-INDEX-STALE",
            message: format!("{outstanding} committed transaction(s) still require index refresh."),
            action: "run_recovery",
            path: index_file.display().to_string(),
            retryable: true,
            review_required: false,
        });
    }
    if !reviews.is_empty() {
        issues.push(DiagnosticIssue {
            code: "SB-REVIEW-REQUIRED",
            message: format!("{} change(s) require a human decision.", reviews.len()),
            action: "review_transactions",
            path: ".secondbrain/transactions".into(),
            retryable: false,
            review_required: true,
        });
    }
    Ok(WorkspaceReport {
        format_version: WORKSPACE_REPORT_FORMAT_VERSION,
        workspace: path
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(path))
            .display()
            .to_string(),
        workspace_id: manifest.workspace_id.to_string(),
        status: if issues.is_empty() {
            "healthy"
        } else {
            "attention"
        },
        index,
        transactions: TransactionHealth {
            total: summaries.len(),
            committed: summaries.iter().filter(|item| item.committed()).count(),
            aborted: summaries.iter().filter(|item| item.aborted()).count(),
            pending,
            index_repairs_outstanding: outstanding,
            reviews_pending: reviews.len(),
        },
        issues,
    })
}

pub fn recover_workspace(path: &Path) -> Result<RecoveryReport> {
    let root = WorkspaceRoot::open(path)?;
    let manifest = load_manifest(root.canonical_path())?;
    let engine = TransactionEngine::new(root, manifest.workspace_id);
    let recovered = engine.recover()?;
    let repaired_ids = recovered
        .iter()
        .filter_map(|action| match action {
            RecoveryAction::IndexRepair { note_id, .. } => Some(*note_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let index = ensure_index(path, &IndexConfig::default())?;
    let index_refreshed = index.rebuilt;
    if index_refreshed {
        for note_id in repaired_ids.iter().copied() {
            engine.record_index_refreshed(note_id)?;
        }
    }
    let actions = recovered.iter().map(report_action).collect::<Vec<_>>();
    let repaired = repaired_ids.len();
    let quarantined = actions
        .iter()
        .filter(|item| item.action == "quarantined")
        .count();
    let abandoned = actions
        .iter()
        .filter(|item| item.action == "abandoned")
        .count();
    let diagnostics = inspect_workspace(path)?;
    let review_required = diagnostics.transactions.reviews_pending + quarantined + abandoned;
    Ok(RecoveryReport {
        format_version: WORKSPACE_REPORT_FORMAT_VERSION,
        workspace: diagnostics.workspace.clone(),
        status: if review_required > 0 {
            "review_required"
        } else if actions.is_empty() && !index_refreshed {
            "nothing_to_recover"
        } else {
            "recovered"
        },
        actions,
        repaired,
        quarantined,
        abandoned,
        review_required,
        index_refreshed,
        diagnostics,
    })
}

fn report_action(action: &RecoveryAction) -> RecoveryReportAction {
    match action {
        RecoveryAction::IndexRepair { transaction_id, note_id, path } => RecoveryReportAction {
            code: "SB-INDEX-REPAIRED",
            action: "index_repaired",
            transaction_id: transaction_id.to_string(),
            note_id: note_id.to_string(),
            path: path.to_string(),
            quarantine_path: None,
            reason: None,
            message: "Recovered Markdown was included in a successful index rebuild.".into(),
            retryable: false,
            review_required: false,
        },
        RecoveryAction::Quarantined { transaction_id, note_id, path, quarantine_path } => RecoveryReportAction {
            code: "SB-JOURNAL-QUARANTINED",
            action: "quarantined",
            transaction_id: transaction_id.to_string(),
            note_id: note_id.to_string(),
            path: path.to_string(),
            quarantine_path: Some(quarantine_path.display().to_string()),
            reason: Some("journal_corruption"),
            message: "A damaged journal suffix was preserved for manual review; note bytes were not discarded.".into(),
            retryable: false,
            review_required: true,
        },
        RecoveryAction::Abandoned { transaction_id, note_id, path, reason } => RecoveryReportAction {
            code: "SB-EDIT-ABANDONED",
            action: "abandoned",
            transaction_id: transaction_id.to_string(),
            note_id: note_id.to_string(),
            path: path.to_string(),
            quarantine_path: None,
            reason: Some(match reason {
                AbandonedReason::OperationsDoNotAnchor => "operations_do_not_anchor",
                AbandonedReason::UnrecognizedFileState => "unrecognized_file_state",
            }),
            message: reason.to_string(),
            retryable: false,
            review_required: true,
        },
    }
}
