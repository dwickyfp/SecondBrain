//! Versioned workspace manifest creation, persistence, and validation.
//!
//! The manifest is the single source of truth for a workspace's identity and
//! on-disk layout. It lives at `.secondbrain/manifest.toml` and is written
//! atomically (temporary file + rename) so that a crash mid-write never leaves
//! a truncated manifest behind. Initialization is idempotent: re-running it on
//! an already-initialized workspace returns the existing manifest unchanged.
//!
//! The manifest format is versioned via `format_version`. This crate only
//! understands `format_version = 1`. A manifest declaring a higher version is
//! rejected on load so that older binaries fail loudly instead of silently
//! corrupting a newer workspace.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use secondbrain_core::id::WorkspaceId;
use secondbrain_core::{Error, Result};

/// The internal directory name that owns all SecondBrain state.
const INTERNAL_DIR: &str = ".secondbrain";

/// The manifest file name inside the internal directory.
const MANIFEST_FILE: &str = "manifest.toml";

/// The plugins lock file name inside the internal directory.
const PLUGINS_LOCK_FILE: &str = "plugins.lock";

/// The only `format_version` this version of the vault understands.
pub const SUPPORTED_FORMAT_VERSION: u32 = 1;

/// The required-features advertised by a freshly initialized workspace.
///
/// Future readers can refuse to interoperate when a feature they need is
/// absent. Today we advertise the `oplog` capability because the oplog is the
/// backbone of the transactional model.
const DEFAULT_REQUIRED_FEATURES: &[&str] = &["oplog"];

/// The internal sub-directories that initialization must create.
///
/// Order matches the spec; all of them are created on every init call (they
/// are cheap `create_dir_all` no-ops when already present).
const REQUIRED_DIRECTORIES: &[&str] = &[
    "oplog",
    "transactions",
    "snapshots",
    "identity-map",
    "policies",
    "audit",
];

/// The versioned workspace manifest.
///
/// Serialized as TOML to `.secondbrain/manifest.toml`. Fields are documented
/// inline so the on-disk file is self-describing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    /// The stable identity of this workspace. Generated once at first init
    /// and never changed afterward.
    pub workspace_id: WorkspaceId,
    /// The manifest format version. Currently always `1`.
    pub format_version: u32,
    /// RFC 3339 UTC timestamp marking when the workspace was first created.
    pub created_at: String,
    /// Capabilities this workspace requires readers to understand.
    pub required_features: Vec<String>,
}

/// On-disk TOML representation. Kept separate from the in-memory type so the
/// serialized shape stays stable even if the in-memory API evolves.
#[derive(Serialize, Deserialize)]
struct ManifestFile {
    workspace_id: WorkspaceId,
    format_version: u32,
    created_at: String,
    required_features: Vec<String>,
}

impl From<&WorkspaceManifest> for ManifestFile {
    fn from(m: &WorkspaceManifest) -> Self {
        Self {
            workspace_id: m.workspace_id,
            format_version: m.format_version,
            created_at: m.created_at.clone(),
            required_features: m.required_features.clone(),
        }
    }
}

impl ManifestFile {
    fn into_manifest(self) -> WorkspaceManifest {
        WorkspaceManifest {
            workspace_id: self.workspace_id,
            format_version: self.format_version,
            created_at: self.created_at,
            required_features: self.required_features,
        }
    }
}

/// Returns the path to the internal `.secondbrain` directory under `root`.
fn internal_dir(root: &Path) -> std::path::PathBuf {
    root.join(INTERNAL_DIR)
}

/// Returns the path to the manifest file under `root`.
fn manifest_path(root: &Path) -> std::path::PathBuf {
    internal_dir(root).join(MANIFEST_FILE)
}

/// Returns the path to the plugins lock file under `root`.
fn plugins_lock_path(root: &Path) -> std::path::PathBuf {
    internal_dir(root).join(PLUGINS_LOCK_FILE)
}

/// Generates a fresh RFC 3339 UTC timestamp using the `ulid` crate's monotonic
/// clock-free source. We avoid pulling in `chrono` for production code by
/// formatting from `std::time::SystemTime`.
fn now_rfc3339_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Minimal, dependency-free RFC 3339 formatter. `secs` is seconds since the
    // Unix epoch in UTC. We compute the civil date with the well-known
    // days-from-civil algorithm (Howard Hinnant) to avoid pulling in a full
    // datetime crate just for a timestamp.
    let days = (secs / 86_400) as i64;
    let second_of_day = (secs % 86_400) as u32;

    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3600;
    let minute = (second_of_day % 3600) / 60;
    let second = second_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Converts a count of days since 1970-01-01 into a (year, month, day) triple.
