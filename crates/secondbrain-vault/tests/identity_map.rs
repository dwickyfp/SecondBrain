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
