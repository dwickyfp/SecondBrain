# Loro 1.13.7 results

## Correctness

All 10 mandatory fixtures pass, including concurrent insert/delete, three-way duplicate delivery, identity-preserving moves, rich marks and cursor mapping, semantic external replacement, per-peer undo, snapshot/truncation, deterministic materialization, and 100K mixed native operations.

## Native APIs and glue

The candidate uses `LoroDoc`, `LoroText`, `LoroMap`, `LoroMovableList::mov`, `UndoManager`, binary all-update/incremental exports, full and shallow snapshots, and cursor APIs. Glue is limited to mapping the spike's block `{id,text}` value into a native movable-list scalar and deterministic materialization.

## Unsupported behavior

One process document is bound to one local peer/actor for undo. Multiple actor IDs mutating the same replica process are explicitly unsupported; the mandatory actor-undo fixture uses the intended one-actor-per-replica collaboration model. No mandatory fixture is unsupported.

## Integration observations

Movable list and shallow snapshot support directly match SecondBrain's ordered blocks and recovery needs. Rust stable integration succeeds. Loro has JavaScript/WASM packages, but its TipTap ecosystem path is less direct than Yjs/Yrs and needs a dedicated adapter.

See `docs/evidence/crdt-comparison-conformance.json` and `docs/evidence/crdt-comparison-results.json`.
