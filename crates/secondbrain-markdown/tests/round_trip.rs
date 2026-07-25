//! Corpus-based round-trip tests for lossless Markdown serialization.
//!
//! Every `.md` fixture under `fixtures/markdown/` is:
//! 1. Parsed into a [`SourceDocument`].
//! 2. Serialized (reconstructed) back to a string.
//! 3. The serialized output is parsed again.
//! 4. Both parses must produce byte-identical serialization and matching
//!    semantic fingerprints.
//!
//! This makes loss detection a permanent release gate: any parser change
//! that alters byte-level output or node structure will fail this test.

use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_markdown::SourceDocument;

/// Recursively collect every `.md` file under the fixture root.
fn collect_fixtures() -> Vec<PathBuf> {
    let root = fixture_root();
    let mut files = Vec::new();

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                out.push(path);
            }
        }
    }

    walk(&root, &mut files);
    files.sort();
    files
}

/// Root directory of the markdown fixtures.
fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
}

/// Read a fixture file as bytes, then convert to a string.
///
/// We read bytes first to detect and report encoding issues precisely.
fn read_fixture(path: &Path) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    String::from_utf8(bytes)
        .unwrap_or_else(|e| panic!("fixture {} is not valid UTF-8: {e}", path.display()))
}

/// Relative path from the fixture root, for readable test names.
fn rel_path(path: &Path) -> String {
    path.strip_prefix(fixture_root())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

// ---------------------------------------------------------------------------
// Corpus round-trip test
// ---------------------------------------------------------------------------

/// Test that every fixture round-trips byte-identically and with stable
/// semantic fingerprints.
///
/// This is a single test function that iterates over all fixtures, so the
/// test binary sees one test case per `cargo test` invocation. Each fixture
/// is asserted independently with a descriptive panic message.
#[test]
fn corpus_round_trip_byte_identical_and_fingerprint_stable() {
    let fixtures = collect_fixtures();
    assert!(
        !fixtures.is_empty(),
        "must have at least one fixture file under fixtures/markdown/"
    );

    for fixture_path in &fixtures {
        let name = rel_path(fixture_path);
        let source = read_fixture(fixture_path);

        // First parse.
        let doc1 = SourceDocument::parse(&source)
            .unwrap_or_else(|e| panic!("[{name}] first parse failed: {e}"));

        // Serialize (reconstruct) the first parse.
        let serialized1 = doc1.reconstruct();

        // Byte-identical reconstruction from the first parse.
        assert_eq!(
            serialized1.as_bytes(),
            source.as_bytes(),
            "[{name}] first reconstruct() must be byte-identical to source"
        );

        // Second parse of the serialized output.
        let doc2 = SourceDocument::parse(&serialized1)
            .unwrap_or_else(|e| panic!("[{name}] second parse failed: {e}"));

        // Serialize the second parse — must match the first serialization.
        let serialized2 = doc2.reconstruct();
        assert_eq!(
            serialized2.as_bytes(),
            serialized1.as_bytes(),
            "[{name}] second reconstruct() must match first reconstruct()"
        );

        // Semantic fingerprints must match across both parses.
        let fp1 = doc1.semantic_fingerprint();
        let fp2 = doc2.semantic_fingerprint();
        assert_eq!(
            fp1, fp2,
            "[{name}] semantic fingerprint must be stable across round-trip"
        );
    }
}

// ---------------------------------------------------------------------------
// Known edge-case tests
// ---------------------------------------------------------------------------

#[test]
fn empty_document_round_trips() {
    let doc = SourceDocument::parse("").expect("empty parse");
    assert_eq!(doc.reconstruct().as_bytes(), b"");
    assert_eq!(doc.semantic_fingerprint(), doc.semantic_fingerprint());
}

#[test]
fn whitespace_only_round_trips() {
    let source = "   \n  \n";
    let doc = SourceDocument::parse(source).expect("whitespace parse");
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());
}

#[test]
fn crlf_line_endings_preserved() {
    let source = "# Title\r\n\r\nParagraph.\r\n";
    let doc = SourceDocument::parse(source).expect("crlf parse");
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());
}

#[test]
fn unicode_content_preserved() {
    let source = "# Héllo 世界 🚀\n\nCafé naïve résumé.\n";
    let doc = SourceDocument::parse(source).expect("unicode parse");
    assert_eq!(doc.reconstruct().as_bytes(), source.as_bytes());

    // Round-trip second parse.
    let s1 = doc.reconstruct();
    let doc2 = SourceDocument::parse(&s1).expect("second parse");
    assert_eq!(doc2.reconstruct().as_bytes(), s1.as_bytes());
    assert_eq!(doc.semantic_fingerprint(), doc2.semantic_fingerprint());
}

#[test]
fn fingerprint_is_deterministic() {
    let source = "# Heading\n\nParagraph.\n";
    let doc_a = SourceDocument::parse(source).expect("parse a");
    let doc_b = SourceDocument::parse(source).expect("parse b");
    assert_eq!(
        doc_a.semantic_fingerprint(),
        doc_b.semantic_fingerprint(),
        "identical inputs must produce identical fingerprints"
    );
}

#[test]
fn fingerprint_differs_for_different_structure() {
    let doc_a = SourceDocument::parse("# Heading\n").expect("parse a");
    let doc_b = SourceDocument::parse("Paragraph.\n").expect("parse b");
    assert_ne!(
        doc_a.semantic_fingerprint(),
        doc_b.semantic_fingerprint(),
        "different node structure should produce different fingerprints"
    );
}
