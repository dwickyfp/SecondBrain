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
secondbrain doctor <workspace>                            # one report on workspace health
```

`--json` is accepted by every command and emits a stable machine contract. Neither form of output ever contains ANSI escapes.

### Previewing and applying a change

`diff` and `transaction apply` are two commands on purpose. `diff` derives the semantic operations an incoming file implies and mutates nothing; `transaction apply` takes a plan an operator has seen, re-checks its preconditions, and applies it. There is no one-shot path from an incoming file to a write, because a change nobody looked at is a change nobody approved.

```bash
secondbrain diff vault notes/meeting.md edited.md --out plan.json
secondbrain transaction apply vault plan.json
```

### Exit codes

| Code | Meaning |
| ---- | ------- |
| `0` | The command completed and nothing needs attention. |
| `1` | The command could not complete. |
| `2` | The command line itself was invalid. |
| `3` | The change is ambiguous and a human must decide what it meant. |
| `4` | The command completed and reported problems with the workspace. |

Code `3` is deliberately distinct from code `1`: a script that cannot tell "needs a person" from "the tool broke" will either page someone for a routine ambiguity or silently drop one. `diff` returns it for a change the semantic diff could not resolve, and `transaction apply` returns it rather than applying such a plan.

Code `4` means the command ran correctly and is reporting on the workspace: `validate` returns it for notes that do not parse, do not round-trip, or claim an identity another note claims; `recovery check` returns it when an edit was abandoned or a journal was quarantined; `doctor` returns it for workspace-state problems. Broken links and orphaned notes are reported as counts but do not affect the exit code — a vault with broken links is a vault, not a broken workspace.

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
