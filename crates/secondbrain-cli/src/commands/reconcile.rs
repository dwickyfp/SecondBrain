//! `secondbrain reconcile` — journal the edits made outside the workspace.
//!
//! An editor that is not this workspace rewrites whole files, so by the time
//! anything here notices, the change is already on disk and the state it
//! replaced is gone. Until that edit is journaled it exists only as bytes: it
//! has no author, no transaction, and no place in the note's history, and
//! `diff` refuses to plan across the gap because a plan derived there would
//! record a version the file never held.
//!
//! This command closes that gap. For every note the workspace has converged on
//! at least once, it compares the file with the converged base and hands the
//! difference to [`ExternalEditCoordinator`], which recovers the *semantic*
//! operations the editor performed and journals them as an attributed
//! transaction. The file itself is not rewritten: the editor's bytes are the
//! result, and the workspace is catching up to them, not overruling them.
//!
//! It is one-shot and local, which is why it is `reconcile` and not `sync`.
//! Nothing here talks to another device.

use std::path::Path;

use secondbrain_core::actor::DeviceId;
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_transaction::{ExternalEditCoordinator, ExternalEditOutcome};
use secondbrain_vault::base_snapshot::BaseSnapshotStore;
use secondbrain_vault::event::WorkspaceEvent;
use serde::Serialize;

use crate::commands::CLI_DEVICE;
use crate::exit::{CliError, OK, REVIEW_REQUIRED, read_file};
use crate::output::{Format, Report, emit, plural};
use crate::workspace::Workspace;

/// The `outcome` tag of a note the file on disk still agrees with.
const UNCHANGED: &str = "unchanged";
/// The `outcome` tag of an external edit journaled as its own transaction.
const ADOPTED: &str = "adopted";
/// The `outcome` tag of an external edit journaled over a workspace change that
/// was rebased onto it.
const MERGED: &str = "merged";
/// The `outcome` tag of a change a person must decide.
const REVIEW: &str = "review_required";
/// The `outcome` tag of a note with no file where it was last known.
///
/// Deliberately not `deleted`. This pass derives its work from each note's
/// converged base, so all it can observe is that nothing is at the path that
/// base names. A file the operator deleted looks like this — and so does a file
/// they moved with a tool the workspace never saw, which is alive and well one
/// directory over. Naming the first of those would be a confident report of
/// data loss that has not happened.
const ABSENT: &str = "absent";

/// What reconciling one note did.
///
/// One flat, tagged object per note rather than a nested shape per kind, so a
/// caller can filter on `outcome` without knowing what every kind carries. The
/// fields that do not apply to a kind are absent rather than null.
#[derive(Serialize)]
struct ReconciledNote {
    path: String,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note_id: Option<NoteId>,
    /// Set when the edit was journaled: the transaction it became.
    #[serde(skip_serializing_if = "Option::is_none")]
    transaction_id: Option<TransactionId>,
    /// Set when the edit was journaled: the version the note converged at.
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<NoteVersion>,
    /// Set for a review: where the descriptor a person must resolve was filed.
    #[serde(skip_serializing_if = "Option::is_none")]
    descriptor_path: Option<String>,
    /// Set for a copy: the note the file was copied from.
    #[serde(skip_serializing_if = "Option::is_none")]
    source_note_id: Option<NoteId>,
}

impl ReconciledNote {
    /// An entry whose tag is decided before the value exists.
    ///
    /// Every constructor starts here, so this type is never a value with an
    /// `outcome` that means nothing — not even for the width of a struct
    /// literal that a later arm is trusted to finish. The per-kind fields are
    /// added by the builders below, each of which names what it is adding.
    fn tagged(path: &WorkspacePath, outcome: &'static str) -> Self {
        Self {
            path: path.to_string(),
            outcome,
            note_id: None,
            transaction_id: None,
            version: None,
            descriptor_path: None,
            source_note_id: None,
        }
    }

    #[must_use]
    fn of_note(mut self, note_id: NoteId) -> Self {
        self.note_id = Some(note_id);
        self
    }

    /// The transaction the edit was journaled as, and the version it reached.
    #[must_use]
    fn journaled(mut self, transaction_id: TransactionId, version: NoteVersion) -> Self {
        self.transaction_id = Some(transaction_id);
        self.version = Some(version);
        self
    }

    /// The transaction a review is filed under, and the descriptor to open.
    #[must_use]
    fn filed_for_review(mut self, transaction_id: TransactionId, descriptor: &Path) -> Self {
        self.transaction_id = Some(transaction_id);
        self.descriptor_path = Some(descriptor.display().to_string());
        self
    }

