# Task 50 Native Installer Evidence

Status: **PENDING - this template is not completion evidence.**

Ordinary CI is expected to produce unsigned DMG, MSI, and DEB package artifacts. A
protected, manually dispatched workflow may sign only when platform credentials
exist and native verification succeeds. No local native artifacts or signatures
are represented by this file.

## Provenance

| Field | Value |
| --- | --- |
| Full commit | pending |
| CI run URL | pending |
| Source repository | pending |
| Desktop version | pending |
| Tauri CLI / Rust / Node versions | pending |
| Markdown fixture manifest SHA-256 | pending |

## Exact Artifacts

| Platform | Primary package artifact | SHA-256 | Exact-artifact launch smoke | Signing | Notarization | Evidence JSON |
| --- | --- | --- | --- | --- | --- | --- |
| macOS | DMG: pending | pending | PENDING | PENDING | PENDING | pending |
| Windows | MSI: pending | pending | PENDING | PENDING | not applicable | pending |
| Linux | DEB: pending | pending | PENDING | PENDING | not applicable | pending |

Allowed signing values are `unsigned`, `signed`, `not-configured`, and `failed`.
`signed` requires native verification of the exact installer. macOS
`notarized` additionally requires `xcrun stapler validate` on the exact DMG.

The automated macOS smoke mounts the exact DMG and launches its app. Windows and
Linux smoke administratively extract the exact MSI or DEB and launch the packaged
executable from that extraction; these checks do not claim installation, upgrade,
uninstall, Start Menu, registry, or package-manager semantics.

## Required Linkage

- [ ] Link immutable CI artifacts containing the exact package artifact, `SHA256SUMS`, `readiness.json`, `native-memory.json`, and `evidence.json`.
- [ ] Verify each manifest with `node apps/desktop/scripts/verify-package.mjs <path>/SHA256SUMS`.
- [ ] Link Task 48 manual assistive-technology runs to these exact artifact names and SHA-256 values.
- [ ] Record unsupported signing/notarization environments and missing credentials explicitly.
- [ ] Record any package-format alternative only if a primary format is unavailable, with reason and equivalent exact-artifact smoke evidence.
- [ ] Confirm each `native-memory.json` contains raw parent-plus-descendant RSS samples and is SHA-256-bound by `performance.sample_sha256`; hosted CI must say `recorded-not-enforced`, while a named reference run must strictly pass `<250 MiB`.

## Manual AT Linkage

The pending template at `docs/evidence/task-48-manual-at-pending.md` must be
completed against the exact native artifacts above. Browser E2E, a development
binary, or an installer rebuilt from the same commit is not equivalent evidence.
