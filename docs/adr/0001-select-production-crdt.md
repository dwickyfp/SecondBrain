# ADR 0001: Select Loro as the production CRDT

**Status:** Accepted
**Date:** 2026-07-26
**Decision owners:** SecondBrain architecture spike

## Context

SecondBrain needs one per-note CRDT for Rust-owned semantic external edits, identity-preserving block reordering, rich text and relative positions, offline convergence, per-actor undo, compact recovery, and a future TipTap bridge. A permanent dual-engine abstraction is explicitly rejected. Correctness is mandatory; performance cannot rescue a failed acceptance scenario.

## Candidates and evidence

- Loro 1.13.7
- Yrs 0.27.3
- Contract and candidates: `../../spikes/crdt-comparison/`
- Raw conformance: `../evidence/crdt-comparison-conformance.json`
- Raw benchmark runs: `../evidence/crdt-comparison-results.json`
- Environment and exact method: `../evidence/crdt-comparison-environment.md`

The candidate protocol is a strict one-request/one-response JSON process contract. Ten mandatory shared fixtures execute native APIs. Unsupported behavior is data, not an emulated success.

## Acceptance matrix

| Mandatory scenario | Loro 1.13.7 | Yrs 0.27.3 |
|---|---|---|
| Concurrent same-position insertion | Pass | Pass |
| Concurrent delete/edit | Pass | Pass |
| Paragraph/list identity-preserving move | Pass, native movable list | **Unsupported**, no native array move |
| Rich mark and relative position mapping | Pass | Pass |
| External replacement represented as semantic ops | Pass | Pass |
| Three offline replicas, reordered duplicate delivery | Pass | Pass |
| Per-actor/per-peer undo | Pass | Pass |
| Full snapshot/restore and truncation rejection | Pass | Pass |
| Deterministic materialization | Pass | Pass |
| 100K mixed native operations | Pass | Pass |

Loro is eligible. Yrs is ineligible because an ordered move cannot be represented by a public native identity-preserving operation in 0.27.3. Modeling it as delete-plus-insert would destroy block CRDT identity and weaken the approved external-edit semantics.

## Benchmark summary

Release means below are from three raw repetitions; `±` is sample standard deviation. Results include process startup. They are directional, not cross-machine budgets.

| Shape | Loro mean ± sd | Yrs mean ± sd | Notes |
|---|---:|---:|---|
| Text, 10K, 2 replicas | 4.84 ± 0.21 ms | 34.54 ± 0.35 ms | Loro faster |
| Text, 100K, 2 replicas | 22.34 ± 0.24 ms | 2551.25 ± 7.16 ms | Loro faster in this append-heavy implementation |
| Properties, 10K, 100 replicas | 11.01 ± 0.30 ms | 134.50 ± 3.32 ms | Loro faster |
| Native moves, 10K, 10 replicas | 29.75 ± 0.26 ms | 3.35 ± 0.17 ms | Yrs timing is unsupported/no work and is not comparable |
| Offline merge, 10K, 10 replicas | 5.89 ± 0.21 ms | 12.16 ± 1.41 ms | Loro faster |
| Offline merge, 100K, 100 replicas | 139.01 ± 1.36 ms | 87.36 ± 0.95 ms | Yrs faster |
| Incremental update, 10K | 6.11 ± 0.43 ms | 38.89 ± 0.30 ms | Native state-vector delta for both |
| Full snapshot/cold restore, 100K | 210.78 ± 1.12 ms | 12436.63 ± 462.34 ms | Snapshot bytes: 929,148 vs 2,335,442 |
| Shallow/compacted restore, 100K | 207.78 ± 10.48 ms | 12400.61 ± 584.25 ms | Yrs compacted mode unsupported; full update used only to retain process evidence |

