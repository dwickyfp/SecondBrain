# Phase 0 end-to-end evidence

Recorded at `2026-07-25T19:37:20Z` against commit
`ac9b0c8ecb8f23ede6f5733db28bfff5ca2bf545` plus the Task 22 files described
below.

## Environment

- OS: macOS Darwin 25.5.0, arm64
- Rust: `rustc 1.91.1 (ed61e7d7e 2025-11-07)`
- Cargo: `cargo 1.91.1 (ea2d97820 2025-10-10)`
- Fixture: `fixtures/markdown/obsidian-vault`, three Markdown notes and one
  `.obsidian/app.json`

## Command

```text
cargo test -p secondbrain-cli --test e2e_phase0 -- --nocapture
```

Result:

```text
running 1 test
test phase_zero_cli_survives_external_merge_and_crash_recovery ... ok

test result: ok. 1 passed; 0 failed; 0 ignored
```

## Scenario Proven

The test copies the fixture to a temporary vault and drives only the real
`secondbrain` executable for mutations. It verifies:

- `init` and `index rebuild` leave every fixture byte unchanged;
- three notes are indexed and the Obsidian settings file is ignored;
- FTS search and wikilink backlinks resolve;
- a transaction hard-aborted after durable oplog append is merged with an
  independent external whole-file edit by `reconcile`;
- a second hard-aborted transaction is completed by a fresh `recovery check`
  process;
- the final Markdown contains the internal, external, and recovered changes;
- identity evidence and the converged base describe the final bytes;
- every durable transaction is terminal and no index repair remains owed;
- deleting `index.sqlite`, rebuilding through the binary, and comparing
  `LogicalDump` produces identical notes and links;
- `doctor --json` reports no pending transaction or workspace problem.

The child process is terminated with the debug-only
`SECONDBRAIN_TEST_FAILPOINT=after_operations_durable` boundary. The test asserts
process failure rather than a platform-specific signal code.
