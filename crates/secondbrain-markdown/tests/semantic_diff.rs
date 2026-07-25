//! Semantic diff and apply tests driven by TOML fixture cases.
//!
//! Each `.case.toml` under `fixtures/external-edits/` defines a `base`
//! source, an `incoming` source, expected operation counts/kinds, and
//! the expected result after applying the operations.
//!
//! The test harness:
//! 1. Loads all fixture cases.
//! 2. For each: parses base and incoming, calls `diff_documents`, asserts
//!    operation count and kinds match expectations.
//! 3. Calls `apply_operations` and asserts the result equals the expected
//!    source.
//! 4. A determinism test verifies repeated diff calls produce identical
//!    operations.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_markdown::SourceDocument;
use secondbrain_markdown::apply::apply_operations;
use secondbrain_markdown::diff::diff_documents;
use secondbrain_markdown::operation::SemanticOperation;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

/// Root directory of the external-edits fixtures.
fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("external-edits")
}

/// Recursively collect every `.case.toml` file under the fixture root.
fn collect_fixtures() -> Vec<PathBuf> {
    let root = fixture_root();
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(&root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// Relative path from the fixture root, for readable test names.
fn rel_path(path: &Path) -> String {
    path.strip_prefix(fixture_root())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

// ---------------------------------------------------------------------------
// TOML schema for fixture cases
// ---------------------------------------------------------------------------

/// Expected operations specification.
#[derive(Debug, Deserialize)]
struct ExpectedOperations {
    /// Exact count (if set). Mutually exclusive with `min_count`.
    count: Option<usize>,
    /// Minimum count (alternative to `count`).
    #[serde(default)]
    min_count: Option<usize>,
    /// Expected operation kinds (in any order).
    #[serde(default)]
    kinds: Vec<String>,
}

/// Expected result specification.
#[derive(Debug, Deserialize)]
struct ExpectedResult {
    /// The expected source after applying operations.
    source: String,
}

/// The `[case]` table.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CaseData {
    name: String,
    #[allow(dead_code)]
    description: String,
    base: SourceField,
    incoming: SourceField,
    expected_operations: ExpectedOperations,
    expected_result: ExpectedResult,
}

/// A source field (base or incoming).
#[derive(Debug, Deserialize)]
struct SourceField {
    source: String,
}

/// Top-level fixture file: a single `[case]` table.
#[derive(Debug, Deserialize)]
struct FixtureFile {
    case: CaseData,
}

/// Load and parse a fixture case from a TOML file.
fn load_fixture(path: &Path) -> FixtureFile {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    toml::from_str(&content)
        .unwrap_or_else(|e| panic!("failed to parse fixture {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Helper: get operation kind as string
// ---------------------------------------------------------------------------

/// Return the kind name of an operation for assertion comparison.
fn op_kind_name(op: &SemanticOperation) -> &'static str {
    match op {
        SemanticOperation::InsertNode { .. } => "InsertNode",
        SemanticOperation::DeleteNode { .. } => "DeleteNode",
        SemanticOperation::ReplaceNode { .. } => "ReplaceNode",
        SemanticOperation::MoveNode { .. } => "MoveNode",
        SemanticOperation::SetProperty { .. } => "SetProperty",
        SemanticOperation::RemoveProperty { .. } => "RemoveProperty",
        SemanticOperation::NeedsReview { .. } => "NeedsReview",
    }
}

// ---------------------------------------------------------------------------
// Fixture-driven tests
// ---------------------------------------------------------------------------

/// Test that every fixture case produces the expected operations and result.
///
/// Each fixture is tested independently with a descriptive panic message.
#[test]
fn fixture_cases_produce_expected_operations_and_result() {
    let fixtures = collect_fixtures();
    assert!(
        !fixtures.is_empty(),
        "must have at least one fixture file under fixtures/external-edits/"
    );

    for fixture_path in &fixtures {
        let name = rel_path(fixture_path);
        let fixture = load_fixture(fixture_path);
        let case = &fixture.case;

        // Parse base and incoming.
        let base_doc = SourceDocument::parse(&case.base.source)
            .unwrap_or_else(|e| panic!("[{name}] base parse failed: {e}"));
        let incoming_doc = SourceDocument::parse(&case.incoming.source)
            .unwrap_or_else(|e| panic!("[{name}] incoming parse failed: {e}"));

        // Diff.
        let ops = diff_documents(&base_doc, &incoming_doc);

        // Assert operation count.
        if let Some(expected_count) = case.expected_operations.count {
            assert_eq!(
                ops.len(),
                expected_count,
                "[{name}] expected exactly {expected_count} operations, got {}",
                ops.len()
            );
        }
        if let Some(min) = case.expected_operations.min_count {
            assert!(
                ops.len() >= min,
                "[{name}] expected at least {min} operations, got {}",
                ops.len()
            );
        }

        // Assert operation kinds (as a set — order-independent for fixture
        // assertions; determinism is verified separately).
        let actual_kinds: HashSet<&str> = ops.iter().map(op_kind_name).collect();
        let expected_kinds: HashSet<&str> = case
            .expected_operations
            .kinds
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(
            actual_kinds, expected_kinds,
            "[{name}] operation kinds mismatch: expected {expected_kinds:?}, got {actual_kinds:?}"
        );

        // Apply operations and check result.
        let result = apply_operations(&case.base.source, &ops)
            .unwrap_or_else(|e| panic!("[{name}] apply_operations failed: {e}"));
        assert_eq!(
            result, case.expected_result.source,
            "[{name}] apply result does not match expected"
        );
    }
}

// ---------------------------------------------------------------------------
// Determinism test
// ---------------------------------------------------------------------------

/// Verify that calling `diff_documents` multiple times on the same inputs
/// produces identical operations every time.
#[test]
fn diff_is_deterministic() {
    let fixtures = collect_fixtures();
    assert!(!fixtures.is_empty(), "need fixtures for determinism test");

    for fixture_path in &fixtures {
        let name = rel_path(fixture_path);
        let fixture = load_fixture(fixture_path);
        let case = &fixture.case;

        let base_doc = SourceDocument::parse(&case.base.source)
            .unwrap_or_else(|e| panic!("[{name}] base parse failed: {e}"));
        let incoming_doc = SourceDocument::parse(&case.incoming.source)
            .unwrap_or_else(|e| panic!("[{name}] incoming parse failed: {e}"));

        // Run diff 5 times and verify identical results.
        let first = diff_documents(&base_doc, &incoming_doc);
        for iteration in 1..=4 {
            let subsequent = diff_documents(&base_doc, &incoming_doc);
            assert_eq!(
                first, subsequent,
                "[{name}] diff is not deterministic: iteration {iteration} differs from first"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Empty diff test
// ---------------------------------------------------------------------------

/// Diffing identical documents should produce zero operations.
#[test]
fn diff_identical_documents_produces_no_operations() {
    let source = "# Title\n\nSome paragraph.\n\n- Item 1\n- Item 2\n";
    let base = SourceDocument::parse(source).expect("base parse");
    let incoming = SourceDocument::parse(source).expect("incoming parse");
    let ops = diff_documents(&base, &incoming);
    assert!(
        ops.is_empty(),
        "diffing identical documents should produce no operations, got {ops:?}"
    );
}
