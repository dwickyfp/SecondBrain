//! Turning normalized external filesystem changes into attributed transactions.
//!
//! [`ExternalEditCoordinator`] is the bridge between the watcher's
//! [`WorkspaceEvent`]s and the transaction engine. It owns no write logic of
//! its own: every byte that reaches a note goes through
//! [`TransactionEngine::commit`], and every attribution goes through
//! [`TransactionEngine::adopt_external`].
//!
//! # What an external edit is
//!
//! An external editor rewrites the whole file, so the new content is already on
//! disk before the workspace hears about it. The coordinator therefore reads
//! the file, diffs it against the note's last converged base
//! ([`crate::base_snapshot`]) to recover the *semantic* operations the editor
//! performed, and journals those operations without rewriting the file.
//!
//! # Convergence
//!
//! Because the edit is already materialized, the only thing that can still be
//! lost is a workspace change the editor's whole-file write clobbered. When a
//! note has operations that are journaled but never materialized — a crash
//! between the durable oplog append and the Markdown write — the coordinator
//! rebases them onto the external content, so both changes survive instead of
//! recovery having to abandon one of them. Operations that no longer anchor are
//! left to recovery rather than guessed at.

use std::fs;
use std::io;
use std::path::PathBuf;

use secondbrain_core::actor::{ActorId, DeviceId, IdentityError};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId, WorkspaceId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::apply::{apply_operations, review_reason_summary};
use secondbrain_markdown::diff::diff_documents;
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_markdown::parse::ParseError;
use secondbrain_markdown::{Fingerprint, SourceDocument};
use secondbrain_vault::event::WorkspaceEvent;
use secondbrain_vault::{IdentityMap, RecoveryOutcome, WorkspaceRoot};
use serde::Serialize;
use thiserror::Error;

use crate::base_snapshot::{BaseSnapshot, BaseSnapshotStore, SnapshotError};
use crate::engine::{TransactionEngine, TransactionError, TransactionRequest};
use crate::failpoint;
use crate::oplog::{LocalMutationLog, OplogError};
use crate::paths;
use crate::record::LocalOperationRecord;

/// The version a note's converged base starts at, before any transaction.
const GENESIS_VERSION: NoteVersion = NoteVersion::new(0);

/// The review descriptor format label.
const REVIEW_FORMAT: &str = "sb-external-review-v1";

/// Refreshes derived index state for one note.
///
/// The transaction crate must not depend on `secondbrain-index` — which today
/// offers only a full rebuild — so the coordinator states what it needs and the
/// CLI wires an implementation.
pub trait IndexRefresh {
    /// Refreshes the derived state of `note_id`, now living at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the derived state could not be refreshed. The
    /// transaction stays committed: the Markdown is the source of truth and a
    /// rebuild can always recreate the index.
    fn refresh(&self, note_id: NoteId, path: &WorkspacePath)
    -> Result<(), secondbrain_core::Error>;
}

