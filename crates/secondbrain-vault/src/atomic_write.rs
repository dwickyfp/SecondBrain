//! Atomic file writing primitive.
//!
//! Writing to a temporary file in the same directory as the destination and
//! then renaming it over the destination guarantees that the destination is
//! either fully replaced or left completely intact — there is never a
//! half-written file visible to readers. On any error the temporary file is
//! removed so no garbage is left behind.
//!
//! This module is the low-level primitive; workspace confinement is enforced
//! by [`crate::root::WorkspaceRoot`] before this function is ever called.

use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use secondbrain_core::{Error, Result};

/// A receipt for a successful atomic write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteReceipt {
    /// The absolute path that was written.
    pub path: PathBuf,
    /// The number of bytes written.
    pub bytes_written: usize,
}

/// Atomically writes `bytes` to `target`.
///
/// Steps:
/// 1. Create a temporary file in the same directory as `target`.
/// 2. Write and fsync the temp file.
/// 3. Atomically rename the temp file over `target`.
/// 4. fsync the parent directory so the rename is durable.
///
/// On any error, the temporary file is removed. If the target already exists,
/// it is left completely intact on failure.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> Result<WriteReceipt> {
    let parent = target.parent().ok_or_else(|| Error::CorruptRecord {
        record: target.to_string_lossy().into_owned(),
        summary: "write target has no parent directory".into(),
    })?;

    // Build a unique temp file name in the same directory. We use a fixed
    // prefix plus the process id and a counter-free approach: rely on
    // tempfile::NamedTempFile for uniqueness. This keeps the temp file in the
    // same directory so the rename is atomic on POSIX.
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| Error::Io {
        operation: "create temp file",
        source,
    })?;

    // Write and fsync. If either fails, NamedTempFile's Drop cleans up the
    // temp file automatically when it goes out of scope.
    if let Err(source) = temp.write_all(bytes) {
        return Err(Error::Io {
            operation: "write temp file",
            source,
        });
    }

    if let Err(source) = temp.as_file().sync_all() {
        return Err(Error::Io {
            operation: "fsync temp file",
            source,
        });
    }

    // Atomic rename. On POSIX this is atomic; on all platforms the temp file
    // is in the same directory so the rename is at least crash-safe.
    let persist_result = temp.persist(target);
    match persist_result {
        Ok(_) => {}
        Err(err) => {
            // The error contains the path to the temp file that may still
            // exist on disk after a failed persist. Clean it up best-effort.
            let temp_path = err.file.path().to_path_buf();
            let _ = fs::remove_file(&temp_path);
            return Err(Error::CorruptRecord {
                record: target.to_string_lossy().into_owned(),
                summary: format!("atomic rename failed: {err}"),
            });
        }
    }

    // fsync the parent directory so the directory entry update (the rename)
    // is durable. On Windows this is a no-op handled by the OS.
    #[cfg(unix)]
    {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(WriteReceipt {
        path: target.to_path_buf(),
        bytes_written: bytes.len(),
    })
}
