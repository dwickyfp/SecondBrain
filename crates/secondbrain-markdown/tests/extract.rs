//! Extraction tests: wikilinks, tags, headings, tasks, and YAML properties.
//!
//! These tests verify that `extract()` correctly identifies and extracts
//! Obsidian-style wikilinks, hashtag tags, headings, GFM task list items,
//! and YAML frontmatter properties from a parsed Markdown document.

use secondbrain_markdown::SourceDocument;
use secondbrain_markdown::extract::{ExtractedNote, ExtractedTask, PropertyValue};

/// Helper: read a fixture file from `fixtures/markdown/extract/`.
fn read_fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
        .join("extract")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

/// Helper: parse a fixture and extract.
fn extract_fixture(name: &str) -> ExtractedNote {
    let source = read_fixture(name);
    let doc = SourceDocument::parse(&source).expect("parse must succeed");
    secondbrain_markdown::extract::extract(&doc)
}

// ---------------------------------------------------------------------------
// Wikilinks
// ---------------------------------------------------------------------------

#[test]
fn extract_simple_wikilink() {
    let note = extract_fixture("wikilinks.md");
    let simple = note
        .links
        .iter()
        .find(|l| l.target == "Note" && l.alias.is_none() && l.heading.is_none())
        .expect("should find simple [[Note]] link");
    assert!(!simple.embed, "non-embed link should have embed=false");
    // Span should point somewhere in the source.
    assert!(simple.span.start < simple.span.end);
}

#[test]
fn extract_aliased_wikilink() {
    let note = extract_fixture("wikilinks.md");
    let aliased = note
        .links
        .iter()
        .find(|l| l.target == "Note" && l.alias.as_deref() == Some("Alias"))
        .expect("should find aliased [[Note|Alias]] link");
    assert!(!aliased.embed);
}

#[test]
fn extract_heading_wikilink() {
    let note = extract_fixture("wikilinks.md");
    let heading_link = note
        .links
        .iter()
        .find(|l| l.target == "Note" && l.heading.as_deref() == Some("Section"))
        .expect("should find heading link [[Note#Section]]");
    assert!(!heading_link.embed);
}

#[test]
fn extract_embed_wikilink() {
    let note = extract_fixture("wikilinks.md");
    let embed = note
        .links
        .iter()
        .find(|l| l.target == "image.png")
        .expect("should find embed ![[image.png]]");
    assert!(embed.embed, "embed link should have embed=true");
}

#[test]
fn wikilinks_ordered_by_span() {
    let note = extract_fixture("wikilinks.md");
    // Links should be in source order.
    for window in note.links.windows(2) {
        assert!(
            window[0].span.start <= window[1].span.start,
            "links should be ordered by span start: {:?} before {:?}",
            window[0],
            window[1]
        );
    }
}

// ---------------------------------------------------------------------------
// Ignored wikilinks (in code, escaped)
// ---------------------------------------------------------------------------

#[test]
fn ignore_wikilinks_in_inline_code() {
    let note = extract_fixture("ignored.md");
    assert!(
        !note.links.iter().any(|l| l.target == "NotALink"),
        "wikilinks in inline code should be ignored"
    );
}

#[test]
fn ignore_wikilinks_in_code_block() {
    let note = extract_fixture("ignored.md");
    assert!(
        !note.links.iter().any(|l| l.target == "NotACodeLink"),
        "wikilinks in code blocks should be ignored"
    );
}

// ---------------------------------------------------------------------------
// Unicode tags and dedup
// ---------------------------------------------------------------------------

#[test]
fn extract_unicode_tags() {
    let note = extract_fixture("unicode_tags.md");
    let tag_texts: Vec<&str> = note.tags.iter().map(|t| t.text.as_str()).collect();
    assert!(
        tag_texts.contains(&"café"),
        "should extract #café tag, got: {tag_texts:?}"
    );
    assert!(
        tag_texts.contains(&"日本語"),
        "should extract #日本語 tag, got: {tag_texts:?}"
    );
}

#[test]
fn duplicate_tags_deduplicated() {
    let note = extract_fixture("unicode_tags.md");
    let café_count = note.tags.iter().filter(|t| t.text == "café").count();
    assert_eq!(
        café_count, 1,
        "duplicate #café should be deduplicated to 1 occurrence"
    );
}

