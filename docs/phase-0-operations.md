# Phase 0 Operations

## Safe Workflow

```text
secondbrain init VAULT
secondbrain index rebuild VAULT
secondbrain search VAULT QUERY
secondbrain diff VAULT PATH INCOMING --out PLAN.json
secondbrain transaction apply VAULT PLAN.json
secondbrain reconcile VAULT
secondbrain recovery check VAULT
secondbrain doctor VAULT
```

`diff` is read-only. `transaction apply` rechecks workspace, note, hash,
version, and review preconditions. External whole-file writes must be followed
by `reconcile`; `index rebuild` never silently adopts external content.

## Recovery

Run `recovery check` after an interrupted process. It is safe to repeat. A
successful recovery may request one derived-index rebuild; a second pass should
reach a fixed point with no action.

## Canonical State

The canonical per-note converged state is Loro 1.13.7 in:

```text
.secondbrain/crdt/<NoteId>.sbcrdt
```

Markdown remains the user-visible materialization and SQLite remains derived.
Legacy snapshots are read-only migration input. The old local oplog reader
remains available for compatibility with the current crash/rebase state machine
until its migration window is retired.

## External Agent Contract

An agent such as OpenCode should communicate through the `secondbrain` binary:
consume JSON stdout, stable exit codes, stderr diagnostics, plan files, and
workspace files. Preview with `diff`, apply an explicit plan, and treat stale
or ambiguous plans as failed-closed. Phase 0 does not yet provide authenticated
agent identity or capability scopes; those belong to the Agent-First phase.
