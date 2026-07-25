//! YAML frontmatter parse-and-patch tests.
//!
//! These tests verify that note metadata (YAML frontmatter) can be parsed
//! and surgically updated while preserving all body bytes and unrelated
//! frontmatter formatting.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use secondbrain_core::id::NoteId;
use secondbrain_markdown::{ensure_note_id, parse_metadata};

/// Helper: read a fixture file as UTF-8 string.
fn read_fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
        .join("frontmatter")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

/// Helper: read a fixture file as raw bytes.
fn read_fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
        .join("frontmatter")
        .join(name);
    fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

const CANONICAL_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

// ===========================================================================
// Test 1: no frontmatter — parse returns empty metadata
// ===========================================================================

#[test]
fn parse_no_frontmatter_returns_empty_metadata() {
    let source = read_fixture("no_frontmatter.md");
    let metadata = parse_metadata(&source).expect("parse should succeed");

    assert!(metadata.id.is_none(), "no id expected");
    assert!(metadata.title.is_none(), "no title expected");
    assert!(metadata.properties.is_empty(), "no properties expected");
}

// ===========================================================================
// Test 2: valid YAML frontmatter — id, title, and properties parsed
// ===========================================================================

#[test]
fn parse_valid_yaml_extracts_title_and_properties() {
    let source = read_fixture("valid_yaml.md");
    let metadata = parse_metadata(&source).expect("parse should succeed");

    assert!(metadata.id.is_none(), "no id in this fixture");
    assert_eq!(
        metadata.title.as_deref(),
        Some("My Valid Note"),
        "title should be extracted"
    );

    // The tags property should be a sequence.
    let tags_value = metadata
        .properties
        .get(serde_yaml::Value::String("tags".to_string()))
        .expect("tags property should exist");
    assert!(tags_value.is_sequence(), "tags should be a YAML sequence");
}

// ===========================================================================
// Test 3: UTF-8 BOM — BOM is stripped before parsing frontmatter
// ===========================================================================

#[test]
fn parse_strips_utf8_bom_before_frontmatter() {
    let source = read_fixture("bom_prefix.md");
    // Verify the source actually starts with a BOM.
    assert!(
        source.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]),
        "fixture should start with UTF-8 BOM"
    );

    let metadata = parse_metadata(&source).expect("parse should succeed with BOM");
    assert_eq!(
        metadata.title.as_deref(),
        Some("BOM Note"),
        "title should be extracted despite BOM"
    );
}

// ===========================================================================
// Test 4: comments and quoted values — parsed correctly
// ===========================================================================

#[test]
fn parse_comments_and_quoted_values() {
    let source = read_fixture("comments_quoted.md");
    let metadata = parse_metadata(&source).expect("parse should succeed");

    assert_eq!(
        metadata.title.as_deref(),
        Some("Quoted Title"),
        "double-quoted title should be extracted"
    );

    let author = metadata
        .properties
        .get(serde_yaml::Value::String("author".to_string()))
        .expect("author property should exist");
    assert_eq!(
        author.as_str(),
        Some("Single Quoted"),
        "single-quoted value should be extracted"
    );
}

// ===========================================================================
// Test 5: malformed YAML — returns an error diagnostic
// ===========================================================================

#[test]
fn parse_malformed_yaml_returns_error() {
    let source = read_fixture("malformed.md");
    let result = parse_metadata(&source);
    assert!(result.is_err(), "malformed YAML must produce an error");
}

// ===========================================================================
// Test 6: inserting id leaves body byte-identical
// ===========================================================================

#[test]
fn inserting_id_preserves_body_bytes() {
    let source = read_fixture("valid_yaml.md");
    let original_bytes = source.as_bytes();

    let generated = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");
    let patch = ensure_note_id(&source, generated).expect("ensure_note_id should succeed");

    assert!(patch.changed, "patch should report a change");
    assert_eq!(patch.note_id, generated);

    // The patched source must contain the id line.
    assert!(
        patch.source.contains("id:"),
        "patched source should contain id field"
    );
    assert!(
        patch.source.contains(CANONICAL_ID),
        "patched source should contain the generated id value"
    );

    // The body after the frontmatter must be byte-identical.
    // Extract body from original: everything after the closing "---\n".
    let original_body = extract_body(&source);
    let patched_body = extract_body(&patch.source);
    assert_eq!(
        original_body.as_bytes(),
        patched_body.as_bytes(),
        "body must be byte-identical after inserting id"
    );

    // The title and other frontmatter should still be present.
    assert!(
        patch.source.contains("title: My Valid Note"),
        "existing frontmatter must be preserved"
    );
    assert!(
        patch.source.contains("tags:"),
        "existing frontmatter tags must be preserved"
    );

    // Original source should NOT be modified (ensure_note_id takes &str).
    assert_eq!(
        original_bytes,
        source.as_bytes(),
        "original source must not be mutated"
    );
}

