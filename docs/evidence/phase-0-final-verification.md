# Phase 0 Final Verification

This document records the local release gate and the completed GitHub Actions
matrix. Remote matrix: [CI 30175460707](https://github.com/dwickyfp/SecondBrain/actions/runs/30175460707)
for commit `46a9a95`.

## Completed Locally

- Task 22 binary E2E passed, including external merge, two real process crashes,
  restart recovery, identity/base checks, doctor, and logical SQLite rebuild.
- CRDT contract and fake-candidate rejection passed; Loro conformance passed;
  Yrs failure is recorded as mandatory move incompatibility.
- CRDT benchmark ran 66 isolated release processes with fixed seeds and raw JSON.
- Task 28 Loro canonical state facade, framed persistence, CRC validation,
  legacy migration, and idempotence tests passed.
- Platform contract passed locally and on macOS, Windows, and Linux matrix jobs.
- External-agent binary test passed preview, apply, stale retry rejection,
  external reconciliation, search, and doctor without linking libraries.

## Benchmark Baseline

Reference machine: macOS Darwin arm64, Rust 1.91.1, release binary.

| Notes | Init | Index rebuild | Search | Reconcile | No-op reconcile | Rebuild after index delete |
|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 25 ms | 14.86 s | 10.0 ms | 582 ms | 72 ms | 130 ms |
| 10,000 | 34 ms | 189.59 s | 36.7 ms | 8.98 s | 997 ms | 2.05 s |

The raw output is `docs/evidence/phase-0-performance.json`. These are baseline
measurements, not cross-platform budgets. The 10K result identifies
incremental indexing and batched reconcile as the next performance priorities.

## Local Commands

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test -p secondbrain-cli --test external_agent_binary
cargo test -p secondbrain-cli --test e2e_phase0
cargo test -p secondbrain-vault --test platform -- --nocapture
```

All passed locally after the canonical-state and identity-collision fixes,
including `cargo deny --locked check -D warnings`, 10,000 release property cases,
and release crash recovery. The remote matrix also passed all quality, platform,
E2E, benchmark-smoke, artifact-upload, and dependency-policy jobs.