/// What integrating one external change did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExternalEditOutcome {
    /// A file the workspace had not tracked was given an identity, and its
    /// current content was recorded as the base future edits are diffed
    /// against. Nothing was written or journaled.
    Registered {
        /// The identity assigned to the file.
        note_id: NoteId,
    },
    /// A note the workspace already tracks had no converged base to diff
    /// against, so the edit that prompted this could not be recovered. The
    /// content now on disk was recorded as the base instead; nothing was
    /// written or journaled, and whatever the edit changed is unattributed.
    ///
    /// This is not [`Self::Registered`]: the note was known, so a base was
    /// expected. It means the note predates this pipeline, or its snapshot was
    /// lost — the latter being data loss worth surfacing rather than absorbing.
    BaseRecovered {
        /// The note whose base had to be reconstructed from the current file.
        note_id: NoteId,
    },
    /// The file already held the note's converged base. Nothing to do.
    Unchanged {
        /// The note the file belongs to.
        note_id: NoteId,
    },
    /// The external edit was journaled as an attributed transaction. The file
    /// itself was not rewritten — it already held the result.
    Adopted {
        /// The note that was edited.
        note_id: NoteId,
        /// The transaction the edit was journaled as.
        transaction_id: TransactionId,
        /// The version the note converged at.
        version: NoteVersion,
    },
    /// The external edit was journaled, and operations the edit had clobbered
    /// were rebased onto it and materialized, so both changes survive.
    Merged {
        /// The note that was edited.
        note_id: NoteId,
        /// The transaction whose materialization produced [`Self::Merged::source_hash`].
        transaction_id: TransactionId,
        /// The version the note converged at.
        version: NoteVersion,
        /// Hash of the merged Markdown, for the watcher's internal-write receipt.
        source_hash: ContentHash,
    },
    /// The change was ambiguous. A review descriptor was written and the file
    /// was left exactly as the external editor wrote it.
    ReviewRequired {
        /// The transaction the review is filed under.
        transaction_id: TransactionId,
        /// The descriptor describing what needs review.
        descriptor: PathBuf,
    },
    /// A note moved. Its identity and converged base followed it; no bytes
    /// changed, so there was nothing to journal.
    Renamed {
        /// The note that moved.
        note_id: NoteId,
        /// Its new path.
        path: WorkspacePath,
    },
    /// A file was found to be a copy of a tracked note and was given an
    /// identity of its own.
    Copied {
        /// The identity assigned to the copy.
        note_id: NoteId,
        /// The note the copy was made from.
        source_note_id: NoteId,
    },
    /// A file was removed. Phase 0 has no delete transaction, so the deletion
    /// is reported rather than acted on; identity and converged base are kept
    /// so the note is recovered if the file comes back.
    Deleted {
        /// The path that disappeared.
        path: WorkspacePath,
    },
}

