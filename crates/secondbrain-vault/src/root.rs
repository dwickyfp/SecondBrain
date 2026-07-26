//! Workspace root confinement.
//!
//! [`WorkspaceRoot`] is the single gate through which all vault writes pass.
//! It canonicalizes the root directory on construction so that symlinks in the
//! root path itself are resolved once and for all. Every relative
//! [`WorkspacePath`] is then resolved against the canonical root and the
//! resolved path is checked to stay within it, defending against symlink
//! escapes and other traversal tricks.

use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_core::path::WorkspacePath;
use secondbrain_core::{Error, Result};

use crate::atomic_write::WriteReceipt;
use crate::atomic_write::{atomic_create as do_atomic_create, atomic_write as do_atomic_write};

/// The canonical, write-confining root of a SecondBrain workspace.
///
/// All writes performed through this type are guaranteed to land inside the
/// canonical root and to either fully replace the destination file or leave
/// the previous file intact.
#[derive(Clone, Debug)]
pub struct WorkspaceRoot {
    canonical: PathBuf,
}

impl WorkspaceRoot {
    /// Opens a workspace root, canonicalizing the path so that symlinks in the
    /// root path itself are resolved once and for all.
    ///
    /// The directory must already exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let raw = path.as_ref();
        let canonical = fs::canonicalize(raw).map_err(|source| Error::Io {
            operation: "canonicalize workspace root",
            source,
        })?;
        Ok(Self { canonical })
    }

    /// Returns the canonical root path.
    #[must_use]
    pub fn canonical_path(&self) -> &Path {
        &self.canonical
    }

    /// Resolves a workspace-relative path against the canonical root, rejecting
    /// any result that escapes the root after following symlinks.
    ///
    /// The `WorkspacePath` type already rejects `..`, absolute paths, and
    /// non-normalized forms lexically. This method adds a *semantic* check:
    /// after joining the relative path to the canonical root and canonicalizing
    /// the result, the canonicalized result must still be inside the canonical
    /// root. This catches symlink escapes where a path component inside the
    /// workspace is a symlink pointing outside.
    pub fn resolve(&self, path: &WorkspacePath) -> Result<PathBuf> {
        let joined = self.canonical.join(path.as_path());

        // Create parent directories eagerly so that canonicalize can resolve
        // intermediate symlinks. We do NOT create the final component because
        // the file may not exist yet (it is the write target). We canonicalize
        // the parent and re-attach the file name.
        let parent = joined.parent().ok_or_else(|| Error::WorkspaceEscape {
            path: joined.clone(),
        })?;
        let file_name = joined
            .file_name()
            .ok_or_else(|| Error::WorkspaceEscape {
                path: joined.clone(),
            })?
            .to_owned();

        // Canonicalize the parent. If the parent doesn't exist yet, create it
        // first so the canonical form is well-defined. This is safe because the
        // parent is a sub-path of the canonical root and we re-check containment
        // after canonicalization.
        let canonical_parent = if parent.exists() {
            fs::canonicalize(parent).map_err(|source| Error::Io {
                operation: "canonicalize parent",
                source,
            })?
        } else {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                operation: "create parent dirs",
                source,
            })?;
            fs::canonicalize(parent).map_err(|source| Error::Io {
                operation: "canonicalize created parent",
                source,
            })?
        };

        // Re-check that the canonicalized parent is still inside the root.
        if !canonical_parent.starts_with(&self.canonical) {
            return Err(Error::WorkspaceEscape {
                path: canonical_parent,
            });
        }

        let resolved = canonical_parent.join(&file_name);

        // If the file already exists, canonicalize it too so that a symlink at
        // the final component is caught.
        let resolved = if resolved.exists() {
            fs::canonicalize(&resolved).map_err(|source| Error::Io {
                operation: "canonicalize target",
                source,
            })?
        } else {
            resolved
        };

        // Final containment check on the fully resolved path.
        if !resolved.starts_with(&self.canonical) {
            return Err(Error::WorkspaceEscape {
                path: resolved.clone(),
            });
        }

        Ok(resolved)
    }

    /// Resolves a path for inspection without creating missing parent directories.
    pub fn resolve_read_only(&self, path: &WorkspacePath) -> Result<PathBuf> {
        let joined = self.canonical.join(path.as_path());
        let parent = joined.parent().ok_or_else(|| Error::WorkspaceEscape {
            path: joined.clone(),
        })?;
        let mut existing = parent;
        while !existing.exists() {
            existing = existing.parent().ok_or_else(|| Error::WorkspaceEscape {
                path: joined.clone(),
            })?;
        }
        let canonical_parent = fs::canonicalize(existing).map_err(|source| Error::Io {
            operation: "canonicalize existing parent",
            source,
        })?;
        if !canonical_parent.starts_with(&self.canonical) {
            return Err(Error::WorkspaceEscape {
                path: canonical_parent,
            });
        }
        if joined.exists() {
            let canonical = fs::canonicalize(&joined).map_err(|source| Error::Io {
                operation: "canonicalize target",
                source,
            })?;
            if !canonical.starts_with(&self.canonical) {
                return Err(Error::WorkspaceEscape { path: canonical });
            }
        }
        Ok(joined)
    }

    /// Atomically writes `bytes` to the workspace-relative `path`.
    ///
    /// The write is performed to a temporary file in the same directory as the
    /// destination, then atomically renamed over the destination. On any error
    /// the temporary file is cleaned up, so the previous file (if any) is left
    /// intact.
    pub fn atomic_write(&self, path: &WorkspacePath, bytes: &[u8]) -> Result<WriteReceipt> {
        let target = self.resolve(path)?;
        do_atomic_write(&target, bytes)
    }

    /// Atomically creates a new workspace file, failing if the target exists.
    pub fn atomic_create(&self, path: &WorkspacePath, bytes: &[u8]) -> Result<WriteReceipt> {
        let target = self.resolve(path)?;
        do_atomic_create(&target, bytes)
    }
}
