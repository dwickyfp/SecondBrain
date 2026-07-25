//! Opening a workspace, and the one real [`IndexRefresh`] implementation.
//!
//! `secondbrain-transaction` must not depend on `secondbrain-index`, so the
//! transaction layer states what it needs from a derived index as a trait and
//! its tests drive that trait with a recording fake. This binary is the first
//! thing that may depend on both crates, so the production implementation lives
//! here: [`WorkspaceIndex`] is what turns "refresh the index for this note"
//! into an actual index.

use std::path::{Path, PathBuf};

use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_index::{IndexConfig, IndexDatabase, rebuild};
use secondbrain_transaction::external_edit::IndexRefresh;
use secondbrain_vault::{WorkspaceManifest, WorkspaceRoot, load_manifest};

use crate::exit::CliError;

/// An initialized workspace, opened once per command.
pub struct Workspace {
    root: WorkspaceRoot,
    manifest: WorkspaceManifest,
}

impl Workspace {
    /// Opens an initialized workspace.
    ///
    /// Fails when the directory holds no manifest: a command that read notes
    /// out of an uninitialized directory would be reporting on something that
    /// is not a workspace yet, and the operator would have no way to tell.
    pub fn open(path: &Path) -> Result<Self, CliError> {
        let root = WorkspaceRoot::open(path)?;
        let manifest = load_manifest(root.canonical_path())?;
        Ok(Self { root, manifest })
    }

    #[must_use]
    pub const fn root(&self) -> &WorkspaceRoot {
        &self.root
    }

    #[must_use]
    pub const fn manifest(&self) -> &WorkspaceManifest {
        &self.manifest
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.root.canonical_path()
    }

    /// The derived index of this workspace.
    #[must_use]
    pub fn index(&self) -> WorkspaceIndex {
        WorkspaceIndex::new(self.path())
    }

    /// Opens the derived index for querying.
    ///
    /// Fails when no index has been built. Opening a SQLite file that is not
    /// there would create an empty one, and every query would then answer
    /// "nothing found" — indistinguishable from a workspace whose notes really
    /// do not match, which is the wrong thing to tell an operator.
    pub fn open_index(&self) -> Result<IndexDatabase, CliError> {
        let path = secondbrain_index::index_path(self.path());
        if !path.exists() {
            return Err(CliError::IndexMissing(path));
        }
        Ok(IndexDatabase::open(&path)?)
    }
}

/// Refreshes a workspace's derived index.
///
/// The index crate offers a full rebuild and nothing narrower, so that is what
/// a refresh is today. It is correct, if wasteful: the index is derived state
/// and rebuilding it from the Markdown is the operation the whole design leans
/// on.
pub struct WorkspaceIndex {
    root: PathBuf,
    config: IndexConfig,
}

impl WorkspaceIndex {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            config: IndexConfig::default(),
        }
    }

    /// Rebuilds the whole index, reporting what it found.
    pub fn rebuild(&self) -> Result<secondbrain_index::IndexReport, CliError> {
        Ok(rebuild(&self.root, &self.config)?)
    }
}

/// The production [`IndexRefresh`], and the reason this crate may depend on both
/// `secondbrain-index` and `secondbrain-transaction`.
///
/// The note and path are ignored because a rebuild covers every note; they stay
/// in the signature because a narrower refresh will want them, and because the
/// trait is the transaction layer's statement of what it needs, not this
/// implementation's statement of what it happens to do.
impl IndexRefresh for &WorkspaceIndex {
    fn refresh(
        &self,
        _note_id: NoteId,
        _path: &WorkspacePath,
    ) -> Result<(), secondbrain_core::Error> {
        rebuild(&self.root, &self.config)
            .map(|_| ())
            .map_err(|error| secondbrain_core::Error::Sqlite {
                operation: "refresh derived index",
                source: Box::new(error),
            })
    }
}

#[cfg(test)]
mod tests {
    use secondbrain_core::id::NoteId;
    use secondbrain_index::IndexDatabase;

    use super::*;

    #[test]
    fn refreshing_through_the_trait_rebuilds_the_real_index() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path();
        std::fs::create_dir_all(root.join("notes")).expect("create notes directory");
        std::fs::write(root.join("notes/alpha.md"), "# Alpha\n\nRollout.\n").expect("write note");
        secondbrain_vault::initialize_workspace(root).expect("initialize workspace");

        let index = WorkspaceIndex::new(root);
        // Driven through the trait the transaction layer holds, not through the
        // inherent method, because the trait is what the coordinator will call.
        let refresher: &dyn IndexRefresh = &&index;
        refresher
            .refresh(
                NoteId::new(),
                &WorkspacePath::new("notes/alpha.md").unwrap(),
            )
            .expect("refresh");

        let database =
            IndexDatabase::open(secondbrain_index::index_path(root)).expect("open the index");
        assert!(
            database
                .note_by_path("notes/alpha.md")
                .expect("query")
                .is_some(),
            "a refresh that built no index would leave the coordinator's writes unsearchable"
        );
    }
}
