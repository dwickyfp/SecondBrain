use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use secondbrain_core::path::WorkspacePath;
use secondbrain_vault::WorkspaceRoot;
use secondbrain_vault::event::WorkspaceEvent;
use secondbrain_vault::watcher::{Normalizer, RawEvent, RawEventKind, RawRenameMode};
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::symlink;

fn setup() -> (tempfile::TempDir, WorkspaceRoot, Normalizer) {
    let dir = tempdir().expect("tempdir");
    let root = WorkspaceRoot::open(dir.path()).expect("root");
    let normalizer = Normalizer::new(root.clone(), Vec::new(), Duration::from_secs(2));
    (dir, root, normalizer)
}

fn raw(root: &WorkspaceRoot, kind: RawEventKind, paths: &[&str]) -> RawEvent {
    RawEvent {
        kind,
        rename_mode: None,
        tracker: None,
        paths: paths
            .iter()
            .map(|path| root.canonical_path().join(path))
            .collect::<Vec<PathBuf>>(),
    }
}

#[cfg(unix)]
#[test]
fn symlink_escapes_never_emit_workspace_events() {
    let (_dir, root, mut normalizer) = setup();
    let external = tempdir().expect("external tempdir");
    fs::write(external.path().join("secret.md"), "external secret").expect("external file");
    fs::create_dir(external.path().join("notes")).expect("external directory");
    fs::write(
        external.path().join("notes/secret.md"),
        "external directory secret",
    )
    .expect("external nested file");
    symlink(
        external.path().join("secret.md"),
        root.canonical_path().join("escaped.md"),
    )
    .expect("file symlink");
    symlink(
        external.path().join("notes"),
        root.canonical_path().join("escaped-dir"),
    )
    .expect("directory symlink");

    let events = normalizer
        .normalize(
            [
                raw(&root, RawEventKind::Create, &["escaped.md"]),
                raw(&root, RawEventKind::Modify, &["escaped-dir/secret.md"]),
            ],
            Instant::now(),
        )
        .expect("symlink escapes are ignored");

    assert!(events.is_empty());
}

#[test]
fn raw_event_preserves_both_rename_mode_and_tracker() {
    let mut event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Name(
        notify::event::RenameMode::Both,
    )));
    event.paths = vec![PathBuf::from("old.md"), PathBuf::from("new.md")];
    event.attrs.set_tracker(42);

    let raw = RawEvent::from_notify(event);
    assert_eq!(raw.rename_mode, Some(RawRenameMode::Both));
    assert_eq!(raw.tracker, Some(42));
}

#[test]
fn split_rename_halves_pair_by_tracker() {
    let (_dir, root, mut n) = setup();
    fs::write(root.canonical_path().join("new.md"), "moved").unwrap();
    let mut from = raw(&root, RawEventKind::Rename, &["old.md"]);
    from.rename_mode = Some(RawRenameMode::From);
    from.tracker = Some(7);
    let mut to = raw(&root, RawEventKind::Rename, &["new.md"]);
    to.rename_mode = Some(RawRenameMode::To);
    to.tracker = Some(7);

    assert!(
        matches!(n.normalize([from, to], Instant::now()).unwrap().as_slice(),
        [WorkspaceEvent::Renamed { from, to }]
        if from.as_str() == "old.md" && to.as_str() == "new.md")
    );
}

#[test]
fn untracked_split_rename_pairs_only_when_unambiguous() {
    let (_dir, root, mut n) = setup();
    fs::write(root.canonical_path().join("new.md"), "moved").unwrap();
    let mut from = raw(&root, RawEventKind::Rename, &["old.md"]);
    from.rename_mode = Some(RawRenameMode::From);
    let mut to = raw(&root, RawEventKind::Rename, &["new.md"]);
    to.rename_mode = Some(RawRenameMode::To);

    assert!(
        matches!(n.normalize([from, to], Instant::now()).unwrap().as_slice(),
        [WorkspaceEvent::Renamed { from, to }]
        if from.as_str() == "old.md" && to.as_str() == "new.md")
    );
}

#[test]
fn two_path_rename_does_not_drop_later_content_event() {
    let (_dir, root, mut n) = setup();
    fs::write(root.canonical_path().join("new.md"), "renamed").unwrap();
    fs::write(root.canonical_path().join("other.md"), "changed").unwrap();

    let events = n
        .normalize(
            [
                raw(&root, RawEventKind::Rename, &["old.md", "new.md"]),
                raw(&root, RawEventKind::Modify, &["other.md"]),
            ],
            Instant::now(),
        )
        .unwrap();

    assert!(matches!(&events[0], WorkspaceEvent::Renamed { from, to }
        if from.as_str() == "old.md" && to.as_str() == "new.md"));
    assert!(
        matches!(&events[1], WorkspaceEvent::ContentChanged { path, .. }
        if path.as_str() == "other.md")
    );
}

