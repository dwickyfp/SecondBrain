//! The last converged base of each note.
//!
//! A converged base is the exact Markdown source of a note as of the last
//! transaction that agreed on it. External editors rewrite whole files, so by
//! the time the workspace sees a change the edit is already on disk and the
//! previous state is gone. Without a recorded base there is nothing to diff an
//! external edit against, and the workspace can only overwrite or ignore it.
//!
//! Records live in `.secondbrain/snapshots/<NoteId>.json`, a directory
//! [`crate::initialize_workspace`] already reserves, and are written atomically
//! so a crash mid-write leaves the previous record intact. They are durable
//! workspace state under `.secondbrain/`, which is why they live in this crate
//! beside [`crate::identity_map`] rather than with the transaction engine that
//! was their first writer: every layer that takes responsibility for a note —
//! the indexer, the transaction engine, the external-edit coordinator — has to
//! be able to record and read one.
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

#[cfg(test)]
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::SourceDocument;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::root::WorkspaceRoot;

/// The only converged-base record format this version understands.
pub const BASE_SNAPSHOT_FORMAT: &str = "sb-base-snapshot-v1";

/// The version a note's converged base starts at, before any transaction.
///
/// Stated once, here, because the genesis base is what every later version is
/// counted from: a caller that recorded a first base at some other number would
/// be handing the transaction engine a precondition no plan can match.
pub const GENESIS_VERSION: NoteVersion = NoteVersion::new(0);

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
    #[error("converged base could not be parsed for identity evidence: {0}")]
    Parse(#[from] secondbrain_markdown::parse::ParseError),
    #[error("canonical CRDT state failed: {0}")]
    Crdt(#[from] secondbrain_crdt::Error),
    #[error("converged base for {note_id} has unsupported format {format}")]
    UnsupportedFormat {
        /// The note whose record could not be read.
        note_id: NoteId,
        /// The format label found on disk.
        format: String,
    },
}

/// Reads and writes the converged base of every note in one workspace.
///
/// It also keeps each note's identity evidence in step with the base it writes
/// — see [`Self::save`].
pub struct BaseSnapshotStore {
    workspace: WorkspaceRoot,
    #[allow(dead_code)]
    directory: PathBuf,
}

impl BaseSnapshotStore {
    /// Opens the store for a workspace without touching the filesystem.
    #[must_use]
    pub fn new(workspace: &WorkspaceRoot) -> Self {
        Self {
            directory: snapshot_dir(workspace.canonical_path()),
            workspace: workspace.clone(),
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
        Ok(
            secondbrain_crdt::NoteStore::new(self.workspace.canonical_path())
                .load(note_id)?
                .map(base_snapshot),
        )
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
        Ok(
            secondbrain_crdt::NoteStore::new(self.workspace.canonical_path())
                .list()?
                .into_iter()
                .map(base_snapshot)
                .collect(),
        )
    }

    /// Records `source` as the converged base of `note_id` at `version`, and
    /// refreshes the note's identity evidence to the same bytes.
    ///
    /// The two move together because they answer the same question: what this
    /// note looks like now. [`crate::IdentityMap`] recovers a renamed note by
    /// matching the file against the `source_hash` and `fingerprint` its record
    /// holds, and a record still describing the note's first observation stops
    /// matching the moment the note is edited — so a base written without that
    /// refresh would leave every note the workspace has ever converged on
    /// unrecoverable across a rename.
    ///
    /// Doing it here rather than in each caller is the point. This is the one
    /// place a converged base is written, from the transaction engine, the
    /// recovery pass, the external-edit coordinator and indexing alike; a pair
    /// of independent setters is what let the two drift, and no setter for the
    /// evidence is exposed outside this crate at all.
    ///
    /// Nothing here reads or writes the note itself, and a note whose identity
    /// is not in the map — one carrying its own frontmatter id — simply has no
    /// evidence to refresh.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be serialized or written, or if
    /// `source` is not Markdown this build can parse — the fingerprint is
    /// derived from its structure, and a base whose evidence could not be
    /// derived is exactly the drift this method exists to prevent.
    pub fn save(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        version: NoteVersion,
        source: &str,
    ) -> Result<BaseSnapshot, SnapshotError> {
        let fingerprint = SourceDocument::parse(source)?.semantic_fingerprint();
        let state = secondbrain_crdt::NoteStore::new(self.workspace.canonical_path())
            .save(note_id, path, version, source)?;
        crate::IdentityMap::record_convergence(
            &self.workspace,
            note_id,
            state.source_hash,
            fingerprint,
        )?;
        Ok(base_snapshot(state))
    }

    /// Records `source` as the genesis base of `note_id` unless it has one.
    ///
    /// This is the rule that a note the workspace has taken responsibility for
    /// has a converged base. Identity is the moment responsibility begins, so
    /// whatever establishes a note's identity calls this: without it the note
    /// has no earlier state for an external editor to have diverged from, and
    /// the whole external-edit pipeline has nothing to measure against — an
    /// edit made outside the workspace can then only be overwritten or ignored,
    /// which is exactly what "external edits are semantic operations, never
    /// silent overwrites" forbids.
    ///
    /// An existing base is never replaced, and `Ok(None)` says so. A note whose
    /// file no longer holds its base has been edited outside the workspace, and
    /// overwriting the base with the editor's bytes would adopt that edit
    /// silently — unattributed, unjournaled, and unrecoverable. Reconciliation
    /// is what resolves that; this is only ever what bootstraps it.
    ///
    /// Nothing here reads or writes the note itself: `source` is supplied by
    /// the caller that already read it, and the only write is the record under
    /// `.secondbrain/`.
    ///
    /// # Errors
    ///
    /// Returns an error if an existing record cannot be read or understood, or
    /// if the new record cannot be serialized or written.
    pub fn ensure_genesis(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        source: &str,
    ) -> Result<Option<BaseSnapshot>, SnapshotError> {
        if self.load(note_id)?.is_some() {
            return Ok(None);
        }
        self.save(note_id, path, GENESIS_VERSION, source).map(Some)
    }

    /// Ensures independent genesis records with bounded parallel filesystem
    /// durability waits. Each note still follows [`Self::ensure_genesis`], so
    /// existing bases are never replaced and every successful return is fully
    /// durable.
    pub fn ensure_genesis_batch(
        &self,
        notes: &[(NoteId, WorkspacePath, String)],
    ) -> Result<(), SnapshotError> {
        if notes.is_empty() {
            return Ok(());
        }
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(notes.len());
        let chunk_size = notes.len().div_ceil(workers);
        std::thread::scope(|scope| {
            let handles = notes
                .chunks(chunk_size)
                .map(|chunk| {
                    scope.spawn(|| {
                        let store = Self::new(&self.workspace);
                        chunk.iter().try_for_each(|(id, path, source)| {
                            store.ensure_genesis(*id, path, source).map(|_| ())
                        })
                    })
                })
                .collect::<Vec<_>>();
            for handle in handles {
                handle.join().map_err(|_| {
                    SnapshotError::Crdt(secondbrain_crdt::Error::Io(std::io::Error::other(
                        "genesis writer panicked",
                    )))
                })??;
            }
            Ok(())
        })
    }

    /// Moves an existing base to the path its note now occupies.
    ///
    /// Only the location changes. The source, hash and version stay exactly as
    /// they were, so discovering a rename during indexing cannot adopt an
    /// external content edit as if the workspace had converged on it.
    pub fn update_path(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
    ) -> Result<Option<BaseSnapshot>, SnapshotError> {
        Ok(
            secondbrain_crdt::NoteStore::new(self.workspace.canonical_path())
                .update_path(note_id, path)?
                .map(base_snapshot),
        )
    }
}

fn base_snapshot(state: secondbrain_crdt::NoteState) -> BaseSnapshot {
    BaseSnapshot {
        format: BASE_SNAPSHOT_FORMAT.to_owned(),
        note_id: state.note_id,
        path: state.path,
        version: state.version,
        source_hash: state.source_hash,
        source: state.markdown,
    }
}

#[cfg(test)]
impl BaseSnapshotStore {
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
        fs::create_dir_all(&store.directory).expect("canonical directory");
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
    fn a_note_without_a_base_is_given_its_current_content_at_genesis() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let store = BaseSnapshotStore::new(&root);
        let note_id = NoteId::new();
        let path = WorkspacePath::new("notes/a.md").expect("path");

        let recorded = store
            .ensure_genesis(note_id, &path, "# Note\n")
            .expect("ensure")
            .expect("a note with no base gets one");

        assert_eq!(recorded.version, GENESIS_VERSION);
        assert_eq!(recorded.source, "# Note\n");
        assert_eq!(store.load(note_id).expect("load"), Some(recorded));
    }

    #[test]
    fn a_base_that_the_file_has_moved_past_is_not_overwritten() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let store = BaseSnapshotStore::new(&root);
        let note_id = NoteId::new();
        let path = WorkspacePath::new("notes/a.md").expect("path");
        let converged = store
            .save(note_id, &path, NoteVersion::new(4), "# Note\n")
            .expect("save");

        let recorded = store
            .ensure_genesis(note_id, &path, "# Note\n\nEdited elsewhere.\n")
            .expect("ensure");

        assert_eq!(
            recorded, None,
            "a note that already has a base is not bootstrapped again"
        );
        assert_eq!(
            store.load(note_id).expect("load"),
            Some(converged),
            "replacing the base with the editor's bytes would adopt an external \
             edit silently, which is the one thing this pipeline exists to prevent"
        );
    }

    #[test]
    fn record_from_an_unknown_format_is_refused() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let store = BaseSnapshotStore::new(&root);
        let note_id = NoteId::new();
        fs::create_dir_all(
            store
                .workspace
                .canonical_path()
                .join(".secondbrain/snapshots"),
        )
        .expect("legacy snapshot directory");
        let path = store.record_path(note_id);
        let record = serde_json::json!({
            "format": "sb-base-snapshot-v2",
            "note_id": note_id,
            "path": WorkspacePath::new("notes/a.md").expect("path"),
            "version": NoteVersion::new(1),
            "source_hash": ContentHash::digest(b"# Note\n"),
            "source": "# Note\n"
        });
        fs::write(&path, serde_json::to_vec(&record).expect("encode")).expect("write");

        let error = store.load(note_id).expect_err("unsupported format");

        assert!(
            matches!(&error, SnapshotError::Crdt(secondbrain_crdt::Error::UnsupportedFormat { format, .. })
                if format == "sb-base-snapshot-v2"),
            "{error}"
        );
    }
}
