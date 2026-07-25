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

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId, WorkspaceId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_transaction::TransactionRequest;
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

impl TransactionPlan {
    /// Parses a plan file, rejecting a format this binary does not understand.
    pub fn parse(json: &str) -> Result<Self, CliError> {
        let plan: Self = serde_json::from_str(json)?;
        if plan.format != PLAN_FORMAT {
            return Err(CliError::PlanFormat {
                expected: PLAN_FORMAT,
                found: plan.format,
            });
        }
        Ok(plan)
    }

    /// The transaction this plan asks for, as a fresh attempt attributed to
    /// `actor` on `device`.
    #[must_use]
    pub fn request(&self, actor: ActorId, device: DeviceId) -> TransactionRequest {
        TransactionRequest {
            id: TransactionId::new(),
            actor,
            device,
            note_id: self.note_id,
            path: self.path.clone(),
            expected_hash: self.expected_hash,
            expected_version: self.expected_version,
            operations: self.operations.clone(),
        }
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

/// Whether any operation demands human review.
#[must_use]
pub fn needs_review(operations: &[SemanticOperation]) -> bool {
    operations
        .iter()
        .any(|operation| matches!(operation, SemanticOperation::NeedsReview { .. }))
}