#[test]
fn tags_preserve_first_occurrence_span() {
    let note = extract_fixture("unicode_tags.md");
    let café = note
        .tags
        .iter()
        .find(|t| t.text == "café")
        .expect("should find café tag");
    // The first occurrence of #café is on line 3.
    let source = read_fixture("unicode_tags.md");
    let span_text = &source[café.span.start..café.span.end];
    assert!(
        span_text.contains("café"),
        "span should point at first occurrence, got: {span_text:?}"
    );
}

// ---------------------------------------------------------------------------
// Headings
// ---------------------------------------------------------------------------

#[test]
fn extract_heading_levels_and_text() {
    let note = extract_fixture("headings.md");
    let heading_texts: Vec<&str> = note.headings.iter().map(|h| h.text.as_str()).collect();
    assert!(heading_texts.contains(&"Headings"), "should find H1");
    assert!(
        heading_texts.contains(&"First Section"),
        "should find H2 First Section"
    );
    assert!(
        heading_texts.contains(&"Subsection"),
        "should find H3 Subsection"
    );
    assert!(
        heading_texts.contains(&"Second Section"),
        "should find H2 Second Section"
    );
}

#[test]
fn heading_levels_correct() {
    let note = extract_fixture("headings.md");
    let h1 = note
        .headings
        .iter()
        .find(|h| h.text == "Headings")
        .expect("should find H1");
    assert_eq!(h1.level, 1, "Headings should be level 1");

    let h2 = note
        .headings
        .iter()
        .find(|h| h.text == "First Section")
        .expect("should find H2");
    assert_eq!(h2.level, 2, "First Section should be level 2");

    let h3 = note
        .headings
        .iter()
        .find(|h| h.text == "Subsection")
        .expect("should find H3");
    assert_eq!(h3.level, 3, "Subsection should be level 3");
}

#[test]
fn headings_in_source_order() {
    let note = extract_fixture("headings.md");
    let texts: Vec<&str> = note.headings.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["Headings", "First Section", "Subsection", "Second Section"],
        "headings should be in source order"
    );
}

#[test]
fn heading_spans_valid() {
    let note = extract_fixture("headings.md");
    for h in &note.headings {
        assert!(
            h.span.start < h.span.end,
            "heading span should be non-empty: {:?}",
            h
        );
    }
}

// ---------------------------------------------------------------------------
// Task markers
// ---------------------------------------------------------------------------

#[test]
fn extract_task_markers() {
    let note = extract_fixture("tasks.md");
    let unchecked: Vec<&ExtractedTask> = note.tasks.iter().filter(|t| !t.checked).collect();
    let checked: Vec<&ExtractedTask> = note.tasks.iter().filter(|t| t.checked).collect();

    assert_eq!(
        unchecked.len(),
        2,
        "should have 2 unchecked tasks, got: {unchecked:?}"
    );
    assert_eq!(
        checked.len(),
        1,
        "should have 1 checked task, got: {checked:?}"
    );
}

#[test]
fn task_text_excludes_marker() {
    let note = extract_fixture("tasks.md");
    let checked_task = note
        .tasks
        .iter()
        .find(|t| t.checked)
        .expect("should find checked task");
    assert!(
        checked_task.text.contains("Checked task"),
        "task text should contain 'Checked task', got: {:?}",
        checked_task.text
    );
    assert!(
        !checked_task.text.contains("[x]"),
        "task text should not contain '[x]' marker"
    );
}

#[test]
fn non_task_list_items_not_collected() {
    let note = extract_fixture("tasks.md");
    // "Normal list item" and "Not a task" should not appear in tasks.
    assert!(
        !note
            .tasks
            .iter()
            .any(|t| t.text.contains("Normal list item")),
        "non-task list items should not be collected as tasks"
    );
    assert!(
        !note.tasks.iter().any(|t| t.text.contains("Not a task")),
        "non-task list items should not be collected as tasks"
    );
}

// ---------------------------------------------------------------------------
// YAML properties
// ---------------------------------------------------------------------------

#[test]
fn extract_string_property() {
    let note = extract_fixture("properties.md");
    let title = note
        .properties
        .iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v);
    assert_eq!(
        title,
        Some(&PropertyValue::Str("Properties Test".to_string())),
        "title should be extracted as string"
    );
}

