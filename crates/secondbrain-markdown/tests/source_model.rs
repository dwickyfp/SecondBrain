//! Source preservation tests for the loss-aware Markdown source model.
//!
//! These tests verify that parsing Markdown into positioned semantic nodes
//! retains exact source slices for unchanged regions, including UTF-8
//! multibyte content and both LF/CRLF line endings.

use std::fs;
use std::path::Path;

use secondbrain_markdown::{LineEnding, SemanticKind, SourceDocument, SourceSpan};

/// Helper: read a fixture file as UTF-8 string.
fn read_fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
        .join("source-preservation")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

// ---------------------------------------------------------------------------
// RED test 1: exact UTF-8 byte ranges
// ---------------------------------------------------------------------------

#[test]
fn parse_tracks_exact_utf8_byte_ranges() {
    let source = "# Héllo\r\n\r\nBody\r\n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");

    // The complete original source is retained.
    assert_eq!(doc.source(), source);

    // The first node (heading) should cover exactly "# Héllo" in bytes.
    // `é` is two bytes in UTF-8, so "# Héllo" = 7 bytes.
    let nodes = doc.nodes();
    assert!(!nodes.is_empty(), "document must contain at least one node");
    let heading = &nodes[0];
    assert_eq!(
        heading.kind,
        SemanticKind::Heading,
        "first node should be a heading"
    );
    let raw = heading.raw(source);
    assert_eq!(
        raw, "# Héllo",
        "heading raw slice must be exactly the source text"
    );

    // Byte span: 0..8 (8 bytes for "# Héllo" — é is 2 bytes in UTF-8)
    assert_eq!(
        heading.span,
        SourceSpan { start: 0, end: 8 },
        "heading byte span must match UTF-8 byte offsets"
    );
}

// ---------------------------------------------------------------------------
// RED test 2: untouched document round-trips byte-identically
// ---------------------------------------------------------------------------

#[test]
fn untouched_document_serializes_byte_identically() {
    let source = read_fixture("mixed.md");
    let doc = SourceDocument::parse(&source).expect("parse must succeed");

    // Reconstruct the document from source slices.
    let reconstructed = doc.reconstruct();
    assert_eq!(
        reconstructed.as_bytes(),
        source.as_bytes(),
        "untouched document must serialize byte-identically"
    );
}

// ---------------------------------------------------------------------------
// RED test 3: LF line endings preserved
// ---------------------------------------------------------------------------

#[test]
fn lf_line_endings_preserved() {
    let source = read_fixture("lf.md");
    assert!(
        source.contains('\n') && !source.contains("\r\n"),
        "fixture should use LF only"
    );

    let doc = SourceDocument::parse(&source).expect("parse must succeed");
    assert_eq!(
        doc.line_ending(),
        LineEnding::Lf,
        "LF document should detect LineEnding::Lf"
    );

    let reconstructed = doc.reconstruct();
    assert_eq!(
        reconstructed.as_bytes(),
        source.as_bytes(),
        "LF document must round-trip exactly"
    );
}

// ---------------------------------------------------------------------------
// RED test 4: CRLF line endings preserved
// ---------------------------------------------------------------------------

#[test]
fn crlf_line_endings_preserved() {
    let source = read_fixture("crlf.md");
    assert!(source.contains("\r\n"), "fixture should use CRLF");

    let doc = SourceDocument::parse(&source).expect("parse must succeed");
    assert_eq!(
        doc.line_ending(),
        LineEnding::Crlf,
        "CRLF document should detect LineEnding::Crlf"
    );

    let reconstructed = doc.reconstruct();
    assert_eq!(
        reconstructed.as_bytes(),
        source.as_bytes(),
        "CRLF document must round-trip exactly"
    );
}

// ---------------------------------------------------------------------------
// RED test 5: unsupported/HTML constructs preserved with exact bytes
// ---------------------------------------------------------------------------

