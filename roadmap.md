# SecondBrain — Roadmap

> **Repository:** `~/Public/Research/SecondBrain` · [github.com/dwickyfp/SecondBrain](https://github.com/dwickyfp/SecondBrain) (private)
> **Design spec:** `docs/specs/2026-07-25-secondbrain-design.md`
> **Phase 0 plan:** `docs/plans/2026-07-25-phase-0-core-and-crdt-spike.md`
> **Last updated:** 2026-07-26

---

## Overview

SecondBrain is a local-first collaborative knowledge workspace for humans and agents, built on plain Markdown. Rust-first hybrid CRDT architecture. Plain `.md` files are the durable canonical representation; `.secondbrain/` holds portable identity/transaction state; SQLite is derived and rebuildable.

**Tech stack:** Rust 2024 · Tauri 2 · Svelte 5 + TypeScript · TipTap/ProseMirror · CodeMirror 6 · SQLite WAL + FTS5 · Per-note CRDT (Loro vs Yrs, spike pending) · Axum self-hosted · QUIC + mDNS P2P · Wasmtime WASM plugins · Native MCP.

---

## Progress Summary

| Metric | Value |
|---|---|
| **Phase 0 tasks completed** | 30 / 30; local and cross-platform CI evidence complete |
| **Tests passing** | 348+ |
| **Crates** | 6 (`cli`, `core`, `markdown`, `vault`, `index`, `transaction`) |
| **CI** | macOS · Windows · Linux |
| **Current commit** | `ac9b0c8` fix(reconcile): preserve note identity across real vault flows |

### Milestone completion

| Milestone | Tasks | Status |
|---|---|---|
| A — Repository and domain foundations | 1–4 | ✅ Done |
| B — Loss-aware Markdown and stable note identity | 5–9 | ✅ Done |
| C — Durable filesystem, identity map, and transactions | 10–15 | ✅ Done |
| D — Derived SQLite index and search | 16–18 | ✅ Done |
| E — Filesystem watcher and external-edit pipeline | 19–20 | ✅ Done |
| F — Phase 0 CLI and end-to-end verification | 21–22 | ✅ Done |
| G — Mandatory Loro vs Yrs spike | 23–27 | ✅ Done |
| H — Phase 0 closure | 28–30 | ✅ Done |

---

## Phase 0 — Core Correctness

**Goal:** Loss-aware Markdown handling, stable note identity, semantic external edits, durable transactions, crash recovery, SQLite indexing, CLI access, and an evidence-based Loro-vs-Yrs decision.

**Scope:** No Tauri/Svelte, TipTap, MCP, P2P, E2EE, enterprise identity, or WASM plugins.

### Milestone A — Repository and domain foundations

| # | Task | Commit | Status |
|---|---|---|---|
| 1 | Bootstrap Rust workspace and quality gates | `43e68cd` | ✅ |
| 2 | Define strongly typed IDs, paths, hashes, actors, versions | `6e80fcb` | ✅ |
| 3 | Define error taxonomy and stable diagnostic codes | `6412227` | ✅ |
| 4 | Implement workspace manifest creation and validation | `87e34e6` | ✅ |

**Deliverables:** Cargo workspace with 5 crates, pinned toolchain (1.91.1), shared deps, CI on 3 platforms, `cargo deny`, typed domain primitives (`NoteId`, `WorkspacePath`, `ContentHash`, etc.), stable error codes (`SB-*`), `.secondbrain/manifest.toml`.

### Milestone B — Loss-aware Markdown and stable note identity

| # | Task | Commit | Status |
|---|---|---|---|
| 5 | Build loss-aware Markdown source model | `0ba498b` | ✅ |
| 6 | Parse and patch YAML frontmatter without rewriting body | `c5cd389` | ✅ |
| 7 | Extract wikilinks, tags, headings, tasks, properties | `1eb45d3` | ✅ |
| 8 | Add Markdown round-trip corpus and property tests | `8278b79` | ✅ |
| 9 | Implement semantic Markdown operations and diff | `893a321` | ✅ |

**Deliverables:** `SourceDocument` with positioned semantic nodes, byte-exact round-trip preservation, surgical frontmatter patching, `ExtractedNote` (links/tags/headings/tasks/properties), `SemanticOperation` enum with `NeedsReview` for ambiguous edits, CommonMark/GFM/Obsidian fixture corpus, proptest generators.

### Milestone C — Durable filesystem, identity map, and transactions

| # | Task | Commit | Status |
|---|---|---|---|
| 10 | Workspace-root confinement and atomic file writing | `07ccbe7` | ✅ |
| 11 | Portable identity map and duplicate-ID recovery | `c642479` | ✅ |
| 12 | Define versioned local mutation-journal records | `ee9a728` | ✅ |
| 13 | Implement append-only per-note local mutation journals | `3d08f5d` | ✅ |
| 14 | Transaction state machine and single-note commit pipeline | `ce65a88` | ✅ |
| 15 | Crash injection and startup recovery | `af7ccb0` | ✅ |

**Deliverables:** `WorkspaceRoot` with confinement + atomic rename + parent sync, `IdentityMap` (path history, fingerprint recovery, duplicate detection), `sb-local-oplog-v1` hash-chained journal with CRC corruption detection, `PREPARED → OPERATIONS_DURABLE → MATERIALIZING → COMMITTED` state machine, real process-crash injection with 20× deterministic recovery, evidence at `docs/evidence/phase-0-crash-recovery.md`.

### Milestone D — Derived SQLite index and search

| # | Task | Commit | Status |
|---|---|---|---|
| 16 | Create SQLite schema, migrations, and rebuild metadata | `8a4c620` | ✅ |
| 17 | Implement full workspace indexing and deterministic rebuild | `059e8f7` | ✅ |
| 18 | Implement search, backlinks, broken links, and orphan queries | `6e9e148` | ✅ |

**Deliverables:** WAL-mode SQLite with FTS5, tables (notes/paths/properties/links/tags/headings/tasks/index_state), atomic index rebuild into temp file → swap, `IndexReport` (indexed/skipped/warning/error/orphan/broken-link counts), typed query APIs (`search`, `backlinks`, `broken_links`, `orphans`), deterministic ordering by path → note ID.

### Milestone E — Filesystem watcher and external-edit pipeline

| # | Task | Commit | Status |
|---|---|---|---|
| 19 | Normalize and debounce filesystem events | `989d60f` | ✅ |
| 20 | Integrate external file changes with semantic transactions | `d50cbe3`–`ac9b0c8` | ✅ |

**Task 20 scope:** `ExternalEditCoordinator` that turns normalized watcher events into attributed transactions. Tests: external paragraph edit → actor `external:<device>`, simultaneous internal merge, stale base rebase, ambiguous raw-block → `NeedsReview` conflict file, external rename preserves ID, external copy gets new ID, incremental index refresh, self-write loop suppression.

### Milestone F — Phase 0 CLI and end-to-end verification

| # | Task | Status |
|---|---|---|
| 21 | Build the `secondbrain` CLI surface | ✅ |
| 22 | Add end-to-end Phase 0 scenario | ✅ |

**Task 21 scope:** `secondbrain` binary with commands: `init`, `validate`, `index rebuild`, `search`, `note inspect`, `diff`, `transaction apply`, `recovery check`, `doctor`. JSON output with `--json`, stable exit codes, no ANSI when piped. CLI calls library APIs only.

**Task 22 scope:** Full E2E against Obsidian-compatible fixture vault — init → index → search → external edit → crash → recovery → index rebuild → compare. Evidence at `docs/evidence/phase-0-e2e.md`.

### Milestone G — Mandatory Loro vs Yrs CRDT spike

| # | Task | Status |
|---|---|---|
| 23 | Define black-box CRDT acceptance contract | ✅ |
| 24 | Implement and test Loro candidate | ✅ |
| 25 | Implement and test Yrs candidate | ✅ |
| 26 | Benchmark both candidates reproducibly | ✅ |
| 27 | Select CRDT winner and write ADR | ✅ Loro |

**Spike acceptance scenarios:** concurrent text edits, paragraph/list moves, TipTap position mapping, external whole-file rewrites, offline merge, per-actor undo, snapshots/compaction, deterministic replay, truncated-log recovery, 100K operations, memory/serialized-size measurement, Rust/TS bridge ergonomics. One engine wins; no permanent dual-engine abstraction.

### Milestone H — Phase 0 closure

| # | Task | Status |
|---|---|---|
| 28 | Integrate selected CRDT into production transaction core | ✅ |
| 29 | Add cross-platform CI fixtures and platform-specific guards | ✅ CI matrix passed |
| 30 | Run Phase 0 release gates and publish verification evidence | ✅ CI matrix and artifacts published |

**Phase 0 acceptance checklist** (all must be supported by real evidence):

- [x] Existing Markdown opened without destructive migration
- [x] Unchanged parse/serialize byte-identical across corpus
- [x] Unknown/raw syntax survives
- [x] Stable note IDs survive rename and missing-frontmatter recovery
- [x] Duplicate note IDs detected and repaired transactionally
- [x] External edits become deterministic semantic operations
- [x] Ambiguous edits enter review, never silently overwrite
- [x] Writes are root-confined and atomic
- [x] Oplog replay detects corruption and truncation
- [x] Confirmed operations survive every injected crash boundary
- [x] SQLite can be deleted and rebuilt to same logical state
- [x] Search, backlinks, broken links, and orphans verified
- [x] Watcher events don't create self-write loops
- [x] CLI exercises same library paths as future product surfaces
- [x] Loro and Yrs run identical correctness scenarios
- [x] One CRDT selected through ADR; loser not in production
- [x] Selected per-note CRDT integrated into transaction/recovery path
- [x] Provisional converged-base state has one idempotent migration path
- [x] macOS, Windows, and Linux CI pass
- [x] Formatting, clippy, tests, dependency policy clean
- [x] No P0/P1 issue deferred

---

## Phase 1 — Single-User Desktop

**Goal:** Tauri/Svelte app with WYSIWYG/Source/Split editor, tabs/panes, search, backlinks, daily notes, properties, graph, and Obsidian-compatible vault import.

**Plan:** `docs/plans/2026-07-26-phase-1-single-user-desktop.md`

| Task | Deliverable | Status |
|---:|---|:---:|
| 31 | Define Phase 1 milestones and acceptance gates | ✅ |
| 32 | Create Tauri 2 + Svelte 5 desktop shell | ✅ |
| 33 | Add typed workspace-open and search backend contract | ✅ |
| 34 | Gate the desktop shell in CI | ✅ |
| 35 | Add note read contract and source rendering | ✅ |
| 36 | Add tabs, split panes, and navigation history | ✅ |
| 37 | Add outline and backlinks panels | ✅ local |
| 38 | Add keyboard navigation and command palette foundation | ✅ local |
| 39 | Specify TipTap/ProseMirror transaction adapter | ⬜ |
| 40 | Implement WYSIWYG editor | ⬜ |
| 41 | Implement CodeMirror source editor | ⬜ |
| 42 | Add WYSIWYG/source/split synchronization | ⬜ |
| 43 | Enforce editor latency and Markdown-survival gates | ⬜ |
| 44 | Add properties editing | ⬜ |
| 45 | Add daily notes workflow | ⬜ |
| 46 | Add graph view | ⬜ |
| 47 | Add Obsidian-compatible vault import | ⬜ |
| 48 | Add accessibility and recovery UX acceptance suite | ⬜ |
| 49 | Enforce cold-start, open-note, search, and memory budgets | ⬜ |
| 50 | Produce desktop installers and Phase 1 evidence | ⬜ |

**Tasks 32–35 verification:** CI run [`30191106041`](https://github.com/dwickyfp/SecondBrain/actions/runs/30191106041) passed the desktop frontend, desktop backend on macOS/Windows/Linux, core quality matrix, and both dependency-policy checks for commit `ed1c01f`.

**Task 36 verification:** CI run [`30191974390`](https://github.com/dwickyfp/SecondBrain/actions/runs/30191974390) passed the typed navigation tests, responsive desktop frontend build, desktop backend matrix, core quality matrix, and dependency policies for commit `e62b2ca`.

**Key deliverables:**
- Tauri 2 desktop shell (macOS, Windows, Linux)
- Svelte 5 + TypeScript frontend
- TipTap/ProseMirror WYSIWYG editor (Typora-like)
- CodeMirror 6 source editor
- Tab/split-pane navigation, command palette, keyboard nav
- Properties panel, backlinks panel, outline, search, graph view
- Daily notes
- Obsidian vault import
- Performance gates: cold start <1s (small) / <3s (10K notes), FTS p95 <100ms, open note p95 <50ms, keystroke latency <16ms, idle memory <250MB

**Depends on:** Phase 0 complete (CRDT integrated, CLI verified).

---

## Phase 2 — Agent-First

**Goal:** MCP server, complete CLI, transaction preview/diff/approval, agent identity and scopes, audit, rollback, and context bundles.

**Key deliverables:**
- Native MCP tools: `vault_list`, `vault_search`, `note_read`, `note_create`, `note_patch`, `note_move`, `note_delete`, `note_backlinks`, `note_history`, `query_execute`, `context_bundle`, `transaction_preview`, `transaction_apply`, `transaction_rollback`, `agent_request_approval`
- Agent as distinct principal with workspace/path scope, capabilities, max transaction size, approval policy, session expiration, model/provider constraints
- Bulk edits dry-run by default; delete/purge/permissions/policy/key operations require explicit human approval
- UI represents agent edits as attributed change sets (not human cursors) with diff, approval, rollback controls
- Audit trail with E2EE detailed records

**Depends on:** Phase 1 desktop shell.

---

## Phase 3 — Multi-Device P2P

**Goal:** Device identity/pairing, E2EE operations and attachment chunks, offline merge, key rotation/revocation, LAN discovery.

**Key deliverables:**
- Device pairing via QR code or recovery passphrase
- mDNS LAN discovery + QUIC authenticated encrypted transport
- Per-note CRDT sync with offline convergence
- Workspace key hierarchy (root key → purpose/epoch-specific keys)
- XChaCha20-Poly1305 or AES-256-GCM (spike decision)
- Ed25519 signatures, X25519 key exchange, HKDF-SHA256, Argon2id
- Key rotation/revocation: revoked devices can't decrypt future epochs
- Encrypted attachment chunk sync (content-addressed, resumable)
- NAT traversal spike (post-LAN)

**Depends on:** Phase 2 (agent infrastructure + signed operations).

---

## Phase 4 — Team Collaboration

**Goal:** Presence, cursors, comments, suggestions, invitations, RBAC, selective sync, and self-hosted relay.

**Key deliverables:**
- Real-time presence and shared cursors
- Threaded comments and suggestions
- Invitation and membership management
- RBAC roles: Owner, Admin, Editor, Commenter, Viewer, Guest, Agent, Auditor
- Scoped capabilities + policy conditions (path, note type, classification, time, device, network zone, actor type, plugin publisher, agent model/provider, approval requirement)
- Selective sync per device/folder
- Self-hosted Rust + Axum relay server (encrypted operation relay, presence, attachment/snapshot storage)
- Docker Compose single-node deployment

**Depends on:** Phase 3 (P2P + E2EE foundation).

---

## Phase 5 — Organization Foundation

**Goal:** OIDC, custom roles, audit export, retention/legal hold, backup/disaster recovery, and policy management.

**Key deliverables:**
- OIDC identity federation (first IdP)
- Custom roles and policy packs
- Audit export (content-free server-visible + E2EE detailed)
- Deletion lifecycle: Active → Trash tombstone → retention window → purge
- Legal hold blocks purge
- Backup/disaster recovery (encrypted snapshots, trusted peer restore, self-hosted encrypted backup)
- PostgreSQL for identity mappings, envelope metadata, audit fields
- S3-compatible or filesystem ciphertext blob storage
- Kubernetes Helm for HA

**Depends on:** Phase 4 (team collaboration + RBAC).

---

## Phase 6 — Plugin Platform

**Goal:** Wasmtime component runtime, capabilities, SDK, signing, organization catalog, and resource controls.

**Key deliverables:**
- Wasmtime + WASI Component Model plugin runtime
- No ambient filesystem/network/environment/process/keychain/database access
- Signed plugin manifests declaring scopes, endpoints, UI extensions, commands, notifications, clipboard, search, agent invocation
- CPU/memory/time/state quotas enforced by host
- All mutations through transaction engine
- Enterprise: signed plugins, approved publishers, capability allowlists, no-network/no-agent policies
- Plugin SDK (Rust + TypeScript)
- Organization plugin catalog
- Plugin crashes cannot crash host

**Depends on:** Phase 5 (organization policy foundation).

---

## Phase 7 — Mobile

**Goal:** iOS and Android, quick capture, selective loading, background sync, offline operation, and platform-specific plugin restrictions.

**Key deliverables:**
- iOS and Android native apps
- Quick capture
- Selective loading (workspace subsets)
- Background sync
- Full offline operation
- Platform-specific plugin restrictions (stricter than desktop)

**Depends on:** Phase 6 (plugin platform) + Phase 3 (sync foundation).

---

## Enterprise-Ready Quality Milestone

**Goal:** SAML, LDAP, SCIM, HA deployment, advanced audit, policy packs, signed-plugin governance, and long-term support policy. Same product, not a separate edition.

**Security gates before stable release:**
- Documented threat model
- Public deterministic protocol test vectors
- SBOM and dependency/license scanning
- Fuzzing of parsers, protocol decoders, sync state machines
- Authorization property tests
- Plugin sandbox penetration test
- Independent cryptography review
- Independent application security audit
- Key-rotation and disaster-recovery drills
- Signed/attested release pipeline
- Signed desktop binaries, server images, update metadata
- All P0/P1 findings remediated

---

## Architectural Invariants (never violated)

1. Markdown and attachments remain directly user-readable files
2. Stable note ID survives rename and move
3. CRDT is per-note; cross-note changes use signed transaction envelopes
4. External edits are semantic operations, never silent overwrites
5. SQLite is rebuildable, never the canonical note store
6. Secrets/private keys never enter the workspace
7. Network payloads and detailed audits are E2EE
8. Servers see only the minimum approved audit envelope
9. All mutations converge through one transaction engine
10. Plugins have no ambient authority
11. Unsupported Markdown survives round-trip
12. Delete follows Trash/retention/legal-hold semantics
13. Cache loss cannot make Markdown inaccessible
14. Confirmed edits survive process and power failure
15. Production CRDT and wire encoding selected by evidence-producing spikes

---

## Open Decisions (resolved by spikes)

| # | Decision | Method |
|---|---|---|
| 1 | Loro vs Yrs | Phase 0 Task 23-27 spike + ADR |
| 2 | Protobuf vs postcard/CBOR wire encoding | Dedicated spike (Phase 3) |
| 3 | P2P/NAT traversal beyond LAN QUIC/mDNS | Dedicated spike (Phase 3) |
| 4 | XChaCha20-Poly1305 vs AES-256-GCM | Platform/audit evaluation (Phase 3) |

---

## Performance Budgets

| Operation | Target |
|---|---:|
| Cold start (small workspace) | <1s |
| Cold start (10,000 notes) | <3s |
| FTS search p95 | <100ms |
| Open note p95 | <50ms |
| Local keystroke latency | <16ms |
| Local materialization after debounce | <100ms |
| Local collaboration update | <50ms |
| Remote collaboration p95 (normal network) | <250ms |
| Idle desktop memory | <250MB |

Benchmark corpora: 1K / 10K / 100K notes, 1M links, 10GB attachments, 100 collaborators, 100K operations in active note. Benchmarks run on all three desktop OSes and gate regressions.