    #[must_use]
    fn copied_from(mut self, source_note_id: NoteId) -> Self {
        self.source_note_id = Some(source_note_id);
        self
    }

    /// The entry for a note whose file still holds its converged base.
    fn unchanged(path: &WorkspacePath, note_id: NoteId) -> Self {
        Self::tagged(path, UNCHANGED).of_note(note_id)
    }

    /// The entry for a note with no file at the path it was last known at.
    fn absent(path: &WorkspacePath, note_id: NoteId) -> Self {
        Self::tagged(path, ABSENT).of_note(note_id)
    }

    /// The entry for what the coordinator made of one changed or vanished note.
    fn integrated(path: &WorkspacePath, outcome: &ExternalEditOutcome) -> Self {
        match outcome {
            ExternalEditOutcome::Registered { note_id } => {
                Self::tagged(path, "registered").of_note(*note_id)
            }
            ExternalEditOutcome::BaseRecovered { note_id } => {
                Self::tagged(path, "base_recovered").of_note(*note_id)
            }
            ExternalEditOutcome::Unchanged { note_id } => {
                Self::tagged(path, UNCHANGED).of_note(*note_id)
            }
            ExternalEditOutcome::Adopted {
                note_id,
                transaction_id,
                version,
            } => Self::tagged(path, ADOPTED)
                .of_note(*note_id)
                .journaled(*transaction_id, *version),
            ExternalEditOutcome::Merged {
                note_id,
                transaction_id,
                version,
                ..
            } => Self::tagged(path, MERGED)
                .of_note(*note_id)
                .journaled(*transaction_id, *version),
            ExternalEditOutcome::ReviewRequired {
                transaction_id,
                descriptor,
            } => Self::tagged(path, REVIEW).filed_for_review(*transaction_id, descriptor),
            ExternalEditOutcome::Renamed { note_id, path } => {
                Self::tagged(path, "renamed").of_note(*note_id)
            }
            ExternalEditOutcome::Copied {
                note_id,
                source_note_id,
            } => Self::tagged(path, "copied")
                .of_note(*note_id)
                .copied_from(*source_note_id),
            // Reported as absence rather than deletion: what the workspace
            // knows is that no file is at the path this note was last seen at.
            // See [`ABSENT`].
            ExternalEditOutcome::Deleted { path, note_id } => {
                let entry = Self::tagged(path, ABSENT);
                match note_id {
                    Some(note_id) => entry.of_note(*note_id),
                    None => entry,
                }
            }
        }
    }
}

/// What one reconciliation pass did.
#[derive(Serialize)]
struct ReconcileReport {
    workspace: String,
    notes: Vec<ReconciledNote>,
    considered: usize,
    adopted: usize,
    merged: usize,
    reviews_required: usize,
    /// Notes with no file at the path they were last known at. See [`ABSENT`].
    absent: usize,
    unchanged: usize,
    /// Whether this pass refreshed the derived index.
    ///
    /// False for a pass that found nothing new to do, including one that
    /// reports the same absence it reported before: a fact restated is not work
    /// performed, and rebuilding the whole index to discover that would be
    /// neither cheap nor honest.
    index_refreshed: bool,
}

impl Report for ReconcileReport {
    fn render(&self) -> String {
        if self.notes.is_empty() {
            return format!(
                "Reconciled {}\n  no note has a converged base yet, so there is nothing an \
                 external edit could have diverged from",
                self.workspace
            );
        }
        let mut text = format!(
            "Reconciled {}\n  {} considered",
            self.workspace,
            plural(self.considered, "tracked note", "tracked notes")
        );
        for note in &self.notes {
            text.push_str(&format!("\n    {} {}", note.outcome, note.path));
            if let Some(version) = note.version {
                text.push_str(&format!(" (version {})", version.get()));
            }
            if let Some(descriptor) = &note.descriptor_path {
                text.push_str(&format!("\n      review filed at {descriptor}"));
            }
        }
        if self.absent > 0 {
            text.push_str(&format!(
                "\n  {}; a note moved out of the workspace by a tool this one never saw \
                 looks exactly like this",
                plural(
                    self.absent,
                    "note has no file at its last known path",
                    "notes have no file at their last known paths"
                )
            ));
        }
        if self.index_refreshed {
            text.push_str("\n  the index was rebuilt");
        }
        if self.reviews_required > 0 {
            text.push_str(&format!(
                "\n  {} waiting on a person",
                plural(self.reviews_required, "change", "changes")
            ));
        }
        text
    }
}

