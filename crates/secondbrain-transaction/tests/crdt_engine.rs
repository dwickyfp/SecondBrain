use secondbrain_core::{
    actor::{ActorId, DeviceId},
    hash::ContentHash,
    id::{NoteId, NoteVersion, TransactionId, WorkspaceId},
    path::WorkspacePath,
};
use secondbrain_crdt::NoteStore;
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_transaction::{TransactionEngine, TransactionRequest};
use secondbrain_vault::WorkspaceRoot;
use tempfile::tempdir;

#[test]
fn transaction_commit_advances_loro_markdown_state_as_the_canonical_base() {
    let directory = tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("notes")).unwrap();
    let source = "# Note\n";
    std::fs::write(directory.path().join("notes/test.md"), source).unwrap();
    let note_id = NoteId::new();
    let path = WorkspacePath::new("notes/test.md").unwrap();
    let request = TransactionRequest {
        id: TransactionId::new(),
        actor: ActorId::new("alice").unwrap(),
        device: DeviceId::new("laptop").unwrap(),
        note_id,
        path: path.clone(),
        expected_hash: ContentHash::digest(source.as_bytes()),
        expected_version: NoteVersion::new(0),
        operations: vec![SemanticOperation::SetProperty {
            key: "status".into(),
            value: "done".into(),
        }],
    };
    let engine = TransactionEngine::new(
        WorkspaceRoot::open(directory.path()).unwrap(),
        WorkspaceId::new(),
    );
    engine.commit(request).unwrap();

    let state = NoteStore::new(directory.path())
        .load(note_id)
        .unwrap()
        .unwrap();
    assert_eq!(state.version, NoteVersion::new(1));
    assert_eq!(state.markdown, "---\nstatus: done\n---\n\n# Note\n");
    assert!(
        directory
            .path()
            .join(format!(".secondbrain/crdt/{note_id}.sbcrdt"))
            .is_file()
    );
}
