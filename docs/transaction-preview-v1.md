# Transaction Preview Adapter Contract v1

`secondbrain-transaction` exposes `sb-transaction-preview-v1` for callers that
have a workspace root, a workspace-relative Markdown path, a proposed complete
source, and validated actor/device identities.

The preview is JSON serializable and contains `workspace_id`, `note_id`, `path`,
`actor`, `device`, `expected_hash`, `expected_version`,
`proposed_source_hash`, `review_required`, `summary`, and semantic `operations`.
Hashes are lowercase BLAKE3 hex digests of exact UTF-8 source bytes. `summary` is
stable: operation kind names are sorted and counted (`ReplaceNode x1`), with
`no semantic change` for an empty operation list.

Consumers must treat `format` as a required discriminator and reject unknown
versions. Apply revalidates the manifest, index, indexed identity, on-disk hash,
converged version, review flag, summary, and proposed hash before committing.
`NeedsReview` previews are descriptive and cannot be applied automatically.

A successful apply returns `transaction_id`, `note_id`, `path`, `changed`,
`version`, and `index_refreshed`. A no-op has `changed: false`, keeps the
expected version, and does not rebuild the index.

The CLI continues to emit and consume `sb-transaction-plan-v1`; it is a
compatibility adapter over this contract and intentionally has no proposed source
field. TypeScript consumers should use the preview shape above rather than the
CLI plan shape.
