# SecondBrain Phase 0 Core and CRDT Spike Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task. Require spec-compliance review first and code-quality review second for every task.

**Goal:** Build and verify the Phase 0 Rust foundation for loss-aware Markdown handling, stable note identity, semantic external edits, durable transactions, crash recovery, SQLite indexing, CLI access, and an evidence-based Loro-versus-Yrs decision.

**Architecture:** A Cargo workspace separates domain types, Markdown processing, filesystem durability, indexing, transaction orchestration, CLI, and throwaway CRDT candidates. Plain Markdown remains the durable representation; `.secondbrain/` contains portable identity and transaction state; SQLite remains derived and rebuildable. The CRDT spike uses a shared black-box contract without introducing a permanent dual-engine abstraction into production crates.

**Tech Stack:** Rust 2024, `markdown` 1.0.0, `serde` 1.0.229, `serde_yaml`, `toml`, `ulid` 3.0.0, `blake3`, `crc32fast`, `tempfile` 3.27.0, `notify` 8.2.0 stable, `rusqlite` 0.40.1 with bundled SQLite, `clap`, `thiserror`, `tracing`, `proptest` 1.11.0, `criterion` 0.8.2, Loro 1.13.7, Yrs 0.27.3.

**Approved design:** `docs/specs/2026-07-25-secondbrain-design.md`

---

## Scope and execution rules

This plan covers Phase 0 and the CRDT spike only. It does not scaffold Tauri/Svelte, TipTap, MCP, P2P, E2EE, enterprise identity, or WASM plugins.

Implementation rules:

1. Use TDD: write the test, observe the expected failure, implement minimally, observe success.
2. Commit after every task.
3. Keep production crates independent of both Loro and Yrs until the spike ADR selects a winner.
4. Do not invent the final network wire protocol. Phase 0 local records use an explicitly versioned, hash-chained but unsigned development format that cannot be synchronized or treated as a trusted remote record. Device-key signing is mandatory before Phase 3 and must replace/migrate this format; this bounded deferral avoids fake key storage before the approved identity/keychain work exists.
5. Never normalize or rewrite an unchanged Markdown note.
6. Never write outside the canonicalized workspace root.
7. Do not claim a task complete without real command output.
8. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` at every milestone.

## Planned repository layout

```text
SecondBrain/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── deny.toml
├── crates/
│   ├── secondbrain-core/
│   ├── secondbrain-markdown/
│   ├── secondbrain-vault/
│   ├── secondbrain-index/
│   ├── secondbrain-transaction/
│   └── secondbrain-cli/
├── spikes/
│   └── crdt-comparison/
│       ├── contract/
│       ├── loro-candidate/
│       ├── yrs-candidate/
│       └── benches/
├── fixtures/
│   ├── markdown/
│   ├── external-edits/
│   └── crash-recovery/
├── docs/
│   ├── adr/
│   ├── evidence/
│   ├── plans/
│   └── specs/
└── .github/workflows/ci.yml
```

---

## Milestone A — Repository and domain foundations

### Task 1: Bootstrap the Rust workspace and quality gates

**Objective:** Create a compilable multi-crate workspace with pinned toolchain, shared dependency versions, formatting, lint, test, and dependency-policy commands.

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `deny.toml`
- Create: `crates/secondbrain-core/Cargo.toml`
- Create: `crates/secondbrain-core/src/lib.rs`
- Create: `.github/workflows/ci.yml`
- Create: `README.md`

**Step 1: Write the workspace smoke test**

Create `crates/secondbrain-core/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_smoke_test() {
        assert_eq!(env!("CARGO_PKG_NAME"), "secondbrain-core");
    }
}
```

**Step 2: Add workspace configuration**

Root `Cargo.toml` must declare resolver 3, edition 2024, MSRV equal to the pinned stable toolchain, `crates/secondbrain-core` as the initial member, and shared dependencies. Use stable `notify = "8.2.0"`, not the 9.0 release candidate. Do not list crates that do not exist yet: each later crate-creation task must add that crate to `workspace.members` in the same commit. Add `crdt-spike-contract` in Task 23, `loro-candidate` in Task 24, `yrs-candidate` in Task 25, and the benchmark crate in Task 26 only after each manifest exists.

Required profiles:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"

[profile.test]
debug = 1
```

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.91.1"
components = ["clippy", "rustfmt"]
profile = "minimal"
```

**Step 3: Run the initial gates**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Expected: all commands exit 0; one smoke test passes.

**Step 4: Add CI**

CI must run on `macos-latest`, `windows-latest`, and `ubuntu-latest`, cache Cargo artifacts, and execute the three commands above. Add `cargo deny check` as a Linux job after installing `cargo-deny`.

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore deny.toml README.md crates .github

git commit -m "chore: bootstrap SecondBrain Rust workspace"
```

---

### Task 2: Define strongly typed IDs, paths, hashes, actors, and versions

**Objective:** Establish domain primitives that prevent accidental mixing of note, workspace, transaction, actor, and version values.

**Files:**
- Create: `crates/secondbrain-core/src/id.rs`
- Create: `crates/secondbrain-core/src/path.rs`
- Create: `crates/secondbrain-core/src/hash.rs`
- Create: `crates/secondbrain-core/src/actor.rs`
- Modify: `crates/secondbrain-core/src/lib.rs`
- Test: colocated unit tests

**Step 1: Write failing tests**

Cover:

```rust
#[test]
fn note_id_round_trips_as_canonical_ulid() {
    let id = NoteId::new();
    assert_eq!(id.to_string().parse::<NoteId>().unwrap(), id);
}

#[test]
fn workspace_path_rejects_parent_escape() {
    assert!(WorkspacePath::parse("../secret.md").is_err());
}

#[test]
fn content_hash_changes_when_content_changes() {
    assert_ne!(ContentHash::of(b"a"), ContentHash::of(b"b"));
}
```

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-core -- --nocapture
```

Expected: compile failure because types do not exist.

**Step 3: Implement minimal domain types**

Required types:

```rust
pub struct WorkspaceId(Ulid);
pub struct NoteId(Ulid);
pub struct TransactionId(Ulid);
pub struct ActorId(String);
pub struct DeviceId(String);
pub struct WorkspaceEpoch(u64);
pub struct NoteVersion(u64);
pub struct ContentHash([u8; 32]);
pub struct WorkspacePath(PathBuf);
```

Requirements:

- `WorkspacePath` only accepts normalized relative UTF-8 paths.
- Reject absolute paths, `..`, empty path, NUL, and `.secondbrain` as a user note prefix.
- Hash uses BLAKE3 over exact source bytes.
- IDs implement `Display`, `FromStr`, `Serialize`, `Deserialize`, `Clone`, `Eq`, `Ord`, and `Hash` where meaningful.
- `ActorId` and `DeviceId` reject blank or control-character values.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-core
cargo clippy -p secondbrain-core --all-targets -- -D warnings
```