/// Reconciles every tracked note in `workspace` with the file on disk.
pub fn run(format: Format, workspace: &Path) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    // The roster is the set of notes the workspace has converged on. A note it
    // has never agreed on has no earlier state for an editor to have diverged
    // from, so there is nothing to recover and nothing to attribute.
    let bases = BaseSnapshotStore::new(workspace.root()).list()?;
    // The coordinator announces the writes it causes to whatever watcher is
    // running, so that a write loop cannot start. This process runs none — it
    // is one pass and then it exits — so it takes the default sink rather than
    // announcing to a watcher it would construct and immediately drop. The
    // desktop app, which does run one, wires it with `announcing_to`.
    let mut coordinator = ExternalEditCoordinator::new(
        workspace.root().clone(),
        workspace.manifest().workspace_id,
        DeviceId::new(CLI_DEVICE)?,
        workspace.index(),
    )?;

    let mut notes = Vec::with_capacity(bases.len());
    let mut changed = false;
    for base in bases {
        let absolute = workspace.root().resolve(&base.path)?;
        let outcome = if absolute.exists() {
            let source = read_file("read note", &absolute)?;
            let hash = ContentHash::digest(source.as_bytes());
            if base.describes(hash) {
                // The file is exactly what the workspace last agreed on. There
                // is nothing to journal, and nothing may be written: a note
                // nobody edited must come out of this command byte-identical.
                notes.push(ReconciledNote::unchanged(&base.path, base.note_id));
                continue;
            }
            coordinator.integrate(WorkspaceEvent::ContentChanged {
                path: base.path.clone(),
                hash,
            })?
        } else if index_describes(&workspace, &base.path)? {
            // The whole of what an absence asks for is that nothing derived
            // keeps describing a file that is not there, and that is the
            // coordinator's to do.
            coordinator.integrate(WorkspaceEvent::Deleted {
                path: base.path.clone(),
            })?
        } else {
            // Phase 0 cannot journal a deletion, so the absence is outstanding
            // and gets reported again on every pass. It is only *work* the
            // first time: once nothing derived points at the path, a later pass
            // has the same fact and nothing to do about it. Handing it to the
            // coordinator anyway would rebuild the whole index, forever, to
            // discover that.
            notes.push(ReconciledNote::absent(&base.path, base.note_id));
            continue;
        };
        changed |= !matches!(outcome, ExternalEditOutcome::Unchanged { .. });
        notes.push(ReconciledNote::integrated(&base.path, &outcome));
    }

    // The coordinator refreshed the derived state of every note it integrated.
    // This pass is for what it could not: a note left for review still has the
    // editor's bytes on disk, and the index is derived from what is on disk.
    if changed {
        workspace.index().rebuild()?;
    }

    let count = |outcome: &str| notes.iter().filter(|note| note.outcome == outcome).count();
    let reviews_required = count(REVIEW);
    let report = ReconcileReport {
        workspace: workspace.path().display().to_string(),
        considered: notes.len(),
        adopted: count(ADOPTED),
        merged: count(MERGED),
        reviews_required,
        absent: count(ABSENT),
        unchanged: count(UNCHANGED),
        index_refreshed: changed,
        notes,
    };
    emit(format, &report)?;

    // A review is not a failure: the pass completed and said what it found. It
    // is reported with the code `diff` already uses for the same fact, so a
    // script branching on "needs a person" keeps working unchanged.
    Ok(if reviews_required > 0 {
        REVIEW_REQUIRED
    } else {
        OK
    })
}

/// Whether the derived index still describes `path`.
///
/// This is not a second opinion on whether the note exists — the file's absence
/// already settled that. It is the question of whether there is any derived
/// state left to clean up, which is the only thing an absence can ask for while
/// Phase 0 has no delete transaction. The rule that a vanished file must leave
/// nothing describing it stays the coordinator's; this only decides whether
/// there is anything for it to do.
///
/// A workspace with no index at all describes nothing, so there is nothing to
/// clean up there either.
fn index_describes(workspace: &Workspace, path: &WorkspacePath) -> Result<bool, CliError> {
    match workspace.open_index() {
        Ok(database) => Ok(database.note_by_path(path.as_str())?.is_some()),
        Err(CliError::IndexMissing(_)) => Ok(false),
        Err(error) => Err(error),
    }
}
