//! Normalized external filesystem changes become attributed transactions.
//!
//! Every test drives the real coordinator against a real temporary workspace:
//! the identity map, the transaction engine, the oplog, the transaction markers
//! and the converged-base snapshots are all production types. Only index
//! refresh is a fake, because no incremental index API exists yet.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId, WorkspaceId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::SourceDocument;
use secondbrain_markdown::diff::diff_documents;
use secondbrain_transaction::base_snapshot::{BaseSnapshot, BaseSnapshotStore};
use secondbrain_transaction::external_edit::{
    ExternalEditCoordinator, ExternalEditError, ExternalEditOutcome, IndexRefresh,
};
use secondbrain_transaction::oplog::LocalMutationLog;
use secondbrain_transaction::record::LocalOperationRecord;
use secondbrain_transaction::{TransactionEngine, TransactionError, TransactionRequest};
use secondbrain_vault::event::WorkspaceEvent;
use secondbrain_vault::watcher::{Normalizer, RawEvent, RawEventKind};
use secondbrain_vault::{IdentityMap, WorkspaceRoot};
use tempfile::{TempDir, tempdir};

/// The device the external edits are attributed to.
const DEVICE: &str = "laptop";

/// The actor every external edit must be committed as.
const EXTERNAL_ACTOR: &str = "external:laptop";

/// The note every content test edits.
const NOTE: &str = "notes/meeting.md";

/// The version a note's converged base starts at.
const GENESIS: NoteVersion = NoteVersion::new(0);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/external-edits/integration")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Records which notes an index refresh was requested for.
#[derive(Default)]
struct RecordingIndex {
    refreshed: RefCell<Vec<(NoteId, WorkspacePath)>>,
}

impl RecordingIndex {
    fn refreshed(&self) -> Vec<(NoteId, WorkspacePath)> {
        self.refreshed.borrow().clone()
    }
}

impl IndexRefresh for &RecordingIndex {
    fn refresh(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
    ) -> Result<(), secondbrain_core::Error> {
        self.refreshed.borrow_mut().push((note_id, path.clone()));
        Ok(())
    }
}

type Coordinator<'a> = ExternalEditCoordinator<&'a RecordingIndex>;

/// An initialized temporary workspace.
struct Workspace {
    _directory: TempDir,
    root: WorkspaceRoot,
    id: WorkspaceId,
}

fn workspace() -> Workspace {
    let directory = tempdir().expect("tempdir");
    let manifest = secondbrain_vault::initialize_workspace(directory.path()).expect("initialize");
    let root = WorkspaceRoot::open(directory.path()).expect("root");
    Workspace {
        _directory: directory,
        root,
        id: manifest.workspace_id,
    }
}

impl Workspace {
    fn absolute(&self, path: &str) -> PathBuf {
        self.root.canonical_path().join(path)
    }

    fn write(&self, path: &str, source: &str) {
        let absolute = self.absolute(path);
        fs::create_dir_all(absolute.parent().expect("parent")).expect("create parent");
        fs::write(absolute, source).expect("write note");
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.absolute(path)).expect("read note")
    }

    fn coordinator<'a>(&self, index: &'a RecordingIndex) -> Coordinator<'a> {
        ExternalEditCoordinator::new(
            self.root.clone(),
            self.id,
            DeviceId::new(DEVICE).expect("device"),
            index,
        )
        .expect("coordinator")
    }

    fn engine(&self) -> TransactionEngine {
        TransactionEngine::new(self.root.clone(), self.id)
    }

    fn records(&self, note_id: NoteId) -> Vec<LocalOperationRecord> {
        LocalMutationLog::open(self.root.canonical_path(), note_id)
            .expect("open oplog")
            .replay()
            .expect("replay oplog")
            .records
    }

    fn marker(&self, transaction_id: TransactionId) -> serde_json::Value {
        let path = self
            .root
            .canonical_path()
            .join(format!(".secondbrain/transactions/{transaction_id}.json"));
        serde_json::from_slice(&fs::read(&path).expect("read marker")).expect("parse marker")
    }

    fn base_record(&self, note_id: NoteId) -> BaseSnapshot {
        BaseSnapshotStore::new(&self.root)
            .load(note_id)
            .expect("load base")
            .expect("base exists")
    }

    fn base(&self, note_id: NoteId) -> String {
        self.base_record(note_id).source
    }

    /// Every transaction marker in the workspace, whichever note it belongs to.
    fn markers(&self) -> Vec<serde_json::Value> {
        let directory = self.root.canonical_path().join(".secondbrain/transactions");
        let mut markers = Vec::new();
        for entry in fs::read_dir(&directory).expect("read transactions") {
            let path = entry.expect("entry").path();
            let is_marker = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.parse::<TransactionId>().is_ok());
            if is_marker {
                markers.push(
                    serde_json::from_slice(&fs::read(&path).expect("read marker"))
                        .expect("parse marker"),
                );
            }
        }
        markers
    }
}

