//! Portable identity map: persists note ID/path/fingerprint history.
//!
//! Each note's identity record is stored as a versioned JSON file under
//! `.secondbrain/identity-map/<NoteId>.json`. Records are written atomically
//! using the workspace's [`WorkspaceRoot::atomic_write`] so that a crash
//! mid-write never leaves a truncated record — the previous record (if any)
//! is always intact.
//!
//! # Recovery
//!
//! [`IdentityMap::resolve_identity`] resolves the stable [`NoteId`] for a file
//! that may have been renamed, had its frontmatter `id` removed, or been
//! copied. The resolution strategy, in priority order:
//!
//! 1. **Path match**: if the current path matches a record's `current_path`
//!    or appears in `historical_paths`, the record's ID is returned.
//! 2. **Exact match at same path**: if the structural fingerprint and content
//!    hash match a record at the *same* path, the record's ID is returned.
//! 3. **Exact match at different path (duplicate)**: if the fingerprint and
//!    hash match a record at a *different* path (with no path match),
//!    [`RecoveryOutcome::Duplicate`] is returned. If multiple records match
//!    (pre-existing duplicates), the first match is returned as Duplicate.
//! 4. **Fingerprint-only match (rename with body change)**: if the structural
//!    fingerprint matches but the content hash differs (body text changed but
//!    structure preserved) and there is no exact match, the record's ID is
//!    returned.
//! 5. **Ambiguity**: if two or more records match with fingerprint-only
//!    evidence (no path match, no exact match), [`RecoveryOutcome::NeedsReview`]
//!    is returned with the candidate IDs.
//! 6. **No matches**: [`RecoveryOutcome::New`] is returned.
//!
//! # Privacy
//!
//! Records store only workspace-relative paths, content hashes, structural
//! fingerprints, and timestamps. No OS-absolute paths, no file contents,
//! and no secrets are persisted.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_core::{Error, Result};

use crate::root::WorkspaceRoot;

/// The subdirectory under `.secondbrain/` that holds identity records.
const IDENTITY_MAP_DIR: &str = "identity-map";

/// The current identity record format version.
const RECORD_FORMAT_VERSION: u32 = 1;

/// The file extension for identity record files.
const RECORD_EXTENSION: &str = "json";

/// A portable, serializable representation of a 128-bit semantic fingerprint.
///
/// Stored as two `u64` fields to avoid depending on serde derives in the
/// markdown crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FingerprintRecord {
    /// Low 64 bits of the fingerprint.
    pub lo: u64,
    /// High 64 bits of the fingerprint.
    pub hi: u64,
}

/// A versioned identity record for a single note.
///
/// Serialized as JSON to `.secondbrain/identity-map/<NoteId>.json`.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdentityRecord {
    /// The record format version. Currently always `1`.
    pub version: u32,
    /// The stable note identifier this record tracks.
    pub note_id: NoteId,
    /// The current workspace-relative path of the note.
    pub current_path: WorkspacePath,
    /// All paths this note has occupied, including `current_path`.
    pub historical_paths: Vec<WorkspacePath>,
    /// The BLAKE3 hash of the note's source bytes at last observation.
    pub source_hash: ContentHash,
    /// The structural fingerprint at last observation.
    pub fingerprint: FingerprintRecord,
    /// RFC 3339 UTC timestamp of the last observation.
    pub last_observed: String,
}

/// The outcome of attempting to resolve a note's identity.
///
/// Callers should branch on this enum rather than on error codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// A single matching record was found; the note retains this ID.
    Resolved(NoteId),
    /// The file is an exact copy of an existing note (same fingerprint and
    /// hash at a different path). The caller should assign a new ID.
    Duplicate {
        /// The ID of the pre-existing note that this file duplicates.
        existing_id: NoteId,
        /// The path of the pre-existing note.
        existing_path: WorkspacePath,
    },
    /// Two or more records match with equal evidence; manual review needed.
    NeedsReview {
        /// The candidate note IDs that could not be disambiguated.
        candidates: Vec<NoteId>,
    },
    /// No matching record was found; this is a new note.
    New,
}

