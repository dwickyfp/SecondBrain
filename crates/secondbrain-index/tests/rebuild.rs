use std::fs;
use std::path::Path;
use std::time::SystemTime;

use secondbrain_core::id::NoteId;
use secondbrain_index::{
    IndexConfig, IndexError, IndexHealth, ensure_index, index_health, logical_dump, rebuild,
};
use secondbrain_vault::WorkspaceRoot;
use secondbrain_vault::base_snapshot::{BaseSnapshotStore, GENESIS_VERSION};
use tempfile::tempdir;

const ALPHA_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const BETA_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW";
const GAMMA_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAX";

fn note(id: &str, title: &str, aliases: &str, body: &str) -> String {
    format!("---\nid: {id}\ntitle: {title}\naliases: [{aliases}]\n---\n# {title}\n\n{body}\n")
}

fn valid_workspace(root: &Path) {
    fs::create_dir_all(root.join("notes/nested")).unwrap();
    fs::write(
        root.join("notes/alpha.md"),
        note(
            ALPHA_ID,
            "Alpha",
            "A",
            "Links [[notes/nested/beta.md]], [[Beta]], [[Bee]], and [[Missing]]. #one",
        ),
    )
    .unwrap();
    fs::write(
        root.join("notes/nested/beta.md"),
        note(BETA_ID, "Beta", "Bee", "Back to [[Alpha]]."),
    )
    .unwrap();
    fs::write(
        root.join("gamma.markdown"),
        note(GAMMA_ID, "Gamma", "G", "No links."),
    )
    .unwrap();
    fs::write(
        root.join("ignored.md"),
        note("01ARZ3NDEKTSV4RRFFQ69G5FAY", "Ignored", "I", "ignored"),
    )
    .unwrap();
    fs::create_dir_all(root.join(".secondbrain")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join(".secondbrain/private.md"),
        note("01ARZ3NDEKTSV4RRFFQ69G5FAZ", "Private", "P", "ignored"),
    )
    .unwrap();
    fs::write(
        root.join(".git/history.md"),
        note("01ARZ3NDEKTSV4RRFFQ69G5FB0", "Git", "Git", "ignored"),
    )
    .unwrap();
}

fn config() -> IndexConfig {
    IndexConfig {
        exclusions: vec!["ignored.md".into()],
    }
}

#[test]
fn indexes_known_fixture_count_and_audits_ignored_files() {
    let dir = tempdir().unwrap();
    valid_workspace(dir.path());
    let report = rebuild(dir.path(), &config()).unwrap();
    assert_eq!(report.indexed, 3);
    assert_eq!(report.skipped, 3);
    assert_eq!(report.broken_links, 1);
    assert_eq!(report.errors, 0);
    assert_eq!(report.orphans, 1);
}

#[test]
fn repeated_rebuild_and_deleted_database_have_same_logical_dump() {
    let dir = tempdir().unwrap();
    valid_workspace(dir.path());
    rebuild(dir.path(), &config()).unwrap();
    let first = logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap();
    rebuild(dir.path(), &config()).unwrap();
    assert_eq!(
        first,
        logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap()
    );
    fs::remove_file(dir.path().join(".secondbrain/index.sqlite")).unwrap();
    rebuild(dir.path(), &config()).unwrap();
    assert_eq!(
        first,
        logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap()
    );
}

#[test]
fn rebuild_closes_and_removes_temporary_sqlite_sidecars_before_swap() {
    let workspace = tempdir().unwrap();
    valid_workspace(workspace.path());
    rebuild(workspace.path(), &config()).unwrap();

    let temporary = workspace.path().join(".secondbrain/index.sqlite.rebuild");
    assert!(!temporary.exists());
    assert!(!temporary.with_extension("rebuild-wal").exists());
    assert!(!temporary.with_extension("rebuild-shm").exists());
}

