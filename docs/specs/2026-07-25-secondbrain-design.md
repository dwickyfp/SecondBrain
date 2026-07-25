# SecondBrain — Product and System Design

**Status:** Approved design baseline  
**Date:** 2026-07-25  
**Repository:** `/Users/dwickyferiansyahputra/Public/Research/SecondBrain`  
**Target:** Desktop macOS, Windows, and Linux first; mobile later

## 1. Product Definition

SecondBrain is a local-first collaborative knowledge workspace for humans and agents, built on plain Markdown.

It serves solo users and organizations of any size in one product. Enterprise capabilities are not a separate edition: identity federation, governance, audit, retention, and scalable self-hosting are integrated capabilities that appear when configured and authorized.

### 1.1 Product principles

1. **Markdown-first:** ordinary `.md` files and ordinary attachment files remain durable, user-owned representations.
2. **Local-first:** reading and editing never depend on a server or internet connection.
3. **External-editor compatible:** VS Code, Git, scripts, Hermes, and other tools may edit files directly while collaboration is active.
4. **Convergent collaboration:** external, internal, remote-human, plugin, CLI, API, and agent edits merge through one transaction/CRDT engine; silent overwrite is forbidden.
5. **Agent-first:** agents are first-class, scoped principals using MCP/API/CLI with attribution, policy, preview, audit, and rollback.
6. **Private deployment:** local-only, peer-to-peer, or self-hosted operation; no managed cloud dependency.
7. **E2EE network:** servers and relays cannot read note contents, detailed audit payloads, attachment names, or attachment contents.
8. **Safe extensibility:** plugins run as capability-restricted WebAssembly components without ambient filesystem or network authority.
9. **Progressive complexity:** solo users encounter a simple note workspace; organization controls appear only when enabled and permitted.
10. **Correctness before scale:** every phase is gated by tests, recovery evidence, security checks, and benchmarks; P0/P1 defects are not deferred.

## 2. Approved Technology Direction

SecondBrain uses the Rust-first hybrid CRDT architecture.

| Layer | Choice |
|---|---|
| Desktop shell | Tauri 2 |
| Core engine | Rust |
| Frontend | Svelte 5 + TypeScript |
| WYSIWYG editor | TipTap over ProseMirror |
| Source editor | CodeMirror 6 |
| Local derived store | SQLite WAL + FTS5 |
| Collaboration | Per-note Loro 1.13.7 CRDT; selected by [ADR 0001](../adr/0001-select-production-crdt.md) |
| Self-hosted server | Rust + Axum |
| P2P transport | QUIC + mDNS; libp2p only if justified by spike |
| Plugin runtime | Wasmtime + WASI Component Model |
| Agent protocol | Native MCP plus CLI and local API |
| Organization metadata | PostgreSQL |
| Encrypted remote blobs | S3-compatible object storage or filesystem for small deployments |

Rust owns filesystem I/O, parsing and serialization, semantic diff, CRDT/oplog integration, indexing, cryptography, synchronization, policy, audit, MCP/API/CLI, and plugin execution. TypeScript owns user-interface behavior and editor integration, not duplicated domain logic.

## 3. System Architecture

```text
Desktop UI ─┐
CLI ────────┤
MCP ────────┼── Rust Core Transaction API
Local API ──┤         │
Plugin API ─┘         ├── Markdown AST + serializer
                      ├── semantic external-edit bridge
                      ├── per-note CRDT + workspace transactions
                      ├── SQLite index/search/graph
                      ├── policy + audit
                      ├── E2EE + identity
                      ├── P2P/self-hosted sync
                      └── WASM capability runtime
                               │
                    Plain Markdown + attachments
                    Portable oplog/snapshots/policies
```

No product surface may implement independent domain rules. Every mutating surface calls the same core transaction API.

## 4. Canonical State Model

SecondBrain distinguishes two complementary forms of truth:

- **Markdown is the canonical durable representation** for ownership, interoperability, direct inspection, backup, and recovery.
- **The signed CRDT operation log is the canonical concurrent history** for merging, attribution, versioning, and deterministic replay.

SQLite is derived and rebuildable. The self-hosted server is never the primary content store.

### 4.1 Workspace layout

