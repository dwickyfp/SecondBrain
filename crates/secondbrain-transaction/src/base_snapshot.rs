//! The last converged base of each note.
//!
//! A converged base is the exact Markdown source of a note as of the last
//! transaction that agreed on it. External editors rewrite whole files, so by
//! the time the workspace sees a change the edit is already on disk and the
//! previous state is gone. Without a recorded base there is nothing to diff an
//! external edit against, and the workspace can only overwrite or ignore it.
//!
//! Records live in `.secondbrain/snapshots/<NoteId>.json`, a directory
//! [`secondbrain_vault::initialize_workspace`] already reserves, and are
//! written atomically so a crash mid-write leaves the previous record intact.
//!
//! # Provisional format
//!
//! [`BASE_SNAPSHOT_FORMAT`] is provisional Phase 0 state in the same spirit as
//! the oplog's `sb-local-oplog-v1` label. Task 28 (CRDT integration) supersedes
//! it: the CRDT document becomes the converged state, and these records are its
//! migration input.
//!
//! # Privacy
//!
//! Records store the note's own source text, a workspace-relative path, a
//! content hash, and a note version. No OS-absolute paths and no secrets.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion};
use secondbrain_core::path::WorkspacePath;
use secondbrain_vault::WorkspaceRoot;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The only converged-base record format this version understands.
pub const BASE_SNAPSHOT_FORMAT: &str = "sb-base-snapshot-v1";

/// The subdirectory under `.secondbrain/` that holds converged bases.
const SNAPSHOT_DIR: &str = "snapshots";

/// The last state of a note that the workspace and its editors agreed on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BaseSnapshot {
    /// Record format label, always [`BASE_SNAPSHOT_FORMAT`] when written.
    pub format: String,
    /// The note this base belongs to.
    pub note_id: NoteId,
    /// The note's workspace-relative path when the base was recorded.
    pub path: WorkspacePath,
    /// The note version this base represents.
    pub version: NoteVersion,
    /// Hash of [`Self::source`], so callers can compare without re-hashing.
    pub source_hash: ContentHash,
    /// The converged Markdown source.
    pub source: String,
}

impl BaseSnapshot {
    /// Whether this base is the content that hashes to `source_hash`.
    ///
    /// "Has this note been edited outside the workspace since it last
    /// converged" is the question every caller that reads a file and a base
    /// together has to answer, and getting it wrong means deriving operations
    /// from a state the file no longer holds. It is therefore asked here rather
    /// than re-derived by each caller from [`Self::source_hash`].
    #[must_use]
    pub fn describes(&self, source_hash: ContentHash) -> bool {
        self.source_hash == source_hash
    }
}

/// Why a converged base could not be read or written.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("converged base I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("converged base serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("converged base write failed: {0}")]
    Vault(#[from] secondbrain_core::Error),
    #[error("converged base for {note_id} has unsupported format {format}")]
    UnsupportedFormat {
        /// The note whose record could not be read.
        note_id: NoteId,
        /// The format label found on disk.
        format: String,
    },
}

/// Reads and writes the converged base of every note in one workspace.
pub struct BaseSnapshotStore {
    directory: PathBuf,
}

impl BaseSnapshotStore {
    /// Opens the store for a workspace without touching the filesystem.
    #[must_use]
    pub fn new(workspace: &WorkspaceRoot) -> Self {
        Self {
            directory: snapshot_dir(workspace.canonical_path()),
        }
    }

    /// Loads a note's converged base, or `None` when none was ever recorded.
    ///
    /// # Errors
    ///
    /// Returns an error if the record exists but cannot be read, parsed, or
    /// understood. A record that cannot be trusted is never silently ignored:
    /// treating it as absent would make the next external edit look like the
    /// note's first observation.
    pub fn load(&self, note_id: NoteId) -> Result<Option<BaseSnapshot>, SnapshotError> {
        let path = self.record_path(note_id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(SnapshotError::Io(source)),
        };
        let snapshot: BaseSnapshot = serde_json::from_slice(&bytes)?;
        if snapshot.format != BASE_SNAPSHOT_FORMAT {
            return Err(SnapshotError::UnsupportedFormat {
                note_id,
                format: snapshot.format,
            });
        }
        Ok(Some(snapshot))
    }