#[test]
fn shared_health_contract_reuses_valid_and_rebuilds_missing_added_changed_and_deleted_notes() {
    let dir = tempdir().unwrap();
    plain_workspace(dir.path());
    assert_eq!(
        index_health(dir.path(), &IndexConfig::default()).unwrap(),
        IndexHealth::Missing
    );
    assert!(
        ensure_index(dir.path(), &IndexConfig::default())
            .unwrap()
            .rebuilt
    );
    assert_eq!(
        index_health(dir.path(), &IndexConfig::default()).unwrap(),
        IndexHealth::Valid
    );
    assert!(
        !ensure_index(dir.path(), &IndexConfig::default())
            .unwrap()
            .rebuilt
    );

    fs::write(dir.path().join("notes/alpha.md"), "# Changed\n").unwrap();
    assert_eq!(
        index_health(dir.path(), &IndexConfig::default()).unwrap(),
        IndexHealth::Stale
    );
    assert!(
        ensure_index(dir.path(), &IndexConfig::default())
            .unwrap()
            .rebuilt
    );
    fs::write(dir.path().join("added.md"), "# Added\n").unwrap();
    assert_eq!(
        index_health(dir.path(), &IndexConfig::default()).unwrap(),
        IndexHealth::Stale
    );
    ensure_index(dir.path(), &IndexConfig::default()).unwrap();
    fs::remove_file(dir.path().join("added.md")).unwrap();
    assert_eq!(
        index_health(dir.path(), &IndexConfig::default()).unwrap(),
        IndexHealth::Stale
    );
}

#[test]
fn valid_index_reuse_performs_no_rebuild_sidecar_writes() {
    let dir = tempdir().unwrap();
    plain_workspace(dir.path());
    ensure_index(dir.path(), &IndexConfig::default()).unwrap();
    let before = rebuild_sidecar_modified_times(dir.path());

    let report = ensure_index(dir.path(), &IndexConfig::default()).unwrap();

    assert!(!report.rebuilt);
    assert_eq!(rebuild_sidecar_modified_times(dir.path()), before);
}

fn rebuild_sidecar_modified_times(root: &Path) -> Vec<(String, SystemTime)> {
    ["identity-map", "crdt"]
        .into_iter()
        .flat_map(|directory| {
            fs::read_dir(root.join(".secondbrain").join(directory))
                .unwrap()
                .map(move |entry| {
                    let entry = entry.unwrap();
                    (
                        format!("{directory}/{}", entry.file_name().to_string_lossy()),
                        entry.metadata().unwrap().modified().unwrap(),
                    )
                })
        })
        .collect()
}

#[test]
fn backlinks_resolve_by_path_title_and_alias() {
    let dir = tempdir().unwrap();
    valid_workspace(dir.path());
    rebuild(dir.path(), &config()).unwrap();
    let dump = logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap();
    let resolved = dump
        .links
        .iter()
        .filter(|link| link.resolved_note_id.as_deref() == Some(BETA_ID))
        .count();
    assert_eq!(resolved, 3);
    assert!(
        dump.links
            .iter()
            .any(|link| link.target == "Missing" && link.resolved_note_id.is_none())
    );
}

#[test]
fn duplicate_id_is_reported_before_replacing_valid_index() {
    let dir = tempdir().unwrap();
    valid_workspace(dir.path());
    rebuild(dir.path(), &config()).unwrap();
    let before = logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap();
    fs::write(
        dir.path().join("duplicate.md"),
        note(ALPHA_ID, "Duplicate", "D", "bad"),
    )
    .unwrap();
    assert!(matches!(
        rebuild(dir.path(), &config()),
        Err(IndexError::DuplicateId { .. })
    ));
    assert_eq!(
        before,
        logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap()
    );
}