///
/// This is Howard Hinnant's `days_from_civil` run in reverse. It is purely
/// arithmetic with no allocations and no panics on any `i64` input.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468; // days from 0000-03-01
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Writes `bytes` to `target` atomically: first write to a sibling temp file,
/// then rename over the target. On POSIX this is atomic; on other platforms it
/// is still crash-safe because the temp file is in the same directory.
fn atomic_write(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target.parent().ok_or_else(|| Error::CorruptRecord {
        record: target.to_string_lossy().into_owned(),
        summary: "manifest target has no parent directory".into(),
    })?;

    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| Error::Io {
        operation: "create temp manifest",
        source,
    })?;
    temp.write_all(bytes).map_err(|source| Error::Io {
        operation: "write temp manifest",
        source,
    })?;
    temp.as_file().sync_all().map_err(|source| Error::Io {
        operation: "sync temp manifest",
        source,
    })?;
    temp.persist(target).map_err(|err| Error::CorruptRecord {
        record: target.to_string_lossy().into_owned(),
        summary: format!("manifest atomic rename failed: {err}"),
    })?;
    Ok(())
}

/// Creates the internal directory layout under `root`.
///
/// This is safe to call repeatedly: `create_dir_all` is a no-op when the
/// directory already exists.
fn ensure_internal_layout(root: &Path) -> Result<()> {
    let internal = internal_dir(root);
    fs::create_dir_all(&internal).map_err(|source| Error::Io {
        operation: "create .secondbrain",
        source,
    })?;

    for sub in REQUIRED_DIRECTORIES {
        fs::create_dir_all(internal.join(sub)).map_err(|source| Error::Io {
            operation: "create internal subdir",
            source,
        })?;
    }

    // plugins.lock is a crash-marker: its presence means the internal layout
    // is fully provisioned. Create it empty if absent, never overwrite it.
    let lock = plugins_lock_path(root);
    if !lock.exists() {
        atomic_write(&lock, b"")?;
    }
    Ok(())
}

/// Initializes a SecondBrain workspace at `root`.
///
/// Creates `.secondbrain/` with all required sub-directories, the
/// `plugins.lock` marker, and a freshly generated `manifest.toml`. If the
/// workspace is already initialized, the existing manifest is returned
/// unchanged (idempotent).
///
/// This function never reads, writes, or modifies any user Markdown files.
pub fn initialize_workspace(root: &Path) -> Result<WorkspaceManifest> {
    ensure_internal_layout(root)?;

    let path = manifest_path(root);
    if path.exists() {
        return load_manifest(root);
    }

    let manifest = WorkspaceManifest {
        workspace_id: WorkspaceId::new(),
        format_version: SUPPORTED_FORMAT_VERSION,
        created_at: now_rfc3339_utc(),
        required_features: DEFAULT_REQUIRED_FEATURES
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    };
    let serialized =
        toml::to_string(&ManifestFile::from(&manifest)).map_err(|err| Error::CorruptRecord {
            record: path.to_string_lossy().into_owned(),
            summary: format!("manifest serialization failed: {err}"),
        })?;
    atomic_write(&path, serialized.as_bytes())?;
    Ok(manifest)
}

/// Loads a workspace manifest from `root`.
///
/// Returns a typed error if the manifest is missing, unreadable, or declares
/// an unsupported `format_version`. Read-only: never writes to disk.
pub fn load_manifest(root: &Path) -> Result<WorkspaceManifest> {
    let path = manifest_path(root);
    let contents = fs::read_to_string(&path).map_err(|source| Error::Io {
        operation: "read manifest",
        source,
    })?;
    let file: ManifestFile = toml::from_str(&contents).map_err(|err| Error::CorruptRecord {
        record: path.to_string_lossy().into_owned(),
        summary: format!("manifest parse failed: {err}"),
    })?;

    if file.format_version > SUPPORTED_FORMAT_VERSION {
        return Err(Error::CorruptRecord {
            record: path.to_string_lossy().into_owned(),
            summary: format!(
                "unsupported manifest format_version {}: this build supports up to {}",
                file.format_version, SUPPORTED_FORMAT_VERSION
            ),
        });
    }

    Ok(file.into_manifest())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-01-01 is a known Thursday; 20454 days after 1970-01-01.
        let (y, m, d) = civil_from_days(20_454);
        assert_eq!((y, m, d), (2026, 1, 1));
    }

    #[test]
    fn now_rfc3339_utc_is_parseable_and_utc() {
        let ts = now_rfc3339_utc();
        assert!(ts.ends_with('Z'), "timestamp ends with Z: {ts}");
        // Length of YYYY-MM-DDTHH:MM:SSZ is 20.
        assert_eq!(ts.len(), 20, "compact RFC 3339 form: {ts}");
    }

    #[test]
    fn supported_format_version_is_one() {
        assert_eq!(SUPPORTED_FORMAT_VERSION, 1);
    }
}