```text
Workspace/
├── <user folders and Markdown notes>
├── Attachments/
└── .secondbrain/
    ├── manifest.toml
    ├── oplog/<note-id>/
    ├── transactions/
    ├── snapshots/
    ├── identity-map/
    ├── policies/
    ├── audit/
    └── plugins.lock
```

OS application data contains rebuildable or device-specific state:

```text
SecondBrain app data/
├── index.sqlite
├── attachment-chunks/
├── embeddings/
├── thumbnails/
├── device-state/
├── logs/
└── plugin-cache/
```

Secrets and private keys are stored in the OS keychain, never in the workspace.

### 4.2 Note identity

Every managed note receives a stable ULID in YAML frontmatter:

```markdown
---
id: 01K0W9M4QY8A7F2C6N5J3H1BXR
title: FoundationDB
type: knowledge
---
```

The identity map tracks current path, historical paths, content fingerprints, and recovery candidates. Rename, move, and title changes do not alter identity. Duplicate IDs created by copying a file are detected; the copy receives a new ID transactionally. If frontmatter is removed externally, identity recovery uses the sidecar map and history before considering the file new.

### 4.3 Block identity

Block identities stay in the sidecar CRDT map by default. Structural anchors and content fingerprints recover mappings after external rewrites. Explicit Markdown block IDs are created only when a block is directly referenced. This avoids polluting ordinary Markdown while retaining stable anchors for comments, references, and collaboration.

### 4.4 CRDT granularity

Each note has an independent CRDT document. Cross-note changes use a signed `WorkspaceTransaction` with actor, preconditions, operations, signature, and state. Examples include note rename plus backlink repair, folder moves, bulk property edits, and attachment rename plus reference repair.

Readers internal to SecondBrain see only committed version sets. Logical atomicity is provided even though ordinary filesystems do not provide atomic multi-file transactions.

## 5. Core Protocol Contracts

Primary envelopes:

- `DocumentOperation`
- `WorkspaceTransaction`
- `EncryptedSyncEnvelope`
- `AttachmentChunkManifest`
- `AuditEnvelope`
- `PolicyDecision`
- `DeviceAnnouncement`
- `SnapshotManifest`
- `Tombstone`
- `LegalHold`

Every envelope is versioned, canonically serialized, signed where applicable, replay-protected, safely decodable when truncated or corrupt, and independent of transport. Protocol compatibility covers at least two major client versions.

Final wire encoding—Protobuf versus postcard/CBOR—is selected through a spike using correctness, canonical encoding, forward compatibility, payload size, interoperability, and auditability criteria.

## 6. Edit and Materialization Flows

### 6.1 Internal editor

```text
TipTap transaction
→ Rust transaction validation
→ CRDT operations
→ signed operation append + fsync
→ audit event
→ index update
→ atomic Markdown materialization
→ encrypted peer/server synchronization
```

### 6.2 External file edit

```text
Filesystem event
→ internal-origin/hash detection
→ parse old and new Markdown
→ semantic AST diff
→ CRDT operations
→ rebase/merge with current state
→ signed audit attribution
→ deterministic rematerialization
```

Semantic diff recognizes headings, paragraphs, list items, tables, code blocks, properties, and supported raw regions. It must identify moves where possible rather than reducing every move to delete-plus-insert.

Unsupported syntax is preserved as lossless raw nodes. Ambiguous concurrent edits to unsupported/raw regions enter conflict review rather than being guessed.

## 7. Consistency and Conflict Semantics

Every write declares expected note versions, file hashes, and workspace epoch. If preconditions are stale, the engine rebases and semantically merges instead of overwriting.

| Concurrent change | Required behavior |
|---|---|
| Different paragraphs | Merge automatically |
| Concurrent list additions | Preserve both |
| Rename plus content edit | Merge rename and edit |
| Different properties | Merge automatically |
| Same scalar property | Attributed resolution plus conflict history |
| Git whole-file replacement during live editing | Parse and rebase as semantic operations |
| Ambiguous raw block changes | Conflict workspace |

Unresolvable conflicts open a four-way review surface: Base, Current, Incoming, and Proposed Merge. Users can resolve per hunk, edit manually, or ask an agent for a proposal requiring approval. Normal conflict handling never creates `Note (conflicted copy).md`.

