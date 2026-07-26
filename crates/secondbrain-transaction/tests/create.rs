use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_index::{IndexConfig, IndexDatabase, index_path, rebuild};
use secondbrain_transaction::{
    DailyDate, DailyNote, NoteCreateError, RecoveryAction, TransactionEngine, apply_note_creation,
    open_or_preview_daily_note, preview_note_creation,
};
use secondbrain_vault::{BaseSnapshotStore, IdentityMap, WorkspaceRoot, initialize_workspace};
use tempfile::tempdir;

fn identities() -> (ActorId, DeviceId) {
    (
        ActorId::new("external-agent").unwrap(),
        DeviceId::new("test-device").unwrap(),
    )
}

fn workspace() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    initialize_workspace(directory.path()).unwrap();
    rebuild(directory.path(), &IndexConfig::default()).unwrap();
    directory
}

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.path().is_dir() {
                visit(root, &entry.path(), result);
            } else {
                result.insert(
                    entry.path().strip_prefix(root).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn preview_is_read_only_and_apply_establishes_every_note_state() {
    let directory = workspace();
    let before = files(directory.path());
    let (actor, device) = identities();
    let preview = preview_note_creation(
        directory.path(),
        "notes/new.md",
        "# New\n\nsearchable-create-canary\n",
        actor,
        device,
    )
    .unwrap();
    assert_eq!(files(directory.path()), before);
    assert!(!directory.path().join("notes").exists());
    assert!(preview.source.contains(&format!("id: {}", preview.note_id)));

    let outcome = apply_note_creation(directory.path(), &preview).unwrap();
    assert!(outcome.created && outcome.index_refreshed);
    assert_eq!(
        fs::read_to_string(directory.path().join("notes/new.md")).unwrap(),
        preview.source
    );
    let root = WorkspaceRoot::open(directory.path()).unwrap();
    assert_eq!(
        IdentityMap::open(&root).unwrap().note_at(&preview.path),
        Some(preview.note_id)
    );
    let base = BaseSnapshotStore::new(&root)
        .load(preview.note_id)
        .unwrap()
        .unwrap();
    assert_eq!(base.source, preview.source);
    assert_eq!(base.version.get(), 0);
    assert_eq!(
        IndexDatabase::open(index_path(directory.path()))
            .unwrap()
            .note_by_path("notes/new.md")
            .unwrap()
            .unwrap()
            .note_id,
        preview.note_id
    );
}

#[test]
fn retry_is_idempotent_but_collision_and_tampering_fail_closed() {
    let directory = workspace();
    let (actor, device) = identities();
    let preview = preview_note_creation(
        directory.path(),
        "new.md",
        "# New\n",
        actor.clone(),
        device.clone(),
    )
    .unwrap();
    assert!(
        apply_note_creation(directory.path(), &preview)
            .unwrap()
            .created
    );
    assert!(
        !apply_note_creation(directory.path(), &preview)
            .unwrap()
            .created
    );

    let collision = preview_note_creation(
        directory.path(),
        "new.md",
        "# Other\n",
        actor.clone(),
        device.clone(),
    )
    .unwrap_err();
    assert!(matches!(collision, NoteCreateError::TargetExists(_)));
    let other =
        preview_note_creation(directory.path(), "other.md", "# Other\n", actor, device).unwrap();
    fs::write(directory.path().join("other.md"), "third party\n").unwrap();
    assert!(matches!(
        apply_note_creation(directory.path(), &other),
        Err(NoteCreateError::Transaction(_))
    ));
    assert_eq!(
        fs::read_to_string(directory.path().join("other.md")).unwrap(),
        "third party\n"
    );
    let mut tampered = preview.clone();
    tampered.source.push_str("tampered\n");
    assert!(matches!(
        apply_note_creation(directory.path(), &tampered),
        Err(NoteCreateError::PreviewModified("source_hash"))
    ));
}

#[test]
fn daily_note_uses_only_the_explicit_validated_date_and_opens_after_creation() {
    for invalid in ["2026-2-03", "2026-02-30", "0000-01-01", "2024-13-01"] {
        assert!(DailyDate::new(invalid).is_err(), "{invalid}");
    }
    assert!(DailyDate::new("2024-02-29").is_ok());
    let directory = workspace();
    let (actor, device) = identities();
    let daily = open_or_preview_daily_note(
        directory.path(),
        DailyDate::new("2026-07-26").unwrap(),
        actor.clone(),
        device.clone(),
    )
    .unwrap();
    let DailyNote::Create { preview } = daily else {
        panic!("expected creation")
    };
    assert_eq!(preview.path.as_str(), "Daily/2026-07-26.md");
    assert!(preview.source.ends_with("# 2026-07-26\n"));
    apply_note_creation(directory.path(), &preview).unwrap();
    let reopened = open_or_preview_daily_note(
        directory.path(),
        DailyDate::new("2026-07-26").unwrap(),
        actor,
        device,
    )
    .unwrap();
    assert!(
        matches!(reopened, DailyNote::Existing { note_id, ref path } if note_id == preview.note_id && path == "Daily/2026-07-26.md")
    );
}

#[test]
fn portable_paths_reject_traversal_backslashes_and_case_collisions() {
    let directory = workspace();
    fs::write(directory.path().join("Daily.md"), "# Existing\n").unwrap();
    rebuild(directory.path(), &IndexConfig::default()).unwrap();
    for path in ["../outside.md", "Daily\\note.md", "/absolute.md"] {
        let (actor, device) = identities();
        assert!(preview_note_creation(directory.path(), path, "# Bad\n", actor, device).is_err());
    }
    let (actor, device) = identities();
    assert!(matches!(
        preview_note_creation(directory.path(), "daily.MD", "# Collision\n", actor, device),
        Err(NoteCreateError::PathCollision { .. } | NoteCreateError::TargetExists(_))
    ));
}

#[test]
fn create_crash_child() {
    let Some(root) = std::env::var_os("SECONDBRAIN_CREATE_CRASH_ROOT") else {
        return;
    };
    let boundary = std::env::var("SECONDBRAIN_CREATE_CRASH_BOUNDARY").unwrap();
    let root = PathBuf::from(root);
    initialize_workspace(&root).unwrap();
    rebuild(&root, &IndexConfig::default()).unwrap();
    let (actor, device) = identities();
    let preview = preview_note_creation(
        &root,
        "Daily/2026-07-26.md",
        "# 2026-07-26\n",
        actor,
        device,
    )
    .unwrap();
    secondbrain_transaction::failpoint::set(Some(&boundary));
    secondbrain_transaction::failpoint::set_abort(true);
    let _ = apply_note_creation(&root, &preview);
    panic!("failpoint did not abort");
}

#[test]
fn real_process_creation_crashes_recover_without_partial_or_duplicate_notes() {
    for boundary in [
        "create_before_append",
        "create_after_operations_durable",
        "create_after_rename_before_commit",
        "create_after_commit_before_index",
    ] {
        let directory = tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "create_crash_child", "--nocapture"])
            .env("SECONDBRAIN_CREATE_CRASH_ROOT", directory.path())
            .env("SECONDBRAIN_CREATE_CRASH_BOUNDARY", boundary)
            .status()
            .unwrap();
        assert!(!status.success(), "{boundary}");
        let root = WorkspaceRoot::open(directory.path()).unwrap();
        let manifest = secondbrain_vault::load_manifest(directory.path()).unwrap();
        let engine = TransactionEngine::new(root, manifest.workspace_id);
        let actions = engine.recover().unwrap();
        if boundary == "create_before_append" {
            assert!(!directory.path().join("Daily/2026-07-26.md").exists());
        } else {
            assert!(directory.path().join("Daily/2026-07-26.md").exists());
            rebuild(directory.path(), &IndexConfig::default()).unwrap();
            for action in actions {
                if let RecoveryAction::IndexRepair { note_id, .. } = action {
                    engine.record_index_refreshed(note_id).unwrap();
                }
            }
        }
        assert!(engine.recover().unwrap().is_empty());
    }
}
