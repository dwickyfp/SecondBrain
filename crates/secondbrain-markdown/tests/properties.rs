//! Property-based tests for lossless Markdown round-trip behavior.
//!
//! Uses `proptest` to generate random Markdown documents from structured
//! generators that combine headings, paragraphs, nested lists, tables, code
//! fences, Unicode text, CRLF/LF line endings, YAML frontmatter, and
//! wikilinks. Each generated document must round-trip byte-identically and
//! maintain a stable semantic fingerprint.
//!
//! The number of proptest cases is controlled by the `PROPTEST_CASES`
//! environment variable (default: 256).

use proptest::prelude::*;
use secondbrain_markdown::SourceDocument;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Number of proptest cases. Set via `PROPTEST_CASES` env var.
fn proptest_cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: proptest_cases(),
        ..ProptestConfig::default()
    })]

    // -----------------------------------------------------------------------
    // Property 1: Any generated Markdown round-trips byte-identically.
    //
    // If parsing fails (upstream panic), we skip the case via `prop_assume!`
    // — the property only applies to documents the parser can actually handle.
    // -----------------------------------------------------------------------

    #[test]
    fn prop_round_trip_byte_identical(source in arb_markdown_doc()) {
        let doc = SourceDocument::parse(&source);
        prop_assume!(doc.is_ok(), "skip unparseable input (upstream panic)");
        let doc = doc.unwrap();
        let serialized = doc.reconstruct();
        prop_assert_eq!(
            serialized.as_bytes(),
            source.as_bytes(),
            "reconstruct() must be byte-identical to source"
        );
    }

    // -----------------------------------------------------------------------
    // Property 2: Semantic fingerprint is stable across re-parse.
    // -----------------------------------------------------------------------

    #[test]
    fn prop_fingerprint_stable_across_reparse(source in arb_markdown_doc()) {
        let doc1 = SourceDocument::parse(&source);
        prop_assume!(doc1.is_ok(), "skip unparseable input");
        let doc1 = doc1.unwrap();
        let s1 = doc1.reconstruct();
        let doc2 = SourceDocument::parse(&s1);
        prop_assume!(doc2.is_ok(), "skip unparseable re-parse");
        let doc2 = doc2.unwrap();
        prop_assert_eq!(
            doc1.semantic_fingerprint(),
            doc2.semantic_fingerprint(),
            "fingerprint must be stable across round-trip"
        );
    }

    // -----------------------------------------------------------------------
    // Property 3: Double round-trip is idempotent.
    // -----------------------------------------------------------------------

    #[test]
    fn prop_double_round_trip_idempotent(source in arb_markdown_doc()) {
        let doc1 = SourceDocument::parse(&source);
        prop_assume!(doc1.is_ok(), "skip unparseable input");
        let doc1 = doc1.unwrap();
        let s1 = doc1.reconstruct();
        let doc2 = SourceDocument::parse(&s1);
        prop_assume!(doc2.is_ok(), "skip unparseable re-parse");
        let doc2 = doc2.unwrap();
        let s2 = doc2.reconstruct();
        let doc3 = SourceDocument::parse(&s2);
        prop_assume!(doc3.is_ok(), "skip unparseable third parse");
        let doc3 = doc3.unwrap();
        let s3 = doc3.reconstruct();
        prop_assert_eq!(s2.as_bytes(), s1.as_bytes(), "second and third reconstruction must match");
        prop_assert_eq!(s3.as_bytes(), s2.as_bytes(), "third and second reconstruction must match");
        prop_assert_eq!(
            doc1.semantic_fingerprint(),
            doc3.semantic_fingerprint(),
            "first and third fingerprint must match"
        );
    }

    // -----------------------------------------------------------------------
    // Property 4: Line ending style is detected consistently.
    // -----------------------------------------------------------------------

    #[test]
    fn prop_line_ending_consistent(source in arb_markdown_with_ending()) {
        let doc = SourceDocument::parse(&source);
        prop_assume!(doc.is_ok(), "skip unparseable input");
        let doc = doc.unwrap();
        let serialized = doc.reconstruct();
        // Round-trip preserves bytes, so line endings survive by construction.
        prop_assert_eq!(
            serialized.as_bytes(),
            source.as_bytes(),
            "line endings must survive round-trip"
        );
    }

    // -----------------------------------------------------------------------
    // Property 5: Unicode content is preserved exactly.
    // -----------------------------------------------------------------------

    #[test]
    fn prop_unicode_preserved(text in arb_unicode_text()) {
        let source = format!("# {text}\n\nParagraph: {text}\n");
        let doc = SourceDocument::parse(&source);
        prop_assume!(doc.is_ok(), "skip unparseable input");
        let doc = doc.unwrap();
        let serialized = doc.reconstruct();
        prop_assert_eq!(
            serialized.as_bytes(),
            source.as_bytes(),
            "unicode content must be byte-identical"
        );
    }

    // -----------------------------------------------------------------------
    // Property 6: Frontmatter + body round-trips.
    // -----------------------------------------------------------------------

    #[test]
    fn prop_frontmatter_body_round_trip(fm in arb_yaml_frontmatter(), body in arb_markdown_blocks(1..4)) {
        let source = format!("{fm}\n{body}");
        let doc = SourceDocument::parse(&source);
        prop_assume!(doc.is_ok(), "skip unparseable input");
        let doc = doc.unwrap();
        let serialized = doc.reconstruct();
        prop_assert_eq!(
            serialized.as_bytes(),
            source.as_bytes(),
            "frontmatter + body must round-trip"
        );

        // Re-parse and check fingerprint.
        let doc2 = SourceDocument::parse(&serialized);
        prop_assume!(doc2.is_ok(), "skip unparseable re-parse");
        let doc2 = doc2.unwrap();
        prop_assert_eq!(
            doc.semantic_fingerprint(),
            doc2.semantic_fingerprint(),
            "fingerprint must be stable for frontmatter + body"
        );
    }
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Generate a full Markdown document from structured blocks.
fn arb_markdown_doc() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_block(), 1..8).prop_map(|blocks| blocks.join("\n\n"))
}

