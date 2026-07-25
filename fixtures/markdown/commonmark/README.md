# Markdown Round-Trip Corpus Fixtures

Curated Markdown fixtures for lossless round-trip testing. Every `.md` file in
this directory tree is parsed, serialized (reconstructed), parsed again, and
checked for byte-identical round-trip and semantic-fingerprint stability.

## Layout

- `commonmark/` — CommonMark spec examples and edge cases
- `gfm/` — GitHub-Flavored Markdown extensions (tables, strikethrough, task lists, autolinks)
- `obsidian/` — Obsidian-specific syntax (wikilinks, embeds, callouts, tags)
- `edge/` — Mixed line endings, Unicode, deeply nested structures, empty/whitespace documents
