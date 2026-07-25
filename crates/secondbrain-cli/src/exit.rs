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
    ExternalEdit(#[from] secondbrain_transaction::ExternalEditError),
    #[error("{0}")]
    Markdown(#[from] secondbrain_markdown::parse::ParseError),
    #[error("invalid actor or device identity: {0}")]
    Identity(#[from] IdentityError),
    /// This binary could not serialize something it was about to print.
    ///
    /// Deliberately distinct from a plan that would not parse: one is this
    /// tool failing, the other is a file an operator wrote.
    #[error("could not encode output as JSON: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("could not {operation} {}: {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("no index at {}; run `secondbrain index rebuild` first", .0.display())]
    IndexMissing(PathBuf),
    #[error("plan file is not valid JSON: {source}")]
    PlanUnreadable {
        #[source]
        source: serde_json::Error,
    },
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
            // One code for the transaction layer's failures, whichever of its
            // error types carried them: an operator branching on this has the
            // same work to do either way, and the message says which.
            Self::Transaction(_) | Self::Snapshot(_) | Self::ExternalEdit(_) => "SB-TXN",
            Self::Markdown(_) => "SB-MD-INVALID",
            Self::Identity(_) => "SB-ID-INVALID",
            Self::Encode(_) => "SB-OUTPUT-ENCODE",
            Self::Io { .. } => "SB-IO",
            // The index being absent is this binary's problem to name: it is
            // the one who decided not to create an empty one. A note missing
            // from an index that answered is the domain's, and is named in
            // `secondbrain_core::Error` so every surface reports it alike.
            Self::IndexMissing(_) => "SB-INDEX-MISSING",
            Self::PlanUnreadable { .. } | Self::PlanFormat { .. } | Self::PlanWorkspace { .. } => {
                "SB-PLAN-INVALID"
            }
            Self::ReviewRequired(_) => "SB-REVIEW-REQUIRED",
        }
    }

    /// The exit code this failure ends the process with.
    ///
    /// A note that diverged from its converged base ends at [`REVIEW_REQUIRED`]
    /// rather than [`FAILED`] for the reason this module opens with: a person
    /// has to decide what the unjournaled edit meant, and a vault that is ahead
    /// of its journal is not a broken tool.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::ReviewRequired(_) | Self::Core(secondbrain_core::Error::NoteDiverged { .. }) => {
                REVIEW_REQUIRED
            }
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

#[cfg(test)]
mod tests {
    use secondbrain_core::id::{NoteId, NoteVersion};

    use super::*;