Expected: all tests pass and no warnings.

**Step 5: Commit**

```bash
git add crates/secondbrain-core

git commit -m "feat(core): add typed workspace domain primitives"
```

---

### Task 3: Define error taxonomy and stable diagnostic codes

**Objective:** Provide actionable, machine-readable errors shared by CLI, future MCP, desktop, and logs.

**Files:**
- Create: `crates/secondbrain-core/src/error.rs`
- Modify: `crates/secondbrain-core/src/lib.rs`
- Test: `crates/secondbrain-core/tests/error_contract.rs`

**Step 1: Write failing contract tests**

Test that every error exposes a stable code and source chaining:

```rust
#[test]
fn stale_precondition_has_stable_code() {
    let error = Error::StalePrecondition {
        resource: "note:01ABC".into(),
        expected: "4".into(),
        actual: "5".into(),
    };
    assert_eq!(error.code(), "SB-TXN-STALE-PRECONDITION");
}
```

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-core --test error_contract
```

Expected: compile failure for missing `Error`.

**Step 3: Implement the taxonomy**

At minimum:

- `InvalidId`
- `InvalidWorkspacePath`
- `WorkspaceEscape`
- `InvalidMarkdown`
- `UnsupportedEncoding`
- `DuplicateNoteId`
- `StalePrecondition`
- `TransactionState`
- `CorruptRecord`
- `SignatureInvalid` reserved for later use
- `Io`
- `Sqlite`

Expose `type Result<T> = std::result::Result<T, Error>` and stable `code()` strings. Do not leak full file contents in `Display`.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-core
```

Expected: all tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-core

git commit -m "feat(core): define stable error and diagnostic contract"
```

---

### Task 4: Implement workspace manifest creation and validation

**Objective:** Initialize and validate `.secondbrain/manifest.toml` without touching existing Markdown content.

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/secondbrain-vault/Cargo.toml`
- Create: `crates/secondbrain-vault/src/lib.rs`
- Create: `crates/secondbrain-vault/src/manifest.rs`
- Test: `crates/secondbrain-vault/tests/manifest.rs`

**Step 1: Write failing tests**

Tests must verify:

- initialization creates required directories;
- manifest has `workspace_id`, `format_version = 1`, `created_at`, and required feature list;
- initialization is idempotent;
- unsupported future format is rejected read-only;
- existing user files are byte-identical.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-vault --test manifest
```

Expected: missing crate/API failure.

**Step 3: Implement minimal manifest API**

Required API:

```rust
pub struct WorkspaceManifest {
    pub workspace_id: WorkspaceId,
    pub format_version: u32,
    pub created_at: String,
    pub required_features: Vec<String>,
}

pub fn initialize_workspace(root: &Path) -> Result<WorkspaceManifest>;
pub fn load_manifest(root: &Path) -> Result<WorkspaceManifest>;
```

Create only:

```text
.secondbrain/oplog/
.secondbrain/transactions/
.secondbrain/snapshots/
.secondbrain/identity-map/
.secondbrain/policies/
.secondbrain/audit/
.secondbrain/plugins.lock
```

Use temporary file + atomic rename for the manifest.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-vault --test manifest
```

Expected: all manifest tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-vault

git commit -m "feat(vault): add versioned workspace manifest"
```

---

## Milestone B — Loss-aware Markdown and stable note identity

### Task 5: Build the loss-aware Markdown source model

**Objective:** Parse Markdown into positioned semantic nodes while retaining exact source slices for unchanged regions.

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/secondbrain-markdown/Cargo.toml`
- Create: `crates/secondbrain-markdown/src/lib.rs`
- Create: `crates/secondbrain-markdown/src/source.rs`
- Create: `crates/secondbrain-markdown/src/ast.rs`
- Create: `crates/secondbrain-markdown/src/parse.rs`
- Test: `crates/secondbrain-markdown/tests/source_model.rs`
- Fixtures: `fixtures/markdown/source-preservation/*.md`

**Step 1: Write failing tests**

Required assertions:

```rust
#[test]
fn parse_tracks_exact_utf8_byte_ranges() {
    let source = "# Héllo\r\n\r\nBody\r\n";
    let doc = parse_document(source).unwrap();
    assert_eq!(doc.source(), source);
    assert_eq!(doc.nodes()[0].raw(source), "# Héllo");
}

#[test]
fn untouched_document_serializes_byte_identically() {
    let source = include_str!("../../../fixtures/markdown/source-preservation/mixed.md");
    assert_eq!(parse_document(source).unwrap().serialize(), source);
}
```

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-markdown --test source_model
```

Expected: missing API failure.

**Step 3: Implement positioned source model**

Use `markdown` 1.0.0 `to_mdast` with GFM, frontmatter, and math options. Wrap mdast positions into internal byte-span types. Keep the complete original `Arc<str>` and a node table. Do not depend on an AST-to-Markdown serializer for unchanged nodes. Preserve every byte range not represented by a supported semantic node as an opaque source slice; do not assume `markdown` will emit a dedicated unknown-node variant for Obsidian/plugin syntax.

Required concepts:

```rust
pub struct SourceDocument {
    source: Arc<str>,
    root: SemanticNode,
    line_ending: LineEnding,
}

pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}
```

Unknown or unsupported constructs, including gaps inside parser text nodes that the SecondBrain extension recognizer cannot interpret safely, become `SemanticKind::Raw`, retaining exact spans.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-markdown --test source_model
```

Expected: exact-byte preservation tests pass for LF and CRLF.

**Step 5: Commit**

```bash
git add crates/secondbrain-markdown fixtures/markdown/source-preservation

git commit -m "feat(markdown): add positioned loss-aware source model"
```

---

### Task 6: Parse and patch YAML frontmatter without rewriting the body

**Objective:** Read and surgically update note metadata while preserving all body bytes and unrelated frontmatter formatting where possible.

**Files:**
- Create: `crates/secondbrain-markdown/src/frontmatter.rs`
- Modify: `crates/secondbrain-markdown/src/lib.rs`
- Test: `crates/secondbrain-markdown/tests/frontmatter.rs`
- Fixtures: `fixtures/markdown/frontmatter/*.md`

**Step 1: Write failing tests**

Cover:

- no frontmatter;
- valid YAML;
- UTF-8 BOM;
- comments and quoted values;
- malformed YAML diagnostic;
- inserting `id` leaves body byte-identical;
- existing canonical `id` produces no write;
- duplicate `id` keys are rejected.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-markdown --test frontmatter
```

Expected: missing functions.

**Step 3: Implement surgical metadata API**

Required API:

```rust
pub struct NoteMetadata {
    pub id: Option<NoteId>,
    pub title: Option<String>,
    pub properties: serde_yaml::Mapping,
}

pub fn parse_metadata(source: &str) -> Result<NoteMetadata>;
pub fn ensure_note_id(source: &str, generated: NoteId) -> Result<MetadataPatch>;