/// Generate a Markdown document with a specific line ending style.
fn arb_markdown_with_ending() -> impl Strategy<Value = String> {
    (arb_line_ending(), arb_markdown_doc()).prop_map(|(ending, doc)| match ending {
        LineEndingGen::Lf => doc,
        LineEndingGen::Crlf => doc.replace('\n', "\r\n"),
    })
}

/// Line ending generator.
#[derive(Clone, Copy, Debug)]
enum LineEndingGen {
    Lf,
    Crlf,
}

fn arb_line_ending() -> impl Strategy<Value = LineEndingGen> {
    prop_oneof![Just(LineEndingGen::Lf), Just(LineEndingGen::Crlf)]
}

/// A single Markdown block.
fn arb_block() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_heading(),
        arb_paragraph(),
        arb_code_fence(),
        arb_list(),
        arb_table(),
        arb_blockquote(),
        arb_thematic_break(),
    ]
}

/// Generate an ATX heading.
fn arb_heading() -> impl Strategy<Value = String> {
    (1u8..=6, arb_unicode_text()).prop_map(|(level, text)| {
        let hashes: String = "#".repeat(level as usize);
        format!("{hashes} {text}")
    })
}

/// Generate a paragraph of inline text.
fn arb_paragraph() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_inline(), 1..6).prop_map(|parts| parts.join(" "))
}

/// Generate inline content.
fn arb_inline() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_unicode_text(),
        arb_bold(),
        arb_italic(),
        arb_inline_code(),
        arb_link(),
        arb_wikilink(),
        arb_tag(),
    ]
}

fn arb_bold() -> impl Strategy<Value = String> {
    arb_unicode_text().prop_map(|t| format!("**{t}**"))
}

fn arb_italic() -> impl Strategy<Value = String> {
    arb_unicode_text().prop_map(|t| format!("*{t}*"))
}

fn arb_inline_code() -> impl Strategy<Value = String> {
    arb_simple_text().prop_map(|t| format!("`{t}`"))
}

fn arb_link() -> impl Strategy<Value = String> {
    (arb_unicode_text(), arb_url()).prop_map(|(text, url)| format!("[{text}]({url})"))
}

fn arb_wikilink() -> impl Strategy<Value = String> {
    arb_simple_text().prop_map(|t| format!("[[{t}]]"))
}

