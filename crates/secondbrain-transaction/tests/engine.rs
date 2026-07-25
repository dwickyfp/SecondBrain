use std::fs;

use secondbrain_core::{
    actor::{ActorId, DeviceId},
    hash::ContentHash,
    id::{NoteId, NoteVersion, TransactionId, WorkspaceId},
    path::WorkspacePath,
};
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_transaction::{
    TransactionEngine, TransactionError, TransactionRequest, TransactionState,
};
use secondbrain_vault::WorkspaceRoot;
use tempfile::tempdir;

fn request(source: &str, operations: Vec<SemanticOperation>) -> TransactionRequest {
    TransactionRequest {
        id: TransactionId::new(),
        actor: ActorId::new("alice").unwrap(),
        device: DeviceId::new("laptop").unwrap(),
        note_id: NoteId::new(),
        path: WorkspacePath::new("notes/test.md").unwrap(),
        expected_hash: ContentHash::digest(source.as_bytes()),
        expected_version: NoteVersion::new(7),
        operations,
    }
}

fn engine(root: &std::path::Path) -> TransactionEngine {
    TransactionEngine::new(WorkspaceRoot::open(root).unwrap(), WorkspaceId::new())
}

#[test]
fn valid_state_sequence_reaches_committed() {
    let mut state = TransactionState::Prepared;
    state
        .transition_to(TransactionState::OperationsDurable)
        .unwrap();
    state
        .transition_to(TransactionState::Materializing)
        .unwrap();
    state.transition_to(TransactionState::Committed).unwrap();
    assert_eq!(state, TransactionState::Committed);
}

#[test]
fn illegal_state_transition_is_rejected() {
    let mut state = TransactionState::Prepared;
    let error = state
        .transition_to(TransactionState::Committed)
        .unwrap_err();
    assert!(error.to_string().contains("PREPARED"));
    assert_eq!(state, TransactionState::Prepared);
}

#[test]
fn stale_hash_is_rejected_before_oplog_append() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("notes")).unwrap();
    fs::write(dir.path().join("notes/test.md"), "current\n").unwrap();
    let request = request(
        "stale\n",
        vec![SemanticOperation::SetProperty {
            key: "x".into(),
            value: "y".into(),
        }],
    );
    let note_id = request.note_id;

    let error = engine(dir.path()).commit(request).unwrap_err();
    assert!(matches!(error, TransactionError::StaleHash { .. }));
    let oplog = dir
        .path()
        .join(format!(".secondbrain/oplog/{note_id}/local-mutations.log"));
    assert!(!oplog.exists() || fs::metadata(oplog).unwrap().len() == 0);
}

#[test]
fn needs_review_is_rejected_before_oplog_append() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("notes")).unwrap();
    let source = "# Note\n";
    fs::write(dir.path().join("notes/test.md"), source).unwrap();
    let request = request(
        source,
        vec![SemanticOperation::NeedsReview {
            reason: "ambiguous".into(),
            span: None,
        }],
    );
    let note_id = request.note_id;

    let error = engine(dir.path()).commit(request).unwrap_err();
    assert!(matches!(error, TransactionError::NeedsReview));
    let oplog = dir
        .path()
        .join(format!(".secondbrain/oplog/{note_id}/local-mutations.log"));
    assert!(!oplog.exists() || fs::metadata(oplog).unwrap().len() == 0);
}

#[test]
fn committed_edit_materializes_markdown_increments_version_and_persists_state() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("notes")).unwrap();
    let source = "# Note\n";
    fs::write(dir.path().join("notes/test.md"), source).unwrap();
    let request = request(
        source,
        vec![SemanticOperation::SetProperty {
            key: "status".into(),
            value: "done".into(),
        }],
    );
    let transaction_id = request.id;
    let note_id = request.note_id;

    let outcome = engine(dir.path()).commit(request).unwrap();

    assert!(outcome.changed);
    assert_eq!(outcome.version, NoteVersion::new(8));
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/test.md")).unwrap(),
        "---\nstatus: done\n---\n\n# Note\n"
    );
    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(
            dir.path()
                .join(format!(".secondbrain/transactions/{transaction_id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(state["state"], "COMMITTED");
    let log = secondbrain_transaction::oplog::LocalMutationLog::open(dir.path(), note_id).unwrap();
    assert_eq!(log.replay().unwrap().records.len(), 1);
}

#[test]
fn unchanged_edit_does_not_write_append_or_increment_version() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("notes")).unwrap();
    let source = "# Note\n";
    fs::write(dir.path().join("notes/test.md"), source).unwrap();
    let request = request(source, vec![]);
    let note_id = request.note_id;
    let before = fs::metadata(dir.path().join("notes/test.md"))
        .unwrap()
        .modified()
        .unwrap();

    let outcome = engine(dir.path()).commit(request).unwrap();

    assert!(!outcome.changed);
    assert_eq!(outcome.version, NoteVersion::new(7));
    assert_eq!(
        fs::metadata(dir.path().join("notes/test.md"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    let oplog = dir
        .path()
        .join(format!(".secondbrain/oplog/{note_id}/local-mutations.log"));
    assert!(!oplog.exists() || fs::metadata(oplog).unwrap().len() == 0);
}