pub struct MetadataPatch {
    pub changed: bool,
    pub source: String,
    pub note_id: NoteId,
}
```

When adding an ID, edit only the frontmatter region. If no frontmatter exists, prepend a minimal block and preserve the original source exactly afterward. Do not alphabetize or rewrite unrelated YAML.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-markdown --test frontmatter
```

Expected: all tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-markdown fixtures/markdown/frontmatter

git commit -m "feat(markdown): preserve body during metadata updates"
```

---

### Task 7: Extract wikilinks, tags, headings, tasks, and properties

**Objective:** Produce a deterministic derived metadata record for SQLite indexing and backlinks.

**Files:**
- Create: `crates/secondbrain-markdown/src/extract.rs`
- Modify: `crates/secondbrain-markdown/src/lib.rs`
- Test: `crates/secondbrain-markdown/tests/extract.rs`
- Fixtures: `fixtures/markdown/extract/*.md`

**Step 1: Write failing tests**

Cover:

- `[[Note]]`, `[[Note|Alias]]`, `[[Note#Heading]]`, and embeds;
- escaped/literal wikilinks and wikilinks inside code are ignored;
- Unicode tags;
- duplicate tags deduplicate deterministically;
- headings retain source order and byte ranges;
- task states retain marker and source span;
- YAML properties retain typed JSON-compatible values.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-markdown --test extract
```

Expected: missing extraction API.

**Step 3: Implement extraction**

Required output:

```rust
pub struct ExtractedNote {
    pub links: Vec<ExtractedLink>,
    pub tags: Vec<ExtractedTag>,
    pub headings: Vec<ExtractedHeading>,
    pub tasks: Vec<ExtractedTask>,
    pub properties: BTreeMap<String, PropertyValue>,
    pub plain_text: String,
}
```

All lists must be deterministically ordered. Every extractable item includes a source span.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-markdown --test extract
```

Expected: all extraction tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-markdown fixtures/markdown/extract

git commit -m "feat(markdown): extract links tags tasks and properties"
```

---

### Task 8: Add Markdown round-trip corpus and property tests

**Objective:** Make loss detection a permanent release gate.

**Files:**
- Create: `crates/secondbrain-markdown/tests/round_trip.rs`
- Create: `crates/secondbrain-markdown/tests/properties.rs`
- Create: `fixtures/markdown/commonmark/README.md`
- Add: curated CommonMark/GFM/Obsidian fixtures under `fixtures/markdown/`

**Step 1: Add corpus tests**

For every fixture:

```rust
let first = parse_document(source)?;
let serialized = first.serialize();
let second = parse_document(&serialized)?;
assert_eq!(serialized, source);
assert_eq!(first.semantic_fingerprint(), second.semantic_fingerprint());
```

**Step 2: Add proptest generators**

Generate combinations of headings, paragraphs, nested lists, tables, code fences, Unicode, CRLF/LF, YAML properties, wikilinks, and raw extension blocks. Bound generated document size for unit runs.

**Step 3: Verify the suite**

Run:

```bash
PROPTEST_CASES=1000 cargo test -p secondbrain-markdown --test round_trip
```

Expected: no source or semantic changes.

**Step 4: Record evidence**

Create `docs/evidence/phase-0-markdown-roundtrip.md` with command, environment, fixture count, proptest case count, and actual result.

**Step 5: Commit**

```bash
git add crates/secondbrain-markdown/tests fixtures/markdown docs/evidence

git commit -m "test(markdown): gate lossless round-trip behavior"
```

---

### Task 9: Implement semantic Markdown operations and diff

**Objective:** Convert an external whole-file edit into deterministic semantic operations without writing files yet.

**Files:**
- Create: `crates/secondbrain-markdown/src/operation.rs`
- Create: `crates/secondbrain-markdown/src/diff.rs`
- Create: `crates/secondbrain-markdown/src/apply.rs`
- Test: `crates/secondbrain-markdown/tests/semantic_diff.rs`
- Fixtures: `fixtures/external-edits/*.case.toml`

**Step 1: Define failing acceptance tests**

Fixtures contain base, incoming, expected operations, and expected result. Cases:

- paragraph text update;
- paragraph insertion/deletion;
- heading rename;
- list item move;
- task-state toggle;
- independent property changes;
- raw block replacement;
- ambiguous duplicate paragraphs marked `NeedsReview` rather than guessed.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-markdown --test semantic_diff
```

Expected: missing `semantic_diff`.

**Step 3: Implement operations**

Initial operation enum:

```rust
pub enum SemanticOperation {
    InsertNode { parent: NodeAnchor, index: usize, markdown: String },
    DeleteNode { target: NodeAnchor, expected_hash: ContentHash },
    ReplaceNode { target: NodeAnchor, expected_hash: ContentHash, markdown: String },
    MoveNode { target: NodeAnchor, new_parent: NodeAnchor, new_index: usize },
    SetProperty { key: String, expected: Option<PropertyValue>, value: PropertyValue },
    RemoveProperty { key: String, expected: PropertyValue },
    NeedsReview { reason: String, source_ranges: Vec<SourceSpan> },
}
```

Use structural path, node type, neighboring fingerprints, and content hash for anchors. Implement moves only when identity confidence is unambiguous. Prefer `NeedsReview` to a destructive guess.

**Step 4: Verify GREEN and determinism**

Run:

```bash
cargo test -p secondbrain-markdown --test semantic_diff
cargo test -p secondbrain-markdown semantic_diff_is_deterministic -- --exact
```

Expected: operations and applied result match fixtures; repeated runs produce identical operations.

**Step 5: Commit**

```bash
git add crates/secondbrain-markdown fixtures/external-edits

git commit -m "feat(markdown): translate external edits into semantic operations"
```

---

## Milestone C — Durable filesystem, identity map, and transactions

### Task 10: Implement workspace-root confinement and atomic file writing

**Objective:** Guarantee that all writes stay inside the workspace and either replace a complete file or leave the previous file intact.

**Files:**
- Create: `crates/secondbrain-vault/src/root.rs`
- Create: `crates/secondbrain-vault/src/atomic_write.rs`
- Test: `crates/secondbrain-vault/tests/atomic_write.rs`

**Step 1: Write failing tests**

Cover:

- normal write;
- overwrite;
- parent traversal;
- absolute path;
- symlink escape on supported platforms;
- temp file cleanup after injected pre-rename failure;
- original file survives injected failure;
- requested file permissions are retained where supported.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-vault --test atomic_write
```

Expected: missing API.

**Step 3: Implement confined atomic writes**

Required API:

```rust
pub struct WorkspaceRoot { /* canonical root */ }

impl WorkspaceRoot {
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;
    pub fn resolve(&self, path: &WorkspacePath) -> Result<PathBuf>;
    pub fn atomic_write(&self, path: &WorkspacePath, bytes: &[u8]) -> Result<WriteReceipt>;
}
```

Write a same-directory temporary file, flush, `sync_all`, rename atomically, then sync the parent directory where supported. Add test-only failure hooks behind `cfg(test)` rather than production environment variables.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-vault --test atomic_write
```

