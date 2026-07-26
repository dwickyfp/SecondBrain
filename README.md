# SecondBrain

SecondBrain is a Rust-first project for a local-first knowledge system with Markdown as its durable representation.

This repository contains the completed Phase 0 Rust workspace and the Phase 1
desktop implementation under `apps/desktop`. The Tauri 2 + Svelte 5 app provides
WYSIWYG, source, and split editing over the same Rust manifest, index, and
transaction paths used by the CLI.

The mandatory CRDT spike selected **Loro 1.13.7** for production integration. Loro passed all shared mandatory scenarios; Yrs 0.27.3 was ineligible because it lacks a native identity-preserving ordered move and also requires an unstable compiler feature on the pinned Rust toolchain. See [ADR 0001](docs/adr/0001-select-production-crdt.md). The canonical per-note state is documented in [Phase 0 formats](docs/phase-0-format.md) and operated through [Phase 0 operations](docs/phase-0-operations.md).

## The `secondbrain` CLI

The CLI holds no domain logic of its own. Every command composes calls into the library crates, so that the desktop app, the MCP server, and the local API can later drive the same paths this binary drives.

```bash
cargo run -p secondbrain-cli -- --help

secondbrain init <workspace>                              # create .secondbrain/, leaving Markdown untouched
secondbrain validate <workspace>                          # every note parses, round-trips, and claims a unique id
secondbrain index rebuild <workspace>                     # rebuild the derived SQLite index from Markdown
secondbrain search <workspace> <query>                    # full-text search over the index
secondbrain note inspect <workspace> <path>               # identity, convergence, and links in both directions
secondbrain diff <workspace> <path> <incoming-file>       # preview a transaction plan; writes nothing
secondbrain transaction apply <workspace> <plan-file>     # check a plan's preconditions and apply it
secondbrain recovery check <workspace>                    # finish interrupted transactions and report the cost
secondbrain reconcile <workspace>                         # journal the edits made outside the workspace
secondbrain doctor <workspace>                            # one report on workspace health
```

