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
//! 3. **Exact match at different path (rename or duplicate)**: if the
//!    fingerprint and hash match a record at a *different* path (with no path
//!    match), the file is either a copy of that note or that note having moved.
//!    The bytes cannot tell those apart, so the answer depends on what the
//!    caller can say about the workspace — see [`IdentityMap::resolve_in_scan`].
//!    [`IdentityMap::resolve_identity`], which is asked about one file and
//!    knows nothing of the rest of the workspace, takes the conservative
//!    reading and returns [`RecoveryOutcome::Duplicate`]. If multiple records
//!    match (pre-existing duplicates), the first match is returned as
//!    Duplicate.
//! 4. **Fingerprint-only match (rename with body change)**: if the structural
//!    fingerprint matches but the content hash differs (body text changed but
//!    structure preserved) and there is no exact match, the record's ID is
//!    returned.
//! 5. **Ambiguity**: if two or more records match with fingerprint-only
//!    evidence (no path match, no exact match), [`RecoveryOutcome::NeedsReview`]
//!    is returned with the candidate IDs.
//! 6. **No matches**: [`RecoveryOutcome::New`] is returned.
//!
//! # Keeping the evidence current
//!
//! Rules 2 to 5 compare a file against the `source_hash` and `fingerprint` a
//! record holds, so those are only worth anything while they describe the note
//! as it is now. Nothing outside this crate can set them: they are refreshed by
//! [`crate::BaseSnapshotStore::save`], because the workspace agreeing on new
//! content for a note is exactly the event that makes the old evidence wrong,
//! and a record refreshed anywhere else would be a record that can drift from
//! the converged base. See [`IdentityMap::record_convergence`].
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
    /// Whether this workspace has no persisted identity records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

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

    /// Registers a distinct note after a complete scan established that the
    /// workspace had no prior identity records.
    ///
    /// In that case there is no identity evidence to recover and every path in
    /// the scan needs its own new ID. Avoiding a growing-map search here keeps
    /// first population linear without changing rename or ambiguity policy for
    /// any workspace that already has identity history.
    pub fn register_distinct(
        &mut self,
        path: &WorkspacePath,
        hash: ContentHash,
        fingerprint: secondbrain_markdown::Fingerprint,
    ) -> Result<NoteId> {
        let id = NoteId::new();
        let record = IdentityRecord {
            version: RECORD_FORMAT_VERSION,
            note_id: id,
            current_path: path.clone(),
            historical_paths: vec![path.clone()],
            source_hash: hash,
            fingerprint: FingerprintRecord {
                lo: fingerprint.lo,
                hi: fingerprint.hi,
            },
            last_observed: now_rfc3339_utc(),
        };
        self.save_record(&record)?;
        self.records.push(record);
        Ok(id)
    }

    /// Registers independent notes from a complete scan of an empty map.
    ///
    /// Each record still uses the normal atomic, fully durable write. The only
    /// difference is that unrelated records can wait for filesystem durability
    /// concurrently instead of serializing two sync barriers per note.
    pub fn register_distinct_batch(
        &mut self,
        notes: &[(
            WorkspacePath,
            ContentHash,
            secondbrain_markdown::Fingerprint,
        )],
    ) -> Result<Vec<NoteId>> {
        let records = notes
            .iter()
            .map(|(path, hash, fingerprint)| IdentityRecord {
                version: RECORD_FORMAT_VERSION,
                note_id: NoteId::new(),
                current_path: path.clone(),
                historical_paths: vec![path.clone()],
                source_hash: *hash,
                fingerprint: FingerprintRecord {
                    lo: fingerprint.lo,
                    hi: fingerprint.hi,
                },
                last_observed: now_rfc3339_utc(),
            })
            .collect::<Vec<_>>();
        let directory = identity_map_dir(self.root.canonical_path());
        write_records_parallel(&directory, &records)?;
        let ids = records.iter().map(|record| record.note_id).collect();
        self.records.extend(records);
        Ok(ids)
    }

    /// Registers the identity a note declares in its own frontmatter.
    ///
    /// Indexing has already rejected two files declaring the same ID before it
    /// calls this. An existing record is left untouched: its path and content
    /// evidence describe the last state the workspace converged on, and a
    /// rebuild must not silently adopt an external edit over that evidence.
    ///
    /// # Errors
    ///
    /// Returns an error if the new record cannot be serialized or written.
    pub fn register_known(
        &mut self,
        note_id: NoteId,
        path: &WorkspacePath,
        hash: ContentHash,
        fingerprint: secondbrain_markdown::Fingerprint,
    ) -> Result<()> {
        if self.records.iter().any(|record| record.note_id == note_id) {
            return Ok(());
        }

        let record = IdentityRecord {
            version: RECORD_FORMAT_VERSION,
            note_id,
            current_path: path.clone(),
            historical_paths: vec![path.clone()],
            source_hash: hash,
            fingerprint: FingerprintRecord {
                lo: fingerprint.lo,
                hi: fingerprint.hi,
            },
            last_observed: now_rfc3339_utc(),
        };
        self.save_record(&record)?;
        self.records.push(record);
        Ok(())
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

    /// The note currently living at `path`, when exactly one record claims it.
    ///
    /// [`Self::resolve_identity`] answers a question about a file's *content*
    /// and cannot be asked about a path whose file is gone. A deletion is
    /// exactly that case, so it is answered here instead — from the record's
    /// `current_path` only. Historical paths are deliberately not consulted: a
    /// path a note has moved away from disappearing is that move completing,
    /// not the note going with it.
    ///
    /// Returns `None` when no record claims the path, and also when more than
    /// one does. A file vanishing is not the occasion to resolve an ambiguity
    /// between two records — there is no content left to resolve it with, and
    /// naming one of them would be a guess.
    #[must_use]
    pub fn note_at(&self, path: &WorkspacePath) -> Option<NoteId> {
        let mut claiming = self
            .records
            .iter()
            .filter(|record| &record.current_path == path);
        let first = claiming.next()?;
        claiming.next().is_none().then_some(first.note_id)
    }

    /// Resolves the stable identity for a file at `path` with the given hash
    /// and fingerprint.
    ///
    /// The caller is asked about one file and says nothing about the rest of
    /// the workspace, so an exact match at another path is read conservatively
    /// as a copy. A caller that has just walked the whole workspace can do
    /// better and should use [`Self::resolve_in_scan`].
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
        self.classify(path, hash, fingerprint, None)
    }

    /// Resolves the identity of a file seen during a walk of the whole
    /// workspace, recording the move when the resolution is one.
    ///
    /// `present` is every note path the caller found in that walk. It is what
    /// separates a rename from a copy, which nothing in the bytes can: both
    /// present as the same fingerprint and hash at a path no record claims, and
    /// the only difference is whether the file the matched record describes is
    /// still there. If it is, the new file is a second copy of that note and
    /// must be told apart from it. If it is gone, the note moved, and its
    /// identity has to move with it — forking instead would leave the note's
    /// backlinks, converged base and journal attached to a record claiming a
    /// path nothing occupies.
    ///
    /// The map cannot learn this on its own without walking a workspace it does
    /// not own and cannot scope, so the caller that just walked it says. The
    /// rule for what to do with the answer stays here: a caller that decided
    /// for itself when an identity may move would be writing identity policy.
    ///
    /// The record is rewritten only when the note really did move — a file
    /// found where its record already says it is costs no write.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure, or if the moved record cannot be
    /// written.
    pub fn resolve_in_scan(
        &mut self,
        path: &WorkspacePath,
        hash: ContentHash,
        fingerprint: secondbrain_markdown::Fingerprint,
        present: &BTreeSet<WorkspacePath>,
    ) -> Result<RecoveryOutcome> {
        let outcome = self.classify(path, hash, fingerprint, Some(present))?;
        if let RecoveryOutcome::Resolved(note_id) = outcome
            && self.has_moved_to(note_id, path, present)
        {
            self.update_path(&note_id, path)?;
        }
        Ok(outcome)
    }

    /// Whether `note_id`'s record describes a note that has moved to `path`.
    ///
    /// A move is the record naming some *other* path, and that path standing
    /// empty. A record whose path is still occupied describes a file that is
    /// still there, so whatever resolved to it here — a path this note once
    /// held, most often — is not that note relocating, and rewriting the record
    /// would point it away from a file that exists.
    fn has_moved_to(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        present: &BTreeSet<WorkspacePath>,
    ) -> bool {
        self.records
            .iter()
            .find(|record| record.note_id == note_id)
            .is_some_and(|record| {
                &record.current_path != path && !present.contains(&record.current_path)
            })
    }

    /// The resolution strategy itself, shared by both entry points.
    ///
    /// `present` is `None` when the caller cannot say which paths hold a file.
    fn classify(
        &self,
        path: &WorkspacePath,
        hash: ContentHash,
        fingerprint: secondbrain_markdown::Fingerprint,
        present: Option<&BTreeSet<WorkspacePath>>,
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

        // --- Priority (c): Exact match at DIFFERENT path (rename or copy) ---
        // Same fingerprint + same hash at a different path with no path match.
        // This is either a copy of that note or that note having moved, and the
        // content is the same either way. A caller that walked the workspace
        // can say which: a record whose path no longer holds a file describes a
        // note that moved, and only a note that moved can have moved here.
        if !exact_at_different_path.is_empty() {
            let vacated: Vec<&IdentityRecord> = exact_at_different_path
                .iter()
                .copied()
                .filter(|record| present.is_some_and(|paths| !paths.contains(&record.current_path)))
                .collect();
            // Several notes that all moved and all read alike are matched by
            // this file equally well, and naming one would be a guess dressed
            // as a resolution.
            if vacated.len() > 1 {
                let candidates: Vec<NoteId> = vacated.iter().map(|r| r.note_id).collect();
                return Ok(RecoveryOutcome::NeedsReview { candidates });
            }
            if let [moved] = vacated[..] {
                return Ok(RecoveryOutcome::Resolved(moved.note_id));
            }
            // Every match is a file that is still where its record says. None
            // of them moved, so this is a copy of one of them. If several
            // match (pre-existing duplicates), the first is named — the new
            // file is a copy of an existing note either way.
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
        let fingerprint_candidates: Vec<&IdentityRecord> = fingerprint_only_matches
            .into_iter()
            .filter(|record| present.is_none_or(|paths| !paths.contains(&record.current_path)))
            .collect();
        if fingerprint_candidates.len() == 1 {
            return Ok(RecoveryOutcome::Resolved(fingerprint_candidates[0].note_id));
        }
        if fingerprint_candidates.len() > 1 {
            let candidates: Vec<NoteId> =
                fingerprint_candidates.iter().map(|r| r.note_id).collect();
            return Ok(RecoveryOutcome::NeedsReview { candidates });
        }

        // --- Priority (f): No matches → New ---
        Ok(RecoveryOutcome::New)
    }

    /// Updates the current path of a note's record, preserving the old path
    /// in the historical paths list.
    ///
    /// The record is re-read from disk before the path is changed. A map is
    /// loaded once and can be held for the life of a watcher, so the copy in
    /// memory may predate a convergence that refreshed the record's content
    /// evidence — and writing that copy back out would silently undo the
    /// refresh, leaving the evidence describing bytes the note has moved past.
    ///
    /// # Errors
    ///
    /// Returns an error if the record does not exist or cannot be written.
    pub fn update_path(&mut self, id: &NoteId, new_path: &WorkspacePath) -> Result<()> {
        let slot = self
            .records
            .iter()
            .position(|r| &r.note_id == id)
            .ok_or_else(|| Error::CorruptRecord {
                record: id.to_string(),
                summary: "identity record not found for path update".into(),
            })?;

        if self.records[slot].current_path == *new_path {
            return Ok(());
        }

        let mut record = current_record(&identity_map_dir(self.root.canonical_path()), id)
            .unwrap_or_else(|| self.records[slot].clone());

        if &record.current_path != new_path {
            if !record.historical_paths.contains(new_path) {
                record.historical_paths.push(new_path.clone());
            }
            record.current_path = new_path.clone();
            record.last_observed = now_rfc3339_utc();
        }

        self.save_record(&record)?;
        self.records[slot] = record;
        Ok(())
    }

    /// Refreshes the content evidence of `note_id` to the bytes the workspace
    /// has just converged on.
    ///
    /// This is deliberately not a public setter, and deliberately not a method
    /// on an opened map. The `source_hash` and `fingerprint` a record carries
    /// are the workspace's answer to "what does this note look like now", and
    /// so is the converged base — [`crate::BaseSnapshotStore`] writes both in
    /// one step so that a caller cannot refresh one and forget the other. Two
    /// public setters is what let them drift, and drift makes the whole
    /// rename-recovery strategy inert on any note that has ever been edited.
    ///
    /// The record is read from disk and written back, rather than mutated
    /// through some caller's opened map, because the caller that converges a
    /// note need not hold one — and because reading the single record this
    /// touches keeps a per-note write from costing a scan of every record.
    ///
    /// A note with no record — one whose identity came from its own
    /// frontmatter, for instance — has no evidence to refresh, and that is not
    /// an error. Nor is a record that already describes these bytes: it costs
    /// no write, which is what lets a rebuild over an unchanged workspace stay
    /// free.
    ///
    /// # Errors
    ///
    /// Returns an error if the refreshed record cannot be serialized or
    /// written.
    pub(crate) fn record_convergence(
        root: &WorkspaceRoot,
        note_id: NoteId,
        hash: ContentHash,
        fingerprint: secondbrain_markdown::Fingerprint,
    ) -> Result<()> {
        let directory = identity_map_dir(root.canonical_path());
        let Some(mut record) = current_record(&directory, &note_id) else {
            return Ok(());
        };
        let refreshed = FingerprintRecord {
            lo: fingerprint.lo,
            hi: fingerprint.hi,
        };
        if record.source_hash == hash && record.fingerprint == refreshed {
            return Ok(());
        }
        record.source_hash = hash;
        record.fingerprint = refreshed;
        record.last_observed = now_rfc3339_utc();
        write_record(&directory, &record)
    }

    /// Saves a record atomically to disk.
    fn save_record(&self, record: &IdentityRecord) -> Result<()> {
        write_record(&identity_map_dir(self.root.canonical_path()), record)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Returns the path to the identity-map directory under the workspace root.
fn identity_map_dir(root: &Path) -> PathBuf {
    root.join(".secondbrain").join(IDENTITY_MAP_DIR)
}

/// The record file one note's identity is filed in.
fn record_path(directory: &Path, note_id: &NoteId) -> PathBuf {
    directory.join(format!("{note_id}.{RECORD_EXTENSION}"))
}

/// The record `note_id` has on disk right now, or `None` when it has none this
/// build can read.
///
/// An unreadable record is treated as absent for the same reason
/// [`IdentityMap::open`] skips one: a single bad file may not stop the map
/// working. A caller amending a record it cannot read falls back to whatever it
/// already holds.
fn current_record(directory: &Path, note_id: &NoteId) -> Option<IdentityRecord> {
    let record = load_record(&record_path(directory, note_id)).ok()?;
    (&record.note_id == note_id).then_some(record)
}

/// Writes one identity record atomically into `directory`.
///
/// `WorkspacePath` rejects paths starting with `.secondbrain`, and the
/// identity map lives inside it, so the atomic-write helper is used directly
/// against the workspace's canonical path rather than through
/// [`WorkspaceRoot`].
fn write_record(directory: &Path, record: &IdentityRecord) -> Result<()> {
    let json = serde_json::to_string_pretty(record).map_err(|err| Error::CorruptRecord {
        record: record.note_id.to_string(),
        summary: format!("identity record serialization failed: {err}"),
    })?;

    fs::create_dir_all(directory).map_err(|source| Error::Io {
        operation: "create identity-map dir",
        source,
    })?;

    crate::atomic_write::atomic_write(&record_path(directory, &record.note_id), json.as_bytes())?;
    Ok(())
}

fn write_records_parallel(directory: &Path, records: &[IdentityRecord]) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(records.len());
    let chunk_size = records.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let handles = records
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .try_for_each(|record| write_record(directory, record))
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().map_err(|_| Error::CorruptRecord {
                record: directory.display().to_string(),
                summary: "identity record writer panicked".into(),
            })??;
        }
        Ok(())
    })
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