Observed peak RSS is workload-sensitive and sampled externally. The largest observed Loro value was about 259.5 MiB in 100K restore; Yrs was about 65.5 MiB. Short cases can report zero when they exit within the 2 ms sampling interval. Payload/memory therefore favors neither candidate uniformly: Loro produced smaller snapshots and much faster restore, while Yrs used less observed restore RSS and won the largest offline merge.

## Weighted criteria

Only eligible candidates may win. Scores are 0-10, multiplied by the stated weight.

| Criterion | Weight | Loro | Yrs | Basis |
|---|---:|---:|---:|---|
| Convergence/correctness | Mandatory | Eligible | **Ineligible** | Shared move fixture |
| External-edit semantic operation fit | 20 | 9 | 6 | Native text update/semantic containers; Yrs move gap |
| Ordered move/tree behavior | 15 | 10 | 2 | `LoroMovableList::mov`; no Yrs array move |
| Per-actor undo/history | 10 | 9 | 9 | Native peer/origin-scoped managers |
| Snapshot/compaction/recovery | 15 | 9 | 6 | Loro full + shallow snapshot; Yrs full update only |
| Rust core integration | 10 | 9 | 3 | Yrs fails pinned stable compilation |
| TipTap/TypeScript integration path | 10 | 6 | 10 | Yjs ecosystem strongly favors Yrs |
| Payload and memory | 10 | 8 | 7 | Mixed; Loro smaller snapshots, Yrs lower restore RSS |
| Ecosystem/maturity/maintenance risk | 10 | 7 | 8 | Yjs maturity favors Yrs; current stable compile issue offsets it |
| **Weighted total / 100** | | **8.55** | **6.10, ineligible** | Performance does not override correctness |

## Integration and maintenance considerations

Loro's native movable list, cursor/frontier APIs, update modes, and shallow snapshots closely fit the Rust core. Its JavaScript/WASM integration exists but has a weaker direct TipTap ecosystem path, so a focused TipTap adapter spike remains necessary.

Yrs offers excellent Yjs binary compatibility and a mature ProseMirror/TipTap ecosystem. However, 0.27.3 requires an unstable `if let` guard under Rust 1.91.1. The candidate was executed with a documented spike-only compiler feature injection, not a dependency source patch. Production must not use `RUSTC_BOOTSTRAP`.

Neither spike binary implements signatures, encryption, authorization, or the final network envelope. Native CRDT bytes must be wrapped by the future versioned, signed, encrypted production protocol and fuzzed as untrusted input.

## Decision

Select **Loro 1.13.7** as the production CRDT direction. Reject Yrs 0.27.3 for this product version because it fails mandatory identity-preserving ordered movement. Remove neither candidate spike: it is evidence, but only Loro may enter the production crate in Task 28.

## Consequences and migration risk

- Production persistence will use versioned Loro native updates/snapshots behind a SecondBrain-specific API, not the spike JSON protocol.
- The provisional `sb-local-oplog-v1` needs a one-time idempotent migration into per-note Loro state.
- Block identity must map directly to native movable structures; production must not flatten moves into text replacement.
- TipTap transactions and positions need a dedicated Loro bridge and interoperability tests.
- Adopting Yjs clients later would require an explicit migration/export project; no cross-engine wire compatibility is promised.
- Loro version upgrades require deterministic vectors, corrupt/truncated input tests, and snapshot migration evidence.

## Follow-up work

1. Implement Task 28's production `secondbrain-crdt` crate with Loro only.
2. Define durable per-note update/snapshot manifests and crash ordering.
3. Build the TipTap transaction/position adapter before Phase 1 editor integration.
4. Add randomized multi-replica property tests and native decoder fuzzing.
5. Re-run cross-platform performance and memory gates on macOS, Windows, and Linux.

## Independent review

No separate human or independent agent reviewer was available in this local session. The reproducible review commands are the dedicated Loro conformance command and any fixed-seed benchmark invocation through `run-comparison.py`; this absence is a recorded process blocker and does not alter the mandatory machine result.