#[test]
fn extract_list_property() {
    let note = extract_fixture("properties.md");
    let tags = note
        .properties
        .iter()
        .find(|(k, _)| k == "tags")
        .map(|(_, v)| v);
    match tags {
        Some(PropertyValue::List(items)) => {
            assert!(
                items.contains(&"foo".to_string()),
                "tags should contain 'foo': {items:?}"
            );
            assert!(
                items.contains(&"bar".to_string()),
                "tags should contain 'bar': {items:?}"
            );
        }
        other => panic!("tags should be a List, got: {other:?}"),
    }
}

#[test]
fn extract_int_property() {
    let note = extract_fixture("properties.md");
    let priority = note
        .properties
        .iter()
        .find(|(k, _)| k == "priority")
        .map(|(_, v)| v);
    assert_eq!(
        priority,
        Some(&PropertyValue::Int(5)),
        "priority should be extracted as int"
    );
}

#[test]
fn extract_bool_property() {
    let note = extract_fixture("properties.md");
    let published = note
        .properties
        .iter()
        .find(|(k, _)| k == "published")
        .map(|(_, v)| v);
    assert_eq!(
        published,
        Some(&PropertyValue::Bool(true)),
        "published should be extracted as bool"
    );
}

#[test]
fn empty_note_has_no_extractions() {
    let note = extract_fixture("empty.md");
    assert!(note.links.is_empty(), "empty note should have no links");
    assert!(note.tags.is_empty(), "empty note should have no tags");
    assert!(note.tasks.is_empty(), "empty note should have no tasks");
    // Empty note has one heading ("# Empty Note") but no properties.
    assert_eq!(note.headings.len(), 1);
    assert!(
        note.properties.is_empty(),
        "empty note should have no properties"
    );
}

// ---------------------------------------------------------------------------
// Combined fixture
// ---------------------------------------------------------------------------

#[test]
fn combined_extracts_all_types() {
    let note = extract_fixture("combined.md");
    // Links: [[Other Note]], [[Link]], [[Related]]
    assert_eq!(note.links.len(), 3, "should find 3 links in combined.md");
    assert!(
        note.links.iter().any(|l| l.target == "Other Note"),
        "should find Other Note link"
    );
    assert!(
        note.links.iter().any(|l| l.target == "Link"),
        "should find Link link"
    );
    assert!(
        note.links.iter().any(|l| l.target == "Related"),
        "should find Related link"
    );

    // Tags: #tag, #inline_tag
    let tag_texts: Vec<&str> = note.tags.iter().map(|t| t.text.as_str()).collect();
    assert!(tag_texts.contains(&"tag"), "should find #tag");
    assert!(tag_texts.contains(&"inline_tag"), "should find #inline_tag");

    // Headings: Combined, Section
    let heading_texts: Vec<&str> = note.headings.iter().map(|h| h.text.as_str()).collect();
    assert!(
        heading_texts.contains(&"Combined"),
        "should find Combined heading"
    );
    assert!(
        heading_texts.contains(&"Section"),
        "should find Section heading"
    );

    // Tasks: one unchecked, one checked
    assert_eq!(note.tasks.len(), 2, "should find 2 tasks");
    assert_eq!(
        note.tasks.iter().filter(|t| !t.checked).count(),
        1,
        "should have 1 unchecked task"
    );
    assert_eq!(
        note.tasks.iter().filter(|t| t.checked).count(),
        1,
        "should have 1 checked task"
    );

    // Properties: title, tags
    assert!(
        note.properties.iter().any(|(k, _)| k == "title"),
        "should have title property"
    );
    assert!(
        note.properties.iter().any(|(k, _)| k == "tags"),
        "should have tags property"
    );
}

// ---------------------------------------------------------------------------
// Plain text
// ---------------------------------------------------------------------------

#[test]
fn plain_text_contains_body_content() {
    let note = extract_fixture("empty.md");
    assert!(
        note.plain_text.contains("Empty Note"),
        "plain_text should contain heading text"
    );
    assert!(
        note.plain_text
            .contains("No links, tags, tasks, or properties here."),
        "plain_text should contain paragraph text"
    );
}
