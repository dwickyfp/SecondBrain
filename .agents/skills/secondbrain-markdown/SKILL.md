---
name: secondbrain-markdown
description: Author source-preserving Markdown for SecondBrain and Obsidian-compatible vaults. Use when creating or revising .md note content, wikilinks, embeds, tags, tasks, headings, callouts, or typed YAML properties. Preserve unknown syntax and existing bytes, and pair content changes with the secondbrain-cli preview/apply workflow rather than writing tracked notes directly.
metadata:
  author: secondbrain
  version: "1.0"
---

# SecondBrain Markdown

SecondBrain uses plain Markdown as the durable user-visible representation. It
supports CommonMark, GFM, and the Obsidian-compatible constructs below while
preserving source it does not understand.

When changing an existing tracked note, construct the intended complete source
in a temporary file and use the `secondbrain-cli` skill's preview/apply workflow.
Do not normalize or reserialize unrelated content.

## Preservation Rules

1. Preserve existing line endings, BOM, frontmatter comments, key ordering,
   whitespace, HTML, code blocks, math, and unknown extensions unless the user
   asks to change them.
2. Make the smallest source edit that satisfies the request.
3. Do not add or alter SecondBrain's `id` property manually.
4. Use CLI property commands for property-only changes.
5. Do not resolve ambiguous wikilinks by guessing. Search or inspect the graph,
   then ask when multiple notes are valid targets.
6. Keep code spans and fenced code literal; wikilinks and tags inside them are
   not knowledge-graph edges.

## Internal Links

```markdown
[[Note Name]]
[[Folder/Note Name]]
[[Note Name|Display label]]
[[Note Name#Heading]]
[[Note Name#^block-id]]
[[#Heading in the same note]]
```

Prefer the shortest target that resolves uniquely. If duplicate note names make
a target ambiguous, use the workspace-relative path. Use standard Markdown
links for web URLs:

```markdown
[SecondBrain](https://example.com)
```

## Embeds

```markdown
![[Note Name]]
![[Note Name#Section]]
![[assets/diagram.png]]
![[assets/diagram.png|640]]
```

Attachments remain ordinary user files. Never place user attachments under
`.secondbrain/`.

## Headings, Tasks, And Tags

```markdown
# Project Alpha

## Next actions

- [ ] Draft the proposal
- [x] Capture the decision

#project/alpha #active
```

Use one space after heading markers. Keep task markers exactly `[ ]` or `[x]`.
Tags may use nested slash-separated names. Avoid placing a tag where it would be
read as part of code or a URL.

## Callouts

Obsidian-compatible callouts are preserved as blockquotes:

```markdown
> [!note] Context
> This decision follows [[Architecture]].

> [!warning]- Hidden details
> Expand before changing the migration plan.
```

Common types include `note`, `info`, `tip`, `warning`, `important`, `todo`,
`example`, `question`, `success`, `failure`, `danger`, `bug`, and `quote`.

## Properties

Properties are YAML frontmatter at the beginning of a note:

```yaml
---
title: Project Alpha
status: active
priority: 2
archived: false
tags:
  - project
  - active
aliases:
  - Alpha
---
```

Supported editable property values are JSON-compatible YAML: null, booleans,
numbers, strings, arrays, and objects. Quote strings when YAML could interpret
them as another type. SecondBrain property commands accept the value as JSON,
for example `status '"active"'` or `tags '["project","active"]'`.

For an existing note, never rewrite the whole frontmatter just to change one
property. Use:

```bash
secondbrain --json property set <workspace> <path> <key> '<json-value>' --out /tmp/property.json
secondbrain --json property apply <workspace> /tmp/property.json
```

## Complete New-Note Example

```markdown
---
title: Project Alpha
status: active
tags:
  - project
---

# Project Alpha

Project Alpha implements the decision in [[Architecture#Storage]].

> [!important] Constraint
> Markdown remains the durable representation.

## Next actions

- [ ] Validate the import fixture
- [ ] Review [[Release checklist]]
```

Create this source outside the vault, then use `secondbrain note create` and
`secondbrain note apply-create` as described by the `secondbrain-cli` skill.

## Verification

After a mutation, verify only through supported reads:

```bash
secondbrain --json note inspect <workspace> <path>
secondbrain --json property read <workspace> <path>
secondbrain --json validate <workspace>
```

Validation failures, broken links, and ambiguous links must be reported rather
than repaired by speculative rewriting.