Expected: tests pass; no escaped file appears.

**Step 5: Commit**

```bash
git add crates/secondbrain-vault

git commit -m "feat(vault): add confined crash-safe atomic writes"
```

---

### Task 11: Implement portable identity map and duplicate-ID recovery

**Objective:** Persist note ID/path/fingerprint history and resolve rename, deleted frontmatter ID, and copied-file duplicates deterministically.

**Files:**
- Create: `crates/secondbrain-vault/src/identity_map.rs`
- Test: `crates/secondbrain-vault/tests/identity_map.rs`
- Fixture: `fixtures/external-edits/identity/`

**Step 1: Write failing tests**

Cover:

- new note registration;
- rename preserves ID;
- frontmatter ID removed but fingerprint/path history recovers ID;
- exact copy creates duplicate ID detection;
- duplicate copy receives new ID while original retains old ID;
- ambiguous recovery returns `NeedsReview`;
- interrupted identity-map write preserves previous map.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-vault --test identity_map
```

Expected: missing API.

**Step 3: Implement versioned identity map**

Store portable records under `.secondbrain/identity-map/` using one versioned JSON file per note ID in Phase 0. Use atomic writes. Record current path, historical paths, latest source hash, structural fingerprint, and last observed timestamp. Do not store secrets or OS-absolute paths.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-vault --test identity_map
```

Expected: all recovery and duplicate tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-vault fixtures/external-edits/identity

git commit -m "feat(vault): preserve stable note identity across file changes"
```

---

### Task 12: Define versioned local mutation-journal and transaction records

**Objective:** Establish a deterministic Phase 0 durability journal with corruption detection. This is pre-CRDT recovery plumbing—not the canonical signed CRDT operation log and not the final sync wire protocol.

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/secondbrain-transaction/Cargo.toml`
- Create: `crates/secondbrain-transaction/src/lib.rs`
- Create: `crates/secondbrain-transaction/src/record.rs`
- Test: `crates/secondbrain-transaction/tests/record.rs`

**Step 1: Write failing tests**

Test:

- encode/decode round-trip;
- deterministic encoding for the same record;
- unsupported version rejection;
- CRC corruption rejection;
- truncated record rejection;
- unknown additive JSON field tolerance within version 1;
- actor, device, note, and transaction attribution retained.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-transaction --test record
```

Expected: missing API.

**Step 3: Implement development record format**

Required shape:

```rust
pub struct LocalOperationRecord {
    pub format_version: u16,
    pub transaction_id: TransactionId,
    pub workspace_id: WorkspaceId,
    pub note_id: NoteId,
    pub actor_id: ActorId,
    pub device_id: DeviceId,
    pub sequence: u64,
    pub previous_record_hash: Option<ContentHash>,
    pub operation: SemanticOperation,
    pub crc32: u32,
}
```

Canonicalize JSON object key order before hashing/CRC. Compute CRC over the canonical payload with the `crc32` field omitted; compute the record hash over the complete canonical envelope including the finalized CRC. Length-prefix records in the append-only file so truncation is detectable. Clearly label this local format `sb-local-oplog-v1`; do not reuse it as a network envelope.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-transaction --test record
```

Expected: corruption and truncation are detected.

**Step 5: Commit**

```bash
git add crates/secondbrain-transaction

git commit -m "feat(transaction): add versioned local operation records"
```

---

### Task 13: Implement append-only per-note local mutation journals

**Objective:** Persist and replay local mutation records with hash chaining and explicit durability boundaries; the selected CRDT's signed production oplog replaces/integrates this journal in the later identity/sync phase.

**Files:**
- Create: `crates/secondbrain-transaction/src/oplog.rs`
- Test: `crates/secondbrain-transaction/tests/oplog.rs`

**Step 1: Write failing tests**

Cover:

- append and replay;
- duplicate sequence rejection;
- hash-chain break rejection;
- truncated tail quarantine while retaining valid prefix;
- `sync_all` called before success using a test storage adapter;
- operation logs isolated by note ID.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-transaction --test oplog
```

Expected: missing oplog implementation.

**Step 3: Implement oplog**

Phase 0 paths (provisional and migration-versioned):

```text
.secondbrain/oplog/<note-id>/local-mutations.log
.secondbrain/oplog/<note-id>/quarantine/
```

Append returns only after flush and `sync_all`. Replay returns valid records plus an optional corruption report. Never silently discard invalid bytes.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-transaction --test oplog
```

Expected: replay is deterministic and corruption is reported.

**Step 5: Commit**

```bash
git add crates/secondbrain-transaction

git commit -m "feat(transaction): persist durable per-note operation logs"
```

---

### Task 14: Implement the transaction state machine and single-note commit pipeline

**Objective:** Apply a semantic operation through validation, durable oplog append, Markdown materialization, and committed state.

**Files:**
- Create: `crates/secondbrain-transaction/src/state.rs`
- Create: `crates/secondbrain-transaction/src/engine.rs`
- Test: `crates/secondbrain-transaction/tests/engine.rs`

**Step 1: Write failing tests**

Cover:

- valid `PREPARED → OPERATIONS_DURABLE → MATERIALIZING → COMMITTED` sequence;
- illegal transition rejected;
- stale file hash rejected before append;
- `NeedsReview` operation cannot auto-commit;
- committed edit changes Markdown and increments note version;
- unchanged edit performs no write and no version increment.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-transaction --test engine
```

Expected: missing engine.

**Step 3: Implement minimal engine**

Required request:

```rust
pub struct TransactionRequest {
    pub id: TransactionId,
    pub actor: ActorId,
    pub device: DeviceId,
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub expected_hash: ContentHash,
    pub expected_version: NoteVersion,
    pub operations: Vec<SemanticOperation>,
}
```

Persist transaction state under `.secondbrain/transactions/<transaction-id>.json` atomically at each durable transition. Success is returned only after Markdown atomic rename and committed state persistence.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-transaction --test engine
```

Expected: all state and stale-precondition tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-transaction

git commit -m "feat(transaction): commit semantic edits through durable state machine"
```

---

### Task 15: Add crash injection and startup recovery

**Objective:** Prove confirmed changes survive crashes and incomplete transactions recover deterministically.

**Files:**
- Create: `crates/secondbrain-transaction/src/recovery.rs`
- Create: `crates/secondbrain-transaction/src/failpoint.rs`
- Create: `crates/secondbrain-transaction/tests/support/crash_child.rs`
- Test: `crates/secondbrain-transaction/tests/crash_recovery.rs`
- Fixtures: `fixtures/crash-recovery/`

**Step 1: Write failure-boundary tests**

Inject failures:

- before append;
- after append but before transaction state update;
- after `OPERATIONS_DURABLE`;
- during temp Markdown write;
- after rename before commit marker;
- after commit before derived index update.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-transaction --test crash_recovery
```

