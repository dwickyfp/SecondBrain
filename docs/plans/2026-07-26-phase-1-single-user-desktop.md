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

## Phase 1 Completion Contract

Tasks 37-50 are complete only when their production path, failure states, tests,
and evidence all exist. Mocked frontend transport tests supplement but never
replace Rust integration tests or packaged-application verification.

1. Tasks 37-38 complete the read experience with Rust-derived outline and
   backlink context, identity-based pane routing, a typed command registry, and
   keyboard-only operation.
2. Task 39 must move transaction preview/apply orchestration into a reusable
   Rust library and define versioned cross-language fixtures before either
   editor is admitted.
3. Tasks 40-42 use TipTap and CodeMirror as projections over one versioned note
   session. Neither editor may write files or serialize canonical Markdown
   outside the Rust transaction path.
4. Task 43 gates no-op byte survival over the Markdown corpus and publishes raw
   latency samples; correctness failures cannot be waived for speed.
5. Tasks 44-47 route properties, note creation, graph data, and Obsidian import
   through typed Rust contracts with preview, confinement, idempotency, and
   external-agent interoperability tests.
6. Task 48 combines automated accessibility checks, keyboard E2E, stable
   diagnostics, and real crash/recovery scenarios. Manual assistive-technology
   evidence must identify the exact artifact tested.
7. Task 49 distinguishes indexed cold start from first import/index and retains
   raw percentile and memory samples on each desktop OS. Hosted CI provides
   smoke/regression evidence; absolute release budgets require named reference
   hardware.
8. Task 50 builds native unsigned installers in ordinary CI, smoke-tests the
   exact artifacts, publishes checksums and evidence, and signs/notarizes only
   in protected release environments when platform credentials are available.
   Missing signing credentials are reported explicitly and never replaced by a
   false signed-build claim.

Every milestone runs unit, regression, real-workspace integration, black-box
release CLI interoperability, dependency policy, and relevant benchmark gates.
The final evidence references immutable commits, CI runs, artifact hashes, tool
versions, fixture hashes, and any unsupported release-signing environment.