fn note_path() -> WorkspacePath {
    WorkspacePath::new(NOTE).expect("note path")
}

fn changed(path: &str, source: &str) -> WorkspaceEvent {
    WorkspaceEvent::ContentChanged {
        path: WorkspacePath::new(path).expect("path"),
        hash: ContentHash::digest(source.as_bytes()),
    }
}

/// Brings a note under management: registers its identity and records the
/// converged base that later external edits are diffed against.
fn converge(
    workspace: &Workspace,
    coordinator: &mut Coordinator<'_>,
    path: &str,
    source: &str,
) -> NoteId {
    workspace.write(path, source);
    match coordinator
        .integrate(changed(path, source))
        .expect("genesis integration")
    {
        ExternalEditOutcome::Registered { note_id } => note_id,
        other => panic!("expected genesis registration, got {other:?}"),
    }
}

/// Runs an internal commit of `internal` over `base` and crashes it at
/// `boundary`, returning the transaction that was interrupted.
fn commit_internal_crashing_at(
    workspace: &Workspace,
    note_id: NoteId,
    path: &str,
    base: &str,
    internal: &str,
    boundary: &str,
) -> TransactionId {
    let operations = diff_documents(
        &SourceDocument::parse(base).expect("parse base"),
        &SourceDocument::parse(internal).expect("parse internal"),
    );
    assert!(
        !operations.is_empty(),
        "the internal edit must produce operations"
    );
    let request = TransactionRequest {
        id: TransactionId::new(),
        actor: ActorId::new("alice").expect("actor"),
        device: DeviceId::new("desktop").expect("device"),
        note_id,
        path: WorkspacePath::new(path).expect("path"),
        expected_hash: ContentHash::digest(base.as_bytes()),
        expected_version: GENESIS,
        operations,
    };
    let transaction_id = request.id;
    secondbrain_transaction::failpoint::set(Some(boundary));
    let error = workspace.engine().commit(request).expect_err("failpoint");
    secondbrain_transaction::failpoint::set(None);
    assert!(matches!(error, TransactionError::Io(_)), "{error}");
    transaction_id
}

/// Journals an internal transaction without materializing it, the way a crash
/// between a durable oplog append and the Markdown write leaves the workspace.
fn journal_internal_without_materializing(
    workspace: &Workspace,
    note_id: NoteId,
    base: &str,
    internal: &str,
) -> TransactionId {
    let transaction_id = commit_internal_crashing_at(
        workspace,
        note_id,
        NOTE,
        base,
        internal,
        "after_operations_durable",
    );
    assert_eq!(
        workspace.marker(transaction_id)["state"],
        "OPERATIONS_DURABLE"
    );
    transaction_id
}

// ---------------------------------------------------------------------------
// Genesis
// ---------------------------------------------------------------------------

#[test]
fn first_observation_registers_the_note_and_records_its_converged_base() {
    let base = fixture("converged-base.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);

    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    assert_eq!(workspace.base(note_id), base);
    assert_eq!(workspace.read(NOTE), base, "genesis must not rewrite bytes");
    assert!(
        workspace.records(note_id).is_empty(),
        "observing a note is not a transaction"
    );
    assert_eq!(index.refreshed(), vec![(note_id, note_path())]);
}

// ---------------------------------------------------------------------------
// Case 1: an external paragraph edit is committed as external:<device>
// ---------------------------------------------------------------------------

#[test]
fn external_paragraph_edit_is_journaled_as_the_external_actor_without_rewriting_the_file() {
    let base = fixture("converged-base.md");
    let external = fixture("external-paragraph-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // An external editor rewrites the whole file with one paragraph changed.
    workspace.write(NOTE, &external);
    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate external edit");

    let ExternalEditOutcome::Adopted {
        note_id: adopted_note,
        transaction_id,
        version,
    } = outcome
    else {
        panic!("expected the edit to be adopted, got {outcome:?}");
    };
    assert_eq!(adopted_note, note_id);
    assert_eq!(version, NoteVersion::new(1));

    // The bytes the editor wrote are left exactly as they are.
    assert_eq!(workspace.read(NOTE), external);
    // The edit is journaled, attributed to the external device, and committed.
    let records = workspace.records(note_id);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].transaction_id, transaction_id);
    assert_eq!(records[0].actor_id, ActorId::new(EXTERNAL_ACTOR).unwrap());
    assert_eq!(records[0].device_id, DeviceId::new(DEVICE).unwrap());
    assert_eq!(records[0].operation.kind_name(), "ReplaceNode");
    let marker = workspace.marker(transaction_id);
    assert_eq!(marker["state"], "COMMITTED");
    assert_eq!(marker["committed_version"], 1);
    // The adopted content becomes the base the next edit is diffed against.
    assert_eq!(workspace.base(note_id), external);
}

