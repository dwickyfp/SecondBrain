use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, TransactionId, WorkspaceId};
use secondbrain_markdown::operation::SemanticOperation;
use secondbrain_transaction::oplog::{AppendStorage, LocalMutationLog, OplogError};
use secondbrain_transaction::record::{FORMAT_VERSION_1, LocalOperationRecord};

fn record(note_id: NoteId, sequence: u64, previous: Option<ContentHash>) -> LocalOperationRecord {
    LocalOperationRecord {
        format_version: FORMAT_VERSION_1,
        transaction_id: TransactionId::new(),
        workspace_id: WorkspaceId::new(),
        note_id,
        actor_id: ActorId::new("alice").unwrap(),
        device_id: DeviceId::new("macbook").unwrap(),
        sequence,
        previous_record_hash: previous,
        operation: SemanticOperation::SetProperty {
            key: "sequence".into(),
            value: sequence.to_string(),
        },
        crc32: 0,
    }
}

fn hash(record: &LocalOperationRecord) -> ContentHash {
    ContentHash::digest(record.encode().unwrap())
}

#[test]
fn append_and_replay_records() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let mut log = LocalMutationLog::open(root.path(), note).unwrap();
    let first = record(note, 1, None);
    let second = record(note, 2, Some(hash(&first)));

    log.append(&first).unwrap();
    log.append(&second).unwrap();
    let replay = log.replay().unwrap();

    assert_eq!(replay.records.len(), 2);
    assert_eq!(replay.records[0].sequence, 1);
    assert_eq!(replay.records[1].sequence, 2);
    assert!(replay.corruption.is_none());
}

#[test]
fn duplicate_sequence_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let mut log = LocalMutationLog::open(root.path(), note).unwrap();
    let first = record(note, 1, None);
    log.append(&first).unwrap();

    let error = log
        .append(&record(note, 1, Some(hash(&first))))
        .unwrap_err();
    assert!(matches!(
        error,
        OplogError::Sequence {
            previous: 1,
            next: 1
        }
    ));
}

#[test]
fn hash_chain_break_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let mut log = LocalMutationLog::open(root.path(), note).unwrap();
    let first = record(note, 1, None);
    log.append(&first).unwrap();

    let error = log
        .append(&record(note, 2, Some(ContentHash::digest(b"wrong"))))
        .unwrap_err();
    assert!(matches!(error, OplogError::HashChain { sequence: 2, .. }));
}

#[test]
fn truncated_tail_is_quarantined_and_valid_prefix_retained() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let mut log = LocalMutationLog::open(root.path(), note).unwrap();
    let first = record(note, 1, None);
    let second = record(note, 2, Some(hash(&first)));
    log.append(&first).unwrap();
    log.append(&second).unwrap();

    let path = log.path().to_owned();
    let original = fs::read(&path).unwrap();
    fs::write(&path, &original[..original.len() - 7]).unwrap();

    let replay = log.replay().unwrap();
    assert_eq!(
        replay.records,
        vec![LocalOperationRecord::decode(&first.encode().unwrap()).unwrap()]
    );
    let corruption = replay.corruption.expect("tail corruption reported");
    assert_eq!(corruption.offset, first.encode().unwrap().len() as u64);
    assert_eq!(
        fs::read(&corruption.quarantine_path).unwrap(),
        &original[first.encode().unwrap().len()..original.len() - 7]
    );
    assert_eq!(fs::read(&path).unwrap(), first.encode().unwrap());

    let replay_again = log.replay().unwrap();
    assert!(replay_again.corruption.is_none());
    assert_eq!(replay_again.records.len(), 1);
}

#[derive(Clone)]
struct TrackingStorage(Arc<Mutex<Vec<&'static str>>>);

impl AppendStorage for TrackingStorage {
    fn append_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(bytes)?;
        self.0.lock().unwrap().push("write");
        file.flush()?;
        self.0.lock().unwrap().push("flush");
        file.sync_all()?;
        self.0.lock().unwrap().push("sync_all");
        Ok(())
    }
}