## 8. Crash Safety and Recovery

### 8.1 Durable write order

```text
validate
→ append signed operation
→ fsync oplog
→ persist transaction state
→ write temporary Markdown
→ fsync temporary file
→ atomic rename
→ update derived index
→ mark committed
```

Cross-note transaction states:

```text
PREPARED → OPERATIONS_DURABLE → MATERIALIZING → COMMITTED
```

Startup recovery replays durable operations whose Markdown materialization is incomplete, rebuilds stale indices, quarantines corrupt/truncated payloads, rejects invalid signatures, and restores from the last valid snapshot when necessary.

### 8.2 Recovery hierarchy

```text
current valid operation log
→ last valid local snapshot
→ trusted peer snapshot
→ self-hosted encrypted backup
→ plain Markdown reconstruction
→ manual recovery bundle
```

If `.secondbrain/` is lost, Markdown and attachments remain readable. A new workspace identity and index can be created, although old collaboration history is unavailable.

## 9. Markdown Requirements

SecondBrain supports:

- CommonMark and GitHub Flavored Markdown;
- YAML frontmatter;
- wikilinks, heading links, block references, and embeds;
- callouts;
- tables and task lists;
- footnotes;
- fenced code blocks;
- Mermaid;
- KaTeX/LaTeX;
- raw HTML passthrough;
- ordinary attachments;
- practical Obsidian syntax compatibility.

The parser/serializer must preserve unknown constructs. WYSIWYG and Source switching must not create a change when content did not semantically change. Round-trip invariants are release gates.

## 10. Editor and Workspace UX

The desktop workspace has navigation, document workspace, and contextual panels. It supports resizable/collapsible panes, tabs, split panes, command palette, keyboard navigation, properties, backlinks, outline, comments, history, search, graph, saved queries, structured views, conflict resolution, and agent cowork.

Each document tab provides:

```text
WYSIWYG | Source | Split
```

TipTap/ProseMirror is the primary Typora-like editor. CodeMirror 6 edits actual source Markdown. Unsupported constructs appear as preservable raw nodes and remain editable in Source mode.

Layouts are per-device and never treated as portable workspace state. UI layout uses responsive flex/grid composition without hardcoded panel heights.

## 11. Search, Graph, and Structured Views

SQLite WAL + FTS5 provides full-text search. Derived indices include filenames, properties, tags, outgoing links, backlinks, broken links, orphan notes, graph edges, and saved-query metadata.

Built-in views include Table, Board, Calendar, Timeline, Gallery, List, and Graph. Editing a cell/card writes to standard YAML frontmatter through a transaction. There is no hidden database containing values that diverge from Markdown.

Structured operations include filtering, sorting, grouping, aggregation, formulas, relations, and rollups. Saved views use a declarative, versioned file format rather than arbitrary JavaScript.

Semantic/vector search is optional, local by default, and outside the first MVP.

## 12. Attachments

Attachments remain ordinary files referenced with human-readable Markdown paths. The sync layer builds a content-addressed chunk manifest and transmits encrypted resumable chunks.

- Local files remain directly openable by other applications.
- Deduplication happens in local object indices and encrypted transport storage.
- Names, paths, and MIME details remain inside encrypted manifests.
- Rename is transactional and repairs references.
- Chunks are purged only after retention permits and encrypted reference state reaches zero.

## 13. Synchronization Modes

### 13.1 Local-only

No network traffic and no account requirement.

### 13.2 Peer-to-peer

Device pairing uses QR code or a recovery/passphrase flow. LAN discovery uses mDNS; transport uses QUIC with authenticated encrypted sessions. Offline operations synchronize and converge when peers reconnect. NAT traversal is introduced only after a dedicated spike.

### 13.3 Self-hosted

The self-hosted system supplies encrypted operation relay, presence, encrypted attachment/snapshot storage, identity federation, organization policy, and server-visible audit envelopes. Editing remains available when it is offline.

Deployments:

- standalone binary;
- Docker Compose for single-node and small teams;
- Kubernetes Helm for high availability.

PostgreSQL stores identity mappings, envelope metadata, and visible audit fields. S3-compatible or filesystem storage holds ciphertext blobs. Redis is not required unless benchmarks demonstrate a need.

## 14. Security and Identity

