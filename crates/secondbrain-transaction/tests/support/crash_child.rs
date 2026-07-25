use std::{
    fs,
    path::{Path, PathBuf},
};

use secondbrain_core::{
    actor::{ActorId, DeviceId},
    hash::ContentHash,
    id::{NoteId, NoteVersion, TransactionId, WorkspaceId},
    path::WorkspacePath,
};
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_transaction::{TransactionEngine, TransactionRequest};
use secondbrain_vault::WorkspaceRoot;

pub fn run_from_environment() {
    if std::env::var_os("SECONDBRAIN_CRASH_CHILD").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("SECONDBRAIN_CRASH_ROOT").unwrap());
    let boundary = std::env::var("SECONDBRAIN_CRASH_BOUNDARY").unwrap();
    prepare_and_crash(&root, &boundary, true);
    panic!("failpoint {boundary} did not terminate child");
}

pub fn prepare_and_crash(root: &Path, boundary: &str, hard: bool) {
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(root.join("notes/test.md"), "# Note\n").unwrap();
    if hard {
        secondbrain_transaction::failpoint::set(Some(boundary));
        secondbrain_transaction::failpoint::set_abort(true);
    } else {
        secondbrain_transaction::failpoint::set(Some(boundary));
    }
    let source = "# Note\n";
    let request = TransactionRequest {
        id: TransactionId::new(),
        actor: ActorId::new("crash-test").unwrap(),
        device: DeviceId::new("child").unwrap(),
        note_id: NoteId::new(),
        path: WorkspacePath::new("notes/test.md").unwrap(),
        expected_hash: ContentHash::digest(source.as_bytes()),
        expected_version: NoteVersion::new(1),
        operations: vec![SemanticOperation::SetProperty {
            key: "status".into(),
            value: "done".into(),
        }],
    };
    let engine = TransactionEngine::new(WorkspaceRoot::open(root).unwrap(), WorkspaceId::new());
    let _ = engine.commit(request);
    if !hard {
        secondbrain_transaction::failpoint::set(None);
    }
    secondbrain_transaction::failpoint::set_abort(false);
    secondbrain_transaction::failpoint::set(None);
}

pub fn oplog_path(root: &Path) -> PathBuf {
    let note_dir = fs::read_dir(root.join(".secondbrain/oplog"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    note_dir.join("local-mutations.log")
}

pub fn transaction_path(root: &Path) -> PathBuf {
    fs::read_dir(root.join(".secondbrain/transactions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
}