Expected: missing recovery/failpoint support.

**Step 3: Implement recovery rules**

- `PREPARED` with no durable ops: abort safely.
- Durable oplog with stale transaction marker: reconstruct state.
- `OPERATIONS_DURABLE` or `MATERIALIZING`: replay and materialize.
- Markdown already matches expected post-state: mark committed idempotently.
- Corrupt oplog: quarantine and stop automatic materialization.
- Index repair is emitted as a recovery action, not performed by transaction crate.

Failpoints are test-only interfaces. Unit cases may return injected I/O errors, but durability acceptance cases must spawn a child test process, terminate it at the selected boundary (`abort`/hard process exit), reopen the workspace in a fresh process, and verify recovery. Production builds must not accept arbitrary environment-controlled crash hooks.

**Step 4: Verify GREEN repeatedly**

Run:

```bash
python3 - <<'PY'
import subprocess
for _ in range(20):
    subprocess.run(
        ["cargo", "test", "-p", "secondbrain-transaction", "--test", "crash_recovery"],
        check=True,
    )
PY
```

Expected: every run passes.

**Step 5: Record evidence and commit**

Create `docs/evidence/phase-0-crash-recovery.md`, then:

```bash
git add crates/secondbrain-transaction fixtures/crash-recovery docs/evidence

git commit -m "test(transaction): prove deterministic crash recovery"
```

---

## Milestone D — Derived SQLite index and search

### Task 16: Create SQLite schema, migrations, and rebuild metadata

**Objective:** Establish a derived WAL-mode index that can be deleted and rebuilt without content loss.

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/secondbrain-index/Cargo.toml`
- Create: `crates/secondbrain-index/src/lib.rs`
- Create: `crates/secondbrain-index/src/database.rs`
- Create: `crates/secondbrain-index/src/migrations/0001_initial.sql`
- Test: `crates/secondbrain-index/tests/migrations.rs`

**Step 1: Write failing tests**

Verify:

- WAL mode;
- foreign keys enabled;
- migration idempotence;
- schema version recorded;
- tables: notes, paths, properties, links, tags, headings, tasks, index_state;
- FTS5 virtual table exists;
- deleting DB does not touch workspace files.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-index --test migrations
```

Expected: missing index crate.

**Step 3: Implement schema**

Use `rusqlite` with `bundled` and `modern_sqlite` features. Store note ID as canonical text. Enforce uniqueness of current path and note ID. Add cascades for derived child rows. FTS rows reference note IDs, not filesystem row IDs.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-index --test migrations
```

Expected: all migration tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-index

git commit -m "feat(index): add rebuildable SQLite FTS schema"
```

---

### Task 17: Implement full workspace indexing and deterministic rebuild

**Objective:** Scan Markdown files, establish IDs, extract metadata, and atomically replace derived index state.

**Files:**
- Create: `crates/secondbrain-index/src/indexer.rs`
- Test: `crates/secondbrain-index/tests/rebuild.rs`
- Fixtures: `fixtures/markdown/workspace-small/`

**Step 1: Write failing tests**

Cover:

- index known fixture count;
- ignore `.secondbrain`, `.git`, and configured exclusions;
- deterministic ordering across repeated rebuilds;
- duplicate ID reported before index commit;
- malformed note reported without destroying prior valid index;
- backlinks resolve by current path/title/alias rules;
- remove `index.sqlite`, rebuild, and obtain the same logical dump.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-index --test rebuild
```

Expected: missing indexer.

**Step 3: Implement rebuild**

Build into a temporary SQLite file, run integrity checks, then atomically replace the active index. Return an `IndexReport` containing indexed, skipped, warning, error, orphan, and broken-link counts.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-index --test rebuild
```

Expected: deterministic rebuild succeeds.

**Step 5: Commit**

```bash
git add crates/secondbrain-index fixtures/markdown/workspace-small

git commit -m "feat(index): rebuild search and graph state from Markdown"
```

---

### Task 18: Implement search, backlinks, broken links, and orphan queries

**Objective:** Expose typed read APIs with stable ordering and useful snippets.

**Files:**
- Create: `crates/secondbrain-index/src/query.rs`
- Test: `crates/secondbrain-index/tests/query.rs`

**Step 1: Write failing tests**

Test:

- term and phrase FTS;
- Unicode search;
- path and tag filters;
- backlinks and outgoing links;
- broken links;
- orphan notes;
- snippets escaped for terminal output;
- deterministic tie-breaking by path then note ID.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-index --test query
```

Expected: missing query API.

**Step 3: Implement typed queries**

Required APIs:

```rust
pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>>;
pub fn backlinks(&self, note: NoteId) -> Result<Vec<LinkHit>>;
pub fn broken_links(&self) -> Result<Vec<BrokenLink>>;
pub fn orphans(&self) -> Result<Vec<NoteSummary>>;
```

Parameterize SQL. Never interpolate user query fragments into SQL except through a validated FTS query builder.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-index --test query
```

Expected: all query tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-index

git commit -m "feat(index): query search backlinks and graph diagnostics"
```

---

## Milestone E — Filesystem watcher and external-edit pipeline

### Task 19: Normalize and debounce filesystem events

**Objective:** Convert platform-specific watcher noise into deterministic workspace events.

**Files:**
- Create: `crates/secondbrain-vault/src/watcher.rs`
- Create: `crates/secondbrain-vault/src/event.rs`
- Test: `crates/secondbrain-vault/tests/watcher.rs`

**Step 1: Write failing tests**

Test the normalizer independently from the OS watcher:

- create/write bursts collapse to one `ContentChanged`;
- atomic save rename becomes one content update;
- rename preserves old/new path;
- ignored directories never emit;
- duplicate events collapse;
- delete and recreate are distinguished;
- internal-origin receipts suppress self-generated writes once, not forever.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-vault --test watcher
```

Expected: missing event normalizer.

**Step 3: Implement watcher**

Use stable `notify` 8.2.0. Keep the OS callback minimal and send raw events through a standard channel to a deterministic normalizer worker with an injectable clock and explicit debounce window. Do not depend on the release-candidate `notify-debouncer-full`; implement only the event coalescing semantics required by the tests. Hash the observed file before deciding content changed. Maintain a bounded internal-write receipt cache keyed by relative path, content hash, and expiration.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-vault --test watcher
```

Expected: normalized events match tests.

**Step 5: Commit**

```bash
git add crates/secondbrain-vault