### 14.1 Local data

Markdown remains plaintext for interoperability. Users rely on FileVault, BitLocker, or LUKS for local at-rest protection. The product clearly communicates this boundary.

### 14.2 Identity hierarchy

A local user identity key authorizes separate device keys. Private keys live in the OS keychain. New devices require approval from a trusted device or recovery kit. Revoked devices cannot decrypt future epochs.

Organizations map OIDC, SAML 2.0, or LDAP identities and groups to cryptographic user/device identities. OIDC ships first; SAML, LDAP, and SCIM arrive at the enterprise-ready milestone.

### 14.3 Workspace key hierarchy

A workspace root key derives/wraps purpose-specific and epoch-specific keys for documents, attachments, audit details, snapshots, and invitations. Compromise of one document key must not expose the whole workspace. Membership changes can advance epochs. Servers store only wrapped keys and ciphertext.

Approved standard primitives include XChaCha20-Poly1305 or AES-256-GCM, Ed25519, X25519, HKDF-SHA256, Argon2id, and OS CSPRNG. No custom cryptographic primitives are permitted.

### 14.4 Audit visibility

The self-hosted server may see:

- actor ID;
- device ID;
- workspace ID;
- action type;
- opaque target ID;
- timestamp;
- payload size;
- transaction ID;
- signature;
- policy decision.

Paths, titles, content diffs, comments, properties, attachment names/content, and detailed reasons remain E2EE. Authorized clients can decrypt detailed audit records.

### 14.5 Threat model

The design addresses compromised/curious relays, network attackers, replay/rollback attacks, malicious members, stolen locked devices, revoked devices, malicious plugins, overprivileged agents, corrupt payloads, path traversal and symlink escape, audit tampering, and dependency supply-chain compromise.

It does not claim protection from a fully compromised unlocked device, processes already running with the user's OS permissions, screen capture/keylogging, or absent disk encryption.

## 15. Authorization

Authorization combines RBAC, scoped capabilities, and policy conditions.

Baseline roles:

- Owner
- Admin
- Editor
- Commenter
- Viewer
- Guest
- Agent
- Auditor

Capabilities include workspace, note, comment, audit, agent, plugin, policy, deletion, purge, and key-management actions. Conditions may restrict path, note type, classification, time, managed-device state, network zone, actor type, plugin publisher, agent model/provider, and approval requirement.

## 16. Agent-First Interface

First-class surfaces:

1. Desktop UI
2. CLI
3. MCP
4. Local API
5. Rust SDK
6. Plugin SDK

Initial MCP tools:

- `vault_list`
- `vault_search`
- `note_read`
- `note_create`
- `note_patch`
- `note_move`
- `note_delete`
- `note_backlinks`
- `note_history`
- `query_execute`
- `context_bundle`
- `transaction_preview`
- `transaction_apply`
- `transaction_rollback`
- `agent_request_approval`

An agent is a distinct principal with workspace/path scope, capabilities, maximum transaction size, approval policy, session expiration, and optional model/provider constraints. Bulk edits are dry-run by default. Delete, purge, permissions, policy, and key operations require explicit human approval by default. Agents cannot raise their own privileges.

The UI represents agent edits as attributed change sets—not human cursors—and shows state, reason, scope, model/provider, diff, approval, and rollback controls.

## 17. WebAssembly Plugin Model

Plugins are WebAssembly components hosted by Wasmtime. They have no ambient filesystem, network, environment, process, keychain, or database access.

A signed manifest declares note/path read/write scopes, network endpoints, UI extension points, commands, notifications, clipboard access, search, and agent invocation. The host enforces CPU, memory, execution-time, and state quotas.

Every mutation goes through the transaction engine. Enterprises may require signed plugins, approved publishers, capability allowlists, and no-network/no-agent policies. New capabilities requested by an update require new approval. Plugin crashes cannot crash the host.

## 18. Deletion, Retention, and Legal Hold

Lifecycle:

```text
Active → Trash tombstone → retention window → eligible purge
                                  │
                                  └── legal hold blocks purge
```

Restore preserves note identity and history. Purge requires a separate capability and removes materialized Markdown, eligible snapshots, attachment chunks, and key references according to retention. Content-free audit envelopes may remain according to organization policy.

## 19. Git Integration

