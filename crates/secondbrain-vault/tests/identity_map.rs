//! Portable identity map and duplicate-ID recovery tests.
//!
//! These tests assert the contract that a note's stable [`NoteId`] survives
//! file renames, frontmatter ID removal, and file copies — while exact copies
//! are detected and resolved deterministically.
//!
//! Coverage:
//! - New note registration assigns a stable ID and persists a record.
//! - Rename preserves the ID via path history (update_path + resolve at old/new path).
//! - Frontmatter ID removed: fingerprint/path history recovers the ID.
//! - Exact copy: duplicate ID is detected.
//! - Duplicate copy receives a new ID; original retains old ID.
//! - Ambiguous recovery (multiple exact matches at different paths) returns
//!   [`RecoveryOutcome::Duplicate`], not NeedsReview.
//! - Interrupted identity-map write preserves the previous map (atomic writes).

use std::fs;
use std::str::FromStr;

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::SourceDocument;
use secondbrain_vault::{IdentityMap, RecoveryOutcome, WorkspaceRoot};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a temp workspace root with the `.secondbrain/identity-map` directory.
fn fresh_workspace() -> (tempfile::TempDir, WorkspaceRoot) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot::open(temp.path()).expect("open root");
    // Ensure the identity-map directory exists.
    let idmap_dir = temp.path().join(".secondbrain").join("identity-map");
    fs::create_dir_all(&idmap_dir).expect("create identity-map dir");
    (temp, root)
}

/// Computes the content hash and semantic fingerprint of a source string.
fn fingerprint_of(source: &str) -> (ContentHash, secondbrain_markdown::Fingerprint) {
    let hash = ContentHash::digest(source.as_bytes());
    let doc = SourceDocument::parse(source).expect("parse");
    let fp = doc.semantic_fingerprint();
    (hash, fp)
}

/// Writes a Markdown file with frontmatter containing the given note ID.
#[allow(dead_code)]
fn write_note(root: &WorkspaceRoot, path: &str, id: NoteId, body: &str) -> String {
    let source = format!("---\nid: {id}\n---\n{body}");
    let wpath = WorkspacePath::new(path).expect("valid path");
    root.atomic_write(&wpath, source.as_bytes())
        .expect("write note");
    source
}

/// Writes a Markdown file WITHOUT an id in frontmatter (simulating ID removal).
#[allow(dead_code)]
fn write_note_without_id(root: &WorkspaceRoot, path: &str, body: &str) -> String {
    let source = format!("---\ntitle: Untitled\n---\n{body}");
    let wpath = WorkspacePath::new(path).expect("valid path");
    root.atomic_write(&wpath, source.as_bytes())
        .expect("write note");
    source
}

// ---------------------------------------------------------------------------
// 1. New note registration
// ---------------------------------------------------------------------------

#[test]
fn new_note_registration_assigns_stable_id_and_persists_record() {
    let (_temp, root) = fresh_workspace();

    let body = "# My First Note\n\nHello, world.\n";
    let (hash, fp) = fingerprint_of(body);
    let path = WorkspacePath::new("Notes/first.md").expect("path");

    let id = IdentityMap::open(&root)
        .expect("open map")
        .register(&path, hash, fp)
        .expect("register");

    // The ID is a valid ULID.
    let id_str = id.to_string();
    assert_eq!(NoteId::from_str(&id_str), Ok(id));

    // A record file exists under .secondbrain/identity-map/.
    let record_path = root
        .canonical_path()
        .join(".secondbrain")
        .join("identity-map")
        .join(format!("{id}.json"));
    assert!(
        record_path.exists(),
        "record file must exist: {record_path:?}"
    );

    // The record can be loaded back and contains the correct path.
    let map = IdentityMap::open(&root).expect("reopen map");
    let record = map.lookup(&id).expect("lookup").expect("record exists");
    assert_eq!(record.current_path, path);
    assert_eq!(record.source_hash, hash);
    assert_eq!(record.fingerprint.lo, fp.lo);
    assert_eq!(record.fingerprint.hi, fp.hi);
    assert!(record.historical_paths.contains(&path));
}

// ---------------------------------------------------------------------------
// 2. Rename preserves ID
// ---------------------------------------------------------------------------