git commit -m "feat(vault): normalize external filesystem changes"
```

---

### Task 20: Integrate external file changes with semantic transactions

**Objective:** Turn a normalized external edit into an attributed transaction, merge safely, update Markdown, and refresh index state.

**Files:**
- Create: `crates/secondbrain-transaction/src/external_edit.rs`
- Test: `crates/secondbrain-transaction/tests/external_edit.rs`
- Fixtures: `fixtures/external-edits/integration/`

**Step 1: Write failing integration tests**

Cover:

- external paragraph edit commits as actor `external:<device>`;
- simultaneous internal change to another paragraph merges;
- stale external base rebases;
- ambiguous raw-block edit produces review artifact and does not overwrite;
- external rename keeps note ID and updates identity map;
- external copy gets a new ID;
- successful commit causes incremental index refresh;
- internal materialization event does not loop back.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-transaction --test external_edit
```

Expected: missing coordinator.

**Step 3: Implement coordinator**

Create an `ExternalEditCoordinator` that depends on interfaces for workspace I/O, identity map, transaction engine, and index updater. The coordinator must not contain alternate write logic. `NeedsReview` writes a review descriptor under `.secondbrain/transactions/<id>.conflict.json` and leaves the current file untouched.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-transaction --test external_edit
```

Expected: merge, review, rename, copy, and loop-suppression tests pass.

**Step 5: Commit**

```bash
git add crates/secondbrain-transaction fixtures/external-edits/integration

git commit -m "feat(transaction): merge external edits through core engine"
```

---

## Milestone F — Phase 0 CLI and end-to-end verification

### Task 21: Build the `secondbrain` CLI surface

**Objective:** Provide a real executable for workspace initialization, validation, indexing, search, inspection, semantic diff preview, transaction apply, recovery, and diagnostics.

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/secondbrain-cli/Cargo.toml`
- Create: `crates/secondbrain-cli/src/main.rs`
- Create: `crates/secondbrain-cli/src/commands/*.rs`
- Test: `crates/secondbrain-cli/tests/cli.rs`

**Step 1: Write failing CLI tests**

Use `assert_cmd` and `predicates`. Commands:

```text
secondbrain init <workspace>
secondbrain validate <workspace>
secondbrain index rebuild <workspace>
secondbrain search <workspace> <query>
secondbrain note inspect <workspace> <path>
secondbrain diff <workspace> <path> <incoming-file>
secondbrain transaction apply <workspace> <plan-file>
secondbrain recovery check <workspace>
secondbrain doctor <workspace>
```

Verify JSON output with `--json`, stable non-zero exit codes, and no ANSI when output is not a TTY.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-cli --test cli
```

Expected: missing binary.

**Step 3: Implement commands**

CLI must call library APIs only. Default diff/apply workflow:

1. `diff` prints/writes a transaction plan without mutation.
2. `transaction apply` validates preconditions and applies the explicit plan.
3. Ambiguous edits return a review-required exit code.

Define exit codes in README and tests.

**Step 4: Verify GREEN**

Run:

```bash
cargo test -p secondbrain-cli --test cli
cargo run -p secondbrain-cli -- --help
```

Expected: tests pass and help lists all commands.

**Step 5: Commit**

```bash
git add crates/secondbrain-cli README.md

git commit -m "feat(cli): expose Phase 0 workspace operations"
```

---

### Task 22: Add an end-to-end Phase 0 scenario

**Objective:** Exercise the actual CLI and filesystem against an Obsidian-compatible fixture workspace.

**Files:**
- Create: `crates/secondbrain-cli/tests/e2e_phase0.rs`
- Create: `fixtures/markdown/obsidian-vault/`
- Create: `docs/evidence/phase-0-e2e.md`

**Step 1: Write the E2E test**

Scenario:

1. Copy fixture vault to temp directory.
2. Hash every existing file.
3. Initialize workspace.
4. Confirm existing source bytes are unchanged except files explicitly assigned missing note IDs through a preview/apply transaction.
5. Rebuild index.
6. Search and inspect backlinks.
7. Modify a note externally.
8. Process the event and merge it.
9. Simulate crash after durable oplog.
10. Restart recovery.
11. Confirm final Markdown, transaction status, identity map, and index.
12. Delete `index.sqlite`, rebuild, and compare logical index dump.

**Step 2: Verify RED/GREEN honestly**

Run before final fixture adjustments, observe failure, then fix only product or fixture issues required by the approved behavior.

Run final:

```bash
cargo test -p secondbrain-cli --test e2e_phase0 -- --nocapture
```

Expected: one full scenario passes.

**Step 3: Record evidence**

`docs/evidence/phase-0-e2e.md` must include toolchain, OS, command, test name, resulting counts, and output excerpt.

**Step 4: Run all Phase 0 gates**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p secondbrain-cli -- doctor fixtures/markdown/obsidian-vault --json
```

Expected: every command exits 0.

**Step 5: Commit**

```bash
git add crates/secondbrain-cli fixtures/markdown/obsidian-vault docs/evidence

git commit -m "test: verify Phase 0 end-to-end recovery workflow"
```

---

## Milestone G — Mandatory Loro versus Yrs spike

### Task 23: Define the black-box CRDT acceptance contract

**Objective:** Create identical scenarios and metrics for both candidates without coupling production crates to a generic CRDT interface.

**Files:**
- Modify: `Cargo.toml`
- Create: `spikes/crdt-comparison/README.md`
- Create: `spikes/crdt-comparison/contract/Cargo.toml`
- Create: `spikes/crdt-comparison/contract/src/lib.rs`
- Create: `spikes/crdt-comparison/contract/tests/scenarios.rs`
- Create: `spikes/crdt-comparison/fixtures/*.json`

**Step 1: Define candidate executable protocol**

Each candidate binary accepts JSON scenario input on stdin and emits one JSON result on stdout. Candidate conformance tests resolve the executable from `CRDT_CANDIDATE_BIN`; harness self-tests always run. When the variable is absent during generic `cargo test --workspace`, conformance tests emit an explicit skip message and return success. Dedicated Task 24/25 verification commands must set the variable and assert that every mandatory scenario actually executed, so a skip can never count as candidate evidence. Commands:

```text
create_replica
apply_local
export_updates
import_updates
materialize
undo_actor
snapshot
restore
truncate_restore
metrics
```

This process boundary is temporary spike infrastructure, not a production trait.

**Step 2: Write failing contract tests**

Scenarios:

- concurrent insertion at same position;
- concurrent deletion/edit;
- paragraph and ordered-list moves;
- rich-text marks and relative position mapping;
- external whole-file replacement represented as semantic ops;
- three-way offline merge with reordered/duplicate delivery;
- per-actor undo;
- snapshot/restore;
- truncated update rejection/recovery;
- deterministic materialization;
- 100K sequential and mixed operations.

**Step 3: Implement contract runner and schema validation**

The runner launches a candidate binary, validates every JSON response, captures time/RSS/output bytes, and verifies convergence. Candidate-specific output may contain diagnostics, but required result fields are fixed.

**Step 4: Verify contract harness**

Use a deliberate fake candidate test fixture that fails convergence and confirm the harness catches it. Keep it as test-only code, never as a workspace member.

Run:

```bash
cargo test -p crdt-spike-contract
```

Expected: harness self-tests pass.

**Step 5: Commit**

```bash
git add spikes/crdt-comparison

git commit -m "spike(crdt): define shared acceptance contract"
```

---

### Task 24: Implement and test the Loro candidate

**Objective:** Evaluate Loro 1.13.7 against every shared scenario.

**Files:**
- Modify: `Cargo.toml`
- Create: `spikes/crdt-comparison/loro-candidate/Cargo.toml`
- Create: `spikes/crdt-comparison/loro-candidate/src/main.rs`
- Create: `spikes/crdt-comparison/loro-candidate/src/model.rs`
- Test: shared contract invocation

**Step 1: Add the candidate and observe contract failures**

Run:

```bash
cargo build -p loro-candidate
CRDT_CANDIDATE_BIN=target/debug/loro-candidate \
  cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
```

Expected: failure because candidate is incomplete.

**Step 2: Implement Loro model**

Use `LoroDoc`, text for rich text, map for properties, movable list/tree for block order, `UndoManager`, incremental updates, snapshots, shallow snapshots where applicable, and version/frontier APIs. Record any scenario requiring custom glue.

**Step 3: Run correctness scenarios**

```bash
CRDT_CANDIDATE_BIN=target/debug/loro-candidate \
  cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
```

Expected: all mandatory correctness scenarios pass or a concrete blocker is recorded. Do not mask a failure to make the candidate look viable.

**Step 4: Record candidate notes**

Create `spikes/crdt-comparison/loro-candidate/RESULTS.md` with API ergonomics, custom integration code, unsupported behavior, correctness status, and reproducible commands.

**Step 5: Commit**

```bash
git add spikes/crdt-comparison/loro-candidate

git commit -m "spike(crdt): evaluate Loro candidate"
```

---

### Task 25: Implement and test the Yrs candidate

**Objective:** Evaluate Yrs 0.27.3 against the same scenarios and metrics.

**Files:**
- Modify: `Cargo.toml`
- Create: `spikes/crdt-comparison/yrs-candidate/Cargo.toml`
- Create: `spikes/crdt-comparison/yrs-candidate/src/main.rs`
- Create: `spikes/crdt-comparison/yrs-candidate/src/model.rs`
- Test: shared contract invocation

**Step 1: Add the candidate and observe contract failures**

Run:

```bash
cargo build -p yrs-candidate
CRDT_CANDIDATE_BIN=target/debug/yrs-candidate \
  cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
```

Expected: failure because candidate is incomplete.

**Step 2: Implement Yrs model**

Use `Doc`, `TextRef`/XML types as appropriate for rich text, arrays/maps for block structure/properties, transaction origins, state vectors, v1/v2 updates, snapshots, and undo manager. Evaluate native move support and `yrs_tree` only if the shared scenarios require it; record extra dependency cost explicitly.

**Step 3: Run correctness scenarios**

```bash
CRDT_CANDIDATE_BIN=target/debug/yrs-candidate \
  cargo test -p crdt-spike-contract --test candidate_conformance -- --nocapture
```

Expected: all mandatory correctness scenarios pass or a concrete blocker is recorded.

**Step 4: Record candidate notes**

Create `spikes/crdt-comparison/yrs-candidate/RESULTS.md` with the same headings and evidence shape used by Loro.

**Step 5: Commit**

```bash
git add spikes/crdt-comparison/yrs-candidate

git commit -m "spike(crdt): evaluate Yrs candidate"
```

---

### Task 26: Benchmark both CRDT candidates reproducibly

**Objective:** Measure operation latency, merge latency, import/export, snapshot size, restore time, and peak memory under identical workloads.

**Files:**
- Modify: `Cargo.toml`
- Create: `spikes/crdt-comparison/benches/Cargo.toml`
- Create: `spikes/crdt-comparison/benches/src/main.rs`
- Create: `spikes/crdt-comparison/scripts/run-comparison.py`
- Create: `docs/evidence/crdt-comparison-results.json`
- Create: `docs/evidence/crdt-comparison-environment.md`

**Step 1: Define benchmark cases**

Required sizes:

- 1K, 10K, and 100K operations;
- 2, 10, and 100 replicas;
- text-heavy workload;
- list/tree-move workload;
- properties workload;
- offline branches and merge;
- full snapshot and incremental update;
- cold restore and shallow/compacted restore.

**Step 2: Implement process-level runner**

Run release binaries separately so RSS and wall-clock measurements do not contaminate each other. Random workloads use recorded seeds. Output raw JSON; do not publish only summary averages.

**Step 3: Run benchmarks**

```bash
python3 spikes/crdt-comparison/scripts/run-comparison.py \
  --release \
  --repetitions 10 \
  --output docs/evidence/crdt-comparison-results.json
```

Expected: valid results for both candidates; any crash or unsupported scenario is recorded as failure, not omitted.

**Step 4: Verify reproducibility**

Re-run a fixed seed and confirm materialized state hashes and operation counts match exactly.

**Step 5: Commit**

```bash
git add spikes/crdt-comparison/benches spikes/crdt-comparison/scripts docs/evidence

git commit -m "bench(crdt): compare Loro and Yrs workloads"
```

---

### Task 27: Select the CRDT winner and write the ADR

**Objective:** Make one evidence-backed production choice and record rejected alternatives.

**Files:**
- Create: `docs/adr/0001-select-production-crdt.md`
- Modify: `docs/specs/2026-07-25-secondbrain-design.md` only to record the selected decision and link the ADR
- Modify: `README.md`

**Step 1: Apply mandatory decision rules**

A candidate is ineligible if it fails any mandatory correctness scenario. Among eligible candidates, score:

| Criterion | Weight |
|---|---:|
| Convergence/correctness | mandatory |
| External-edit semantic operation fit | 20 |
| Ordered move/tree behavior | 15 |
| Per-actor undo and history | 10 |
| Snapshot/compaction/recovery | 15 |
| Rust core integration | 10 |
| TipTap/TypeScript integration path | 10 |
| Payload and memory | 10 |
| Ecosystem/maturity/maintenance risk | 10 |

Performance cannot compensate for a correctness failure.

**Step 2: Write ADR**

ADR sections:

- context;
- tested versions;
- acceptance matrix;
- raw evidence links;
- benchmark summary with variance;
- integration complexity;
- security/maintenance considerations;
- selected candidate;
- rejected candidate;
- consequences;
- migration risk;
- follow-up production integration tasks.

**Step 3: Review decision independently**

Have a separate reviewer reproduce at least the correctness suite and one benchmark seed. Record reviewer command/output in the ADR.

**Step 4: Run all tests**

```bash
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: all pass.

**Step 5: Commit**

```bash
git add docs/adr docs/specs README.md