Git is an official backup/history adapter, not the collaboration engine.

- UI exposes status, diff, history, and checkpoints.
- Pull/checkout/merge changes enter as external mutation batches.
- Large changes receive semantic preview.
- The app never stages or commits automatically without user policy.
- Keys, secrets, caches, device state, and volatile workspace state are ignored.
- Oplog backup may be enabled, but manual merging of oplog files is unsupported.

## 20. Testing Strategy

### 20.1 Markdown tests

Use CommonMark/GFM corpora plus wikilinks, malformed frontmatter, nested structures, raw HTML, unknown syntax, mixed line endings, Unicode normalization, and very large files. Required invariant:

```text
parse → serialize → parse
```

preserves semantic state and raw unsupported regions.

### 20.2 CRDT and state-machine tests

Property tests generate operations across many actors, reorder/duplicate/delay delivery, branch offline, and exercise text, formatting, moves, splits/joins, properties, and deletion. Every replica and Markdown materialization must converge.

### 20.3 External-edit tests

Test editor save patterns, atomic rename-save, Git checkout/merge, formatter rewrites, copying/duplicate IDs, simultaneous external and remote edits, watcher duplicates, partial writes, and delayed cloud/antivirus events.

### 20.4 Crash injection

Kill processes before and after each oplog append, fsync, snapshot, temporary write, rename, index update, and cross-note materialization boundary. Confirmed changes may not disappear; unconfirmed transactions may not appear committed.

### 20.5 Authorization and security tests

Property-test actor × role × capability × resource × condition × action. Fuzz parsers, wire decoders, oplogs, and sync state machines. Verify replay rejection, rotation/revocation, corrupt chunks, interrupted transfer, encrypted backup restoration, and server inability to decrypt payloads.

### 20.6 Desktop and accessibility tests

Use Vitest for frontend units, Playwright for E2E, accessibility automation plus manual screen-reader/keyboard testing, and real signed builds on macOS, Windows, and Linux.

## 21. Performance Requirements

| Operation | Target |
|---|---:|
| Cold start small workspace | <1 second |
| Cold start 10,000 notes | <3 seconds |
| FTS search p95 | <100 ms |
| Open note p95 | <50 ms |
| Local keystroke latency | <16 ms |
| Local materialization after debounce | <100 ms |
| Local collaboration update | <50 ms |
| Remote collaboration p95, normal network | <250 ms |
| Idle desktop memory | <250 MB |

Benchmark corpora include 1K, 10K, and 100K notes; 1M links; 10 GB attachments; 100 collaborators; and 100K operations in an active note. Benchmarks run on all three desktop operating systems and gate regressions.

## 22. Observability and Privacy

Clients produce local structured logs, performance traces, index/sync/transaction health, plugin resource usage, and an inspectable diagnostic bundle. No telemetry is sent by default; external submission is explicit.

Servers expose OpenTelemetry, Prometheus metrics, JSON logs, health/readiness endpoints, queue/storage/peer metrics, and content-free audit export. Logs must never contain plaintext content or key material.

## 23. Release Plan

### Phase 0 — Core correctness

Markdown AST and lossless serializer, semantic diff, atomic writes, filesystem watcher, operation log, SQLite index, CLI, properties and links.

### Phase 1 — Single-user desktop

Tauri/Svelte app, WYSIWYG/Source/Split editor, tabs/panes, search, backlinks, daily notes, properties, graph, and Obsidian-compatible vault import.

### Phase 2 — Agent-first

MCP, complete CLI, transaction preview/diff/approval, agent identity and scopes, audit, rollback, and context bundles.

### Phase 3 — Multi-device P2P

Device identity/pairing, E2EE operations and attachment chunks, offline merge, key rotation/revocation, and LAN discovery.

### Phase 4 — Team collaboration

Presence, cursors, comments, suggestions, invitations, RBAC, selective sync, and self-hosted relay.

### Phase 5 — Organization foundation

OIDC, custom roles, audit export, retention/legal hold, backup/disaster recovery, and policy management.

### Phase 6 — Plugin platform

Wasmtime component runtime, capabilities, SDK, signing, organization catalog, and resource controls.

### Phase 7 — Mobile

iOS and Android, quick capture, selective loading, background sync, offline operation, and platform-specific plugin restrictions.

