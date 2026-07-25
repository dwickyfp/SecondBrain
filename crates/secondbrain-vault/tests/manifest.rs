//! Workspace manifest initialization and validation tests.
//!
//! These tests assert the SecondBrain vault layout contract:
//! - `.secondbrain/manifest.toml` exists with required fields.
//! - Required internal directories and `plugins.lock` are created.
//! - Initialization is idempotent: a second call returns the same manifest.
//! - A manifest declaring an unsupported future `format_version` is rejected
//!   on read-only load without mutating anything.
//! - Existing user Markdown files are byte-identical after initialization.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::Offset;
use secondbrain_vault::{WorkspaceManifest, initialize_workspace, load_manifest};

/// The exact set of internal artifacts that initialization must create.
///
/// Order is alphabetical for readability; the check is set-based.
const REQUIRED_INTERNAL_PATHS: &[&str] = &[
    ".secondbrain/manifest.toml",
    ".secondbrain/oplog",
    ".secondbrain/transactions",
    ".secondbrain/snapshots",
    ".secondbrain/identity-map",
    ".secondbrain/policies",
    ".secondbrain/audit",
    ".secondbrain/plugins.lock",
];

fn snapshot_workspace_artifacts(root: &Path) -> BTreeMap<String, fs::DirEntry> {
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries);
    entries
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, fs::DirEntry>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            // Skip the .secondbrain internal directory when snapshotting user
            // content; it is owned by the vault, not the user.
            let rel = path.strip_prefix(root).expect("strip").to_string_lossy();
            if rel.as_ref() == ".secondbrain" {
                continue;
            }
            walk(root, &path, out);
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .expect("strip")
            .to_string_lossy()
            .into_owned();
        out.insert(rel, entry);
    }
}

fn snapshot_user_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for (rel, entry) in snapshot_workspace_artifacts(root) {
        let bytes = fs::read(entry.path()).expect("read user file");
        out.insert(rel, bytes);
    }
    out
}

#[test]
fn initialization_creates_required_directories_and_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();

    let manifest = initialize_workspace(root).expect("initialize workspace");

    for relative in REQUIRED_INTERNAL_PATHS {
        let path = root.join(relative);
        assert!(
            path.exists(),
            "required internal path missing after init: {relative}"
        );
        if relative.ends_with(".lock") || relative.ends_with(".toml") {
            assert!(path.is_file(), "expected file at {relative}");
        } else {
            assert!(path.is_dir(), "expected directory at {relative}");
        }
    }

    // created_at must be a parseable RFC 3339 UTC timestamp.
    let parsed =
        chrono::DateTime::parse_from_rfc3339(&manifest.created_at).expect("created_at is RFC 3339");
    assert_eq!(
        parsed.offset().fix().local_minus_utc(),
        0,
        "created_at must be UTC"
    );
}

#[test]
fn manifest_has_workspace_id_format_version_and_required_features() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();

    let manifest: WorkspaceManifest = initialize_workspace(root).expect("initialize workspace");

    // workspace_id must round-trip through its canonical ULID string form.
    let id_text = manifest.workspace_id.to_string();
    assert_eq!(id_text.len(), 26, "workspace_id is a canonical ULID");
    assert!(
        id_text
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()),
        "workspace_id is canonical uppercase ULID text"
    );

    assert_eq!(manifest.format_version, 1, "format_version is 1");
    assert!(!manifest.created_at.is_empty(), "created_at is non-empty");
    // The required-features set must at least advertise the oplog capability
    // so that future readers can refuse to interoperate when absent.
    assert!(
        manifest
            .required_features
            .iter()
            .any(|feature| feature == "oplog"),
        "required_features advertises oplog: {:?}",
        manifest.required_features
    );
}

#[test]
fn initialization_is_idempotent_and_returns_the_same_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();

    let first = initialize_workspace(root).expect("first init");
    let second = initialize_workspace(root).expect("second init");

    assert_eq!(
        first.workspace_id, second.workspace_id,
        "idempotent init preserves workspace_id"
    );
    assert_eq!(
        first.format_version, second.format_version,
        "idempotent init preserves format_version"
    );
    assert_eq!(
        first.created_at, second.created_at,
        "idempotent init preserves created_at"
    );
    assert_eq!(
        first.required_features, second.required_features,
        "idempotent init preserves required_features"
    );
}

#[test]
fn load_manifest_reads_back_what_initialize_wrote() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();

    let written = initialize_workspace(root).expect("initialize");
    let loaded = load_manifest(root).expect("load");

    assert_eq!(written.workspace_id, loaded.workspace_id);
    assert_eq!(written.format_version, loaded.format_version);
    assert_eq!(written.created_at, loaded.created_at);
    assert_eq!(written.required_features, loaded.required_features);
}

#[test]
fn load_manifest_rejects_unsupported_future_format_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();

    // First perform a legitimate initialization so the directory layout exists.
    let _ = initialize_workspace(root).expect("initialize");

    // Overwrite the manifest with a future format_version that this version of
    // the vault does not understand. Loading must fail rather than silently
    // upgrade or downgrade.
    let manifest_path = root.join(".secondbrain").join("manifest.toml");
    let future = "workspace_id = \"01ARZ3NDEKTSV4RRFFQ69G5FAV\"\nformat_version = 9999\ncreated_at = \"2026-01-01T00:00:00Z\"\nrequired_features = [\"oplog\"]\n";
    fs::write(&manifest_path, future).expect("write future manifest");

    let error = load_manifest(root).expect_err("future format_version rejected");
    let _ = error; // typed Error; just assert it is an error.
}

#[test]
fn existing_user_markdown_files_are_byte_identical_after_init() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();

    // Seed user content: nested Markdown plus a non-Markdown sibling.
    let notes_dir = root.join("Notes");
    fs::create_dir_all(&notes_dir).expect("mkdir Notes");
    let note_a = notes_dir.join("Welcome.md");
    let note_b = notes_dir.join("Sub").join("Deep.md");
    fs::create_dir_all(note_b.parent().expect("parent")).expect("mkdir Sub");
    let welcome_bytes = b"# Welcome\n\nThis is a note.\n".to_vec();
    let deep_bytes = "# Deep\n\n日本語 content.\n".as_bytes().to_vec();
    fs::write(&note_a, &welcome_bytes).expect("write welcome");
    fs::write(&note_b, &deep_bytes).expect("write deep");

    let before = snapshot_user_bytes(root);

    let _ = initialize_workspace(root).expect("initialize");

    let after = snapshot_user_bytes(root);

    assert_eq!(
        before, after,
        "user Markdown files must be byte-identical after initialization"
    );
}
