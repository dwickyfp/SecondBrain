mod support {
    pub mod crash_child;
}

use std::{fs, process::Command};

use secondbrain_core::id::WorkspaceId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::apply::apply_operations;
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_transaction::{
    AbandonedReason, RecoveryAction, TransactionEngine, TransactionError,
};
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
    // Every variant answers "which note" with the same type, so a formatter
    // does not special-case one of them; the preserved journal suffix is a
    // separate fact under `.secondbrain/` and is named for what it is.
    let quarantined = actions
        .iter()
        .find_map(|action| match action {
            RecoveryAction::Quarantined {
                path,
                quarantine_path,
                ..
            } => Some((path, quarantine_path)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a quarantine: {actions:?}"));
    assert_eq!(*quarantined.0, WorkspacePath::new("notes/test.md").unwrap());
    assert!(
        quarantined
            .1
            .components()
            .any(|component| component.as_os_str() == ".secondbrain"),
        "{:?} is not a note path and must not be reported as one",
        quarantined.1
    );
}

#[test]
fn recovery_rewriting_a_marker_preserves_every_field_the_engine_wrote() {
    let dir = tempdir().unwrap();
    support::crash_child::prepare_and_crash(dir.path(), "after_operations_durable", false);
    let marker_path = support::crash_child::transaction_path(dir.path());
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    let written = written.as_object().unwrap().clone();

    let engine =
        TransactionEngine::new(WorkspaceRoot::open(dir.path()).unwrap(), WorkspaceId::new());
    engine.recover().unwrap();

    let rewritten: serde_json::Value =
        serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    let rewritten = rewritten.as_object().unwrap().clone();

    // Recovery rewrites a marker by re-serializing the value it deserialized,
    // so any field it does not model is erased from the marker the first time
    // it runs. Comparing key sets rather than named fields is deliberate: this
    // must fail for a field added later that only one side knows about.
    let mut written_keys: Vec<&String> = written.keys().collect();
    let mut rewritten_keys: Vec<&String> = rewritten.keys().collect();
    written_keys.sort();
    rewritten_keys.sort();
    assert_eq!(
        written_keys, rewritten_keys,
        "recovery erased a field the engine wrote"
    );
    for key in [
        "transaction_id",
        "note_id",
        "path",
        "expected_hash",
        "materialized_hash",
        "expected_version",
        "committed_version",
    ] {
        assert_eq!(written[key], rewritten[key], "recovery altered {key}");
    }
}

#[test]
fn a_marker_without_a_materialized_hash_is_refused_rather_than_half_recovered() {
    let dir = tempdir().unwrap();
    support::crash_child::prepare_and_crash(dir.path(), "after_operations_durable", false);
    let marker_path = support::crash_child::transaction_path(dir.path());
    let mut marker: serde_json::Value =
        serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    marker
        .as_object_mut()
        .unwrap()
        .remove("materialized_hash")
        .expect("the engine records it");
    fs::write(&marker_path, serde_json::to_vec_pretty(&marker).unwrap()).unwrap();

    let engine =
        TransactionEngine::new(WorkspaceRoot::open(dir.path()).unwrap(), WorkspaceId::new());
    let error = engine
        .recover()
        .expect_err("a marker no shipped build can produce must not be half-supported");

    assert!(matches!(error, TransactionError::Json(_)), "{error}");
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/test.md")).unwrap(),
        "# Note\n",
        "a refused marker must not have moved any bytes first"
    );
}

#[test]
fn operations_idempotent_against_third_party_content_are_not_committed_over_it() {
    let dir = tempdir().unwrap();
    support::crash_child::prepare_and_crash(dir.path(), "after_operations_durable", false);

    // Content this transaction never produced, which its operations happen to
    // be idempotent against — the case where re-applying them agrees with the
    // file while the marker's own `materialized_hash` says otherwise.
    let operations = [SemanticOperation::SetProperty {
        key: "status".into(),
        value: "done".into(),
    }];
    let third_party = apply_operations("# Someone Else's Note\n\nTheir words.\n", &operations)
        .expect("the operations anchor");
    assert_eq!(
        apply_operations(&third_party, &operations).expect("anchors"),
        third_party,
        "the fixture only tests anything if the operations are idempotent here"
    );
    fs::write(dir.path().join("notes/test.md"), &third_party).unwrap();

    let engine =
        TransactionEngine::new(WorkspaceRoot::open(dir.path()).unwrap(), WorkspaceId::new());
    let actions = engine.recover().unwrap();

    assert!(
        actions.iter().any(|action| matches!(
            action,
            RecoveryAction::Abandoned {
                reason: AbandonedReason::UnrecognizedFileState,
                ..
            }
        )),
        "committing over content the transaction did not produce: {actions:?}"
    );
    let marker = fs::read_to_string(support::crash_child::transaction_path(dir.path())).unwrap();
    assert!(marker.contains("ABORTED"), "{marker}");
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/test.md")).unwrap(),
        third_party,
        "third-party content is preserved"
    );
    assert!(
        !dir.path().join(".secondbrain/snapshots").exists(),
        "content this transaction never produced must not become its converged base"
    );
}

#[test]
fn an_abandonment_reason_renders_for_the_operator_who_has_to_read_it() {
    for reason in [
        AbandonedReason::OperationsDoNotAnchor,
        AbandonedReason::UnrecognizedFileState,
    ] {
        let rendered = reason.to_string();
        assert_ne!(
            rendered,
            format!("{reason:?}"),
            "`recovery check` prints this to a human, not a Rust variant name"
        );
        assert!(
            rendered.contains("the file on disk is untouched") && rendered.contains("edit is lost"),
            "an operator must be told what this means for their data: {rendered}"
        );
    }
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