/// Why an external change could not be integrated.
#[derive(Debug, Error)]
pub enum ExternalEditError {
    #[error("external edit I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("workspace operation failed: {0}")]
    Vault(#[from] secondbrain_core::Error),
    #[error("external edit could not be parsed: {0}")]
    Parse(#[from] ParseError),
    #[error("transaction failed: {0}")]
    Transaction(#[from] TransactionError),
    #[error("operation log failed: {0}")]
    Oplog(#[from] OplogError),
    #[error("converged base failed: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("external actor identity rejected: {0}")]
    Actor(#[from] IdentityError),
    #[error("review descriptor serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Integrates external filesystem changes into one workspace.
pub struct ExternalEditCoordinator<R: IndexRefresh> {
    workspace: WorkspaceRoot,
    engine: TransactionEngine,
    identity: IdentityMap,
    bases: BaseSnapshotStore,
    actor: ActorId,
    device: DeviceId,
    index: R,
}

impl<R: IndexRefresh> ExternalEditCoordinator<R> {
    /// Opens a coordinator that attributes every edit it sees to `device`.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity map cannot be opened or if `device`
    /// does not form a valid `external:<device>` actor.
    pub fn new(
        workspace: WorkspaceRoot,
        workspace_id: WorkspaceId,
        device: DeviceId,
        index: R,
    ) -> Result<Self, ExternalEditError> {
        let actor = ActorId::new(format!("external:{device}"))?;
        Ok(Self {
            engine: TransactionEngine::new(workspace.clone(), workspace_id),
            identity: IdentityMap::open(&workspace)?,
            bases: BaseSnapshotStore::new(&workspace),
            workspace,
            actor,
            device,
            index,
        })
    }

    /// Integrates one normalized workspace event.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed, if the identity
    /// map or converged base cannot be updated, or if the transaction engine
    /// refuses the derived transaction.
    pub fn integrate(
        &mut self,
        event: WorkspaceEvent,
    ) -> Result<ExternalEditOutcome, ExternalEditError> {
        match event {
            WorkspaceEvent::ContentChanged { path, .. } => self.integrate_content(path),
            WorkspaceEvent::Renamed { from, to } => self.integrate_rename(&from, to),
            WorkspaceEvent::Deleted { path } => Ok(ExternalEditOutcome::Deleted { path }),
        }
    }

    /// Integrates changed bytes at `path`.
    ///
    /// The event's hash is deliberately ignored: by the time an event is
    /// handled the file may have been written again, so the content on disk is
    /// the only trustworthy input and the operations are derived from it.
    fn integrate_content(
        &mut self,
        path: WorkspacePath,
    ) -> Result<ExternalEditOutcome, ExternalEditError> {
        let source = self.read_note(&path)?;
        let document = SourceDocument::parse(&source)?;
        let source_hash = ContentHash::digest(source.as_bytes());

        let note_id =
            match self.identify(&path, &path, source_hash, document.semantic_fingerprint())? {
                NoteIdentity::Tracked(note_id) => note_id,
                NoteIdentity::Fresh(note_id) => {
                    return self.register_base(note_id, &path, &source);
                }
                NoteIdentity::Copy {
                    note_id,
                    source_note_id,
                } => {
                    return self.register_copy(note_id, source_note_id, &path, &source);
                }
                NoteIdentity::Ambiguous(outcome) => return Ok(outcome),
            };

        let Some(base) = self.bases.load(note_id)? else {
            // The note has an identity but no converged base — it was
            // registered before this pipeline saw it, or its record was lost.
            // Its current content is the only base we can honestly claim to
            // have agreed on, and the edit that brought us here is gone.
            self.converge(note_id, &path, GENESIS_VERSION, &source)?;
            return Ok(ExternalEditOutcome::BaseRecovered { note_id });
        };
        if base.source_hash == source_hash {
            // Nothing changed since the workspace last converged this note.
            // This is also what the workspace's own materializations look like
            // if their filesystem event is not suppressed before it arrives.
            return Ok(ExternalEditOutcome::Unchanged { note_id });
        }

        let operations = diff_documents(&SourceDocument::parse(&base.source)?, &document);
        if let Some(reason) = review_reason(&operations) {
            return self.require_review(Some(note_id), &path, source_hash, reason, Vec::new());
        }
        if operations.is_empty() {
            // The bytes changed but nothing semantic did — a formatter touching
            // whitespace, for instance. Converge the base and journal nothing.
            self.converge(note_id, &path, base.version, &source)?;
            return Ok(ExternalEditOutcome::Unchanged { note_id });
        }

        let adoption = self.request(note_id, &path, source_hash, base.version, operations);
        let transaction_id = adoption.id;
        let adopted = self.engine.adopt_external(adoption, &base.source)?;
        let outcome = match self.rebase_pending(note_id, &path, adopted.version)? {
            Some(merge) => ExternalEditOutcome::Merged {
                note_id,
                transaction_id: merge.transaction_id,
                version: merge.version,
                source_hash: merge.source_hash,
            },
            None => ExternalEditOutcome::Adopted {
                note_id,
                transaction_id,
                version: adopted.version,
            },
        };
        self.index.refresh(note_id, &path)?;
        Ok(outcome)
    }

    /// Integrates a move: the identity and converged base follow the bytes.
    fn integrate_rename(
        &mut self,
        from: &WorkspacePath,
        to: WorkspacePath,
    ) -> Result<ExternalEditOutcome, ExternalEditError> {
        let source = self.read_note(&to)?;
        let document = SourceDocument::parse(&source)?;
        let source_hash = ContentHash::digest(source.as_bytes());

        // The note is looked up under the path it moved away from, which is how
        // the identity map recognizes it; the record is then written for its
        // new path.
        match self.identify(from, &to, source_hash, document.semantic_fingerprint())? {
            NoteIdentity::Tracked(note_id) => {
                self.identity.update_path(&note_id, &to)?;
                match self.bases.load(note_id)? {
                    // A move on its own changes no bytes, so the converged
                    // base only follows the note to its new path.
                    Some(base) if base.source_hash == source_hash => {
                        self.bases.save(note_id, &to, base.version, &base.source)?;
                        self.index.refresh(note_id, &to)?;
                        Ok(ExternalEditOutcome::Renamed { note_id, path: to })
                    }
                    // The move also changed the bytes. Re-filing the pre-move
                    // source under the new path would leave a base that does
                    // not describe the file its own record points at, and the
                    // change would stay unattributed until some later event
                    // re-derived it. The identity has moved, so the content is
                    // now integrated exactly as a `ContentChanged` at the new
                    // path would be — including the missing-base case, which
                    // reports itself rather than being absorbed here.
                    _ => self.integrate_content(to),
                }
            }
            NoteIdentity::Fresh(note_id) => self.register_base(note_id, &to, &source),
            NoteIdentity::Copy {
                note_id,
                source_note_id,
            } => self.register_copy(note_id, source_note_id, &to, &source),
            NoteIdentity::Ambiguous(outcome) => Ok(outcome),
        }
    }

    /// Resolves which note a file belongs to, assigning an identity when the
    /// file is new to the workspace.
    ///
    /// `lookup` is the path the identity map is asked about and `record` is the
    /// path a new identity is filed under; they differ only for a rename.
    fn identify(
        &mut self,
        lookup: &WorkspacePath,
        record: &WorkspacePath,
        source_hash: ContentHash,
        fingerprint: Fingerprint,
    ) -> Result<NoteIdentity, ExternalEditError> {
        match self
            .identity
            .resolve_identity(lookup, source_hash, fingerprint)?
        {
            RecoveryOutcome::Resolved(note_id) => Ok(NoteIdentity::Tracked(note_id)),
            RecoveryOutcome::New => Ok(NoteIdentity::Fresh(self.identity.register(
                record,
                source_hash,
                fingerprint,
            )?)),
            RecoveryOutcome::Duplicate { existing_id, .. } => Ok(NoteIdentity::Copy {
                note_id: self.identity.register(record, source_hash, fingerprint)?,
                source_note_id: existing_id,
            }),
            RecoveryOutcome::NeedsReview { candidates } => {
                let outcome = self.require_review(
                    None,
                    record,
                    source_hash,
                    "ambiguous note identity: two or more notes match this file".to_owned(),
                    candidates,
                )?;
                Ok(NoteIdentity::Ambiguous(outcome))
            }
        }
    }

    /// Rebases operations that are journaled but never materialized onto the
    /// content now on disk, so an external whole-file write cannot silently
    /// discard them.
    fn rebase_pending(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        version: NoteVersion,
    ) -> Result<Option<Merge>, ExternalEditError> {
        let pending = self.engine.pending_transactions(note_id)?;
        if pending.is_empty() {
            return Ok(None);
        }
        let replay = LocalMutationLog::open(self.workspace.canonical_path(), note_id)?.replay()?;
        if replay.corruption.is_some() {
            // A damaged journal is recovery's business, not the coordinator's.
            return Ok(None);
        }

        let mut merge: Option<Merge> = None;
        let mut version = version;
        for transaction_id in pending {
            let operations = operations_of(&replay.records, transaction_id);
            if operations.is_empty() {
                continue;
            }
            let source = self.read_note(path)?;
            // Only operations that still anchor in the external content can be
            // rebased. Anything else is a real conflict, and recovery preserves
            // the file rather than guessing which text was meant.
            let Ok(rebased) = apply_operations(&source, &operations) else {
                continue;
            };
            if rebased == source {
                continue;
            }
            let attribution = replay
                .records
                .iter()
                .find(|record| record.transaction_id == transaction_id)
                .map(|record| (record.actor_id.clone(), record.device_id.clone()));
            let Some(attribution) = attribution else {
                continue;
            };
            let mut request = self.request(
                note_id,
                path,
                ContentHash::digest(source.as_bytes()),
                version,
                operations,
            );
            // A rebase carries the attribution its operations were journaled
            // with: the change is the workspace's own, not the editor's.
            (request.actor, request.device) = attribution;
            let rebase_id = request.id;
            let outcome = self.engine.commit(request)?;
            failpoint::hit("after_rebase_before_supersede")?;
            // The rebase is durable before the transaction it carries forward
            // is closed out. `OPERATIONS_DURABLE` is the promise that recovery
            // will finish an edit, so retracting it first would leave a crash
            // window in which the operations are aborted here and never
            // journaled there — reachable from neither end. In the other
            // order a crash leaves the original marker untouched, and
            // recovery closes it out through the state machine it owns:
            // operations that no longer anchor in the file on disk abort, and
            // the file is preserved rather than guessed at.
            self.engine.supersede_transaction(transaction_id)?;
            version = outcome.version;
            merge = Some(Merge {
                transaction_id: rebase_id,
                version,
                source_hash: ContentHash::digest(rebased.as_bytes()),
            });
        }
        Ok(merge)
    }

    /// Files a review descriptor next to the transaction markers and leaves the
    /// file on disk untouched.
    fn require_review(
        &self,
        note_id: Option<NoteId>,
        path: &WorkspacePath,
        source_hash: ContentHash,
        reason: String,
        identity_candidates: Vec<NoteId>,
    ) -> Result<ExternalEditOutcome, ExternalEditError> {
        let transaction_id = TransactionId::new();
        let descriptor = ReviewDescriptor {
            format: REVIEW_FORMAT,
            transaction_id,
            note_id,
            path: path.clone(),
            actor: &self.actor,
            device: &self.device,
            reason,
            source_hash,
            identity_candidates,
        };
        let root = self.workspace.canonical_path();
        fs::create_dir_all(paths::transactions_dir(root))?;
        let target = paths::review_descriptor_path(root, transaction_id);
        secondbrain_vault::atomic_write::atomic_write(
            &target,
            &serde_json::to_vec_pretty(&descriptor)?,
        )?;
        Ok(ExternalEditOutcome::ReviewRequired {
            transaction_id,
            descriptor: target,
        })
    }

    /// Records a file's current content as the base future edits are diffed
    /// against, without writing or journaling anything.
    fn register_base(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        source: &str,
    ) -> Result<ExternalEditOutcome, ExternalEditError> {
        self.converge(note_id, path, GENESIS_VERSION, source)?;
        Ok(ExternalEditOutcome::Registered { note_id })
    }

    /// Gives a copy of a tracked note an identity and a base of its own.
    fn register_copy(
        &self,
        note_id: NoteId,
        source_note_id: NoteId,
        path: &WorkspacePath,
        source: &str,
    ) -> Result<ExternalEditOutcome, ExternalEditError> {
        self.converge(note_id, path, GENESIS_VERSION, source)?;
        Ok(ExternalEditOutcome::Copied {
            note_id,
            source_note_id,
        })
    }

    fn converge(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        version: NoteVersion,
        source: &str,
    ) -> Result<BaseSnapshot, ExternalEditError> {
        let snapshot = self.bases.save(note_id, path, version, source)?;
        self.index.refresh(note_id, path)?;
        Ok(snapshot)
    }

    fn request(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        expected_hash: ContentHash,
        expected_version: NoteVersion,
        operations: Vec<SemanticOperation>,
    ) -> TransactionRequest {
        TransactionRequest {
            id: TransactionId::new(),
            actor: self.actor.clone(),
            device: self.device.clone(),
            note_id,
            path: path.clone(),
            expected_hash,
            expected_version,
            operations,
        }
    }

    fn read_note(&self, path: &WorkspacePath) -> Result<String, ExternalEditError> {
        Ok(fs::read_to_string(self.workspace.resolve(path)?)?)
    }
}

/// Which note a changed file belongs to.
enum NoteIdentity {
    /// A note the workspace already tracks.
    Tracked(NoteId),
    /// A file the workspace had not seen; a new identity was assigned.
    Fresh(NoteId),
    /// A copy of a tracked note; an identity of its own was assigned.
    Copy {
        note_id: NoteId,
        source_note_id: NoteId,
    },
    /// Identity could not be resolved; a review descriptor was written.
    Ambiguous(ExternalEditOutcome),
}

/// A materialized rebase of operations an external write had clobbered.
struct Merge {
    transaction_id: TransactionId,
    version: NoteVersion,
    source_hash: ContentHash,
}

/// What a review descriptor records for the human who resolves it.
#[derive(Serialize)]
struct ReviewDescriptor<'a> {
    format: &'static str,
    transaction_id: TransactionId,
    note_id: Option<NoteId>,
    path: WorkspacePath,
    actor: &'a ActorId,
    device: &'a DeviceId,
    reason: String,
    source_hash: ContentHash,
    identity_candidates: Vec<NoteId>,
}

/// The reason review is required, if any operation demands it.
///
/// Only the summary half of the reason is kept. A descriptor says *why* review
/// is needed; the note's content is on disk already and does not need a second
/// copy inside the workspace's own state, so the incoming source the diff layer
/// embeds is dropped by the accessor that owns that format.
fn review_reason(operations: &[SemanticOperation]) -> Option<String> {
    operations.iter().find_map(|operation| match operation {
        SemanticOperation::NeedsReview { reason, .. } => {
            Some(review_reason_summary(reason).to_owned())
        }
        _ => None,
    })
}

fn operations_of(
    records: &[LocalOperationRecord],
    transaction_id: TransactionId,
) -> Vec<SemanticOperation> {
    records
        .iter()
        .filter(|record| record.transaction_id == transaction_id)
        .map(|record| record.operation.clone())
        .collect()
}
