# CRDT comparison environment

**Run:** 2026-07-26 local process-level comparison
**Raw conformance:** `crdt-comparison-conformance.json`
**Raw benchmarks:** `crdt-comparison-results.json`

## Host

- macOS 26.5.2 (25F84), arm64
- Apple M3, 16 GiB physical memory
- Rust `rustc 1.91.1 (ed61e7d7e 2025-11-07)`, LLVM 21.1.2
- Cargo `1.91.1 (ea2d97820 2025-10-10)`
- Python 3.14.0 as recorded in raw benchmark JSON
- Loro 1.13.7; Yrs 0.27.3; release profile thin LTO, one codegen unit, symbols stripped

## Commands and results

```text
cargo test -p crdt-spike-contract
PASS: 4 tests; generic candidate test explicitly skipped without CRDT_CANDIDATE_BIN.

CRDT_CANDIDATE_BIN=target/debug/loro-candidate cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
PASS: executed_mandatory=10.

CRDT_CANDIDATE_BIN=target/debug/yrs-candidate cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
FAIL at 03-moves.json: unsupported identity_preserving_move.

python3 spikes/crdt-comparison/scripts/run-conformance.py --output docs/evidence/crdt-comparison-conformance.json
20 raw runs: Loro 10 passed; Yrs 9 passed, 1 unsupported.

python3 spikes/crdt-comparison/scripts/run-comparison.py --release --repetitions 3 --output docs/evidence/crdt-comparison-results.json
66 raw runs: all processes exited 0; Loro 33 passed; Yrs 27 passed and 6 unsupported records (3 move, 3 compacted snapshot).
```

Yrs does not compile on the pinned stable toolchain as published because `yrs-0.27.3/src/block.rs` uses experimental `if let` guards. To execute the candidate for spike evidence only, its release build used:

```text
RUSTC_BOOTSTRAP=1 RUSTFLAGS='-Zcrate-attr=feature(if_let_guard)'
```

No source in the dependency was patched. This workaround is not acceptable production configuration and is scored as Rust integration/maintenance risk.

## Benchmark method

The matrix has 11 shapes and 3 repetitions for each candidate, 66 fresh candidate processes total. It covers 1K/10K/100K operations; 2/10/100 replicas; text, native list moves, properties, three-way-style offline merge with duplicate delivery, full snapshot/cold restore, true state-vector incremental export/import, and shallow/compacted restore. Base seed is `764230027`; each case's seed is stored raw.

Three repetitions, rather than the planned ten, were used because the full matrix includes six 100K restore runs for Yrs at roughly 12 seconds each plus release rebuilds. Every repetition is retained; no warmup or outlier was removed. The 1K first-process outliers demonstrate why these are practical local directional results rather than stable regression thresholds.

Wall time includes process startup and JSON I/O. Peak RSS is sampled using macOS `ps` every 2 ms. A raw zero means a short process exited before sampling, not zero allocation. Response bytes and native update/snapshot/materialization byte counts are retained per run.

## Reproducibility

For every candidate/case, all three state hashes are identical (`unique hash count = 1`). Re-running fixed fixtures also produced identical materialized state and operation counts. Unsupported runs remain in raw evidence and were not omitted from summaries.