#[test]
fn unsupported_constructs_become_raw_spans() {
    // HTML blocks are recognized by markdown-rs but classified as Html in
    // our model — they preserve exact bytes as opaque source slices.
    let source = "<div>\nSome content\n</div>\n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");

    let nodes = doc.nodes();
    let html_nodes: Vec<_> = nodes
        .iter()
        .filter(|n| n.kind == SemanticKind::Html || n.kind == SemanticKind::Raw)
        .collect();
    assert!(
        !html_nodes.is_empty(),
        "HTML block should produce at least one Html/Raw node"
    );

    // Every node's slice must be a substring of the source.
    for node in &html_nodes {
        let slice = node.raw(source);
        assert!(
            source.contains(slice),
            "node slice must be exact substring of source"
        );
    }

    // Full reconstruction must be byte-identical.
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());
}

// ---------------------------------------------------------------------------
// RED test 6: empty document
// ---------------------------------------------------------------------------

#[test]
fn empty_document_round_trips() {
    let source = "";
    let doc = SourceDocument::parse(source).expect("empty parse must succeed");
    assert_eq!(doc.source(), source);
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());
    assert_eq!(doc.line_ending(), LineEnding::Lf, "empty defaults to LF");
}

// ---------------------------------------------------------------------------
// RED test 7: document with only whitespace
// ---------------------------------------------------------------------------

#[test]
fn whitespace_only_round_trips() {
    let source = "   \n  \n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());
}

// ---------------------------------------------------------------------------
// RED test 8: heading span covers full heading line
// ---------------------------------------------------------------------------

#[test]
fn heading_span_covers_full_line() {
    let source = "# Title\n\nParagraph.\n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");

    let nodes = doc.nodes();
    let heading = nodes
        .iter()
        .find(|n| n.kind == SemanticKind::Heading)
        .expect("should have a heading");
    assert_eq!(heading.raw(source), "# Title");
}

// ---------------------------------------------------------------------------
// RED test 9: paragraph span covers full paragraph text
// ---------------------------------------------------------------------------

#[test]
fn paragraph_span_covers_text() {
    let source = "Hello world.\n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");

    let nodes = doc.nodes();
    let para = nodes
        .iter()
        .find(|n| n.kind == SemanticKind::Paragraph)
        .expect("should have a paragraph");
    assert_eq!(para.raw(source), "Hello world.");
}

// ---------------------------------------------------------------------------
// RED test 10: code fence span covers entire fenced block
// ---------------------------------------------------------------------------

#[test]
fn code_fence_span_covers_block() {
    let source = "```rust\nfn main() {}\n```\n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");

    let nodes = doc.nodes();
    let code = nodes
        .iter()
        .find(|n| n.kind == SemanticKind::Code)
        .expect("should have a code block");
    let raw = code.raw(source);
    assert!(raw.starts_with("```"), "code span should start with fence");
    assert!(raw.ends_with("```"), "code span should end with fence");
}

// ---------------------------------------------------------------------------
// RED test 11: frontmatter span is preserved
// ---------------------------------------------------------------------------

#[test]
fn frontmatter_span_preserved() {
    let source = "---\ntitle: Test\n---\n\nBody.\n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");

    let nodes = doc.nodes();
    let fm = nodes
        .iter()
        .find(|n| n.kind == SemanticKind::Frontmatter)
        .expect("should have frontmatter");
    let raw = fm.raw(source);
    assert!(raw.starts_with("---"), "frontmatter should start with ---");
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());
}

// ---------------------------------------------------------------------------
// RED test 12: reconstruct from all nodes covers entire source
// ---------------------------------------------------------------------------

#[test]
fn reconstruct_covers_entire_source() {
    let source = "# H\n\nPara.\n\n> Quote.\n";
    let doc = SourceDocument::parse(source).expect("parse must succeed");

    // The union of all node spans (including gaps as Raw) must equal source.
    assert_eq!(
        doc.reconstruct().as_bytes(),
        source.as_bytes(),
        "reconstruct must cover entire source"
    );
}

// ---------------------------------------------------------------------------
// RED test 13: CR-only line endings (classic Mac) detected
// ---------------------------------------------------------------------------

#[test]
fn cr_only_line_endings_detected() {
    let source = "# Title\r\rParagraph.\r";
    let doc = SourceDocument::parse(source).expect("parse must succeed");
    assert_eq!(
        doc.line_ending(),
        LineEnding::Cr,
        "CR-only document should detect LineEnding::Cr"
    );
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());
}
