mod support {
    pub mod crash_child;
}

use std::{fs, process::Command};

use secondbrain_core::id::WorkspaceId;
use secondbrain_transaction::{RecoveryAction, TransactionEngine};
use secondbrain_vault::WorkspaceRoot;
use tempfile::tempdir;

const BOUNDARIES: &[&str] = &[
    "before_append",
    "after_append_before_state",
    "after_operations_durable",
    "during_temp_markdown_write",
    "after_rename_before_commit",
    "after_commit_before_index",
];

#[test]
fn child_crashes_at_selected_boundary() {
    support::crash_child::run_from_environment();
}

#[test]
fn real_process_crashes_recover_deterministically_at_every_boundary() {
    for boundary in BOUNDARIES {
        let dir = tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("child_crashes_at_selected_boundary")
            .arg("--nocapture")
            .env("SECONDBRAIN_CRASH_CHILD", "1")
            .env("SECONDBRAIN_CRASH_BOUNDARY", boundary)
            .env("SECONDBRAIN_CRASH_ROOT", dir.path())
            .status()
            .unwrap();
        assert!(!status.success(), "child must terminate at {boundary}");

        let engine =
            TransactionEngine::new(WorkspaceRoot::open(dir.path()).unwrap(), WorkspaceId::new());
        let first = engine.recover().unwrap();
        let second = engine.recover().unwrap();
        let markdown = fs::read_to_string(dir.path().join("notes/test.md")).unwrap();

        if *boundary == "before_append" {
            assert_eq!(markdown, "# Note\n");
            assert!(first.is_empty());
        } else {
            assert!(markdown.contains("status: done"), "boundary {boundary}");
            assert!(
                first
                    .iter()
                    .any(|a| matches!(a, RecoveryAction::IndexRepair { .. }))
            );
        }
        assert!(
            second.is_empty(),
            "recovery must be idempotent at {boundary}"
        );
    }
}

#[test]
fn corrupt_oplog_is_quarantined_without_materializing() {
    let dir = tempdir().unwrap();
    support::crash_child::prepare_and_crash(dir.path(), "after_append_before_state", false);
    let log = support::crash_child::oplog_path(dir.path());
    fs::OpenOptions::new().append(true).open(&log).unwrap();
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"truncated")
        .unwrap();

    let engine =
        TransactionEngine::new(WorkspaceRoot::open(dir.path()).unwrap(), WorkspaceId::new());
    let actions = engine.recover().unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/test.md")).unwrap(),
        "# Note\n"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, RecoveryAction::Quarantined { .. }))
    );
}

#[test]
fn prepared_without_durable_operations_aborts_safely() {
    let dir = tempdir().unwrap();
    support::crash_child::prepare_and_crash(dir.path(), "before_append", false);
    let engine =
        TransactionEngine::new(WorkspaceRoot::open(dir.path()).unwrap(), WorkspaceId::new());
    assert!(engine.recover().unwrap().is_empty());
    let marker = fs::read_to_string(support::crash_child::transaction_path(dir.path())).unwrap();
    assert!(marker.contains("ABORTED"));
}

#[test]
fn production_release_build_does_not_expose_environment_failpoints() {
    let source = include_str!("../src/failpoint.rs");
    assert!(source.contains("cfg(debug_assertions)"));
}
