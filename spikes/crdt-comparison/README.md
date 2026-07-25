# CRDT comparison spike

This throwaway workspace compares Loro 1.13.7 and Yrs 0.27.3 through a strict process boundary. It does not define a production trait and production crates do not depend on either candidate.

## Contract

Each invocation reads exactly one `secondbrain-crdt-spike-v1` JSON request from stdin and writes exactly one response to stdout. A request contains a complete ordered command stream because process-local replica state must not leak between measurements. Serde rejects unknown fields. The harness verifies protocol/scenario identity, command count, candidate errors, unsupported features, operation count, exact expected states, and convergence groups.

The command vocabulary is `create_replica`, `apply_local`, `apply_workload`, `probe_relative_position`, `export_updates`, `export_incremental`, `import_updates`, `materialize`, `undo_actor`, `snapshot`, `compacted_snapshot`, `restore`, `truncate_restore`, and `metrics`. `apply_workload` expands a recorded kind/count/seed into native candidate calls so the 100K fixture stays reviewable. It does not use a shared state model.

Mandatory fixtures live in `fixtures/`. Both candidates use their actual native text, map, ordered collection, update, undo, snapshot, rich-text mark, and relative-position APIs. Unsupported behavior is returned in `unsupported`; candidate code must not emulate it to pass.

## Reproduction

```bash
cargo test -p crdt-spike-contract
RUSTC_BOOTSTRAP=1 RUSTFLAGS='-Zcrate-attr=feature(if_let_guard)' cargo build --release --manifest-path spikes/crdt-comparison/yrs-candidate/Cargo.toml
cargo build --release -p loro-candidate -p crdt-comparison-bench
CRDT_CANDIDATE_BIN=target/release/loro-candidate cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
CRDT_CANDIDATE_BIN=spikes/crdt-comparison/yrs-candidate/target/release/yrs-candidate cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
python3 spikes/crdt-comparison/scripts/run-conformance.py --output docs/evidence/crdt-comparison-conformance.json
python3 spikes/crdt-comparison/scripts/run-comparison.py --release --repetitions 3 --output docs/evidence/crdt-comparison-results.json
```

Yrs 0.27.3 uses an unstable `if let` guard internally and does not compile with this repository's Rust 1.91.1 unless the displayed spike-only `RUSTC_BOOTSTRAP`/`RUSTFLAGS` workaround is used. This is evidence, not proposed production configuration.

Peak RSS is sampled from each fresh process with macOS `ps` every 2 ms. A zero means the process ended before the first sample, not zero memory. Raw runs, including unsupported outcomes, are retained without filtering.