#[test]
fn malformed_note_is_reported_without_destroying_valid_index() {
    let dir = tempdir().unwrap();
    valid_workspace(dir.path());
    rebuild(dir.path(), &config()).unwrap();
    let before = logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap();
    fs::write(dir.path().join("bad.md"), "---\nid: [oops\n---\n# bad").unwrap();
    assert!(matches!(
        rebuild(dir.path(), &config()),
        Err(IndexError::MalformedNote { .. })
    ));
    assert_eq!(
        before,
        logical_dump(dir.path().join(".secondbrain/index.sqlite")).unwrap()
    );
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
fn portable_case_collision_is_rejected_deterministically() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("Nova.md"), "# Nova\n").unwrap();
    fs::write(dir.path().join("nova.md"), "# nova\n").unwrap();

    let Err(IndexError::PathCollision { first, second }) =
        rebuild(dir.path(), &IndexConfig::default())
    else {
        panic!("expected a portable path collision");
    };
    let mut paths = [first, second];
    paths.sort();
    assert_eq!(paths, ["Nova.md", "nova.md"]);
}

#[cfg(unix)]
#[test]
fn non_utf8_filename_is_rejected_instead_of_lossily_indexed() {
    use std::os::unix::ffi::OsStringExt;

    let dir = tempdir().unwrap();
    let name = std::ffi::OsString::from_vec(vec![b'n', b'o', b't', b'e', 0xff, b'.', b'm', b'd']);
    let path = dir.path().join(name);
    let Ok(()) = fs::write(&path, "# Note\n") else {
        eprintln!("SKIP: filesystem rejects non-UTF-8 filenames");
        return;
    };

    assert!(matches!(
        rebuild(dir.path(), &IndexConfig::default()),
        Err(IndexError::NonUtf8Path { path: rejected }) if rejected == path
    ));
}

#[test]
fn fixture_workspace_is_stable() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/markdown/workspace-small");
    let dir = tempdir().unwrap();
    copy_tree(&fixture, dir.path());
    let report = rebuild(dir.path(), &IndexConfig::default()).unwrap();
    assert_eq!(report.indexed, 3);
}

// ---------------------------------------------------------------------------
// Converged bases
// ---------------------------------------------------------------------------
//
// Indexing is where the workspace takes responsibility for a note, so it is
// where the note's first converged base is recorded. Without one there is
// nothing an external editor's next save can be measured against, and the whole
// external-edit pipeline has no way to start.

const PLAIN_ALPHA: &str = "# Alpha\n\nAlpha links to [[beta]].\n";
const PLAIN_BETA: &str = "# Beta\n\nBeta stands alone.\n";

/// A workspace of ordinary Markdown — no frontmatter, no identity declared.
fn plain_workspace(root: &Path) {
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(root.join("notes/alpha.md"), PLAIN_ALPHA).unwrap();
    fs::write(root.join("notes/beta.md"), PLAIN_BETA).unwrap();
}

fn bases(root: &Path) -> BaseSnapshotStore {
    BaseSnapshotStore::new(&WorkspaceRoot::open(root).unwrap())
}

/// The note id the index assigned to a workspace path.
fn indexed_id(root: &Path, path: &str) -> NoteId {
    logical_dump(secondbrain_index::index_path(root))
        .unwrap()
        .notes
        .into_iter()
        .find(|note| note.path == path)
        .unwrap_or_else(|| panic!("{path} was not indexed"))
        .note_id
        .parse()
        .unwrap()
}

#[test]
fn indexing_records_a_genesis_base_without_touching_a_note() {
    let dir = tempdir().unwrap();
    plain_workspace(dir.path());

    rebuild(dir.path(), &IndexConfig::default()).unwrap();

    let store = bases(dir.path());
    for (path, source) in [
        ("notes/alpha.md", PLAIN_ALPHA),
        ("notes/beta.md", PLAIN_BETA),
    ] {
        let base = store
            .load(indexed_id(dir.path(), path))
            .unwrap()
            .unwrap_or_else(|| panic!("{path} was indexed without a converged base"));
        assert_eq!(base.version, GENESIS_VERSION);
        assert_eq!(base.source, source);
        assert_eq!(base.path.as_str(), path);
        assert_eq!(
            fs::read_to_string(dir.path().join(path)).unwrap(),
            source,
            "recording a base is workspace state; it may not touch a byte of a note"
        );
    }
}