### Enterprise-ready quality milestone

SAML, LDAP, SCIM, HA deployment, advanced audit, policy packs, signed-plugin governance, and long-term support policy. This remains the same product, not a separate enterprise edition.

## 24. MVP Boundary

The first verifiable MVP includes:

- single-user desktop;
- existing Markdown workspace;
- WYSIWYG, Source, and Split modes;
- lossless Markdown round-trip;
- atomic writes and external-edit detection;
- SQLite FTS/backlinks;
- transaction history;
- MCP read/write with diff preview;
- agent attribution and audit;
- locally integrated winner of the Loro-vs-Yrs spike.

The MVP excludes hosted relay, multi-user UI, P2P NAT traversal, OIDC/SAML/LDAP, public plugin marketplace, mobile, semantic search, and legal hold. Their architectural contracts remain defined so MVP choices do not block later phases.

## 25. Mandatory CRDT Spike

Implement throwaway Loro and Yrs prototypes against identical acceptance tests:

- concurrent text edits;
- paragraph/list moves;
- TipTap position mapping;
- external whole-file rewrites;
- offline merge;
- per-actor undo;
- snapshots and compaction;
- deterministic replay;
- truncated-log recovery;
- 100K operations;
- memory usage;
- serialized size;
- Rust/TypeScript bridge ergonomics.

Select one production engine. Do not maintain a permanent dual-engine abstraction or lowest-common-denominator API.

## 26. Security and Stable-Release Gates

Before stable enterprise-capable release:

- documented threat model;
- public deterministic protocol test vectors;
- SBOM and dependency/license scanning;
- fuzzing of parsers, protocol decoders, and sync state machines;
- authorization property tests;
- plugin sandbox penetration test;
- independent cryptography review;
- independent application security audit;
- key-rotation and disaster-recovery drills;
- signed/attested release pipeline;
- signed desktop binaries, server images, and update metadata;
- remediation of all P0/P1 findings.

## 27. Definition of Done

A phase is complete only when:

1. every acceptance requirement has automated tests or documented manual evidence;
2. unit, integration, E2E, lint, and security checks pass;
3. benchmark budgets show no unacceptable regression;
4. migrations, operations, and user documentation exist;
5. failure and recovery paths have been exercised, not merely described;
6. no P0/P1 issue is deferred;
7. artifacts build and run on required target platforms;
8. verification evidence is recorded;
9. architecture deviations are documented as ADRs.

## 28. Explicit Non-Goals for Early Phases

Do not build managed cloud hosting, a public marketplace, bundled local LLMs, email/calendar servers, video/audio editing, blockchain identity, custom cryptography, a complete browser editor, or mobile clients before the desktop correctness and synchronization foundations pass their gates.

## 29. Architectural Invariants

1. Markdown and attachments remain directly user-readable files.
2. A stable note ID survives rename and move.
3. CRDT is per note; cross-note changes use signed transaction envelopes.
4. External edits are semantic operations, never silent overwrites.
5. SQLite is rebuildable and never the canonical note store.
6. Secrets/private keys never enter the workspace.
7. Network payloads and detailed audits are E2EE.
8. Servers see only the minimum approved audit envelope.
9. All human, agent, plugin, CLI, API, and filesystem mutations converge through one transaction engine.
10. Plugins have no ambient authority.
11. Unsupported Markdown survives round-trip.
12. Delete follows Trash/retention/legal-hold semantics.
13. Cache loss cannot make Markdown inaccessible.
14. Confirmed edits survive process and power failure.
15. Production CRDT and wire encoding are selected by evidence-producing spikes.

## 30. Open Decisions Reserved for Spikes

These are deliberately bounded engineering decisions rather than unresolved product requirements:

1. Loro versus Yrs: resolved in favor of Loro 1.13.7 by [ADR 0001](../adr/0001-select-production-crdt.md); production integration remains a separate task.
2. Protobuf versus postcard/CBOR wire encoding.
3. Exact P2P/NAT traversal implementation beyond LAN QUIC/mDNS.
4. XChaCha20-Poly1305 versus AES-256-GCM after platform/audit evaluation.

Each spike must produce reproducible tests, measurements, a written ADR, and a single selected production path.
