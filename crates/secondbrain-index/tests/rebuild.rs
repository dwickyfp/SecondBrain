use std::fs;
use std::path::Path;

use secondbrain_index::{IndexConfig, IndexError, logical_dump, rebuild};
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

#[test]
fn fixture_workspace_is_stable() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/markdown/workspace-small");
    let dir = tempdir().unwrap();
    copy_tree(&fixture, dir.path());
    let report = rebuild(dir.path(), &IndexConfig::default()).unwrap();
    assert_eq!(report.indexed, 3);
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
