use std::fs;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_index::{IndexConfig, IndexDatabase, SearchQuery, index_path, rebuild};
use secondbrain_transaction::{
    ApplyPreviewOutcome, TRANSACTION_PREVIEW_FORMAT_V1, TransactionPreview,
    TransactionPreviewError, apply_preview, preview_transaction,
};
use secondbrain_vault::initialize_workspace;
use tempfile::TempDir;

const NOTE: &str = "notes/alpha.md";
const BASE: &str = "# Alpha\n\nOriginal.\n";

fn actor() -> ActorId {
    ActorId::new("test-actor").unwrap()
}

fn device() -> DeviceId {
    DeviceId::new("test-device").unwrap()
}

fn workspace(source: &str) -> TempDir {
    let directory = TempDir::new().unwrap();
    fs::create_dir_all(directory.path().join("notes")).unwrap();
    fs::write(directory.path().join(NOTE), source).unwrap();
    initialize_workspace(directory.path()).unwrap();
    rebuild(directory.path(), &IndexConfig::default()).unwrap();
    directory
}

#[test]
fn no_op_preview_is_stable_and_apply_writes_nothing() {
    let workspace = workspace(BASE);
    let preview = preview_transaction(workspace.path(), NOTE, BASE, actor(), device()).unwrap();

    assert_eq!(preview.format, TRANSACTION_PREVIEW_FORMAT_V1);
    assert!(preview.operations.is_empty());
    assert_eq!(preview.summary, "no semantic change");
    assert_eq!(preview.proposed_source_hash, ContentHash::digest(BASE));
    let outcome = apply_preview(workspace.path(), &preview).unwrap();
    assert!(!outcome.changed);
    assert!(!outcome.index_refreshed);
    assert_eq!(
        fs::read_to_string(workspace.path().join(NOTE)).unwrap(),
        BASE
    );
}

#[test]
fn stale_preview_fails_closed() {
    let workspace = workspace(BASE);
    let preview = preview_transaction(
        workspace.path(),
        NOTE,
        "# Alpha\n\nProposed.\n",
        actor(),
        device(),
    )
    .unwrap();
    fs::write(workspace.path().join(NOTE), "# Alpha\n\nOther edit.\n").unwrap();

    assert!(matches!(
        apply_preview(workspace.path(), &preview),
        Err(TransactionPreviewError::Transaction(
            secondbrain_transaction::TransactionError::StaleHash { .. }
        ))
    ));
}

#[test]
fn needs_review_preview_cannot_apply() {
    let base = "Duplicate.\n\nMiddle.\n\nDuplicate.\n";
    let workspace = workspace(base);
    let preview = preview_transaction(
        workspace.path(),
        NOTE,
        "Duplicate.\n\nMiddle.\n\nChanged.\n",
        actor(),
        device(),
    )
    .unwrap();

    assert!(preview.review_required);
    assert!(matches!(
        apply_preview(workspace.path(), &preview),
        Err(TransactionPreviewError::NeedsReview(_))
    ));
    assert_eq!(
        fs::read_to_string(workspace.path().join(NOTE)).unwrap(),
        base
    );
}

#[test]
fn apply_is_durable_searchable_and_preserves_raw_syntax() {
    let base = "# Alpha\n\nOriginal.\n\n```custom {raw=yes}\n[[untouched|Alias]]\n```\n";
    let workspace = workspace(base);
    let proposed =
        "# Alpha\n\nSearchable canary.\n\n```custom {raw=yes}\n[[untouched|Alias]]\n```\n";
    let preview = preview_transaction(workspace.path(), NOTE, proposed, actor(), device()).unwrap();
    let outcome = apply_preview(workspace.path(), &preview).unwrap();

    assert!(outcome.changed);
    assert!(outcome.index_refreshed);
    assert_eq!(
        fs::read_to_string(workspace.path().join(NOTE)).unwrap(),
        proposed
    );
    let database = IndexDatabase::open(index_path(workspace.path())).unwrap();
    assert_eq!(
        database
            .search(&SearchQuery::new("Searchable"))
            .unwrap()
            .len(),
        1
    );
    assert!(
        workspace
            .path()
            .join(".secondbrain/oplog")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
}

#[test]
fn proposed_hash_is_an_apply_precondition() {
    let workspace = workspace(BASE);
    let mut preview = preview_transaction(
        workspace.path(),
        NOTE,
        "# Alpha\n\nProposed.\n",
        actor(),
        device(),
    )
    .unwrap();
    preview.proposed_source_hash = ContentHash::digest(b"tampered");

    assert!(matches!(
        apply_preview(workspace.path(), &preview),
        Err(TransactionPreviewError::PreviewModified {
            field: "proposed_source_hash"
        })
    ));
}

#[test]
fn typescript_fixture_matches_the_versioned_rust_shape() {
    let fixture = include_str!("../../../docs/fixtures/transaction-preview-v1.json");
    let preview: TransactionPreview = serde_json::from_str(fixture).unwrap();
    assert_eq!(preview.format, TRANSACTION_PREVIEW_FORMAT_V1);
    assert_eq!(preview.summary, "no semantic change");

    let outcome_fixture = include_str!("../../../docs/fixtures/transaction-apply-outcome-v1.json");
    let outcome: ApplyPreviewOutcome = serde_json::from_str(outcome_fixture).unwrap();
    assert!(outcome.changed);
    assert!(outcome.index_refreshed);
}
