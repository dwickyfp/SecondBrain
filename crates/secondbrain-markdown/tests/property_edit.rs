use secondbrain_markdown::{PropertyEdit, edit_property, read_properties};
use serde_json::json;

#[test]
fn typed_properties_are_read_and_id_is_reserved() {
    let source = "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\nenabled: true\ncount: 3\ntags: [one, two]\nnested: {owner: Ada}\n---\nBody\n";
    let properties = read_properties(source).unwrap();
    assert_eq!(properties["enabled"], json!(true));
    assert_eq!(properties["count"], json!(3));
    assert_eq!(properties["tags"], json!(["one", "two"]));
    assert_eq!(properties["nested"], json!({"owner": "Ada"}));
    assert!(!properties.contains_key("id"));
    assert!(edit_property(source, &PropertyEdit::Remove { key: "id".into() }).is_err());
}

#[test]
fn set_and_remove_are_surgical_idempotent_and_preserve_bom_crlf_and_body() {
    let source = "\u{feff}---\r\n# keep this\r\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\r\ntitle: Old # target\r\nnested:\r\n  owner: Ada\r\n# unrelated\r\n---\r\nBody\r\n---\r\n";
    let set = PropertyEdit::Set {
        key: "title".into(),
        value: json!("New"),
    };
    let patched = edit_property(source, &set).unwrap();
    assert!(patched.changed);
    assert!(patched.source.starts_with("\u{feff}---\r\n# keep this\r\n"));
    assert!(
        patched
            .source
            .contains("title: New\r\nnested:\r\n  owner: Ada\r\n# unrelated\r\n")
    );
    assert!(patched.source.ends_with("---\r\nBody\r\n---\r\n"));
    assert!(!edit_property(&patched.source, &set).unwrap().changed);

    let removed = edit_property(
        &patched.source,
        &PropertyEdit::Remove {
            key: "nested".into(),
        },
    )
    .unwrap();
    assert!(!removed.source.contains("nested:"));
    assert!(!removed.source.contains("  owner: Ada"));
    assert!(removed.source.contains("# unrelated\r\n---\r\nBody"));
}

#[test]
fn absent_frontmatter_is_added_without_moving_bom_or_body() {
    let source = "\u{feff}Body without final newline";
    let patch = edit_property(
        source,
        &PropertyEdit::Set {
            key: "rating".into(),
            value: json!(4.5),
        },
    )
    .unwrap();
    assert_eq!(
        patch.source,
        "\u{feff}---\nrating: 4.5\n---\n\nBody without final newline"
    );
}

#[test]
fn malformed_or_unsafe_frontmatter_fails_closed() {
    for source in [
        "---\ntitle: [broken\nBody\n",
        "---\n'quoted key': value\n---\nBody\n",
        "---\n? complex\n: value\n---\nBody\n",
    ] {
        assert!(
            edit_property(
                source,
                &PropertyEdit::Set {
                    key: "title".into(),
                    value: json!("safe")
                }
            )
            .is_err()
        );
    }
}