#[test]
fn create_write_bursts_collapse_to_one_content_changed() {
    let (_dir, root, mut normalizer) = setup();
    fs::write(root.canonical_path().join("note.md"), "final").expect("write");

    let events = normalizer
        .normalize(
            [
                raw(&root, RawEventKind::Create, &["note.md"]),
                raw(&root, RawEventKind::Modify, &["note.md"]),
                raw(&root, RawEventKind::Modify, &["note.md"]),
            ],
            Instant::now(),
        )
        .expect("normalize");

    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        WorkspaceEvent::ContentChanged { path, .. }
            if path == &WorkspacePath::new("note.md").expect("path")
    ));
}

#[test]
fn atomic_save_rename_becomes_one_content_update() {
    let (_dir, root, mut n) = setup();
    fs::write(root.canonical_path().join("note.md"), "new").unwrap();
    let events = n
        .normalize(
            [
                raw(&root, RawEventKind::Create, &[".note.tmp"]),
                raw(&root, RawEventKind::Rename, &[".note.tmp", "note.md"]),
                raw(&root, RawEventKind::Modify, &["note.md"]),
            ],
            Instant::now(),
        )
        .unwrap();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], WorkspaceEvent::ContentChanged { path, .. } if path.as_str() == "note.md")
    );
}

#[test]
fn rename_preserves_old_and_new_path() {
    let (_dir, root, mut n) = setup();
    fs::write(root.canonical_path().join("new.md"), "same").unwrap();
    assert_eq!(
        n.normalize(
            [raw(&root, RawEventKind::Rename, &["old.md", "new.md"])],
            Instant::now()
        )
        .unwrap(),
        vec![WorkspaceEvent::Renamed {
            from: WorkspacePath::new("old.md").unwrap(),
            to: WorkspacePath::new("new.md").unwrap()
        }]
    );
}

#[test]
fn ignored_directories_never_emit() {
    let (dir, root, _) = setup();
    fs::create_dir_all(dir.path().join("ignored")).unwrap();
    fs::write(dir.path().join("ignored/a.md"), "a").unwrap();
    let mut n = Normalizer::new(
        root.clone(),
        vec![WorkspacePath::new("ignored").unwrap()],
        Duration::from_secs(2),
    );
    assert!(
        n.normalize(
            [
                raw(&root, RawEventKind::Modify, &["ignored/a.md"]),
                raw(&root, RawEventKind::Modify, &[".git/a"]),
                raw(&root, RawEventKind::Modify, &[".secondbrain/state"]),
            ],
            Instant::now()
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn duplicate_events_collapse() {
    let (_dir, root, mut n) = setup();
    fs::write(root.canonical_path().join("a.md"), "a").unwrap();
    let event = raw(&root, RawEventKind::Modify, &["a.md"]);
    assert_eq!(
        n.normalize([event.clone(), event], Instant::now())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn same_batch_remove_then_create_preserves_both_events() {
    let (_dir, root, mut n) = setup();
    fs::write(root.canonical_path().join("a.md"), "old").unwrap();
    n.normalize(
        [raw(&root, RawEventKind::Create, &["a.md"])],
        Instant::now(),
    )
    .unwrap();
    fs::write(root.canonical_path().join("a.md"), "new").unwrap();

    let events = n
        .normalize(
            [
                raw(&root, RawEventKind::Remove, &["a.md"]),
                raw(&root, RawEventKind::Create, &["a.md"]),
            ],
            Instant::now(),
        )
        .unwrap();

    assert!(matches!(&events[0], WorkspaceEvent::Deleted { path } if path.as_str() == "a.md"));
    assert!(
        matches!(&events[1], WorkspaceEvent::ContentChanged { path, .. } if path.as_str() == "a.md")
    );
}

#[test]
fn delete_and_recreate_are_distinguished() {
    let (_dir, root, mut n) = setup();
    let now = Instant::now();
    fs::write(root.canonical_path().join("a.md"), "one").unwrap();
    n.normalize([raw(&root, RawEventKind::Create, &["a.md"])], now)
        .unwrap();
    fs::remove_file(root.canonical_path().join("a.md")).unwrap();
    let deleted = n
        .normalize([raw(&root, RawEventKind::Remove, &["a.md"])], now)
        .unwrap();
    fs::write(root.canonical_path().join("a.md"), "two").unwrap();
    let recreated = n
        .normalize([raw(&root, RawEventKind::Create, &["a.md"])], now)
        .unwrap();
    assert!(matches!(
        deleted.as_slice(),
        [WorkspaceEvent::Deleted { .. }]
    ));
    assert!(matches!(
        recreated.as_slice(),
        [WorkspaceEvent::ContentChanged { .. }]
    ));
}

#[test]
fn internal_receipt_suppresses_once_not_forever() {
    let (_dir, root, mut n) = setup();
    let now = Instant::now();
    let hash = secondbrain_core::hash::ContentHash::digest("internal");
    fs::write(root.canonical_path().join("a.md"), "internal").unwrap();
    n.record_internal_write(WorkspacePath::new("a.md").unwrap(), hash, now);
    assert!(
        n.normalize([raw(&root, RawEventKind::Modify, &["a.md"])], now)
            .unwrap()
            .is_empty()
    );
    fs::write(root.canonical_path().join("a.md"), "external").unwrap();
    assert_eq!(
        n.normalize([raw(&root, RawEventKind::Modify, &["a.md"])], now)
            .unwrap()
            .len(),
        1
    );
}