`--json` is accepted by every command and emits a stable machine contract, documented field by field under [The `--json` contract](#the---json-contract). Neither form of output ever contains ANSI escapes.

## Agent skills

Repository-default skills live in `.agents/skills` and follow the open
[Agent Skills specification](https://agentskills.io/specification). OpenCode and
Codex discover this location automatically when started anywhere in the
repository. Other compatible hosts can import the same directories without
rewriting the skills.

| Skill | Use |
| --- | --- |
| `secondbrain-cli` | Safe JSON CLI workflows for search, preview/apply edits, note and daily-note creation, typed properties, graph, import, recovery, and diagnostics. |
| `secondbrain-markdown` | Source-preserving CommonMark/GFM/Obsidian-compatible authoring guidance for notes, wikilinks, embeds, callouts, tasks, tags, and properties. |

The skills intentionally direct agents through the production CLI and shared
Rust transaction contracts. They do not authorize direct writes to tracked
Markdown or `.secondbrain/` state. Validate them with:

```bash
node scripts/validate-agent-skills.mjs
```

### Previewing and applying a change

`diff` and `transaction apply` are two commands on purpose. `diff` derives the semantic operations an incoming file implies and mutates nothing; `transaction apply` takes a plan an operator has seen, re-checks its preconditions, and applies it. There is no one-shot path from an incoming file to a write, because a change nobody looked at is a change nobody approved.

```bash
secondbrain diff vault notes/meeting.md edited.md --out plan.json
secondbrain transaction apply vault plan.json
```

`diff` refuses a note whose file on disk is no longer the content the workspace last converged on. That gap means an editor outside the workspace saved over the note and nothing journaled it, so a plan derived there would record a version the file never held, and the journal could no longer replay to what is on disk. `note inspect` reports the same fact as `converged`, and `reconcile` is the command that closes it.

### Reconciling edits made outside the workspace

An editor that is not this workspace rewrites whole files, so by the time anything here notices, the change is already on disk and the state it replaced is gone. Until that edit is journaled it exists only as bytes: no author, no transaction, no place in the note's history.

```bash
secondbrain reconcile vault
```

For every note the workspace has converged on at least once, `reconcile` compares the file with that converged base and hands the difference to the external-edit coordinator, which recovers the semantic operations the editor performed and journals them as an attributed transaction. It then refreshes the derived index. Notes are reported one per line — `adopted`, `merged`, `review_required`, `absent`, or `unchanged` — so an operator can see what happened to each.

Three things it deliberately does not do. It never rewrites a note: the editor's bytes are the result, and the workspace is catching up to them. It never touches a note whose file still holds its converged base. And it never guesses at an ambiguous change — that gets a review descriptor and exit code `3`, the same code `diff` uses for the same fact.

It is a routine pass, safe to run repeatedly or from a scheduler. Running it again over an ambiguity nobody has resolved points at the review already waiting rather than filing a second one, and a pass that finds nothing new reports `index_refreshed: false` and rebuilds nothing.

A note the workspace has never converged on has no earlier state to have diverged from, so `reconcile` does not consider it; `diff` and `transaction apply` bring such a note under management by recording its first converged base.

#### `absent` — a note with no file where it was last seen

`reconcile` derives its work from each note's converged base, so what it can observe about a missing note is that nothing is at the path that base names. It reports that as `absent`, and deliberately not as `deleted`, because two different things produce it:

- the file really was deleted, or
- the file was **moved outside the workspace** by a tool that produced no rename this workspace saw. The note is intact at its new path; the identity map still records the old one, so `reconcile` names the note correctly while pointing at a path it no longer occupies.

Nothing is lost either way — the note's identity and converged base are kept, so a file that comes back, or that a later command finds at its new path, is recognized as the same note. Phase 0 has no delete transaction, so the absence is not journaled and is reported again on every pass until the path is healed. Only the first such pass does any work: once the derived index has stopped describing the path, a later pass has the same fact and nothing to do about it.

Rename detection across a move the workspace never observed is not attempted here.

It is one-shot and local. `sync` is the vocabulary of the network phase and is not spent here.

### Exit codes

| Code | Meaning |
| ---- | ------- |
| `0` | The command completed and nothing needs attention. |
| `1` | The command could not complete. |
| `2` | The command line itself was invalid. |
| `3` | The change is ambiguous and a human must decide what it meant. |
| `4` | The command completed and reported problems with the workspace. |

Code `3` is deliberately distinct from code `1`: a script that cannot tell "needs a person" from "the tool broke" will either page someone for a routine ambiguity or silently drop one. `diff` returns it for a change the semantic diff could not resolve and for a note that diverged from its converged base, `transaction apply` returns it rather than applying such a plan, and `reconcile` returns it when an external edit it integrated had to be filed for review. `reconcile` still prints its full report in that case — the pass completed, and some of it needs a person.

Code `4` means the command ran correctly and is reporting on the workspace: `validate` returns it for notes that do not parse, do not round-trip, or claim an identity another note claims; `recovery check` returns it when an edit was abandoned or a journal was quarantined; `doctor` returns it for workspace-state problems. Broken links and orphaned notes are reported as counts but do not affect the exit code — a vault with broken links is a vault, not a broken workspace.

### The `--json` contract

Every command emits one JSON object on stdout. Field names are stable: a consumer branches on them rather than parsing the human rendering, and both forms are produced from the same value so they cannot disagree about what happened. Optional fields marked *(when applicable)* are **absent**, not `null`, when they do not apply. A failure writes nothing to stdout and emits `{"error":{"code","message"}}` on stderr instead — see [Diagnostic codes](#diagnostic-codes).

| Command | Fields |
| ------- | ------ |
| `init` | `workspace`, `workspace_id`, `format_version`, `created_at`, `required_features[]` |
| `validate` | `workspace`, `workspace_id`, `format_version`, `notes_checked`, `problems[]` of `{path, code, message}` |
| `index rebuild` | `workspace`, `index` (the SQLite file), `indexed`, `skipped`, `warnings`, `errors`, `orphans`, `broken_links` |
| `search` | `query`, `hits[]` of `{note_id, path, title, snippet}` |
| `note inspect` | `note_id`, `path`, `title`, `source_hash`, `converged`, `converged_version` (`null` when no base is recorded), `outgoing_links[]` of `{target, note_id, path, title}`, `backlinks[]` of `{note_id, path, title, target}` |
| `diff` | the plan itself: `format`, `workspace_id`, `note_id`, `path`, `expected_hash`, `expected_version`, `review_required`, `operations[]` |
| `diff --out` | `plan` (the file written), `path`, `operations` (a count), `review_required`, `summary` |
| `transaction apply` | `transaction_id`, `note_id`, `path`, `changed`, `version`, `index_refreshed` |
| `recovery check` | `workspace`, `actions[]` (below), `index_repairs`, `quarantined`, `abandoned`, `index_refreshed` |
| `reconcile` | `workspace`, `notes[]` (below), `considered`, `adopted`, `merged`, `reviews_required`, `absent`, `unchanged`, `index_refreshed` |
| `doctor` | `workspace`, `workspace_id`, `format_version`, `index` of `{present, path, notes, links, broken_links, orphans}`, `transactions` of `{total, committed, aborted, pending, index_repairs_outstanding}`, `reviews_pending`, `problems[]` of `{code, message}` |

`recovery check`'s `actions[]` are tagged by `action`, and carry `transaction_id`, `note_id` and `path` in every kind:

| `action` | Extra fields |
| -------- | ------------ |
| `index_repair` | — |
| `quarantined` | `quarantine_path` |
| `abandoned` | `reason` (`operations_do_not_anchor` or `unrecognized_file_state`) and `explanation` |

`reconcile`'s `notes[]` are tagged by `outcome`, and carry `path` in every kind:

| `outcome` | Extra fields | Meaning |
| --------- | ------------ | ------- |
| `unchanged` | `note_id` | The file still holds the note's converged base, or changed no semantics. |
| `adopted` | `note_id`, `transaction_id`, `version` | The external edit was journaled as its own transaction. |
| `merged` | `note_id`, `transaction_id`, `version` | A workspace change the external write had clobbered was rebased onto it. |
| `review_required` | `transaction_id`, `descriptor_path` | The change was ambiguous; a person must decide. Exits `3`. |
| `absent` | `note_id` *(when the identity map names one)* | No file at the path the note was last known at. See [`absent`](#absent--a-note-with-no-file-where-it-was-last-seen). |
| `registered` | `note_id` | A file the workspace had not tracked was given an identity and a base. |
| `base_recovered` | `note_id` | A tracked note had no base; the content on disk became one, unattributed. |
| `renamed` | `note_id` | A move the workspace observed; identity and base followed the bytes. |
| `copied` | `note_id`, `source_note_id` | A copy of a tracked note, given an identity of its own. |

### Diagnostic codes

Every failure carries a stable `SB-*` code, on stderr in both output forms, and callers branch on the code rather than on the message. Codes that describe a *domain* condition are defined once in `secondbrain-core`'s error taxonomy so that every surface reports the same one — `SB-NOTE-DIVERGED` for a note whose file no longer holds its converged base, and `SB-NOTE-NOT-INDEXED` for a note absent from an index that answered. Codes the CLI defines for itself describe the CLI: `SB-PLAN-INVALID` for a plan file it could not use, `SB-INDEX-MISSING` for an index this binary declined to create, `SB-OUTPUT-ENCODE` for output it could not serialize.

## Toolchain

The workspace pins Rust 1.91.1 with the `rustfmt` and `clippy` components through `rust-toolchain.toml`.

## Quality gates

Run the same checks used by CI from the repository root:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny --locked check -D warnings
```

The dependency-policy gate uses `cargo-deny` 0.20.2. Install that exact version when it is not already available:

```bash
cargo install cargo-deny --version 0.20.2 --locked
```
