# SecondBrain

SecondBrain is a Rust-first project for a local-first knowledge system with Markdown as its durable representation.

This repository currently contains the Phase 0 Rust workspace: the domain, Markdown, vault, index, and transaction crates, and the `secondbrain` command-line surface over them. Product features described in `docs/` are design and implementation targets, not completed functionality.

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

`--json` is accepted by every command and emits a stable machine contract. Neither form of output ever contains ANSI escapes.

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

For every note the workspace has converged on at least once, `reconcile` compares the file with that converged base and hands the difference to the external-edit coordinator, which recovers the semantic operations the editor performed and journals them as an attributed transaction. It then refreshes the derived index. Notes are reported one per line — `adopted`, `merged`, `review_required`, `deleted`, or `unchanged` — so an operator can see what happened to each.

Three things it deliberately does not do. It never rewrites a note: the editor's bytes are the result, and the workspace is catching up to them. It never touches a note whose file still holds its converged base. And it never guesses at an ambiguous change — that gets a review descriptor and exit code `3`, the same code `diff` uses for the same fact.

A note the workspace has never converged on has no earlier state to have diverged from, so `reconcile` does not consider it; `diff` and `transaction apply` bring such a note under management by recording its first converged base.

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
