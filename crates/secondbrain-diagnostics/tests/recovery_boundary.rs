use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_index::{IndexDatabase, index_path};
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_transaction::{TransactionEngine, TransactionRequest};
use secondbrain_vault::{WorkspaceRoot, initialize_workspace};
use tempfile::tempdir;

#[test]
fn crash_child() {
    if std::env::var_os("SECONDBRAIN_DIAGNOSTICS_CRASH_CHILD").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("SECONDBRAIN_CRASH_ROOT").unwrap());
    commit_at_failpoint(
        &root,
        &std::env::var("SECONDBRAIN_CRASH_BOUNDARY").unwrap(),
        true,
    );
    panic!("the hard failpoint did not terminate the child");
}

#[test]
fn recovery_repairs_missing_stale_corrupt_and_wrong_schema_indexes_without_transactions() {
    for damage in ["missing", "stale", "corrupt", "wrong-schema"] {
        let workspace = tempdir().unwrap();
        initialize_workspace(workspace.path()).unwrap();
        fs::write(workspace.path().join("note.md"), "# Original\n").unwrap();
        secondbrain_index::rebuild(workspace.path(), &Default::default()).unwrap();
        let index = index_path(workspace.path());

        match damage {
            "missing" => fs::remove_file(&index).unwrap(),
            "stale" => fs::write(workspace.path().join("note.md"), "# Changed\n").unwrap(),
            "corrupt" => fs::write(&index, b"not sqlite").unwrap(),
            "wrong-schema" => {
                let database = IndexDatabase::open(&index).unwrap();
                database
                    .connection()
                    .execute(
                        "UPDATE schema_migrations SET version = 99 WHERE version = 3",
                        [],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }

        let report = secondbrain_diagnostics::recover_workspace(workspace.path()).unwrap();
        assert_eq!(report.status, "recovered", "{damage}");
        assert!(report.actions.is_empty(), "{damage}");
        assert!(report.index_refreshed, "{damage}");
        assert_eq!(report.diagnostics.status, "healthy", "{damage}");
        assert!(
            IndexDatabase::open(&index)
                .unwrap()
                .note_by_path("note.md")
                .unwrap()
                .is_some(),
            "{damage}"
        );
    }
}

#[test]
fn real_subprocess_recovery_refreshes_the_real_index_and_is_idempotent() {
    let workspace = tempdir().unwrap();
    initialize_workspace(workspace.path()).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture"])
        .env("SECONDBRAIN_DIAGNOSTICS_CRASH_CHILD", "1")
        .env("SECONDBRAIN_CRASH_BOUNDARY", "after_rename_before_commit")
        .env("SECONDBRAIN_CRASH_ROOT", workspace.path())
        .status()
        .unwrap();
    assert!(!status.success());

    let first = secondbrain_diagnostics::recover_workspace(workspace.path()).unwrap();
    assert_eq!(first.repaired, 1);
    assert!(first.index_refreshed);
    assert!(
        fs::read_to_string(workspace.path().join("notes/test.md"))
            .unwrap()
            .contains("status: done")
    );
    let index = IndexDatabase::open(index_path(workspace.path())).unwrap();
    assert!(index.note_by_path("notes/test.md").unwrap().is_some());

    let second = secondbrain_diagnostics::recover_workspace(workspace.path()).unwrap();
    assert_eq!(second.status, "nothing_to_recover");
    assert!(second.actions.is_empty());
    assert!(!second.index_refreshed);
}

#[test]
fn corruption_quarantine_is_visible_and_preserves_note_bytes() {
    let workspace = tempdir().unwrap();
    initialize_workspace(workspace.path()).unwrap();
    commit_at_failpoint(workspace.path(), "after_append_before_state", false);
    let log = oplog_path(workspace.path());
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"truncated")
        .unwrap();

    let report = secondbrain_diagnostics::recover_workspace(workspace.path()).unwrap();
    let action = report
        .actions
        .iter()
        .find(|item| item.action == "quarantined")
        .unwrap();
    assert_eq!(action.code, "SB-JOURNAL-QUARANTINED");
    assert!(action.review_required);
    let quarantine = action.quarantine_path.as_ref().unwrap();
    assert!(Path::new(quarantine).exists(), "{quarantine}");
    assert_eq!(
        fs::read_to_string(workspace.path().join("notes/test.md")).unwrap(),
        "# Note\n"
    );
}

fn commit_at_failpoint(root: &Path, boundary: &str, hard: bool) {
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(root.join("notes/test.md"), "# Note\n").unwrap();
    let manifest = secondbrain_vault::load_manifest(root).unwrap();
    secondbrain_transaction::failpoint::set(Some(boundary));
    secondbrain_transaction::failpoint::set_abort(hard);
    let engine = TransactionEngine::new(WorkspaceRoot::open(root).unwrap(), manifest.workspace_id);
    let _ = engine.commit(TransactionRequest {
        id: TransactionId::new(),
        actor: ActorId::new("diagnostics-test").unwrap(),
        device: DeviceId::new("subprocess").unwrap(),
        note_id: NoteId::new(),
        path: WorkspacePath::new("notes/test.md").unwrap(),
        expected_hash: ContentHash::digest(b"# Note\n"),
        expected_version: NoteVersion::new(0),
        operations: vec![SemanticOperation::SetProperty {
            key: "status".into(),
            value: "done".into(),
        }],
    });
    secondbrain_transaction::failpoint::set_abort(false);
    secondbrain_transaction::failpoint::set(None);
}

fn oplog_path(root: &Path) -> PathBuf {
    fs::read_dir(root.join(".secondbrain/oplog"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("local-mutations.log")
}
