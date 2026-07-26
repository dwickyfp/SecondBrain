---
name: secondbrain-cli
description: Operate a SecondBrain Markdown workspace safely through the secondbrain CLI. Use when an agent needs to initialize or import a vault, search or inspect notes, create notes or daily notes, edit content or properties, inspect the graph, reconcile external edits, recover interrupted transactions, or diagnose workspace health. Requires preview-before-apply for every mutation and treats review-required outcomes as human decisions.
compatibility: Requires the secondbrain CLI on PATH, or Cargo and this repository to run it with cargo run -p secondbrain-cli --.
metadata:
  author: secondbrain
  version: "1.0"
---

# SecondBrain CLI

Use the production CLI instead of writing tracked Markdown or `.secondbrain/`
state directly. Request JSON output for automation and branch on exit codes and
stable fields, not human-readable messages.

```bash
secondbrain --json doctor <workspace>
```

If the binary is not installed and this repository is available, replace
`secondbrain` with:

```bash
cargo run -q -p secondbrain-cli --
```

## Safety Rules

1. Never edit `.secondbrain/` or the derived SQLite index directly.
2. Never overwrite an existing tracked note as the normal mutation path.
3. Create an incoming source outside the vault, preview the change, inspect the
   plan, then apply that exact plan.
4. Keep preview files outside the workspace unless the user explicitly chooses a
   workspace-relative scratch location.
5. Do not apply when `review_required` is true, when exit code is `3`, or when
   preconditions are stale. Report the conflict and preserve all files.
6. Use property commands for frontmatter property changes. They preserve
   unrelated source bytes and use the shared transaction path.
7. Use note-creation commands for new notes. Do not create a file in the vault
   first and then attempt to adopt it as a normal create flow.
8. Run `recovery check` after interruption. It is idempotent.

## Read Workflow

```bash
secondbrain --json doctor <workspace>
secondbrain --json search <workspace> "query"
secondbrain --json note inspect <workspace> "folder/note.md"
secondbrain --json graph <workspace>
secondbrain --json property read <workspace> "folder/note.md"
```

Paths passed to note and property commands are workspace-relative. The graph is
derived from the index and reports resolved, broken, and ambiguous links without
silently choosing an ambiguous target.

If the index is missing or intentionally needs regeneration:

```bash
secondbrain --json index rebuild <workspace>
```

SQLite is disposable and derived. Rebuilding the index does not adopt an
unreconciled external edit into transaction history.

## Edit Existing Content

Write the intended complete Markdown source to a temporary file outside the
workspace. Preview first:

```bash
secondbrain --json diff <workspace> "folder/note.md" /tmp/incoming.md --out /tmp/plan.json
```

Inspect `/tmp/plan.json`. Confirm at least:

- `format` is the expected supported plan version;
- `path` is the requested workspace-relative note;
- `review_required` is `false`;
- `operations` match the user's intent;
- no unrelated content is removed.

Only then apply the exact preview:

```bash
secondbrain --json transaction apply <workspace> /tmp/plan.json
```

Apply revalidates note identity, source hash, converged version, and plan
preconditions. If another writer changed the note, generate a fresh preview
rather than bypassing the stale-plan rejection.

## Create Notes

Prepare the complete new note in a temporary source file, then preview and apply:

```bash
secondbrain --json note create <workspace> "Projects/New note.md" /tmp/new-note.md --out /tmp/create.json
secondbrain --json note apply-create <workspace> /tmp/create.json
```

Creation is collision-safe and idempotent. Never use an overwrite flag or remove
an existing target to force creation.

For a daily note, use an explicit ISO date:

```bash
secondbrain --json note daily <workspace> 2026-07-26 --out /tmp/daily.json
secondbrain --json note apply-create <workspace> /tmp/daily.json
```

If the daily note already exists, `note daily` reports the existing identity and
does not emit a creation preview. Do not call `apply-create` in that case.

## Edit Properties

Property values are JSON values. Quote the shell argument so the CLI receives
valid JSON:

```bash
secondbrain --json property set <workspace> "Projects/Alpha.md" status '"active"' --out /tmp/property.json
secondbrain --json property set <workspace> "Projects/Alpha.md" priority '2' --out /tmp/property.json
secondbrain --json property set <workspace> "Projects/Alpha.md" tags '["project","active"]' --out /tmp/property.json
secondbrain --json property remove <workspace> "Projects/Alpha.md" obsolete --out /tmp/property.json
```

Review the preview's property value, transaction path, summary, and
`review_required` field, then apply:

```bash
secondbrain --json property apply <workspace> /tmp/property.json
```

## Adopt An Existing Vault

Import adopts an Obsidian-compatible vault in place. Preview inventories the
vault without writing user files:

```bash
secondbrain --json import preview <vault> --out /tmp/import.json
```

Review parse errors, duplicate identities, collisions, symlinks, broken links,
ambiguous links, and `can_apply`. Apply only the unchanged reviewed preview:

```bash
secondbrain --json import apply <vault> /tmp/import.json
```

Apply creates only SecondBrain internal state and preserves existing Markdown,
attachments, and `.obsidian/` bytes.

## External Edits And Recovery

If another editor already wrote tracked Markdown directly, do not hide or
rewrite those bytes. Journal the observed changes:

```bash
secondbrain --json reconcile <workspace>
```

Treat `review_required` outcomes and exit code `3` as unresolved human work.
After a crash or interrupted apply:

```bash
secondbrain --json recovery check <workspace>
secondbrain --json doctor <workspace>
```

Report quarantined, abandoned, or pending-review outcomes explicitly. Never
describe them as discarded data.

## Exit Codes

| Code | Meaning | Agent behavior |
| ---: | --- | --- |
| `0` | Completed with no required attention | Continue and verify the result. |
| `1` | Command failed | Stop; report the diagnostic code and message. |
| `2` | Invalid command usage | Correct the invocation; do not mutate manually. |
| `3` | Human review required | Stop automation and present the preview/conflict. |
| `4` | Command completed and found workspace problems | Preserve the report and resolve only with an explicit safe workflow. |

Failures emit `{"error":{"code","message"}}` on stderr. Branch on the stable
`SB-*` code, never substring-match the message.

## References

- For supported Markdown and source-preservation guidance, load the
  `secondbrain-markdown` skill.
- See [the CLI contract](../../../README.md#the-secondbrain-cli) for complete
  JSON shapes and diagnostic behavior.
- See [the transaction contract](../../../docs/transaction-preview-v1.md) for
  preview/apply invariants.
