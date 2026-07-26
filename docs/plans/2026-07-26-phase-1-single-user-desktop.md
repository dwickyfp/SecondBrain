# Phase 1 Single-User Desktop Plan

## Goal

Ship a fast, cross-platform desktop application over the Phase 0 Rust libraries
without creating a second implementation of workspace, index, or transaction
semantics.

## Milestones

| Milestone | Tasks | Exit criterion |
|---|---:|---|
| Desktop foundation | 31-34 | Tauri/Svelte shell opens an existing workspace, rebuilds its derived index, lists notes, and searches it on all desktop platforms. |
| Read experience | 35-38 | Tabs, split panes, source rendering, outline, backlinks, and keyboard navigation operate against one backend contract. |
| Editing | 39-43 | TipTap and CodeMirror adapters emit transaction previews, preserve unsupported Markdown, and meet keystroke/materialization budgets. |
| Knowledge workflows | 44-47 | Properties, daily notes, command palette, graph, and Obsidian import are usable end to end. |
| Release closure | 48-50 | Accessibility, recovery UX, performance budgets, installers, and signed evidence pass on macOS, Windows, and Linux. |

## First Vertical Slice: Workspace Browser

Tasks 31-34 deliberately remain read-only. The desktop process must call the
existing Rust libraries directly; it may rebuild the disposable SQLite index,
but it must not mutate Markdown or expose an editing command.

Acceptance criteria:

1. `apps/desktop` builds with pinned Node and Rust dependency locks.
2. The Tauri backend loads the existing workspace manifest, runs the production
   index rebuild, and returns stable note summaries.
3. Search opens the derived index and uses `secondbrain_index::SearchQuery`.
4. The Svelte shell handles unopened, loading, ready, empty, and error states.
5. Backend contract tests use a real temporary workspace and index.
6. Frontend state tests run without a webview.
7. CI checks frontend types/tests/build and compiles/tests the backend.

## Non-goals For The First Slice

- no file-picker plugin or persisted recent-workspace list;
- no Markdown mutation;
- no TipTap or CodeMirror dependency until the transaction adapter contract is
  specified and tested;
- no graph visualization or placeholder fake data.
