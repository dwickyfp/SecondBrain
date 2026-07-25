//! The exit-code contract and the failures that produce it.
//!
//! Exit codes are a public interface: an operator scripting against this binary
//! branches on them, so they are named here once and restated in `README.md`
//! and in `tests/cli.rs` rather than being whatever a `match` arm happened to
//! return. The distinction that matters most is between [`REVIEW_REQUIRED`] and
//! [`FAILED`] — "a human has to decide this" is not "the tool broke", and a
//! script that cannot tell them apart will either page someone for a routine
//! ambiguity or silently drop one.

use std::io;
use std::path::PathBuf;

use secondbrain_core::actor::IdentityError;
use secondbrain_core::id::WorkspaceId;
use secondbrain_core::path::WorkspacePathError;
use thiserror::Error;

/// The command completed and nothing needs attention.
pub const OK: u8 = 0;
/// The command could not complete.
pub const FAILED: u8 = 1;
/// The command line itself was invalid. Emitted by `clap`, whose default this
/// matches deliberately.
pub const USAGE: u8 = 2;
/// The change is ambiguous and a human must decide what it meant.
pub const REVIEW_REQUIRED: u8 = 3;
/// The command completed and reported problems with the workspace.
pub const DIAGNOSTICS: u8 = 4;

/// A failure that ends a command.
///
/// Every variant carries a stable `SB-*` diagnostic code so that callers branch
/// on [`CliError::code`] rather than on display text, which is the same rule
/// [`secondbrain_core::Error`] states for the library taxonomy.
#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Core(#[from] secondbrain_core::Error),
    #[error("{0}")]
    Index(#[from] secondbrain_index::IndexError),
    #[error("{0}")]
    IndexQuery(#[from] secondbrain_index::Error),
    #[error("{0}")]
    Transaction(#[from] secondbrain_transaction::TransactionError),
    #[error("{0}")]
    Snapshot(#[from] secondbrain_transaction::SnapshotError),
    #[error("{0}")]
    Markdown(#[from] secondbrain_markdown::parse::ParseError),
    #[error("invalid actor or device identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("JSON handling failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not {operation} {}: {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("no index at {}; run `secondbrain index rebuild` first", .0.display())]
    IndexMissing(PathBuf),
    #[error("note {0} is not in the index; run `secondbrain index rebuild` first")]
    NoteNotIndexed(String),
    #[error("plan declares format {found}, not {expected}")]
    PlanFormat {
        expected: &'static str,
        found: String,
    },
    #[error("plan belongs to workspace {plan}, but this workspace is {workspace}")]
    PlanWorkspace {
        plan: WorkspaceId,
        workspace: WorkspaceId,
    },
    #[error("this change is ambiguous and needs review: {0}")]
    ReviewRequired(String),
}

impl CliError {
    /// The stable diagnostic code for this failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Core(error) => error.code(),
            Self::Index(_) | Self::IndexQuery(_) => "SB-INDEX",
            Self::Transaction(_) | Self::Snapshot(_) => "SB-TXN",
            Self::Markdown(_) => "SB-MD-INVALID",
            Self::Identity(_) => "SB-ID-INVALID",
            Self::Json(_) => "SB-STORE-CORRUPT",
            Self::Io { .. } => "SB-IO",
            Self::IndexMissing(_) | Self::NoteNotIndexed(_) => "SB-INDEX-MISSING",
            Self::PlanFormat { .. } | Self::PlanWorkspace { .. } => "SB-PLAN-INVALID",
            Self::ReviewRequired(_) => "SB-REVIEW-REQUIRED",
        }
    }

    /// The exit code this failure ends the process with.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::ReviewRequired(_) => REVIEW_REQUIRED,
            _ => FAILED,
        }
    }
}

/// A path an operator typed is rejected through the core taxonomy, so that the
/// diagnostic code is the same one the library would have produced.
impl From<WorkspacePathError> for CliError {
    fn from(source: WorkspacePathError) -> Self {
        Self::Core(source.into())
    }
}

/// Reads a file, naming the operation and the path in any failure.
pub fn read_file(operation: &'static str, path: &std::path::Path) -> Result<String, CliError> {
    std::fs::read_to_string(path).map_err(|source| CliError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })
}