// ===========================================================================
// Test 7: existing canonical id produces no write (changed=false)
// ===========================================================================

#[test]
fn existing_canonical_id_produces_no_write() {
    let source = read_fixture("existing_id.md");
    let existing = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");

    let patch = ensure_note_id(&source, existing).expect("ensure_note_id should succeed");

    assert!(
        !patch.changed,
        "existing canonical id should produce no change"
    );
    assert_eq!(patch.note_id, existing);
    assert_eq!(
        patch.source.as_bytes(),
        source.as_bytes(),
        "source must be byte-identical when no change"
    );
}

// ===========================================================================
// Test 8: duplicate id keys are rejected
// ===========================================================================

#[test]
fn duplicate_id_keys_are_rejected() {
    let source = read_fixture("duplicate_id.md");
    let result = parse_metadata(&source);
    assert!(result.is_err(), "duplicate id keys must be rejected");
}

// ===========================================================================
// Test 9: inserting id when no frontmatter exists — prepends minimal block
// ===========================================================================

#[test]
fn inserting_id_without_frontmatter_prepends_block() {
    let source = read_fixture("no_frontmatter.md");
    let original_body = source.as_str();

    let generated = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");
    let patch = ensure_note_id(&source, generated).expect("ensure_note_id should succeed");

    assert!(patch.changed, "should report a change");

    // The patched source should start with a frontmatter block.
    assert!(
        patch.source.starts_with("---\n"),
        "patched source should start with frontmatter delimiter"
    );

    // The id should be in the frontmatter.
    assert!(
        patch.source.contains(&format!("id: {CANONICAL_ID}")),
        "patched source should contain the id"
    );

    // The original body (everything after the prepended block) must be preserved.
    // The body is the original source, which should appear after the frontmatter block.
    let body_start = patch
        .source
        .find("---\n")
        .and_then(|start| {
            patch.source[start + 4..]
                .find("---\n")
                .map(|end| start + 4 + end + 4)
        })
        .expect("should find end of frontmatter block");
    let patched_body = &patch.source[body_start..];
    assert_eq!(
        patched_body.as_bytes(),
        original_body.as_bytes(),
        "body must be byte-identical when prepending frontmatter"
    );
}

// ===========================================================================
// Test 10: existing id in frontmatter with other properties — surgical patch
// ===========================================================================

#[test]
fn ensure_id_with_existing_frontmatter_is_surgical() {
    let source = read_fixture("comments_quoted.md");
    let original = source.clone();

    let generated = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");
    let patch = ensure_note_id(&source, generated).expect("ensure_note_id should succeed");

    assert!(patch.changed, "should report a change");

    // Comments and quoted values must be preserved.
    assert!(
        patch.source.contains("# This is a comment"),
        "comments must be preserved"
    );
    assert!(
        patch.source.contains("\"Quoted Title\""),
        "quoted values must be preserved"
    );
    assert!(
        patch.source.contains("'Single Quoted'"),
        "single-quoted values must be preserved"
    );

    // Body must be identical.
    let original_body = extract_body(&original);
    let patched_body = extract_body(&patch.source);
    assert_eq!(
        original_body.as_bytes(),
        patched_body.as_bytes(),
        "body must be byte-identical"
    );
}

// ===========================================================================
// Test 11: CRLF frontmatter — body preserved, id inserted with LF in frontmatter
// ===========================================================================

#[test]
fn ensure_id_with_crlf_frontmatter_preserves_body() {
    let source = read_fixture("crlf_frontmatter.md");
    assert!(source.contains("\r\n"), "fixture should use CRLF");

    let generated = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");
    let patch = ensure_note_id(&source, generated).expect("ensure_note_id should succeed");

    assert!(patch.changed, "should report a change");

    // Body after frontmatter must be byte-identical.
    let original_body = extract_body(&source);
    let patched_body = extract_body(&patch.source);
    assert_eq!(
        original_body.as_bytes(),
        patched_body.as_bytes(),
        "CRLF body must be byte-identical"
    );
}

