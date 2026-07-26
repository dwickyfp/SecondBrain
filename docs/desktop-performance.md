# Desktop performance evidence

Task 49 separates first import/index from indexed cold start. `open_workspace_at` uses the shared `secondbrain_index::ensure_index` health contract: a valid current index is reused, while a missing, stale, corrupt, or wrong-schema index is rebuilt. Validity requires SQLite integrity and foreign-key checks, the current schema version, the exact configured Markdown path inventory, and a matching content hash for every note.

## Reproduce

Reference-machine release run (10,000 deterministic notes, 2 independent first indexes, 20 indexed operations):

```sh
SECONDBRAIN_BENCH_ENV=apple-m3-8cpu-16gb-macos-26.5 SECONDBRAIN_BENCH_CLASSIFICATION=local-reference cargo run --release --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --features benchmark-binaries --bin desktop-performance -- 10000 2 20 > docs/evidence/task-49-local-reference.json
```

The checked-in evidence retains a 10,000-note local observation, including every raw sample. It predates source-provenance capture and is explicitly mutable, non-immutable, and not completion evidence. Two first-index samples demonstrate that the workload terminated in practical time, but are not enough to estimate a stable tail percentile; the reported nearest-rank p95 is the slower observation.

Set `SECONDBRAIN_PROFILE_INDEX=1` to print rebuild stage timings to stderr without changing the evidence JSON on stdout. Diagnostic 100-note release samples attributed 0.839-1.315 seconds to durable identity-map and CRDT genesis population, versus 3.0-4.6 milliseconds to Markdown parsing and normally 4.0-4.3 milliseconds to SQLite population (one sample had an 89.6 millisecond SQLite outlier). This identified the actual bottleneck as per-note durable sidecar creation, not the scanner or SQLite transaction.

Hosted CI smoke run (100 notes, 2 first indexes, 20 indexed operations):

```sh
SECONDBRAIN_BENCH_ENV=github-hosted-${RUNNER_OS}-${RUNNER_ARCH} SECONDBRAIN_BENCH_CLASSIFICATION=hosted-ci-smoke-non-reference cargo run --release --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --features benchmark-binaries --bin desktop-performance -- 100 2 20 > hosted-smoke.json
```

The versioned `secondbrain-desktop-performance-v1` JSON retains every timing and RSS sample and reports nearest-rank p50/p95/p99. It records fixture bytes and BLAKE3 hash, generator version, release executable hash, Rust/environment details and environment hash, the exact command, and methodology.

## Interpretation

`first_index_us` creates a new initialized deterministic vault for each sample and invokes the production desktop open operation with no index. `indexed_startup_us` invokes that same operation against an unchanged, valid index. Because correctness requires detecting unobserved external edits, indexed startup includes a complete path scan and content hashing, but not Markdown parsing or SQLite reconstruction.

`search_us` and `open_note_us` invoke the production desktop backend operations used by Tauri commands. They are not synthetic SQLite-only microbenchmarks.

`backend_process_rss_bytes` is release benchmark/backend process RSS sampled after operations. It includes loaded backend code and retained allocations and is useful as a cross-platform smoke/regression signal, but it is neither idle RSS nor packaged Tauri/WebView process-tree RSS. This benchmark therefore sets `native_app_measured` to `false`; the `<250 MiB` packaged idle budget remains unverified until a native package/process-tree measurement is made on the named reference machine.

The roadmap cold-start limits (`<1s` for a small vault and `<3s` for 10,000 notes) apply to indexed startup, not first import/index. First import remains a separately reported informational optimization metric with no absolute release threshold. Absolute release budgets only apply to the named machines in `apps/desktop/performance-budgets.json`. GitHub-hosted runners vary in hardware, so CI runs smoke/regression collection and uploads raw JSON without enforcing those absolute budgets. No hosted regression envelope is configured until enough comparable runner observations exist to define one honestly.

## Current result

`docs/evidence/task-49-local-reference.json` is a legacy local named-machine observation for 10,000 notes, not immutable completion evidence. It retains first-index samples of 105.317 and 109.959 seconds; with only two samples the nearest-rank p50/p95/p99 are 105.317/109.959/109.959 seconds. It records indexed-startup p50/p95/p99 of 267.685/329.234/827.706 milliseconds, search p50/p95/p99 of 0.560/0.690/0.726 milliseconds, open-note p50/p95/p99 of 0.594/0.831/0.949 milliseconds, and backend-process RSS p50/p95/p99 of 133.38/177.98/186.25 MiB.

Indexed startup, search, and open-note pass their named-machine backend-operation budgets at 10,000 notes. Desktop open now uses the shared `ensure_index` contract rather than rebuilding unconditionally, and a regression test verifies that valid reuse performs no identity or CRDT sidecar writes. SQLite population is enclosed by one transaction and has an operation-level regression test for that behavior.

The legacy first-index observations are 105.317 and 109.959 seconds. They are retained as raw optimization evidence and do not fail the indexed cold-start contract; two samples are also insufficient for a stable tail estimate. The remaining first-import bottleneck is the durable state model: first responsibility for 10,000 notes creates 10,000 atomic identity records and 10,000 atomic CRDT genesis records, each with file and directory durability barriers. Improving this likely requires a transactional append/database-backed identity and canonical-state store rather than weakening durability. Exact-artifact native smoke mounts and launches the app from the DMG on macOS and extracts and launches the packaged executable from MSI/DEB contents on Windows/Linux; it does not claim Windows/Linux installation semantics. It measures packaged native parent-plus-descendant RSS after frontend/backend readiness and quiescence; named reference runs enforce `<250 MiB`, while hosted CI records the same raw receipt as non-reference without claiming an absolute pass.
