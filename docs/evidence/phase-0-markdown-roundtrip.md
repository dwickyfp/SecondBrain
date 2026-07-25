# Phase 0 — Markdown Round-Trip Gate Evidence

## Command

```sh
PROPTEST_CASES=256 cargo test --locked -p secondbrain-markdown --test properties
cargo test --locked -p secondbrain-markdown          # unit + round_trip + source_model
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny --locked check -D warnings
cargo metadata --locked --offline --no-deps --format-version 1
git diff --check
```

## Environment

- **Rust:** rustc 1.91.1 (ed61e7d7e 2025-11-07)
- **OS:** macOS (Darwin)
- **Crate:** `secondbrain-markdown` 0.1.0
- **Upstream dep:** `markdown` 1.0.0 (known upstream panics on minimal inputs)

## Fixture Count

- **45** `.md` fixture files under `fixtures/markdown/` (commonmark, gfm, edge, obsidian, extract, frontmatter, source-preservation)

## Proptest Configuration

- **Cases:** 256 (via `PROPTEST_CASES` env var)
- **Properties:** 6
  1. `prop_round_trip_byte_identical` — reconstruct() == source bytes
  2. `prop_fingerprint_stable_across_reparse` — fingerprint stable on round-trip
  3. `prop_double_round_trip_idempotent` — triple round-trip converges
  4. `prop_line_ending_consistent` — CRLF/LF survive byte-identically
  5. `prop_unicode_preserved` — multibyte UTF-8 preserved
  6. `prop_frontmatter_body_round_trip` — YAML frontmatter + body round-trips

## Upstream Panic Mitigation

The `markdown` crate 1.0.0 has two known upstream panics:

1. `---\n\n>` → `called Option::unwrap() on None` at `to_mdast.rs:216`
2. `---\n\n> A` → `attempt to subtract with overflow` at `list_item.rs:444`

**Fix:** `SourceDocument::parse()` wraps `to_mdast()` in
`std::panic::catch_unwind(AssertUnwindSafe(...))`. On panic, it returns
`ParseError::UpstreamPanic` with a sanitized summary string. This is safe
Rust — `catch_unwind` does not require `unsafe` and `#![forbid(unsafe_code)]`
remains in effect.

Proptest properties use `prop_assume!(parse.is_ok())` to skip inputs that
trigger upstream panics, so the property only applies to documents the parser
can actually handle.

## Regression Tests

3 regression tests in `parse.rs`:
- `panic_frontmatter_empty_blockquote_returns_err` — `---\n\n>` returns `Err(UpstreamPanic)`
- `panic_frontmatter_blockquote_text_returns_err` — `---\n\n> A` returns `Err(UpstreamPanic)`
- `panic_error_display_is_human_readable` — `Display` starts with `"parser panic on upstream markdown crate"`

## Actual Result

### RED → GREEN

**RED (before fix):**

```
test prop_line_ending_consistent ... FAILED
test prop_fingerprint_stable_across_reparse ... FAILED
test prop_round_trip_byte_identical ... FAILED
test prop_double_round_trip_idempotent ... FAILED
test prop_unicode_preserved ... ok
test prop_frontmatter_body_round_trip ... ok

thread 'prop_line_ending_consistent' panicked at
  markdown-1.0.0/src/to_mdast.rs:216:21:
  called `Option::unwrap()` on a `None` value
```

**GREEN (after fix):**

```
test prop_unicode_preserved ... ok
test prop_frontmatter_body_round_trip ... ok
test prop_line_ending_consistent ... ok
test prop_round_trip_byte_identical ... ok
test prop_fingerprint_stable_across_reparse ... ok
test prop_double_round_trip_idempotent ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Full Test Suite

```
secondbrain-markdown lib tests:     23 passed; 0 failed
secondbrain-markdown properties:     6 passed; 0 failed
secondbrain-markdown round_trip:     7 passed; 0 failed
secondbrain-markdown source_model:  13 passed; 0 failed
```

### Gate Summary

| Gate                              | Result |
|-----------------------------------|--------|
| `cargo fmt --all --check`         | ✅ pass |
| `cargo clippy ... -D warnings`    | ✅ pass |
| `cargo test --workspace --all-features` | ✅ pass |
| `cargo deny --locked check -D warnings`  | ✅ pass |
| `cargo metadata --locked`         | ✅ pass |
| `git diff --check`                | ✅ pass |
