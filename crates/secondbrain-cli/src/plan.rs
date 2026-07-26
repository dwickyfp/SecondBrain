//! The transaction plan that `diff` writes and `transaction apply` executes.
//!
//! The two commands are deliberately separate: `diff` derives the semantic
//! operations an incoming file implies and mutates nothing, and `transaction
//! apply` takes a plan an operator has seen and applies it. A plan is therefore
//! a file that crosses between two processes, which makes its shape a contract
//! and not an implementation detail — hence the explicit `format` tag and the
//! recorded preconditions.
//!
//! A plan carries no transaction id. The id names one attempt to apply the
//! plan, and it is minted at apply time, so that re-applying a plan can never
//! reuse the identity of an attempt that already happened.

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, WorkspaceId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::operation::SemanticOperation;
use serde::{Deserialize, Serialize};

use crate::exit::CliError;
use crate::output::{Report, plural};

/// The only plan format this binary understands.
pub const PLAN_FORMAT: &str = "sb-transaction-plan-v1";

/// A previewed change to one note.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionPlan {
    /// The plan format, checked on load so an older binary refuses a newer plan
    /// instead of misreading it.
    pub format: String,
    /// The workspace the plan was derived from, checked on apply so a plan
    /// cannot be applied to a workspace whose notes it never saw.
    pub workspace_id: WorkspaceId,
    /// The note the plan edits.
    pub note_id: NoteId,
    /// The note's path.
    pub path: WorkspacePath,
    /// The note content the operations were derived from. The apply refuses if
    /// the file no longer hashes to this.
    pub expected_hash: ContentHash,
    /// The note version the operations were derived from.
    pub expected_version: NoteVersion,
    /// Whether the change is ambiguous. A plan flagged this way describes what
    /// the diff could not decide; it is not something to apply.
    pub review_required: bool,
    /// The semantic operations the change consists of.
    pub operations: Vec<SemanticOperation>,
}

/// The format tag alone, read before the rest of a plan file.
///
/// Unknown fields are ignored, which is the point: a plan from a newer binary
/// may carry fields this one has never heard of, and the format question has to
/// be answerable without them.
#[derive(Deserialize)]
struct PlanFormatTag {
    format: String,
}

impl TransactionPlan {
    /// Parses a plan file, rejecting a format this binary does not understand.
    ///
    /// The format is read first, on its own. Deserializing the whole plan and
    /// checking the tag afterwards would decide the format question by whether
    /// every *other* field happened to parse — so a `sb-transaction-plan-v2`
    /// file with one new required field would be reported as unreadable JSON
    /// rather than as the newer format it announces, which is the one thing the
    /// tag exists to prevent.
    pub fn parse(json: &str) -> Result<Self, CliError> {
        let tag: PlanFormatTag =
            serde_json::from_str(json).map_err(|source| CliError::PlanUnreadable { source })?;
        if tag.format != PLAN_FORMAT {
            return Err(CliError::PlanFormat {
                expected: PLAN_FORMAT,
                found: tag.format,
            });
        }
        serde_json::from_str(json).map_err(|source| CliError::PlanUnreadable { source })
    }

    /// A one-line summary of what the plan would do, per operation kind.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.operations.is_empty() {
            return "no semantic change".to_owned();
        }
        let mut kinds: Vec<&str> = self
            .operations
            .iter()
            .map(SemanticOperation::kind_name)
            .collect();
        kinds.sort_unstable();
        let mut counted = Vec::new();
        for kind in kinds.chunk_by(|left, right| left == right) {
            counted.push(format!("{} x{}", kind[0], kind.len()));
        }
        counted.join(", ")
    }
}

impl Report for TransactionPlan {
    fn render(&self) -> String {
        let mut text = format!(
            "Plan for {}\n  note id:          {}\n  expected hash:    {}\n  expected version: {}\n  {}: {}",
            self.path,
            self.note_id,
            self.expected_hash,
            self.expected_version.get(),
            plural(self.operations.len(), "operation", "operations"),
            self.summary()
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
