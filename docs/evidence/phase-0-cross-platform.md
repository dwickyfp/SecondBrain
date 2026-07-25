# Phase 0 cross-platform evidence

## Contract coverage

`crates/secondbrain-vault/tests/platform.rs` covers:

- portable `WorkspacePath` values use `/` and reject `\\` separators;
- atomic writes preserve raw CRLF bytes, without text-mode normalization;
- Unix non-UTF-8 filenames are preserved by the filesystem but are explicitly
  outside the UTF-8 `WorkspacePath` contract;
- symlink escapes are rejected on Unix, while Windows attempts the same check
  and reports a capability-aware skip when symlink creation is unavailable;
- atomic-save event normalization and bounded watcher behavior remain covered by
  the existing deterministic tests in `tests/watcher.rs`.

Case-collision detection is owned by the indexer and rejects `Nova.md` versus
`nova.md` deterministically on case-sensitive filesystems. SQLite FTS5 is owned
and covered by the existing
`secondbrain-index` migration and query tests (`migrations.rs` and `query.rs`),
so duplicating an index-owned test in this crate would weaken ownership.

## Local verification

Environment: macOS Darwin arm64. Remote CI was not run from this workspace; no
remote CI URL or result is claimed here.

Commands run locally:

```text
cargo test --locked -p secondbrain-vault --test platform -- --nocapture
cargo test --locked -p secondbrain-cli --test e2e_phase0 -- --nocapture
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

Observed results on macOS Darwin arm64:

- `platform`: 5 passed, 0 failed; the non-UTF-8 filename case reported that
  this filesystem rejects the requested byte sequence with `Illegal byte
  sequence`, while still proving the filename is not representable as a UTF-8
  `WorkspacePath`;
- `e2e_phase0`: 1 passed, 0 failed;
- benchmark smoke request: emitted valid JSON for 100 text operations and two
  replicas with seed `764230027`;
- `cargo fmt --all --check`: passed;
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`:
  passed.

Windows and Linux results require the matrix jobs; the Windows symlink test is
intentionally capability-aware because unprivileged symlink creation may be
disabled.

The CI matrix also emits a deterministic 100-operation, two-replica benchmark
request using seed `764230027` and uploads platform logs plus that request as a
pinned `actions/upload-artifact` artifact. It is a smoke workload, not the full
comparison benchmark.