#[test]
fn append_success_requires_flush_and_sync_all() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let storage = TrackingStorage(events.clone());
    let mut log = LocalMutationLog::open_with_storage(root.path(), note, storage).unwrap();

    log.append(&record(note, 1, None)).unwrap();
    assert_eq!(*events.lock().unwrap(), vec!["write", "flush", "sync_all"]);
}

#[test]
fn logs_are_isolated_by_note_id() {
    let root = tempfile::tempdir().unwrap();
    let note_a = NoteId::new();
    let note_b = NoteId::new();
    let mut a = LocalMutationLog::open(root.path(), note_a).unwrap();
    let mut b = LocalMutationLog::open(root.path(), note_b).unwrap();
    a.append(&record(note_a, 1, None)).unwrap();
    b.append(&record(note_b, 7, None)).unwrap();

    assert_ne!(a.path(), b.path());
    assert_eq!(a.replay().unwrap().records[0].sequence, 1);
    assert_eq!(b.replay().unwrap().records[0].sequence, 7);
}

#[test]
fn replay_quarantines_a_hash_chain_break_regression() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let mut log = LocalMutationLog::open(root.path(), note).unwrap();
    let first = record(note, 1, None);
    log.append(&first).unwrap();
    let broken = record(note, 2, Some(ContentHash::digest(b"wrong")))
        .encode()
        .unwrap();
    let mut file = OpenOptions::new().append(true).open(log.path()).unwrap();
    file.write_all(&broken).unwrap();
    file.sync_all().unwrap();

    let replay = log.replay().unwrap();
    assert_eq!(replay.records.len(), 1);
    assert!(replay.corruption.is_some());
}

#[test]
fn append_rejects_record_for_another_note() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let mut log = LocalMutationLog::open(root.path(), note).unwrap();
    let error = log.append(&record(NoteId::new(), 1, None)).unwrap_err();
    assert!(matches!(error, OplogError::NoteMismatch { .. }));
}

#[test]
fn deterministic_quarantine_name_reuses_the_same_path() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let mut log = LocalMutationLog::open(root.path(), note).unwrap();
    log.append(&record(note, 1, None)).unwrap();
    let valid = fs::read(log.path()).unwrap();
    let invalid = b"partial-tail";
    fs::write(log.path(), [valid.as_slice(), invalid].concat()).unwrap();
    let first_path = log.replay().unwrap().corruption.unwrap().quarantine_path;
    fs::write(log.path(), [valid.as_slice(), invalid].concat()).unwrap();
    let second_path = log.replay().unwrap().corruption.unwrap().quarantine_path;
    assert_eq!(first_path, second_path);
}

#[test]
fn journal_uses_required_directory_layout() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let log = LocalMutationLog::open(root.path(), note).unwrap();
    assert_eq!(
        log.path(),
        root.path()
            .join(".secondbrain")
            .join("oplog")
            .join(note.to_string())
            .join("local-mutations.log")
    );
}

#[test]
fn replay_never_silently_discards_invalid_bytes() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let log = LocalMutationLog::open(root.path(), note).unwrap();
    fs::write(log.path(), b"invalid").unwrap();
    let outcome = log.replay().unwrap();
    let report = outcome.corruption.unwrap();
    assert_eq!(fs::read(report.quarantine_path).unwrap(), b"invalid");
}

#[test]
fn quarantine_directory_is_per_note() {
    let root = tempfile::tempdir().unwrap();
    let note = NoteId::new();
    let log = LocalMutationLog::open(root.path(), note).unwrap();
    fs::write(log.path(), b"bad").unwrap();
    let report = log.replay().unwrap().corruption.unwrap();
    assert!(
        report.quarantine_path.starts_with(
            root.path()
                .join(".secondbrain/oplog")
                .join(note.to_string())
                .join("quarantine")
        )
    );
}
