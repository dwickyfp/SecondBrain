# Yrs 0.27.3 results

## Correctness

Yrs converges in shared text/offline scenarios and passes rich mark, sticky position, actor undo, full update snapshot/restore, truncation, deterministic materialization, and 100K mixed operations. It fails the mandatory ordered move fixture because Yrs 0.27.3 has no public native identity-preserving array move.

## Native APIs and glue

The candidate uses `Doc`, `TextRef`, `ArrayRef`, `MapRef`, transaction origins, `UndoManager`, state vectors, v1 updates, update-based snapshots, formatting, and sticky indices. A delete-plus-insert move was deliberately not used because it changes CRDT identity and would hide unsupported behavior.

## Unsupported behavior and blockers

- `identity_preserving_move`: mandatory failure; public arrays expose insert/remove but no move.
- `compacted_snapshot`: full state updates work, but no native shallow/compacted snapshot API was found in 0.27.3.
- Rust 1.91.1 compatibility: the dependency uses unstable `if let` guards. Evidence builds require `RUSTC_BOOTSTRAP=1 RUSTFLAGS='-Zcrate-attr=feature(if_let_guard)'` and ordinary workspace gates fail in dependency code.

## Integration observations

Yrs benefits from direct Yjs binary/update compatibility and the strongest TipTap/ProseMirror ecosystem path. Those strengths cannot compensate for a mandatory correctness failure.

See `docs/evidence/crdt-comparison-conformance.json` and `docs/evidence/crdt-comparison-results.json`.