// ---------------------------------------------------------------------------
// Case 2: a simultaneous internal change to another paragraph merges
// ---------------------------------------------------------------------------

#[test]
fn simultaneous_internal_change_to_another_paragraph_merges() {
    let base = fixture("converged-base.md");
    let internal = fixture("internal-rollout-edit.md");
    let external = fixture("external-paragraph-edit.md");
    let merged = fixture("merged-migration-and-rollout.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // An internal transaction changed the rollout paragraph but never got to
    // materialize; the external editor then saved its own edit of the
    // migration paragraph over the same file.
    let internal_id = journal_internal_without_materializing(&workspace, note_id, &base, &internal);
    workspace.write(NOTE, &external);

    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate external edit");

    let ExternalEditOutcome::Merged {
        note_id: merged_note,
        transaction_id,
        version,
        source_hash,
    } = outcome
    else {
        panic!("expected a merge, got {outcome:?}");
    };
    assert_eq!(merged_note, note_id);
    assert_eq!(source_hash, ContentHash::digest(merged.as_bytes()));
    assert_eq!(version, NoteVersion::new(2));

    // Both changes survive.
    assert_eq!(workspace.read(NOTE), merged);
    assert_eq!(workspace.base(note_id), merged);

    // The external edit keeps external attribution; the rebased internal
    // operations keep the attribution they were journaled with.
    let records = workspace.records(note_id);
    let external_actors: Vec<_> = records
        .iter()
        .filter(|record| record.actor_id == ActorId::new(EXTERNAL_ACTOR).unwrap())
        .collect();
    assert_eq!(external_actors.len(), 1, "{records:?}");
    let rebased: Vec<_> = records
        .iter()
        .filter(|record| record.transaction_id == transaction_id)
        .collect();
    assert_eq!(rebased.len(), 1, "{records:?}");
    assert_eq!(rebased[0].actor_id, ActorId::new("alice").unwrap());
    assert_ne!(
        transaction_id, internal_id,
        "the rebase is its own transaction"
    );

    // The superseded transaction is closed out, so recovery has nothing to
    // replay and cannot clobber the merged file.
    assert_eq!(workspace.marker(internal_id)["state"], "ABORTED");
    assert!(workspace.engine().recover().expect("recover").is_empty());
    assert_eq!(workspace.read(NOTE), merged);
}

#[test]
fn pending_internal_operations_that_cannot_be_rebased_are_left_for_recovery() {
    let base = fixture("converged-base.md");
    // The internal transaction and the external editor both changed the
    // migration paragraph, so the journaled operations no longer anchor.
    let internal = fixture("internal-migration-edit.md");
    let external = fixture("external-paragraph-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);
    let internal_id = journal_internal_without_materializing(&workspace, note_id, &base, &internal);
    workspace.write(NOTE, &external);

    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate external edit");

    assert!(
        matches!(outcome, ExternalEditOutcome::Adopted { .. }),
        "{outcome:?}"
    );
    assert_eq!(workspace.read(NOTE), external, "nothing may be guessed");
    assert_eq!(
        workspace.marker(internal_id)["state"],
        "OPERATIONS_DURABLE",
        "the transaction stays recovery's business"
    );
}

// ---------------------------------------------------------------------------
// Case 3: a stale external base rebases
// ---------------------------------------------------------------------------

#[test]
fn stale_event_hash_rebases_onto_the_current_content() {
    let base = fixture("converged-base.md");
    let first = fixture("external-paragraph-edit.md");
    let second = fixture("external-second-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // The event describes the first external save, but a second save landed
    // before the coordinator got to it.
    workspace.write(NOTE, &first);
    let stale = changed(NOTE, &first);
    workspace.write(NOTE, &second);

    let outcome = coordinator.integrate(stale).expect("integrate stale event");

    assert!(
        matches!(outcome, ExternalEditOutcome::Adopted { version, .. } if version == NoteVersion::new(1)),
        "{outcome:?}"
    );
    // The newer content stands, and the journal describes the edit that is
    // actually on disk rather than the one the event named.
    assert_eq!(workspace.read(NOTE), second);
    assert_eq!(workspace.base(note_id), second);
    let records = workspace.records(note_id);
    assert_eq!(records.len(), 1, "{records:?}");
    let expected = diff_documents(
        &SourceDocument::parse(&base).unwrap(),
        &SourceDocument::parse(&second).unwrap(),
    );
    assert_eq!(records[0].operation, expected[0]);
}

// ---------------------------------------------------------------------------
// Case 4: an ambiguous edit produces a review artifact and does not overwrite
// ---------------------------------------------------------------------------

#[test]
fn ambiguous_duplicate_paragraph_edit_writes_a_review_descriptor_and_leaves_the_file_untouched() {
    let base = fixture("duplicate-paragraph-base.md");
    let external = fixture("duplicate-paragraph-external.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    workspace.write(NOTE, &external);
    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate ambiguous edit");

    let ExternalEditOutcome::ReviewRequired {
        transaction_id,
        descriptor,
    } = outcome
    else {
        panic!("expected review to be required, got {outcome:?}");
    };
    assert_eq!(
        descriptor,
        workspace.root.canonical_path().join(format!(
            ".secondbrain/transactions/{transaction_id}.conflict.json"
        ))
    );
    let review: serde_json::Value =
        serde_json::from_slice(&fs::read(&descriptor).expect("read descriptor")).expect("json");
    assert_eq!(review["note_id"], note_id.to_string());
    assert_eq!(review["path"], NOTE);
    assert_eq!(review["actor"], EXTERNAL_ACTOR);
    assert!(
        review["reason"]
            .as_str()
            .expect("reason")
            .contains("ambiguous"),
        "{review}"
    );
    // The diff layer embeds the whole incoming source in its reason so that
    // applying a NeedsReview operation can return it. A descriptor says why
    // review is needed and must not keep a second copy of the note inside
    // `.secondbrain/`: the note is on disk, and the descriptor is not a backup.
    let written = fs::read_to_string(&descriptor).expect("read descriptor");
    assert!(!written.contains("__INCOMING__"), "{written}");
    for line in external.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            !written.contains(line),
            "the descriptor embeds the note body: {line:?} in {written}"
        );
    }

    // Nothing was written, journaled, or converged.
    assert_eq!(workspace.read(NOTE), external);
    assert_eq!(workspace.base(note_id), base);
    assert!(workspace.records(note_id).is_empty());
    assert!(
        !workspace
            .root
            .canonical_path()
            .join(format!(".secondbrain/transactions/{transaction_id}.json"))
            .exists()
    );
}

#[test]
fn ambiguous_identity_writes_a_review_descriptor() {
    // Two tracked notes share a structure and span layout, so a third file
    // with the same shape matches both on fingerprint alone.
    let first = "# Title\n\nAlpha text.\n";
    let second = "# Title\n\nBravo text.\n";
    let third = "# Title\n\nCarol text.\n";
    let workspace = workspace();
    let mut identity = IdentityMap::open(&workspace.root).expect("identity map");
    for (path, source) in [("notes/first.md", first), ("notes/second.md", second)] {
        workspace.write(path, source);
        identity
            .register(
                &WorkspacePath::new(path).expect("path"),
                ContentHash::digest(source.as_bytes()),
                SourceDocument::parse(source)
                    .expect("parse")
                    .semantic_fingerprint(),
            )
            .expect("register");
    }
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);

    workspace.write("notes/third.md", third);
    let outcome = coordinator
        .integrate(changed("notes/third.md", third))
        .expect("integrate ambiguous identity");

    let ExternalEditOutcome::ReviewRequired { descriptor, .. } = outcome else {
        panic!("expected review to be required, got {outcome:?}");
    };
    let review: serde_json::Value =
        serde_json::from_slice(&fs::read(&descriptor).expect("read descriptor")).expect("json");
    assert_eq!(review["path"], "notes/third.md");
    assert_eq!(review["note_id"], serde_json::Value::Null);
    assert_eq!(
        review["identity_candidates"]
            .as_array()
            .expect("candidates")
            .len(),
        2,
        "{review}"
    );
    assert_eq!(workspace.read("notes/third.md"), third);
}

// ---------------------------------------------------------------------------
// Case 5: an external rename keeps the note ID
// ---------------------------------------------------------------------------

#[test]
fn external_rename_keeps_the_note_id_and_updates_the_identity_map() {
    let base = fixture("converged-base.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // A rename moves bytes without changing them.
    fs::create_dir_all(workspace.absolute("archive")).expect("create archive");
    fs::rename(
        workspace.absolute(NOTE),
        workspace.absolute("archive/meeting.md"),
    )
    .expect("rename note");
    let renamed = WorkspacePath::new("archive/meeting.md").expect("renamed path");
    let outcome = coordinator
        .integrate(WorkspaceEvent::Renamed {
            from: note_path(),
            to: renamed.clone(),
        })
        .expect("integrate rename");

    assert_eq!(
        outcome,
        ExternalEditOutcome::Renamed {
            note_id,
            path: renamed.clone(),
        }
    );
    let record = IdentityMap::open(&workspace.root)
        .expect("identity map")
        .lookup(&note_id)
        .expect("lookup")
        .expect("record");
    assert_eq!(record.current_path, renamed);
    assert!(record.historical_paths.contains(&note_path()));
    // No transaction: a rename changes no bytes.
    assert!(workspace.records(note_id).is_empty());
    assert_eq!(workspace.read("archive/meeting.md"), base);
    assert_eq!(index.refreshed().last().expect("refresh").1, renamed);
}

// ---------------------------------------------------------------------------
// Case 6: an external copy gets a new ID
// ---------------------------------------------------------------------------

#[test]
fn external_copy_gets_a_new_note_id() {
    let base = fixture("converged-base.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // A file manager duplicates the note byte for byte.
    workspace.write("notes/meeting copy.md", &base);
    let outcome = coordinator
        .integrate(changed("notes/meeting copy.md", &base))
        .expect("integrate copy");

    let ExternalEditOutcome::Copied {
        note_id: copy_id,
        source_note_id,
    } = outcome
    else {
        panic!("expected a copy, got {outcome:?}");
    };
    assert_ne!(copy_id, note_id, "a copy must not inherit the identity");
    assert_eq!(source_note_id, note_id);
    let identity = IdentityMap::open(&workspace.root).expect("identity map");
    assert_eq!(
        identity
            .lookup(&copy_id)
            .expect("lookup")
            .expect("record")
            .current_path,
        WorkspacePath::new("notes/meeting copy.md").unwrap()
    );
    assert_eq!(
        identity
            .lookup(&note_id)
            .expect("lookup")
            .expect("record")
            .current_path,
        note_path(),
        "the original keeps its path"
    );
    assert_eq!(workspace.base(copy_id), base);
}

// ---------------------------------------------------------------------------
// Case 7: a successful commit refreshes the index
// ---------------------------------------------------------------------------

#[test]
fn successful_adoption_refreshes_the_index_and_an_unchanged_note_does_not() {
    let base = fixture("converged-base.md");
    let external = fixture("external-paragraph-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);
    assert_eq!(index.refreshed().len(), 1, "genesis refreshes once");

    workspace.write(NOTE, &external);
    coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate external edit");

    assert_eq!(
        index.refreshed(),
        vec![(note_id, note_path()), (note_id, note_path())],
        "an adopted edit refreshes exactly the note it changed"
    );

    // Re-delivering the same state is not a change and must not refresh again.
    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate duplicate event");
    assert_eq!(outcome, ExternalEditOutcome::Unchanged { note_id });
    assert_eq!(index.refreshed().len(), 2);
}

// ---------------------------------------------------------------------------
// Case 8: an internal materialization does not loop back
// ---------------------------------------------------------------------------

#[test]
fn internal_materialization_does_not_loop_back() {
    let base = fixture("converged-base.md");
    let internal = fixture("internal-rollout-edit.md");
    let external = fixture("external-paragraph-edit.md");
    let merged = fixture("merged-migration-and-rollout.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);
    journal_internal_without_materializing(&workspace, note_id, &base, &internal);
    workspace.write(NOTE, &external);

    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate external edit");
    let ExternalEditOutcome::Merged { source_hash, .. } = outcome else {
        panic!("expected a merge, got {outcome:?}");
    };

    // The write the merge performed is announced to the watcher, which then
    // suppresses the filesystem event it generated.
    let mut normalizer =
        Normalizer::new(workspace.root.clone(), Vec::new(), Duration::from_secs(60));
    let now = Instant::now();
    normalizer.record_internal_write(note_path(), source_hash, now);
    let own_write = RawEvent {
        kind: RawEventKind::Modify,
        rename_mode: None,
        tracker: None,
        paths: vec![workspace.absolute(NOTE)],
    };
    assert!(
        normalizer
            .normalize([own_write], now)
            .expect("normalize")
            .is_empty(),
        "the coordinator's own materialization must not become an event"
    );

    // And if the event does arrive anyway, integrating it changes nothing.
    let refreshes = index.refreshed().len();
    let journaled = workspace.records(note_id).len();
    let outcome = coordinator
        .integrate(changed(NOTE, &merged))
        .expect("integrate own materialization");
    assert_eq!(outcome, ExternalEditOutcome::Unchanged { note_id });
    assert_eq!(workspace.read(NOTE), merged);
    assert_eq!(index.refreshed().len(), refreshes);
    assert_eq!(
        workspace.records(note_id).len(),
        journaled,
        "no extra operation may be journaled"
    );
}

// ---------------------------------------------------------------------------
// Deletion and engine contract
// ---------------------------------------------------------------------------

#[test]
fn deleted_note_is_reported_without_discarding_identity_or_base() {
    let base = fixture("converged-base.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    fs::remove_file(workspace.absolute(NOTE)).expect("remove note");
    let outcome = coordinator
        .integrate(WorkspaceEvent::Deleted { path: note_path() })
        .expect("integrate deletion");

    assert_eq!(outcome, ExternalEditOutcome::Deleted { path: note_path() });
    assert_eq!(workspace.base(note_id), base, "the base survives deletion");
    assert!(
        IdentityMap::open(&workspace.root)
            .expect("identity map")
            .lookup(&note_id)
            .expect("lookup")
            .is_some(),
        "identity survives deletion so the note can come back"
    );
}

#[test]
fn adopt_external_rejects_operations_whose_post_state_is_not_on_disk() {
    let base = fixture("converged-base.md");
    let external = fixture("external-paragraph-edit.md");
    let other = fixture("external-second-edit.md");
    let workspace = workspace();
    workspace.write(NOTE, &external);
    let operations = diff_documents(
        &SourceDocument::parse(&base).unwrap(),
        &SourceDocument::parse(&other).unwrap(),
    );
    let request = TransactionRequest {
        id: TransactionId::new(),
        actor: ActorId::new(EXTERNAL_ACTOR).unwrap(),
        device: DeviceId::new(DEVICE).unwrap(),
        note_id: NoteId::new(),
        path: note_path(),
        expected_hash: ContentHash::digest(external.as_bytes()),
        expected_version: GENESIS,
        operations,
    };
    let note_id = request.note_id;

    let error = workspace
        .engine()
        .adopt_external(request, &base)
        .expect_err("the post state is not what is on disk");

    assert!(
        matches!(error, TransactionError::PostStateMismatch { .. }),
        "{error}"
    );
    assert_eq!(workspace.read(NOTE), external);
    assert!(workspace.records(note_id).is_empty());
}

// ---------------------------------------------------------------------------
// Crash windows
// ---------------------------------------------------------------------------

#[test]
fn a_crash_between_the_commit_marker_and_the_converged_base_cannot_replay_the_edit() {
    let base = fixture("converged-base.md");
    let internal = fixture("internal-rollout-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // An internal commit materializes the note, then the process dies before
    // the state both sides now agree on is durable.
    let internal_id = commit_internal_crashing_at(
        &workspace,
        note_id,
        NOTE,
        &base,
        &internal,
        "after_commit_before_index",
    );
    assert_eq!(workspace.read(NOTE), internal, "the Markdown was written");

    // On restart recovery repairs what the crash left behind. Whatever it
    // repairs, the converged base must describe the file as it now is: it is
    // the only record of what the note looked like before the next external
    // edit lands on top of it.
    workspace.engine().recover().expect("recover");
    let recorded = workspace.base_record(note_id);
    assert_eq!(recorded.source, internal);
    assert_eq!(recorded.version, NoteVersion::new(1));

    // So the next external event reports no change, instead of re-deriving the
    // *internal* edit from a stale base and journaling it a second time as the
    // external actor, at a version another marker already claims.
    let outcome = coordinator
        .integrate(changed(NOTE, &internal))
        .expect("integrate");

    assert_eq!(outcome, ExternalEditOutcome::Unchanged { note_id });
    let records = workspace.records(note_id);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].transaction_id, internal_id);
    let committed: Vec<_> = workspace
        .markers()
        .into_iter()
        .filter(|marker| marker["state"] == "COMMITTED")
        .collect();
    assert_eq!(committed.len(), 1, "{committed:?}");
}

#[test]
fn a_crash_inside_an_adoption_leaves_the_converged_base_agreeing_with_the_file() {
    let base = fixture("converged-base.md");
    let external = fixture("external-paragraph-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // The adoption dies in the window between its two durable writes.
    workspace.write(NOTE, &external);
    secondbrain_transaction::failpoint::set(Some("during_adopt_convergence"));
    let error = coordinator
        .integrate(changed(NOTE, &external))
        .expect_err("failpoint");
    secondbrain_transaction::failpoint::set(None);
    assert!(
        matches!(
            error,
            ExternalEditError::Transaction(TransactionError::Io(_))
        ),
        "{error}"
    );

    // Recovery has nothing to do: an adoption never rewrites Markdown, so a
    // half-finished one is journal records that no marker claims.
    assert!(workspace.engine().recover().expect("recover").is_empty());

    // Redelivering the event must not journal the same edit again, at a
    // version a marker already claims.
    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate");

    assert_eq!(outcome, ExternalEditOutcome::Unchanged { note_id });
    let records = workspace.records(note_id);
    assert_eq!(records.len(), 1, "{records:?}");
    let committed: Vec<_> = workspace
        .markers()
        .into_iter()
        .filter(|marker| marker["state"] == "COMMITTED")
        .collect();
    assert!(committed.len() <= 1, "{committed:?}");
}

#[test]
fn a_crash_before_the_rebase_is_durable_leaves_the_superseded_transaction_to_recovery() {
    let base = fixture("converged-base.md");
    let internal = fixture("internal-rollout-edit.md");
    let external = fixture("external-paragraph-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);
    let internal_id = journal_internal_without_materializing(&workspace, note_id, &base, &internal);
    workspace.write(NOTE, &external);

    // The rebase that carries the internal operations forward dies before its
    // own operations are durable.
    secondbrain_transaction::failpoint::set(Some("before_append"));
    let error = coordinator
        .integrate(changed(NOTE, &external))
        .expect_err("failpoint");
    secondbrain_transaction::failpoint::set(None);
    assert!(
        matches!(
            error,
            ExternalEditError::Transaction(TransactionError::Io(_))
        ),
        "{error}"
    );

    // OPERATIONS_DURABLE is the promise that recovery will finish the edit.
    // Nothing may retract that promise before the transaction carrying those
    // operations forward is itself durable, or the operations become
    // unreachable: aborted here, never journaled there.
    assert_eq!(
        workspace.marker(internal_id)["state"],
        "OPERATIONS_DURABLE",
        "the superseded transaction is recovery's to close out"
    );

    // Recovery, not the coordinator, decides what becomes of it, through the
    // state machine it already owns.
    workspace.engine().recover().expect("recover");
    assert_eq!(workspace.marker(internal_id)["state"], "ABORTED");
    assert_eq!(
        workspace.read(NOTE),
        external,
        "recovery preserves the newer file rather than guessing"
    );
}

#[test]
fn one_unreplayable_transaction_does_not_abandon_recovery_for_every_other_note() {
    const OTHER: &str = "notes/other.md";
    let base = fixture("converged-base.md");
    let internal = fixture("internal-rollout-edit.md");
    let external = fixture("external-paragraph-edit.md");
    let merged = fixture("merged-migration-and-rollout.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);
    let internal_id = journal_internal_without_materializing(&workspace, note_id, &base, &internal);
    workspace.write(NOTE, &external);

    // The merge lands, and the process dies before the transaction whose
    // operations it carried forward is closed out.
    secondbrain_transaction::failpoint::set(Some("after_rebase_before_supersede"));
    let error = coordinator
        .integrate(changed(NOTE, &external))
        .expect_err("failpoint");
    secondbrain_transaction::failpoint::set(None);
    assert!(matches!(error, ExternalEditError::Io(_)), "{error}");
    assert_eq!(workspace.read(NOTE), merged, "the merge is durable");
    assert_eq!(workspace.marker(internal_id)["state"], "OPERATIONS_DURABLE");

    // An unrelated note is left mid-transaction by the same crash.
    let other_base = "# Other Notes\n\nDana owns the docs.\n";
    let other_internal = "# Other Notes\n\nGrace owns the docs.\n";
    let other_id = converge(&workspace, &mut coordinator, OTHER, other_base);
    let other_transaction = commit_internal_crashing_at(
        &workspace,
        other_id,
        OTHER,
        other_base,
        other_internal,
        "after_operations_durable",
    );

    // The superseded operations no longer anchor anywhere in the merged file,
    // so replaying them is impossible. That must close out one transaction,
    // not abandon the recovery pass and leave every other note unrecovered.
    workspace
        .engine()
        .recover()
        .expect("one unreplayable transaction must not fail the pass");

    assert_eq!(workspace.marker(internal_id)["state"], "ABORTED");
    assert_eq!(workspace.read(NOTE), merged, "the merged file is preserved");
    assert_eq!(workspace.marker(other_transaction)["state"], "COMMITTED");
    assert_eq!(
        workspace.read(OTHER),
        other_internal,
        "the unrelated note is recovered"
    );
}

#[test]
fn a_tracked_note_whose_converged_base_was_lost_is_reported_rather_than_registered() {
    let base = fixture("converged-base.md");
    let external = fixture("external-paragraph-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);

    // The converged base is the only record of what the note looked like
    // before an external edit. Losing it — or having been registered before
    // this pipeline existed — makes the next edit unrecoverable.
    fs::remove_file(
        workspace
            .root
            .canonical_path()
            .join(format!(".secondbrain/snapshots/{note_id}.json")),
    )
    .expect("remove converged base");
    workspace.write(NOTE, &external);

    let outcome = coordinator
        .integrate(changed(NOTE, &external))
        .expect("integrate edit with no base");

    // Distinguishable from a first observation: the note was already known, so
    // an edit was silently dropped rather than a new file simply being adopted.
    assert_eq!(outcome, ExternalEditOutcome::BaseRecovered { note_id });
    assert_ne!(
        outcome,
        ExternalEditOutcome::Registered { note_id },
        "losing a base is not the same event as meeting a new file"
    );
    // The content on disk becomes the base, and nothing is invented for the
    // edit that could not be derived.
    let recorded = workspace.base_record(note_id);
    assert_eq!(recorded.source, external);
    assert_eq!(recorded.version, GENESIS);
    assert_eq!(workspace.read(NOTE), external);
    assert!(workspace.records(note_id).is_empty());

    // The next edit is derived normally, against the recovered base.
    let second = fixture("external-second-edit.md");
    workspace.write(NOTE, &second);
    let outcome = coordinator
        .integrate(changed(NOTE, &second))
        .expect("integrate next edit");
    assert!(
        matches!(outcome, ExternalEditOutcome::Adopted { version, .. } if version == NoteVersion::new(1)),
        "{outcome:?}"
    );
}

#[test]
fn adopting_no_operations_journals_nothing_and_does_not_bump_the_version() {
    let base = fixture("converged-base.md");
    let workspace = workspace();
    workspace.write(NOTE, &base);
    let request = TransactionRequest {
        id: TransactionId::new(),
        actor: ActorId::new(EXTERNAL_ACTOR).expect("actor"),
        device: DeviceId::new(DEVICE).expect("device"),
        note_id: NoteId::new(),
        path: note_path(),
        expected_hash: ContentHash::digest(base.as_bytes()),
        expected_version: NoteVersion::new(4),
        operations: Vec::new(),
    };
    let note_id = request.note_id;
    let transaction_id = request.id;

    let outcome = workspace
        .engine()
        .adopt_external(request, &base)
        .expect("adopt nothing");

    assert!(!outcome.changed);
    assert_eq!(
        outcome.version,
        NoteVersion::new(4),
        "there is no version to bump for having journaled nothing"
    );
    assert!(workspace.records(note_id).is_empty());
    assert!(
        !workspace
            .root
            .canonical_path()
            .join(format!(".secondbrain/transactions/{transaction_id}.json"))
            .exists(),
        "an empty adoption is not a transaction"
    );
}

#[test]
fn recovered_materialization_updates_the_converged_base() {
    let base = fixture("converged-base.md");
    let internal = fixture("internal-rollout-edit.md");
    let workspace = workspace();
    let index = RecordingIndex::default();
    let mut coordinator = workspace.coordinator(&index);
    let note_id = converge(&workspace, &mut coordinator, NOTE, &base);
    journal_internal_without_materializing(&workspace, note_id, &base, &internal);

    workspace.engine().recover().expect("recover");

    assert_eq!(workspace.read(NOTE), internal);
    assert_eq!(
        workspace.base(note_id),
        internal,
        "recovery converges the note, so it owes the base an update"
    );
}