/// The portable identity map, backed by versioned JSON files.
///
/// Each call to [`IdentityMap::open`] scans the `identity-map` directory and
/// loads all valid records into memory. Corrupt or unreadable records are
/// silently skipped (treated as absent) so that a single bad file never
/// prevents the map from functioning.
pub struct IdentityMap {
    root: WorkspaceRoot,
    records: Vec<IdentityRecord>,
}

impl IdentityMap {
    /// Opens the identity map for the given workspace root.
    ///
    /// Loads all existing records from `.secondbrain/identity-map/`. Corrupt
    /// or unreadable record files are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace root cannot be opened or the
    /// identity-map directory cannot be read.
    pub fn open(root: &WorkspaceRoot) -> Result<Self> {
        let dir = identity_map_dir(root.canonical_path());
        let mut records = Vec::new();

        if dir.exists() {
            for entry in fs::read_dir(&dir).map_err(|source| Error::Io {
                operation: "read identity-map directory",
                source,
            })? {
                let entry = entry.map_err(|source| Error::Io {
                    operation: "read identity-map entry",
                    source,
                })?;

                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some(RECORD_EXTENSION) {
                    continue;
                }

                // Attempt to load the record; skip on failure.
                if let Ok(record) = load_record(&path) {
                    records.push(record);
                }
            }
        }

        Ok(Self {
            root: root.clone(),
            records,
        })
    }

    /// Registers a new note, creating a persistent identity record.
    ///
    /// If a record already exists for this exact path with the same hash and
    /// fingerprint, the existing ID is returned and the record is refreshed.
    /// Otherwise, a new `NoteId` is generated and a new record is written
    /// atomically.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be serialized or written.
    pub fn register(
        &mut self,
        path: &WorkspacePath,
        hash: ContentHash,
        fingerprint: secondbrain_markdown::Fingerprint,
    ) -> Result<NoteId> {
        // Check if there's already a record for this exact path + hash + fp.
        for record in &self.records {
            if &record.current_path == path
                && record.source_hash == hash
                && record.fingerprint.lo == fingerprint.lo
                && record.fingerprint.hi == fingerprint.hi
            {
                // Same path, same content — return the existing ID.
                return Ok(record.note_id);
            }
        }

        // No existing record matches — create a new one.
        let id = NoteId::new();
        let fp_record = FingerprintRecord {
            lo: fingerprint.lo,
            hi: fingerprint.hi,
        };

        let historical = vec![path.clone()];

        // If there's a record with the same fingerprint+hash at a different
        // path (exact copy scenario), we still create a new ID for this path.
        // The existing record is untouched.

        let record = IdentityRecord {
            version: RECORD_FORMAT_VERSION,
            note_id: id,
            current_path: path.clone(),
            historical_paths: historical,
            source_hash: hash,
            fingerprint: fp_record,
            last_observed: now_rfc3339_utc(),
        };

        self.save_record(&record)?;
        self.records.push(record);

        Ok(id)
    }

    /// Looks up the identity record for a given note ID.
    ///
    /// Returns `Ok(None)` if no record exists for this ID.
    ///
    /// # Errors
    ///
    /// Returns an error only on I/O failure (not on missing records).
    pub fn lookup(&self, id: &NoteId) -> Result<Option<IdentityRecord>> {
        Ok(self.records.iter().find(|r| &r.note_id == id).cloned())
    }