#[test]
fn indexing_records_identity_and_genesis_for_notes_that_declare_their_id() {
    let dir = tempdir().unwrap();
    valid_workspace(dir.path());

    rebuild(dir.path(), &config()).unwrap();

    let root = WorkspaceRoot::open(dir.path()).unwrap();
    let map = secondbrain_vault::IdentityMap::open(&root).unwrap();
    for (path, id) in [
        ("notes/alpha.md", ALPHA_ID),
        ("notes/nested/beta.md", BETA_ID),
        ("gamma.markdown", GAMMA_ID),
    ] {
        let note_id: NoteId = id.parse().unwrap();
        let record = map
            .lookup(&note_id)
            .unwrap()
            .unwrap_or_else(|| panic!("{path} was indexed without an identity record"));
        assert_eq!(record.current_path.as_str(), path);
        let base = bases(dir.path())
            .load(note_id)
            .unwrap()
            .unwrap_or_else(|| panic!("{path} was indexed without a converged base"));
        assert_eq!(base.path.as_str(), path);
        assert_eq!(base.version, GENESIS_VERSION);
        assert_eq!(
            base.source,
            fs::read_to_string(dir.path().join(path)).unwrap()
        );
    }
}

#[test]
fn rebuilding_after_a_rename_moves_the_base_without_changing_its_content() {
    let dir = tempdir().unwrap();
    plain_workspace(dir.path());
    rebuild(dir.path(), &IndexConfig::default()).unwrap();
    let note_id = indexed_id(dir.path(), "notes/alpha.md");
    let moved = "archive/alpha.md";
    fs::create_dir_all(dir.path().join("archive")).unwrap();
    fs::rename(dir.path().join("notes/alpha.md"), dir.path().join(moved)).unwrap();

    rebuild(dir.path(), &IndexConfig::default()).unwrap();

    let base = bases(dir.path()).load(note_id).unwrap().unwrap();
    assert_eq!(base.path.as_str(), moved);
    assert_eq!(base.source, PLAIN_ALPHA);
    assert_eq!(
        base.source,
        fs::read_to_string(dir.path().join(moved)).unwrap(),
        "relocating a base changes neither its source nor the note"
    );
}

#[test]
fn a_workspace_whose_notes_have_no_base_is_healed_by_the_next_rebuild() {
    let dir = tempdir().unwrap();
    plain_workspace(dir.path());
    rebuild(dir.path(), &IndexConfig::default()).unwrap();
    let alpha = indexed_id(dir.path(), "notes/alpha.md");
    // What a workspace indexed by a build that recorded no canonical state
    // looks like. The legacy snapshot directory is only migration input now.
    fs::remove_dir_all(dir.path().join(".secondbrain/crdt")).unwrap();
    assert!(bases(dir.path()).load(alpha).unwrap().is_none());

    rebuild(dir.path(), &IndexConfig::default()).unwrap();

    let base = bases(dir.path())
        .load(alpha)
        .unwrap()
        .expect("a tracked note with no base gets one from its current content");
    assert_eq!(base.version, GENESIS_VERSION);
    assert_eq!(base.source, PLAIN_ALPHA);
    assert_eq!(
        indexed_id(dir.path(), "notes/alpha.md"),
        alpha,
        "healing a base must not give the note a new identity"
    );
}

#[test]
fn a_rebuild_never_adopts_an_external_edit_into_the_base() {
    let dir = tempdir().unwrap();
    plain_workspace(dir.path());
    rebuild(dir.path(), &IndexConfig::default()).unwrap();
    let alpha = indexed_id(dir.path(), "notes/alpha.md");
    // An editor outside the workspace saved over the note. Nothing has
    // journaled that edit yet; `reconcile` is what does.
    fs::write(
        dir.path().join("notes/alpha.md"),
        "# Alpha\n\nAlpha links to [[beta]] after the retro.\n",
    )
    .unwrap();

    rebuild(dir.path(), &IndexConfig::default()).unwrap();

    assert_eq!(
        bases(dir.path()).load(alpha).unwrap().unwrap().source,
        PLAIN_ALPHA,
        "moving the base to the editor's bytes would adopt the edit silently — \
         unattributed, unjournaled, and with nothing left to recover it from"
    );
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