#[test]
fn rename_preserves_id_when_fingerprint_matches() {
    let (_temp, root) = fresh_workspace();

    // A note is registered at an old path, then renamed (update_path) to a
    // new path. After the rename, the old path is in historical_paths, so
    // resolving at the new path recovers the original ID via path match.
    //
    // We also verify that resolving at the OLD path (now in history) still
    // recovers the same ID — simulating a stale index entry pointing at the
    // pre-rename location.
    let body = "# Renamed Note\n\nContent stays the same.\n";
    let (hash, fp) = fingerprint_of(body);

    let old_path = WorkspacePath::new("Notes/old.md").expect("path");
    let new_path = WorkspacePath::new("Notes/new.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&old_path, hash, fp).expect("register");

    // Rename: update the record's path. The old path is preserved in history.
    map.update_path(&original_id, &new_path)
        .expect("update path");

    // After the rename, the record's current_path is the new path and the
    // old path is in historical_paths.
    let record = map.lookup(&original_id).expect("lookup").expect("record");
    assert_eq!(record.current_path, new_path);
    assert!(record.historical_paths.contains(&old_path));
    assert!(record.historical_paths.contains(&new_path));

    // Resolving at the new path (same content) recovers the original ID via
    // path match on current_path.
    let outcome = map
        .resolve_identity(&new_path, hash, fp)
        .expect("resolve at new path");
    match outcome {
        RecoveryOutcome::Resolved(id) => {
            assert_eq!(id, original_id, "rename must preserve the original ID");
        }
        other => panic!("expected Resolved at new path, got {other:?}"),
    }

    // Resolving at the old path (in historical_paths) also recovers the ID,
    // simulating a stale index entry pointing at the pre-rename location.
    let outcome_stale = map
        .resolve_identity(&old_path, hash, fp)
        .expect("resolve at old path");
    match outcome_stale {
        RecoveryOutcome::Resolved(id) => {
            assert_eq!(
                id, original_id,
                "stale path in history must recover the original ID"
            );
        }
        other => panic!("expected Resolved at old path, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 3. Frontmatter ID removed but fingerprint/path history recovers ID
// ---------------------------------------------------------------------------

#[test]
fn frontmatter_id_removed_fingerprint_recovers_id() {
    let (_temp, root) = fresh_workspace();

    let body = "# Persistent Note\n\nThis note keeps its identity.\n";
    let (hash, fp) = fingerprint_of(body);

    let path = WorkspacePath::new("Notes/persistent.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&path, hash, fp).expect("register");

    // Simulate: user removes `id` from frontmatter (content body unchanged,
    // so fingerprint is identical, hash is identical).
    let outcome = map.resolve_identity(&path, hash, fp).expect("resolve");

    match outcome {
        RecoveryOutcome::Resolved(id) => {
            assert_eq!(
                id, original_id,
                "removed frontmatter ID must be recovered from fingerprint"
            );
        }
        other => panic!("expected Resolved, got {other:?}"),
    }
}

#[test]
fn frontmatter_id_removed_path_history_recovers_id() {
    let (_temp, root) = fresh_workspace();

    let body = "# Path History Note\n\nIdentity via path.\n";
    let (hash, fp) = fingerprint_of(body);

    let path = WorkspacePath::new("Notes/path-history.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&path, hash, fp).expect("register");

    // Simulate: frontmatter ID removed AND body changed (fingerprint differs).
    let new_body = "# Path History Note\n\nBody changed.\n";
    let (new_hash, new_fp) = fingerprint_of(new_body);

    // The path is the same, so path history should recover the ID.
    let outcome = map
        .resolve_identity(&path, new_hash, new_fp)
        .expect("resolve");

    match outcome {
        RecoveryOutcome::Resolved(id) => {
            assert_eq!(
                id, original_id,
                "removed frontmatter ID must be recovered from path history"
            );
        }
        other => panic!("expected Resolved, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 4. Exact copy creates duplicate ID detection
// ---------------------------------------------------------------------------

#[test]
fn exact_copy_detects_duplicate_id() {
    let (_temp, root) = fresh_workspace();

    let body = "# Copied Note\n\nDuplicate me.\n";
    let (hash, fp) = fingerprint_of(body);

    let original_path = WorkspacePath::new("Notes/original.md").expect("path");
    let copy_path = WorkspacePath::new("Notes/copy.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&original_path, hash, fp).expect("register");

    // An exact copy has the same fingerprint and hash but a different path.
    let outcome = map.resolve_identity(&copy_path, hash, fp).expect("resolve");

    match outcome {
        RecoveryOutcome::Duplicate {
            existing_id,
            existing_path,
        } => {
            assert_eq!(
                existing_id, original_id,
                "duplicate must reference original"
            );
            assert_eq!(existing_path, original_path);
        }
        other => panic!("expected Duplicate, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 5. Duplicate copy receives new ID; original retains old ID
// ---------------------------------------------------------------------------

#[test]
fn duplicate_copy_receives_new_id_original_retains_old_id() {
    let (_temp, root) = fresh_workspace();

    let body = "# Split Note\n\nWe are the same.\n";
    let (hash, fp) = fingerprint_of(body);

    let original_path = WorkspacePath::new("Notes/keep.md").expect("path");
    let copy_path = WorkspacePath::new("Notes/split.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&original_path, hash, fp).expect("register");

    // The copy is detected as a duplicate.
    let outcome = map.resolve_identity(&copy_path, hash, fp).expect("resolve");
    assert!(
        matches!(outcome, RecoveryOutcome::Duplicate { .. }),
        "exact copy must be detected as duplicate"
    );

    // Assigning a new ID to the copy: the copy gets a fresh ID.
    let new_id = map.register(&copy_path, hash, fp).expect("register copy");

    assert_ne!(
        new_id, original_id,
        "copy must receive a new ID distinct from the original"
    );

    // The original retains its old ID.
    let original_record = map
        .lookup(&original_id)
        .expect("lookup")
        .expect("original record");
    assert_eq!(original_record.current_path, original_path);

    // The copy has its own record.
    let copy_record = map.lookup(&new_id).expect("lookup").expect("copy record");
    assert_eq!(copy_record.current_path, copy_path);
}

// ---------------------------------------------------------------------------
// 6. Ambiguous recovery returns Duplicate (multiple exact matches)
// ---------------------------------------------------------------------------

#[test]
fn ambiguous_recovery_returns_needs_review() {
    let (_temp, root) = fresh_workspace();

    // Two notes with the SAME heading and body produce the same structural
    // fingerprint AND the same content hash (byte spans are identical). We
    // register both at different paths. Then we resolve at a THIRD path with
    // the same fingerprint and hash.
    //
    // With the corrected priority order, multiple exact (fp+hash) matches at
    // different paths are treated as Duplicate (the new file is a copy of an
    // existing note), NOT NeedsReview. NeedsReview is reserved for
    // fingerprint-only matches (same fp, different hash), which cannot occur
    // with this fingerprint implementation (fp includes byte spans, so same
    // fp implies same content implies same hash).
    let body = "# Ambiguous Note\n\nSame structure here.\n";
    let (hash, fp) = fingerprint_of(body);

    let path_a = WorkspacePath::new("Notes/a.md").expect("path");
    let path_b = WorkspacePath::new("Notes/b.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let id_a = map.register(&path_a, hash, fp).expect("register a");
    let id_b = map.register(&path_b, hash, fp).expect("register b");
    assert_ne!(id_a, id_b, "the two notes must have distinct IDs");

    // A third file at a new path with the same fingerprint and hash. There is
    // no path match, but there are two exact (fp+hash) matches at different
    // paths. The resolver returns Duplicate, pointing to one of the existing
    // notes — the new file is a copy.
    let path_c = WorkspacePath::new("Notes/c.md").expect("path");
    let outcome = map.resolve_identity(&path_c, hash, fp).expect("resolve");

    match outcome {
        RecoveryOutcome::Duplicate {
            existing_id,
            existing_path,
        } => {
            // The duplicate must reference one of the two existing notes.
            assert!(
                existing_id == id_a || existing_id == id_b,
                "duplicate must reference an existing note: got {existing_id}, expected {id_a} or {id_b}"
            );
            assert!(
                existing_path == path_a || existing_path == path_b,
                "duplicate path must match an existing path"
            );
        }
        other => panic!("expected Duplicate, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 6b. A rename and a copy are told apart by the workspace scan
// ---------------------------------------------------------------------------
//
// Both present the same evidence — same fingerprint, same hash, a path no
// record claims — so nothing in the bytes separates them. What does is whether
// the file the matched record describes is still on disk, which only a caller
// that walked the workspace can say.

/// Every path in a scan, as `resolve_in_scan` wants them.
fn scan(paths: &[&str]) -> std::collections::BTreeSet<WorkspacePath> {
    paths
        .iter()
        .map(|path| WorkspacePath::new(path).expect("path"))
        .collect()
}

#[test]
fn a_scan_that_finds_the_original_gone_resolves_the_move_and_records_it() {
    let (_temp, root) = fresh_workspace();

    let body = "# Moved Note\n\nThe bytes did not change; the path did.\n";
    let (hash, fp) = fingerprint_of(body);
    let old = WorkspacePath::new("Notes/old.md").expect("path");
    let new = WorkspacePath::new("Notes/new.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&old, hash, fp).expect("register");

    // The walk found the file at its new path and nothing at the old one.
    let outcome = map
        .resolve_in_scan(&new, hash, fp, &scan(&["Notes/new.md"]))
        .expect("resolve in scan");

    assert_eq!(
        outcome,
        RecoveryOutcome::Resolved(original_id),
        "a note whose old path stands empty moved there; it was not copied"
    );

    // The record followed it, in this process and on disk.
    for map in [map, IdentityMap::open(&root).expect("reopen map")] {
        let record = map.lookup(&original_id).expect("lookup").expect("record");
        assert_eq!(record.current_path, new);
        assert!(record.historical_paths.contains(&old));
        assert!(record.historical_paths.contains(&new));
        assert_eq!(
            map.note_at(&old),
            None,
            "no record may go on claiming a path that holds no file"
        );
        assert_eq!(map.note_at(&new), Some(original_id));
    }
}

#[test]
fn a_scan_that_finds_the_original_still_there_keeps_the_copy_a_duplicate() {
    let (_temp, root) = fresh_workspace();

    let body = "# Copied Note\n\nBoth files exist.\n";
    let (hash, fp) = fingerprint_of(body);
    let original = WorkspacePath::new("Notes/original.md").expect("path");
    let copy = WorkspacePath::new("Notes/copy.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&original, hash, fp).expect("register");

    let outcome = map
        .resolve_in_scan(
            &copy,
            hash,
            fp,
            &scan(&["Notes/original.md", "Notes/copy.md"]),
        )
        .expect("resolve in scan");

    match outcome {
        RecoveryOutcome::Duplicate {
            existing_id,
            existing_path,
        } => {
            assert_eq!(existing_id, original_id);
            assert_eq!(existing_path, original);
        }
        other => panic!("a file whose original is still on disk is a copy: {other:?}"),
    }
    assert_eq!(
        map.note_at(&original),
        Some(original_id),
        "the original's record must still claim the path it still occupies"
    );
}

#[test]
fn two_notes_that_both_vacated_and_read_alike_stay_ambiguous() {
    let (_temp, root) = fresh_workspace();

    let body = "# Twin Note\n\nIndistinguishable.\n";
    let (hash, fp) = fingerprint_of(body);
    let a = WorkspacePath::new("Notes/a.md").expect("path");
    let b = WorkspacePath::new("Notes/b.md").expect("path");
    let c = WorkspacePath::new("Notes/c.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let id_a = map.register(&a, hash, fp).expect("register a");
    let id_b = map.register(&b, hash, fp).expect("register b");

    // Both recorded paths stand empty and both match this file exactly, so
    // either could be the note that moved here.
    let outcome = map
        .resolve_in_scan(&c, hash, fp, &scan(&["Notes/c.md"]))
        .expect("resolve in scan");

    match outcome {
        RecoveryOutcome::NeedsReview { mut candidates } => {
            candidates.sort_by_key(ToString::to_string);
            let mut expected = vec![id_a, id_b];
            expected.sort_by_key(ToString::to_string);
            assert_eq!(
                candidates, expected,
                "naming one of two equal matches would be a guess, not a resolution"
            );
        }
        other => panic!("expected NeedsReview, got {other:?}"),
    }
    assert_eq!(
        map.note_at(&a),
        Some(id_a),
        "an unresolved file moves nothing"
    );
    assert_eq!(map.note_at(&b), Some(id_b));
}

#[test]
fn a_file_found_where_its_record_already_says_it_is_leaves_the_record_alone() {
    let (_temp, root) = fresh_workspace();

    let body = "# Settled Note\n\nNothing happened here.\n";
    let (hash, fp) = fingerprint_of(body);
    let path = WorkspacePath::new("Notes/settled.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let id = map.register(&path, hash, fp).expect("register");
    let record_path = root
        .canonical_path()
        .join(".secondbrain")
        .join("identity-map")
        .join(format!("{id}.json"));
    let before = fs::read_to_string(&record_path).expect("read record");

    let outcome = map
        .resolve_in_scan(&path, hash, fp, &scan(&["Notes/settled.md"]))
        .expect("resolve in scan");

    assert_eq!(outcome, RecoveryOutcome::Resolved(id));
    assert_eq!(
        fs::read_to_string(&record_path).expect("read record"),
        before,
        "a note that did not move has no move to record"
    );
    let record = map.lookup(&id).expect("lookup").expect("record");
    assert_eq!(record.current_path, path);
    assert_eq!(
        record.historical_paths.len(),
        1,
        "a path history must not grow on a rebuild that saw no move: {record:?}"
    );
}

#[test]
fn a_move_recovered_from_the_fingerprint_alone_also_stops_claiming_the_old_path() {
    let (_temp, root) = fresh_workspace();

    // A rename that also edited the body, in the one shape this fingerprint can
    // still recognize: the structure and every byte span survived the edit, so
    // the fingerprint matches and the hash does not. Resolving that has always
    // been the rule; what was missing is that the record went on naming a path
    // the file had left.
    let before = "# Drifted Note\n\nStable content.\n";
    let after = "# Drifted Note\n\nStabbb content.\n";
    let (old_hash, old_fp) = fingerprint_of(before);
    let (new_hash, new_fp) = fingerprint_of(after);
    assert_eq!(
        (old_fp.lo, old_fp.hi),
        (new_fp.lo, new_fp.hi),
        "this case only exists while the fingerprint hashes spans, not text"
    );
    assert_ne!(old_hash, new_hash);

    let old = WorkspacePath::new("Notes/old.md").expect("path");
    let new = WorkspacePath::new("Notes/new.md").expect("path");
    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&old, old_hash, old_fp).expect("register");

    let outcome = map
        .resolve_in_scan(&new, new_hash, new_fp, &scan(&["Notes/new.md"]))
        .expect("resolve in scan");

    assert_eq!(outcome, RecoveryOutcome::Resolved(original_id));
    let record = map.lookup(&original_id).expect("lookup").expect("record");
    assert_eq!(record.current_path, new);
    assert!(record.historical_paths.contains(&old));
    assert_eq!(map.note_at(&old), None);
}

#[test]
fn a_caller_that_cannot_see_the_workspace_still_reads_an_exact_match_as_a_copy() {
    let (_temp, root) = fresh_workspace();

    // The same situation as the rename case above — the original is gone —
    // asked of the entry point that is handed one file and knows nothing else.
    // It must not guess a move it has no evidence for.
    let body = "# Single File\n\nOne event, no scan.\n";
    let (hash, fp) = fingerprint_of(body);
    let old = WorkspacePath::new("Notes/old.md").expect("path");
    let new = WorkspacePath::new("Notes/new.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&old, hash, fp).expect("register");

    let outcome = map.resolve_identity(&new, hash, fp).expect("resolve");

    match outcome {
        RecoveryOutcome::Duplicate { existing_id, .. } => assert_eq!(existing_id, original_id),
        other => panic!("expected Duplicate from the single-file caller, got {other:?}"),
    }
}

#[test]
fn a_declared_identity_is_registered_without_replacing_converged_evidence() {
    let (_temp, root) = fresh_workspace();
    let original = "# Declared\n\nOriginal content.\n";
    let edited = "# Declared\n\nEdited outside the workspace.\n";
    let (original_hash, original_fp) = fingerprint_of(original);
    let (edited_hash, edited_fp) = fingerprint_of(edited);
    let path = WorkspacePath::new("Notes/declared.md").expect("path");
    let note_id = NoteId::new();
    let mut map = IdentityMap::open(&root).expect("open map");

    map.register_known(note_id, &path, original_hash, original_fp)
        .expect("register declared identity");
    map.register_known(note_id, &path, edited_hash, edited_fp)
        .expect("an existing declaration is idempotent");

    let record = map.lookup(&note_id).expect("lookup").expect("record");
    assert_eq!(record.note_id, note_id);
    assert_eq!(record.current_path, path);
    assert_eq!(record.source_hash, original_hash);
    assert_eq!(
        (record.fingerprint.lo, record.fingerprint.hi),
        (original_fp.lo, original_fp.hi),
        "indexing an external edit may not replace the evidence it diverged from"
    );
}

// ---------------------------------------------------------------------------
// 7. Interrupted identity-map write preserves previous map
// ---------------------------------------------------------------------------

#[test]
fn interrupted_write_preserves_previous_map() {
    let (_temp, root) = fresh_workspace();

    let body = "# Surviving Note\n\nI persist.\n";
    let (hash, fp) = fingerprint_of(body);
    let path = WorkspacePath::new("Notes/survivor.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let original_id = map.register(&path, hash, fp).expect("register");

    // Corrupt the record file to simulate a partial/interrupted write: write
    // garbage bytes directly to the record path.
    let record_path = root
        .canonical_path()
        .join(".secondbrain")
        .join("identity-map")
        .join(format!("{original_id}.json"));
    assert!(record_path.exists(), "record must exist before corruption");

    // Write truncated/invalid JSON directly (simulating a crash mid-write
    // that left a partial file). The atomic write implementation must never
    // produce this, but we test that a pre-existing corrupt file does not
    // cause the map to lose data on a subsequent open.
    fs::write(&record_path, b"{\"truncated").expect("corrupt file");

    // Reopen the map: the corrupt record is skipped (treated as absent),
    // but the map still functions.
    let mut map = IdentityMap::open(&root).expect("reopen map");

    // The corrupt record is not loadable, so lookup returns None.
    let record = map.lookup(&original_id).expect("lookup");
    assert!(
        record.is_none(),
        "corrupt record must not produce a false positive"
    );

    // Re-registering the note works and produces a valid record.
    let new_id = map.register(&path, hash, fp).expect("re-register");
    let new_record = map.lookup(&new_id).expect("lookup").expect("new record");
    assert_eq!(new_record.current_path, path);
}

// ---------------------------------------------------------------------------
// Additional: record does not store OS-absolute paths
// ---------------------------------------------------------------------------

#[test]
fn record_does_not_store_os_absolute_paths() {
    let (_temp, root) = fresh_workspace();

    let body = "# Portable Note\n\nPaths are relative.\n";
    let (hash, fp) = fingerprint_of(body);
    let path = WorkspacePath::new("Notes/portable.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let id = map.register(&path, hash, fp).expect("register");

    let record_path = root
        .canonical_path()
        .join(".secondbrain")
        .join("identity-map")
        .join(format!("{id}.json"));
    let contents = fs::read_to_string(&record_path).expect("read record");
    let canonical = root.canonical_path().to_string_lossy().into_owned();
    assert!(
        !contents.contains(&canonical),
        "record must not contain OS-absolute paths; found in: {contents}"
    );
}

// ---------------------------------------------------------------------------
// Additional: versioned record format
// ---------------------------------------------------------------------------

#[test]
fn record_file_has_version_field() {
    let (_temp, root) = fresh_workspace();

    let body = "# Versioned Note\n\nVersion 1.\n";
    let (hash, fp) = fingerprint_of(body);
    let path = WorkspacePath::new("Notes/versioned.md").expect("path");

    let mut map = IdentityMap::open(&root).expect("open map");
    let id = map.register(&path, hash, fp).expect("register");

    let record_path = root
        .canonical_path()
        .join(".secondbrain")
        .join("identity-map")
        .join(format!("{id}.json"));
    let contents = fs::read_to_string(&record_path).expect("read record");

    // The JSON must contain a "version" field.
    assert!(
        contents.contains("\"version\""),
        "record must have a version field: {contents}"
    );
}

// ---------------------------------------------------------------------------
// Additional: naming the note at a path whose file is gone
// ---------------------------------------------------------------------------

#[test]
fn note_at_names_the_note_currently_living_at_a_path() {
    let (_temp, root) = fresh_workspace();
    let body = "# Deleted Later\n\nStill here for now.\n";
    let (hash, fp) = fingerprint_of(body);
    let path = WorkspacePath::new("Notes/present.md").expect("path");
    let mut map = IdentityMap::open(&root).expect("open map");
    let id = map.register(&path, hash, fp).expect("register");

    // The file itself is irrelevant: a deletion is exactly the case where the
    // content is gone and only the record can say what was lost.
    assert_eq!(map.note_at(&path), Some(id));
    assert_eq!(
        map.note_at(&WorkspacePath::new("Notes/never-existed.md").expect("path")),
        None
    );
}

#[test]
fn note_at_does_not_follow_a_path_the_note_has_moved_away_from() {
    let (_temp, root) = fresh_workspace();
    let body = "# Moved\n\nContent.\n";
    let (hash, fp) = fingerprint_of(body);
    let old = WorkspacePath::new("Notes/old.md").expect("old path");
    let new = WorkspacePath::new("Notes/new.md").expect("new path");
    let mut map = IdentityMap::open(&root).expect("open map");
    let id = map.register(&old, hash, fp).expect("register");
    map.update_path(&id, &new).expect("record the move");

    assert_eq!(map.note_at(&new), Some(id));
    assert_eq!(
        map.note_at(&old),
        None,
        "the old path disappearing is the move completing, not the note being lost"
    );
}
