# Phase 0 Durable Formats

## Native CRDT State

Each note has one framed `SBCRDT01` state file under
`.secondbrain/crdt/<NoteId>.sbcrdt`. The frame contains a version, metadata
length, Loro snapshot length, JSON metadata, native Loro snapshot bytes, and a
CRC32 over the frame. Metadata binds the snapshot to the note ID, relative path,
materialization version, engine (`loro-1.13.7`), and Markdown content hash.

Readers reject truncated frames, length mismatch, CRC mismatch, wrong note ID,
wrong engine, invalid Loro snapshots, and materialized hash mismatch. Writes
use same-directory temporary files and atomic replacement.

## Legacy Migration

`sb-base-snapshot-v1` JSON files under `.secondbrain/snapshots/` are retained as
read-only migration input. On first access they seed a Loro Markdown document;
the resulting `.sbcrdt` state wins on future reads. Repeated migration is
idempotent and does not rewrite legacy input.

`sb-local-oplog-v1` remains a compatibility reader for current transaction
markers and crash/rebase behavior. It is not the canonical converged content
store; new canonical state is the Loro frame.

## Derived State

`.secondbrain/index.sqlite` is WAL-mode, bundled-SQLite FTS5 state. It is
rebuildable from Markdown and must never be treated as the source of truth.
