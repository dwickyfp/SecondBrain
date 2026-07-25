//! `secondbrain note inspect` — everything the workspace knows about one note.
//!
//! Identity, the content the workspace last converged on, and both link
//! directions. This is the command an operator reaches for before writing a
//! plan, so it reports exactly the preconditions a plan records.

use std::path::Path;

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion};
use secondbrain_core::path::WorkspacePath;
use secondbrain_transaction::base_snapshot::BaseSnapshotStore;
use serde::Serialize;

use crate::exit::{CliError, OK, read_file};
use crate::output::{Format, Report, emit, or_none, plural};
use crate::workspace::Workspace;

/// One link leaving the note.
#[derive(Serialize)]
struct OutgoingLink {
    target: String,
    note_id: Option<NoteId>,
    path: Option<String>,
    title: Option<String>,
}

/// One link arriving at the note.
#[derive(Serialize)]
struct Backlink {
    note_id: Option<NoteId>,
    path: Option<String>,
    title: Option<String>,
    target: String,
}

/// Everything known about one note.
#[derive(Serialize)]
struct InspectReport {
    note_id: NoteId,
    path: String,
    title: Option<String>,
    source_hash: ContentHash,
    /// The version of the content the workspace last converged on, or `null`
    /// when no base has been recorded — which is what a note that no
    /// transaction has ever touched looks like.
    converged_version: Option<NoteVersion>,
    converged: bool,
    outgoing_links: Vec<OutgoingLink>,
    backlinks: Vec<Backlink>,
}

impl Report for InspectReport {
    fn render(&self) -> String {
        let mut text = format!(
            "{}\n  note id:      {}\n  title:        {}\n  source hash:  {}\n  converged:    {}",
            self.path,
            self.note_id,
            or_none(self.title.as_ref()),
            self.source_hash,
            match self.converged_version {
                Some(version) if self.converged => format!("yes, at version {}", version.get()),
                Some(version) => format!("no, base is version {}", version.get()),
                None => "no base recorded".to_owned(),
            }
        );
        text.push_str(&format!(
            "\n  {}",
            plural(self.outgoing_links.len(), "outgoing link", "outgoing links")
        ));
        for link in &self.outgoing_links {
            text.push_str(&format!(
                "\n    {} -> {}",
                link.target,
                link.path.as_deref().unwrap_or("(unresolved)")
            ));
        }
        text.push_str(&format!(
            "\n  {}",
            plural(self.backlinks.len(), "backlink", "backlinks")
        ));
        for link in &self.backlinks {
            text.push_str(&format!(
                "\n    {} -> {}",
                link.path.as_deref().unwrap_or("(unknown)"),
                link.target
            ));
        }
        text
    }
}

/// Inspects the note at `path` in `workspace`.
pub fn inspect(format: Format, workspace: &Path, path: &str) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let note_path = WorkspacePath::new(path)?;
    let database = workspace.open_index()?;
    let summary = database
        .note_by_path(note_path.as_str())?
        .ok_or_else(|| CliError::NoteNotIndexed(path.to_owned()))?;

    let source = read_file("read note", &workspace.root().resolve(&note_path)?)?;
    let source_hash = ContentHash::digest(source.as_bytes());
    let base = BaseSnapshotStore::new(workspace.root()).load(summary.note_id)?;

    let outgoing_links = database
        .outgoing_links(summary.note_id)?
        .into_iter()
        .map(|link| OutgoingLink {
            target: link.target,
            note_id: link.note_id,
            path: link.path,
            title: link.title,
        })
        .collect();
    let backlinks = database
        .backlinks(summary.note_id)?
        .into_iter()
        .map(|link| Backlink {
            note_id: link.note_id,
            path: link.path,
            title: link.title,
            target: link.target,
        })
        .collect();

    emit(
        format,
        &InspectReport {
            note_id: summary.note_id,
            path: summary.path,
            title: summary.title,
            source_hash,
            converged_version: base.as_ref().map(|snapshot| snapshot.version),
            converged: base
                .as_ref()
                .is_some_and(|snapshot| snapshot.source_hash == source_hash),
            outgoing_links,
            backlinks,
        },
    )?;
    Ok(OK)
}
