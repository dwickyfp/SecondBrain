# Phase 0 crash-recovery evidence

## Environment

- macOS 26.5.2 (build 25F84), arm64
- rustc 1.91.1 (`ed61e7d7e 2025-11-07`)
- cargo 1.91.1 (`ea2d97820 2025-10-10`)
- Package: `secondbrain-transaction`

## Focused acceptance test

```sh
cargo test --locked -p secondbrain-transaction --test crash_recovery -- --nocapture
```

Outcome: **5 passed, 0 failed**. The suite launches real child processes and verifies deterministic, idempotent recovery after process aborts.

## Crash boundaries

Each run exercises all six durable boundaries:

1. `before_append`
2. `after_append_before_state`
3. `after_operations_durable`
4. `during_temp_markdown_write`
5. `after_rename_before_commit`
6. `after_commit_before_index`

The recovery assertions cover safe abort before durable operations, reconstruction from a stale marker plus durable oplog, replay/materialization, recognition of already-materialized Markdown, corruption quarantine without materialization, one-shot index repair, and an empty second recovery pass.

## 20-run real-process stress loop

Executed with Python `subprocess.run`, invoking this exact Cargo test for every iteration:

```sh
cargo test --locked -p secondbrain-transaction --test crash_recovery \
  real_process_crashes_recover_deterministically_at_every_boundary -- \
  --exact --nocapture
```

Outcome: **20/20 passed** (all exits 0), total wall-clock duration **11.492 seconds**. Each iteration covers six child-process aborts, for **120 injected process crashes** in total.

## Production safety

Environment-selected failpoints and process abort behavior are compiled only under `cfg(debug_assertions)`. Release builds retain the inert `hit` call surface but contain no environment lookup or environment-triggered crash hook.