// ===========================================================================
// Test 12: body with thematic break (---) is not confused with frontmatter
// ===========================================================================

#[test]
fn body_with_thematic_break_not_confused() {
    let source = read_fixture("body_with_dashes.md");
    let metadata = parse_metadata(&source).expect("parse should succeed");

    assert_eq!(
        metadata.title.as_deref(),
        Some("Dash Body"),
        "title from frontmatter should be extracted"
    );

    let generated = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");
    let patch = ensure_note_id(&source, generated).expect("ensure_note_id should succeed");

    assert!(patch.changed, "should report a change");

    // Body (including the thematic break) must be preserved.
    let original_body = extract_body(&source);
    let patched_body = extract_body(&patch.source);
    assert_eq!(
        original_body.as_bytes(),
        patched_body.as_bytes(),
        "body with thematic break must be preserved"
    );
}

// ===========================================================================
// Test 13: parse_metadata extracts existing canonical id
// ===========================================================================

#[test]
fn parse_metadata_extracts_existing_id() {
    let source = read_fixture("existing_id.md");
    let metadata = parse_metadata(&source).expect("parse should succeed");

    let id = metadata.id.expect("should have an id");
    assert_eq!(
        id.to_string(),
        CANONICAL_ID,
        "id should match the canonical form"
    );
}

// ===========================================================================
// Test 14: parse_metadata with no frontmatter returns empty properties
// ===========================================================================

#[test]
fn parse_metadata_empty_source() {
    let metadata = parse_metadata("").expect("empty source should parse");
    assert!(metadata.id.is_none());
    assert!(metadata.title.is_none());
    assert!(metadata.properties.is_empty());
}

// ===========================================================================
// Test 15: ensure_note_id on empty source prepends frontmatter
// ===========================================================================

#[test]
fn ensure_note_id_empty_source() {
    let generated = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");
    let patch = ensure_note_id("", generated).expect("ensure_note_id should succeed");

    assert!(patch.changed);
    assert!(
        patch.source.starts_with("---\n"),
        "should start with frontmatter"
    );
    assert!(
        patch.source.contains(&format!("id: {CANONICAL_ID}")),
        "should contain the id"
    );
}

// ===========================================================================
// Test 16: BOM source — ensure_note_id preserves BOM and body
// ===========================================================================

#[test]
fn ensure_note_id_preserves_bom_source() {
    let bytes = read_fixture_bytes("bom_prefix.md");
    let source = String::from_utf8(bytes).expect("valid UTF-8");

    let generated = NoteId::from_str(CANONICAL_ID).expect("valid NoteId");
    let patch = ensure_note_id(&source, generated).expect("ensure_note_id should succeed");

    assert!(patch.changed);

    // BOM should be preserved at the start.
    assert!(
        patch.source.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]),
        "BOM must be preserved"
    );

    // Body must be identical.
    let original_body = extract_body(&source);
    let patched_body = extract_body(&patch.source);
    assert_eq!(
        original_body.as_bytes(),
        patched_body.as_bytes(),
        "body must be byte-identical with BOM source"
    );
}

// ---------------------------------------------------------------------------
// Helper: extract the body portion (everything after frontmatter).
// If no frontmatter, returns the entire source.
// ---------------------------------------------------------------------------

fn extract_body(source: &str) -> &str {
    // Strip BOM if present.
    let s = source.strip_prefix('\u{FEFF}').unwrap_or(source);

    // A valid frontmatter block starts at the beginning with "---\n" (or "---\r\n").
    let lines: Vec<&str> = s.lines().collect();
    if lines.is_empty() || lines[0].trim_end_matches('\r') != "---" {
        return source;
    }

    // Find the closing "---" line.
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim_end_matches('\r') == "---" {
            // Body starts after this line.
            // We need to find the byte offset of the character after this line.
            let body_start = find_byte_after_line(s, i + 1);
            return &s[body_start..];
        }
    }

    // No closing delimiter — not valid frontmatter, return entire source.
    source
}

/// Find the byte offset of the start of the (1-indexed) `line_num`-th line.
fn find_byte_after_line(source: &str, line_num: usize) -> usize {
    let mut offset = 0usize;
    let mut current_line = 1usize;

    for (i, byte) in source.bytes().enumerate() {
        if current_line == line_num {
            return i;
        }
        if byte == b'\n' {
            current_line += 1;
            offset = i + 1;
        }
    }

    // If we're looking for a line past the end, return the source length.
    if current_line < line_num {
        source.len()
    } else {
        offset
    }
}