    fn malformed_json() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").expect_err("fixture must be invalid")
    }

    /// Every `CliError` variant beside the exact code it must keep answering.
    ///
    /// Written the way `secondbrain-core`'s `error_contract.rs` is written, and
    /// for the same reason: an operator scripts against these codes, so a
    /// variant that quietly changed which one it answers would break a caller
    /// that never saw a compiler error.
    fn every_variant() -> Vec<(CliError, &'static str)> {
        vec![
            (
                CliError::Core(secondbrain_core::Error::CorruptRecord {
                    record: "note:01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                    summary: "content hash mismatch".into(),
                }),
                "SB-STORE-CORRUPT",
            ),
            (
                CliError::Index(secondbrain_index::IndexError::MalformedNote {
                    path: "notes/broken.md".into(),
                    message: "unterminated front matter".into(),
                }),
                "SB-INDEX",
            ),
            (
                CliError::IndexQuery(secondbrain_index::Error::InvalidStoredNoteId {
                    value: "not-a-ulid".into(),
                }),
                "SB-INDEX",
            ),
            (
                CliError::Transaction(secondbrain_transaction::TransactionError::VersionOverflow),
                "SB-TXN",
            ),
            (
                CliError::Snapshot(secondbrain_transaction::SnapshotError::UnsupportedFormat {
                    note_id: NoteId::new(),
                    format: "sb-base-snapshot-v2".into(),
                }),
                "SB-TXN",
            ),
            (
                CliError::ExternalEdit(secondbrain_transaction::ExternalEditError::Transaction(
                    secondbrain_transaction::TransactionError::VersionOverflow,
                )),
                "SB-TXN",
            ),
            (
                CliError::Markdown(secondbrain_markdown::parse::ParseError::UpstreamPanic(
                    "the parser gave up".into(),
                )),
                "SB-MD-INVALID",
            ),
            (
                CliError::Identity(
                    secondbrain_core::actor::ActorId::new("")
                        .expect_err("an empty actor is invalid"),
                ),
                "SB-ID-INVALID",
            ),
            (CliError::Encode(malformed_json()), "SB-OUTPUT-ENCODE"),
            (
                CliError::Io {
                    operation: "read note",
                    path: PathBuf::from("notes/alpha.md"),
                    source: io::Error::new(io::ErrorKind::PermissionDenied, "private contents"),
                },
                "SB-IO",
            ),
            (
                CliError::IndexMissing(PathBuf::from(".secondbrain/index.sqlite")),
                "SB-INDEX-MISSING",
            ),
            (
                CliError::Core(secondbrain_core::Error::NoteNotIndexed {
                    path: PathBuf::from("notes/alpha.md"),
                }),
                "SB-NOTE-NOT-INDEXED",
            ),
            (
                CliError::PlanUnreadable {
                    source: malformed_json(),
                },
                "SB-PLAN-INVALID",
            ),
            (
                CliError::PlanFormat {
                    expected: "sb-transaction-plan-v1",
                    found: "sb-transaction-plan-v2".into(),
                },
                "SB-PLAN-INVALID",
            ),
            (
                CliError::PlanWorkspace {
                    plan: WorkspaceId::new(),
                    workspace: WorkspaceId::new(),
                },
                "SB-PLAN-INVALID",
            ),
            (
                CliError::ReviewRequired("two paragraphs are identical".into()),
                "SB-REVIEW-REQUIRED",
            ),
            (
                CliError::Core(secondbrain_core::Error::NoteDiverged {
                    path: PathBuf::from("notes/alpha.md"),
                    version: NoteVersion::new(1),
                }),
                "SB-NOTE-DIVERGED",
            ),
        ]
    }

    #[test]
    fn every_variant_has_its_exact_stable_code() {
        for (error, expected) in every_variant() {
            assert_eq!(error.code(), expected, "variant: {error:?}");
        }
    }

    #[test]
    fn every_code_is_in_the_shared_namespace() {
        for (error, _) in every_variant() {
            let code = error.code();
            assert!(
                code.starts_with("SB-") && code.to_uppercase() == code,
                "codes this binary invents share one namespace with the library's: {code}"
            );
        }
    }

    #[test]
    fn only_the_failures_a_person_must_decide_use_the_review_exit_code() {
        for (error, _) in every_variant() {
            let expected = match error {
                CliError::ReviewRequired(_)
                | CliError::Core(secondbrain_core::Error::NoteDiverged { .. }) => REVIEW_REQUIRED,
                _ => FAILED,
            };
            assert_eq!(
                error.exit_code(),
                expected,
                "`needs a human` and `broke` must stay tellable apart: {error:?}"
            );
        }
    }

    #[test]
    fn a_plan_that_is_not_json_is_not_reported_as_store_corruption() {
        // An operator hand-editing a plan file has produced a bad plan, not a
        // corrupt store: the store is what this workspace wrote for itself.
        let error = CliError::PlanUnreadable {
            source: malformed_json(),
        };

        assert_ne!(error.code(), "SB-STORE-CORRUPT");
        assert_eq!(error.code(), "SB-PLAN-INVALID");
    }

    #[test]
    fn a_note_absent_from_a_present_index_does_not_claim_the_index_is_missing() {
        assert_ne!(
            CliError::Core(secondbrain_core::Error::NoteNotIndexed {
                path: PathBuf::from("notes/alpha.md"),
            })
            .code(),
            CliError::IndexMissing(PathBuf::from(".secondbrain/index.sqlite")).code(),
            "one of these is fixed by a rebuild of an index that exists"
        );
    }

    /// The two conditions that are the domain's rather than this binary's.
    ///
    /// A note that diverged from its converged base and a note absent from the
    /// derived index are facts about a workspace, not about a command line.
    /// They are defined once in `secondbrain_core::Error` so the MCP server and
    /// the local API report the codes this binary reports, instead of each
    /// surface minting its own name for the same condition.
    #[test]
    fn the_shared_conditions_answer_the_taxonomy_code_rather_than_a_local_one() {
        for (error, code) in [
            (
                secondbrain_core::Error::NoteDiverged {
                    path: PathBuf::from("notes/alpha.md"),
                    version: NoteVersion::new(1),
                },
                "SB-NOTE-DIVERGED",
            ),
            (
                secondbrain_core::Error::NoteNotIndexed {
                    path: PathBuf::from("notes/alpha.md"),
                },
                "SB-NOTE-NOT-INDEXED",
            ),
        ] {
            assert_eq!(error.code(), code);
            assert_eq!(
                CliError::Core(error).code(),
                code,
                "the CLI must report the taxonomy's code, not one of its own"
            );
        }
    }
}