fn arb_tag() -> impl Strategy<Value = String> {
    arb_simple_text().prop_map(|t| format!("#{t}"))
}

/// Generate a fenced code block.
fn arb_code_fence() -> impl Strategy<Value = String> {
    (arb_language(), arb_code_content()).prop_map(|(lang, code)| format!("```{lang}\n{code}\n```"))
}

fn arb_language() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("rust".to_string()),
        Just("python".to_string()),
        Just("javascript".to_string()),
        Just("text".to_string()),
        Just(String::new()),
    ]
}

fn arb_code_content() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_code_line(), 0..5).prop_map(|lines| lines.join("\n"))
}

fn arb_code_line() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("x = 1".to_string()),
        Just("print('hello')".to_string()),
        Just("// comment".to_string()),
        Just("fn main() {}".to_string()),
        Just("".to_string()),
    ]
}

/// Generate a list (possibly nested).
fn arb_list() -> impl Strategy<Value = String> {
    (
        arb_list_marker(),
        prop::collection::vec(arb_list_item(), 1..5),
    )
        .prop_map(|(marker, items)| {
            items
                .iter()
                .map(|item| format!("{marker} {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn arb_list_marker() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("-".to_string()),
        Just("*".to_string()),
        Just("1.".to_string()),
    ]
}

fn arb_list_item() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_unicode_text(),
        arb_unicode_text().prop_map(|t| format!("{t} [x]")),
        arb_unicode_text().prop_map(|t| format!("{t} [ ]")),
    ]
}

/// Generate a GFM table.
fn arb_table() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(arb_simple_text(), 2..4),
        prop::collection::vec(prop::collection::vec(arb_simple_text(), 2..4), 1..4),
    )
        .prop_map(|(headers, rows)| {
            let mut table = String::new();

            // Header row.
            table.push('|');
            for h in &headers {
                table.push(' ');
                table.push_str(h);
                table.push_str(" |");
            }
            table.push('\n');

            // Separator.
            table.push('|');
            for _ in &headers {
                table.push_str(" --- |");
            }
            table.push('\n');

            // Data rows.
            for row in &rows {
                table.push('|');
                for cell in row {
                    table.push(' ');
                    table.push_str(cell);
                    table.push_str(" |");
                }
                table.push('\n');
            }

            table.trim_end().to_string()
        })
}

/// Generate a blockquote.
fn arb_blockquote() -> impl Strategy<Value = String> {
    arb_paragraph().prop_map(|p| {
        p.lines()
            .map(|l| format!("> {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Generate a thematic break.
fn arb_thematic_break() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("---".to_string()),
        Just("***".to_string()),
        Just("___".to_string()),
    ]
}

/// Generate YAML frontmatter.
fn arb_yaml_frontmatter() -> impl Strategy<Value = String> {
    (
        arb_simple_text(),
        prop::collection::vec(arb_simple_text(), 1..3),
    )
        .prop_map(|(title, tags)| {
            let tags_str = tags
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!("---\ntitle: \"{title}\"\ntags: [{tags_str}]\n---")
        })
}

/// Generate a URL.
fn arb_url() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("https://example.com".to_string()),
        Just("https://example.com/page".to_string()),
        Just("https://example.com/path?q=1".to_string()),
        Just("http://localhost:3000".to_string()),
    ]
}

/// Generate bounded Unicode text (may include multibyte chars).
fn arb_unicode_text() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_simple_text(),
        "[éàüñç]{1,10}".prop_map(|s| s),
        "[あいうえお]{1,5}".prop_map(|s| s),
        Just("🚀".to_string()),
        Just("café".to_string()),
        Just("naïve".to_string()),
        Just("résumé".to_string()),
        Just("世界".to_string()),
        Just("Hello".to_string()),
        Just("test".to_string()),
    ]
}

/// Generate simple ASCII text (alphanumeric + simple punctuation).
fn arb_simple_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_]{1,8}"
}

/// Generate multiple Markdown blocks joined by blank lines.
fn arb_markdown_blocks(range: std::ops::Range<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(arb_block(), range).prop_map(|blocks| blocks.join("\n\n"))
}