    /// Every converged base this workspace has recorded, ordered by path.
    ///
    /// This is the roster of notes the workspace has agreed on at least once,
    /// and therefore the only notes an external edit can be measured against.
    /// A one-shot reconciliation needs exactly that set, and asking here keeps
    /// the record layout — one file per note under `.secondbrain/` — the
    /// store's own business rather than something a caller re-derives.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read, or if any record in it
    /// cannot be parsed or understood. One unreadable base is not skipped, for
    /// the reason [`Self::load`] gives: a caller told "this note has no base"
    /// would treat its next edit as the note's first observation.
    pub fn list(&self) -> Result<Vec<BaseSnapshot>, SnapshotError> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            // A workspace that has converged nothing has no directory yet, and
            // that is an empty roster rather than a failure.
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(SnapshotError::Io(source)),
        };
        let mut snapshots = Vec::new();
        for entry in entries {
            let path = entry?.path();
            // A file this store did not write is not a record it may read.
            let Some(note_id) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<NoteId>().ok())
            else {
                continue;
            };
            if let Some(snapshot) = self.load(note_id)? {
                snapshots.push(snapshot);
            }
        }
        snapshots.sort_by(|left, right| {
            left.path
                .as_str()
                .cmp(right.path.as_str())
                .then_with(|| left.note_id.to_string().cmp(&right.note_id.to_string()))
        });
        Ok(snapshots)
    }

    /// Records `source` as the converged base of `note_id` at `version`.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be serialized or written.
    pub fn save(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        version: NoteVersion,
        source: &str,
    ) -> Result<BaseSnapshot, SnapshotError> {
        let snapshot = BaseSnapshot {
            format: BASE_SNAPSHOT_FORMAT.to_owned(),
            note_id,
            path: path.clone(),
            version,
            source_hash: ContentHash::digest(source.as_bytes()),
            source: source.to_owned(),
        };
        let bytes = serde_json::to_vec_pretty(&snapshot)?;
        fs::create_dir_all(&self.directory)?;
        // The record lives under `.secondbrain/`, which `WorkspacePath` rejects,
        // so it is written through the atomic-write helper directly — the same
        // route the identity map takes.
        secondbrain_vault::atomic_write::atomic_write(&self.record_path(note_id), &bytes)?;
        Ok(snapshot)
    }

    fn record_path(&self, note_id: NoteId) -> PathBuf {
        self.directory.join(format!("{note_id}.json"))
    }
}

fn snapshot_dir(root: &Path) -> PathBuf {
    root.join(".secondbrain").join(SNAPSHOT_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_record_is_not_an_error() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");

        let loaded = BaseSnapshotStore::new(&root)
            .load(NoteId::new())
            .expect("load");

        assert_eq!(loaded, None);
    }

    #[test]
    fn saved_record_round_trips() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let store = BaseSnapshotStore::new(&root);
        let note_id = NoteId::new();
        let path = WorkspacePath::new("notes/a.md").expect("path");

        let saved = store
            .save(note_id, &path, NoteVersion::new(3), "# Note\n")
            .expect("save");
        let loaded = store.load(note_id).expect("load").expect("record");

        assert_eq!(loaded, saved);
        assert_eq!(loaded.source, "# Note\n");
        assert_eq!(loaded.source_hash, ContentHash::digest("# Note\n"));
        assert_eq!(loaded.version, NoteVersion::new(3));
        assert!(loaded.describes(ContentHash::digest("# Note\n")));
        assert!(
            !loaded.describes(ContentHash::digest("# Note\n\nEdited elsewhere.\n")),
            "a file an external editor rewrote is not the state this base records"
        );
    }

    #[test]
    fn a_workspace_that_converged_nothing_lists_no_bases() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");

        let listed = BaseSnapshotStore::new(&root).list().expect("list");

        assert!(
            listed.is_empty(),
            "a missing directory is an empty roster, not a failure"
        );
    }

    #[test]
    fn listed_bases_are_ordered_by_path_and_exclude_files_the_store_did_not_write() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let store = BaseSnapshotStore::new(&root);
        let second = NoteId::new();
        let first = NoteId::new();
        store
            .save(
                second,
                &WorkspacePath::new("notes/z.md").expect("path"),
                NoteVersion::new(2),
                "# Z\n",
            )
            .expect("save");
        store
            .save(
                first,
                &WorkspacePath::new("notes/a.md").expect("path"),
                NoteVersion::new(1),
                "# A\n",
            )
            .expect("save");
        fs::write(store.directory.join("notes.txt"), "not a record").expect("write foreign file");

        let listed = store.list().expect("list");

        assert_eq!(
            listed
                .iter()
                .map(|snapshot| snapshot.note_id)
                .collect::<Vec<_>>(),
            vec![first, second],
            "an operator reads this roster, so its order cannot depend on the filesystem"
        );
        assert_eq!(listed[0].source, "# A\n");
    }

    #[test]
    fn record_from_an_unknown_format_is_refused() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let store = BaseSnapshotStore::new(&root);
        let note_id = NoteId::new();
        store
            .save(
                note_id,
                &WorkspacePath::new("notes/a.md").expect("path"),
                NoteVersion::new(1),
                "# Note\n",
            )
            .expect("save");
        let path = store.record_path(note_id);
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        record["format"] = serde_json::Value::from("sb-base-snapshot-v2");
        fs::write(&path, serde_json::to_vec(&record).expect("encode")).expect("write");

        let error = store.load(note_id).expect_err("unsupported format");

        assert!(
            matches!(&error, SnapshotError::UnsupportedFormat { format, .. }
                if format == "sb-base-snapshot-v2"),
            "{error}"
        );
    }
}