git commit -m "docs(adr): select SecondBrain production CRDT"
```

---

### Task 28: Integrate the selected CRDT into the production transaction core

**Objective:** Replace the provisional local mutation model with one selected per-note CRDT implementation while preserving the transaction, Markdown materialization, and recovery boundaries.

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/secondbrain-crdt/Cargo.toml`
- Create: `crates/secondbrain-crdt/src/lib.rs`
- Create: `crates/secondbrain-crdt/src/document.rs`
- Create: `crates/secondbrain-crdt/src/persistence.rs`
- Modify: `crates/secondbrain-transaction/src/engine.rs`
- Modify: `crates/secondbrain-transaction/src/recovery.rs`
- Test: `crates/secondbrain-crdt/tests/convergence.rs`
- Test: `crates/secondbrain-transaction/tests/crdt_engine.rs`

**Step 1: Write failing production integration tests**

Verify:

- one CRDT document per note;
- semantic operations apply through the selected engine;
- two offline replicas converge;
- duplicate/out-of-order updates are idempotent;
- deterministic Markdown materialization;
- snapshot/reopen preserves version and content;
- provisional mutation-journal records migrate once and idempotently;
- transaction preconditions and conflict review remain enforced;
- no production crate depends on the losing CRDT.

**Step 2: Verify RED**

Run:

```bash
cargo test -p secondbrain-crdt
cargo test -p secondbrain-transaction --test crdt_engine
```

Expected: crate/API is missing and tests fail before implementation.

**Step 3: Implement the selected engine only**

Create a narrow production API shaped by SecondBrain's per-note requirements, not by the union of Loro and Yrs. Persist the selected engine's native updates/snapshots under a versioned per-note layout. Keep the Phase 0 local journal only as migration input and crash-intent metadata where still required; do not maintain two canonical histories. Update transaction recovery so durable CRDT state plus transaction state deterministically rematerializes Markdown.

**Step 4: Verify GREEN and dependency isolation**

Run:

```bash
cargo test -p secondbrain-crdt
cargo test -p secondbrain-transaction --test crdt_engine
cargo tree -p secondbrain-transaction
cargo tree -p secondbrain-crdt
```

Expected: convergence/recovery tests pass and the losing candidate does not appear in either production dependency tree.

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/secondbrain-crdt crates/secondbrain-transaction

git commit -m "feat(crdt): integrate selected engine into transaction core"
```

---

## Milestone H — Phase 0 closure

### Task 29: Add cross-platform CI fixtures and platform-specific guards

**Objective:** Verify filesystem, path, line-ending, SQLite, and watcher behavior on macOS, Windows, and Linux.

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `crates/secondbrain-vault/tests/platform.rs`
- Create: `docs/evidence/phase-0-cross-platform.md`

**Step 1: Add platform tests**

Cover:

- case-collision detection (`Nova.md` versus `nova.md`);
- path separators;
- CRLF preservation;
- non-UTF-8 filename policy;
- symlink behavior with Windows privilege-aware skip;
- watcher atomic-save pattern;
- SQLite FTS availability.

**Step 2: Run locally applicable tests**

```bash
cargo test --workspace
```

Expected: local platform passes; platform-specific tests are explicitly gated with reasons, not silently ignored.

**Step 3: Update CI matrix**

Upload test reports and benchmark-smoke artifacts. Do not run full 100K/100-replica benchmarks on every PR; run a deterministic smoke workload on PR and full comparison manually/nightly during the spike.

**Step 4: Verify CI**

Push or run through available CI tooling, then record job URLs/results in `docs/evidence/phase-0-cross-platform.md`.

**Step 5: Commit**

```bash
git add .github crates/secondbrain-vault/tests docs/evidence

git commit -m "ci: gate Phase 0 across desktop platforms"
```

---

### Task 30: Run Phase 0 release gates and publish verification evidence

**Objective:** Close Phase 0 only after all correctness, recovery, lint, dependency, and benchmark-smoke gates pass.

**Files:**
- Create: `docs/evidence/phase-0-final-verification.md`
- Create: `docs/phase-0-operations.md`
- Create: `docs/phase-0-format.md`
- Modify: `README.md`

**Step 1: Run complete gates**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
PROPTEST_CASES=10000 cargo test --release -p secondbrain-markdown --test round_trip
cargo test --release -p secondbrain-transaction --test crash_recovery
cargo run -p secondbrain-cli -- doctor fixtures/markdown/obsidian-vault --json
```

Expected: every command exits 0. Save exact output, toolchain, OS, and timestamp.

**Step 2: Run performance smoke gates**

Generate and index 1K and 10K-note fixtures, measuring startup/index/search/open-note-equivalent library calls. Phase 0 must at least show no unbounded behavior and record actual numbers; desktop cold-start gates become enforceable in Phase 1.

**Step 3: Audit scope**

Confirm:

- no Tauri/UI code;
- no network sync implementation;
- no custom cryptography;
- no final wire protocol claim;
- no production dependency on the losing CRDT candidate;
- no P0/P1 open findings.

**Step 4: Document operations and local formats**

Explain workspace initialization, validation, recovery, index rebuild, conflict review artifacts, and the explicitly provisional `sb-local-oplog-v1` format.

**Step 5: Commit**

```bash
git add docs README.md

git commit -m "docs: close Phase 0 with verification evidence"
```

---

## Phase 0 acceptance checklist

Phase 0 is complete only when all boxes are supported by real evidence:

- [ ] Existing Markdown can be opened without destructive migration.
- [ ] Unchanged parse/serialize is byte-identical across the corpus.
- [ ] Unknown/raw syntax survives.
- [ ] Stable note IDs survive rename and missing-frontmatter recovery.
- [ ] Duplicate note IDs are detected and repaired transactionally.
- [ ] External edits become deterministic semantic operations.
- [ ] Ambiguous edits enter review and never silently overwrite.
- [ ] Writes are root-confined and atomic.
- [ ] Oplog replay detects corruption and truncation.
- [ ] Confirmed operations survive every injected crash boundary.
- [ ] SQLite can be deleted and rebuilt to the same logical state.
- [ ] Search, backlinks, broken links, and orphans are verified.
- [ ] Watcher events do not create self-write loops.
- [ ] CLI exercises the same library paths as future product surfaces.
- [ ] Loro and Yrs run identical correctness scenarios.
- [ ] One CRDT is selected through ADR; the other is not retained in production architecture.
- [ ] The selected per-note CRDT is integrated into the production transaction/recovery path and passes local convergence tests.
- [ ] The provisional local mutation journal has one idempotent migration path and is not a second canonical history.
- [ ] macOS, Windows, and Linux CI pass.
- [ ] Formatting, clippy, tests, and dependency policy are clean.
- [ ] No P0/P1 issue is deferred.

## Execution handoff

Plan complete and saved. Execute using `subagent-driven-development`: dispatch a fresh implementation subagent per task, then a separate spec-compliance reviewer, then a code-quality reviewer. Start with Task 1 only, and do not advance when either review reports unresolved issues.
