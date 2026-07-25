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
    /// The entry for a note whose file still holds its converged base.
    fn unchanged(path: &WorkspacePath, note_id: NoteId) -> Self {
        Self {
            path: path.to_string(),
            outcome: "unchanged",
            note_id: Some(note_id),
            transaction_id: None,
            version: None,
            descriptor_path: None,
            source_note_id: None,
        }
    }

    /// The entry for what the coordinator made of one changed or vanished note.
    fn integrated(path: &WorkspacePath, outcome: &ExternalEditOutcome) -> Self {
        let base = Self {
            path: path.to_string(),
            outcome: "",
            note_id: None,
            transaction_id: None,
            version: None,
            descriptor_path: None,
            source_note_id: None,
        };
        match outcome {
            ExternalEditOutcome::Registered { note_id } => Self {
                outcome: "registered",
                note_id: Some(*note_id),
                ..base
            },
            ExternalEditOutcome::BaseRecovered { note_id } => Self {
                outcome: "base_recovered",
                note_id: Some(*note_id),
                ..base
            },
            ExternalEditOutcome::Unchanged { note_id } => Self {
                outcome: "unchanged",
                note_id: Some(*note_id),
                ..base
            },
            ExternalEditOutcome::Adopted {
                note_id,
                transaction_id,
                version,
            } => Self {
                outcome: "adopted",
                note_id: Some(*note_id),
                transaction_id: Some(*transaction_id),
                version: Some(*version),
                ..base
            },
            ExternalEditOutcome::Merged {
                note_id,
                transaction_id,
                version,
                ..
            } => Self {
                outcome: "merged",
                note_id: Some(*note_id),
                transaction_id: Some(*transaction_id),
                version: Some(*version),
                ..base
            },
            ExternalEditOutcome::ReviewRequired {
                transaction_id,
                descriptor,
            } => Self {
                outcome: "review_required",
                transaction_id: Some(*transaction_id),
                descriptor_path: Some(descriptor.display().to_string()),
                ..base
            },
            ExternalEditOutcome::Renamed { note_id, path } => Self {
                path: path.to_string(),
                outcome: "renamed",
                note_id: Some(*note_id),
                ..base
            },
            ExternalEditOutcome::Copied {
                note_id,
                source_note_id,
            } => Self {
                outcome: "copied",
                note_id: Some(*note_id),
                source_note_id: Some(*source_note_id),
                ..base
            },
            ExternalEditOutcome::Deleted { path, note_id } => Self {
                path: path.to_string(),
                outcome: "deleted",
                note_id: *note_id,
                ..base
            },
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
    deleted: usize,
    unchanged: usize,
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
        } else {
            coordinator.integrate(WorkspaceEvent::Deleted {
                path: base.path.clone(),
            })?
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
    let reviews_required = count("review_required");
    let report = ReconcileReport {
        workspace: workspace.path().display().to_string(),
        considered: notes.len(),
        adopted: count("adopted"),
        merged: count("merged"),
        reviews_required,
        deleted: count("deleted"),
        unchanged: count("unchanged"),
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
