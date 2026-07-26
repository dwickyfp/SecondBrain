use std::fs;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_index::{IndexConfig, rebuild};
use secondbrain_markdown::PropertyEdit;
use secondbrain_transaction::{apply_property_preview, preview_property, read_note_properties};
use secondbrain_vault::initialize_workspace;
use serde_json::json;
use tempfile::tempdir;

fn identities() -> (ActorId, DeviceId) {
    (
        ActorId::new("property-test").unwrap(),
        DeviceId::new("local").unwrap(),
    )
}

#[test]
fn real_workspace_property_preview_apply_noop_and_stale_behave_like_transactions() {
    let workspace = tempdir().unwrap();
    initialize_workspace(workspace.path()).unwrap();
    let path = "note.md";
    let source = "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\ntitle: Old # keep elsewhere\n---\n# Body\n";
    fs::write(workspace.path().join(path), source).unwrap();
    rebuild(workspace.path(), &IndexConfig::default()).unwrap();
    let (actor, device) = identities();

    let preview = preview_property(
        workspace.path(),
        path,
        PropertyEdit::Set {
            key: "title".into(),
            value: json!("New"),
        },
        actor.clone(),
        device.clone(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join(path)).unwrap(),
        source
    );
    assert_eq!(preview.properties["title"], json!("New"));
    let outcome = apply_property_preview(workspace.path(), &preview).unwrap();
    assert!(outcome.changed);
    assert_eq!(
        read_note_properties(workspace.path(), path).unwrap()["title"],
        json!("New")
    );

    let noop = preview_property(
        workspace.path(),
        path,
        PropertyEdit::Set {
            key: "title".into(),
            value: json!("New"),
        },
        actor.clone(),
        device.clone(),
    )
    .unwrap();
    assert!(noop.transaction.operations.is_empty());
    assert!(
        !apply_property_preview(workspace.path(), &noop)
            .unwrap()
            .changed
    );

    let stale = preview_property(
        workspace.path(),
        path,
        PropertyEdit::Set {
            key: "title".into(),
            value: json!("Later"),
        },
        actor,
        device,
    )
    .unwrap();
    fs::write(workspace.path().join(path), "external\n").unwrap();
    assert!(
        apply_property_preview(workspace.path(), &stale)
            .unwrap_err()
            .to_string()
            .contains("stale note hash")
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join(path)).unwrap(),
        "external\n"
    );
}

#[test]
fn property_preview_rejects_tampered_intent_and_confines_paths() {
    let workspace = tempdir().unwrap();
    initialize_workspace(workspace.path()).unwrap();
    fs::write(workspace.path().join("note.md"), "# Body\n").unwrap();
    rebuild(workspace.path(), &IndexConfig::default()).unwrap();
    let (actor, device) = identities();
    assert!(
        preview_property(
            workspace.path(),
            "../outside.md",
            PropertyEdit::Set {
                key: "safe".into(),
                value: json!(true)
            },
            actor.clone(),
            device.clone(),
        )
        .is_err()
    );
    let mut preview = preview_property(
        workspace.path(),
        "note.md",
        PropertyEdit::Set {
            key: "safe".into(),
            value: json!(true),
        },
        actor,
        device,
    )
    .unwrap();
    preview.edit = PropertyEdit::Set {
        key: "safe".into(),
        value: json!(false),
    };
    assert!(apply_property_preview(workspace.path(), &preview).is_err());
    assert_eq!(
        fs::read_to_string(workspace.path().join("note.md")).unwrap(),
        "# Body\n"
    );
}