    /// Resolves the stable identity for a file at `path` with the given hash
    /// and fingerprint.
    ///
    /// See the module-level documentation for the resolution strategy.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    pub fn resolve_identity(
        &self,
        path: &WorkspacePath,
        hash: ContentHash,
        fingerprint: secondbrain_markdown::Fingerprint,
    ) -> Result<RecoveryOutcome> {
        let fp_lo = fingerprint.lo;
        let fp_hi = fingerprint.hi;

        // Collect all records that match by any evidence.
        // A record matches if:
        //   - Path matches (current or historical), OR
        //   - Fingerprint + hash match (exact), OR
        //   - Fingerprint-only matches (body text changed but structure same)

        let mut path_matches: Vec<&IdentityRecord> = Vec::new();
        let mut exact_at_same_path: Vec<&IdentityRecord> = Vec::new();
        let mut exact_at_different_path: Vec<&IdentityRecord> = Vec::new();
        let mut fingerprint_only_matches: Vec<&IdentityRecord> = Vec::new();

        for record in &self.records {
            let path_match =
                &record.current_path == path || record.historical_paths.iter().any(|p| p == path);
            let fp_match = record.fingerprint.lo == fp_lo && record.fingerprint.hi == fp_hi;
            let hash_match = record.source_hash == hash;

            if path_match {
                path_matches.push(record);
            }
            if fp_match && hash_match {
                if path_match {
                    exact_at_same_path.push(record);
                } else {
                    exact_at_different_path.push(record);
                }
            }
            if fp_match && !hash_match && !path_match {
                fingerprint_only_matches.push(record);
            }
        }

        // --- Priority (a): Path match ---
        // If the current path matches a record's current_path or historical_paths,
        // resolve to that record's ID. This covers renames where the file was
        // moved and its path history is known, and frontmatter ID removal where
        // the path is unchanged.
        if path_matches.len() == 1 {
            return Ok(RecoveryOutcome::Resolved(path_matches[0].note_id));
        }
        if path_matches.len() > 1 {
            let candidates: Vec<NoteId> = path_matches.iter().map(|r| r.note_id).collect();
            return Ok(RecoveryOutcome::NeedsReview { candidates });
        }

        // --- Priority (b): Exact match at SAME path ---
        // Same fingerprint + same hash + same path → resolved (ID removed from
        // frontmatter but content unchanged). Since we already handled path
        // matches above, this branch is reached only when there's no path match
        // but there's an exact (fp+hash) match at the same path — which is
        // technically subsumed by path_matches, but we keep it for clarity.
        if exact_at_same_path.len() == 1 {
            return Ok(RecoveryOutcome::Resolved(exact_at_same_path[0].note_id));
        }
        if exact_at_same_path.len() > 1 {
            let candidates: Vec<NoteId> = exact_at_same_path.iter().map(|r| r.note_id).collect();
            return Ok(RecoveryOutcome::NeedsReview { candidates });
        }

        // --- Priority (c): Exact match at DIFFERENT path (Duplicate) ---
        // Same fingerprint + same hash but at a different path with no path
        // match → this is a duplicate copy of an existing note. If multiple
        // records match (pre-existing duplicates), return the first match as
        // Duplicate — the new file is still a copy of an existing note.
        if !exact_at_different_path.is_empty() {
            let r = exact_at_different_path[0];
            return Ok(RecoveryOutcome::Duplicate {
                existing_id: r.note_id,
                existing_path: r.current_path.clone(),
            });
        }

        // --- Priority (d)/(e): Fingerprint-only match (rename with body change) ---
        // If exactly one record has a matching fingerprint but different hash
        // at a different path, this is likely a rename where the body changed.
        // Resolve to that record's ID. If two or more records match with equal
        // evidence, manual review is needed.
        if fingerprint_only_matches.len() == 1 {
            return Ok(RecoveryOutcome::Resolved(
                fingerprint_only_matches[0].note_id,
            ));
        }
        if fingerprint_only_matches.len() > 1 {
            let candidates: Vec<NoteId> =
                fingerprint_only_matches.iter().map(|r| r.note_id).collect();
            return Ok(RecoveryOutcome::NeedsReview { candidates });
        }

        // --- Priority (f): No matches → New ---
        Ok(RecoveryOutcome::New)
    }

    /// Updates the current path of a note's record, preserving the old path
    /// in the historical paths list.
    ///
    /// # Errors
    ///
    /// Returns an error if the record does not exist or cannot be written.
    pub fn update_path(&mut self, id: &NoteId, new_path: &WorkspacePath) -> Result<()> {
        let record = self
            .records
            .iter_mut()
            .find(|r| &r.note_id == id)
            .ok_or_else(|| Error::CorruptRecord {
                record: id.to_string(),
                summary: "identity record not found for path update".into(),
            })?;

        if &record.current_path != new_path {
            if !record.historical_paths.contains(new_path) {
                record.historical_paths.push(new_path.clone());
            }
            record.current_path = new_path.clone();
            record.last_observed = now_rfc3339_utc();
        }

        // Save the updated record.
        let record_clone = record.clone();
        self.save_record(&record_clone)
    }

    /// Saves a record atomically to disk.
    fn save_record(&self, record: &IdentityRecord) -> Result<()> {
        let json = serde_json::to_string_pretty(record).map_err(|err| Error::CorruptRecord {
            record: record.note_id.to_string(),
            summary: format!("identity record serialization failed: {err}"),
        })?;

        let file_name = format!("{}.{}", record.note_id, RECORD_EXTENSION);
        let rel_path_str = format!(".secondbrain/{IDENTITY_MAP_DIR}/{file_name}");

        // WorkspacePath rejects paths starting with ".secondbrain", but the
        // identity-map lives inside .secondbrain. We write directly via the
        // workspace root's canonical path instead.
        let target = identity_map_dir(self.root.canonical_path()).join(&file_name);

        // Ensure the directory exists.
        fs::create_dir_all(target.parent().unwrap_or(Path::new("."))).map_err(|source| {
            Error::Io {
                operation: "create identity-map dir",
                source,
            }
        })?;

        // Use the atomic_write function directly (not through WorkspaceRoot,
        // since the path is under .secondbrain which WorkspacePath rejects).
        crate::atomic_write::atomic_write(&target, json.as_bytes())?;

        // Suppress unused variable warning — rel_path_str is for documentation.
        let _ = rel_path_str;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Returns the path to the identity-map directory under the workspace root.
fn identity_map_dir(root: &Path) -> PathBuf {
    root.join(".secondbrain").join(IDENTITY_MAP_DIR)
}

/// Loads a single identity record from a JSON file path.
fn load_record(path: &Path) -> Result<IdentityRecord> {
    let contents = fs::read_to_string(path).map_err(|source| Error::Io {
        operation: "read identity record",
        source,
    })?;

    serde_json::from_str(&contents).map_err(|err| Error::CorruptRecord {
        record: path.to_string_lossy().into_owned(),
        summary: format!("identity record parse failed: {err}"),
    })
}

/// Generates a minimal RFC 3339 UTC timestamp without external dependencies.
fn now_rfc3339_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let days = (secs / 86_400) as i64;
    let second_of_day = (secs % 86_400) as u32;

    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3600;
    let minute = (second_of_day % 3600) / 60;
    let second = second_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Converts days since 1970-01-01 into (year, month, day). Howard Hinnant's
/// algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

// Suppress unused import warning for BTreeSet — it may be useful in future
// extensions but is not currently used.
#[allow(dead_code)]
fn _ensure_btree_set_linked() -> BTreeSet<u64> {
    BTreeSet::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        let (y, m, d) = civil_from_days(20_454);
        assert_eq!((y, m, d), (2026, 1, 1));
    }

    #[test]
    fn now_rfc3339_utc_is_parseable_and_utc() {
        let ts = now_rfc3339_utc();
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
    }

    #[test]
    fn record_format_version_is_one() {
        assert_eq!(RECORD_FORMAT_VERSION, 1);
    }
}
