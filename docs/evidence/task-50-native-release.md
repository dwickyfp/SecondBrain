# Task 50 Native Installer Evidence

Status: **AUTOMATED UNSIGNED RELEASE EVIDENCE COMPLETE.**

Ordinary CI produced unsigned DMG, MSI, and DEB artifacts from a clean immutable
commit. CI smoke-tested the exact artifacts, generated checksums and package
receipts, and verified those receipts before upload. A protected, manually
dispatched workflow may sign only when platform credentials exist and native
verification succeeds; no signing or notarization is claimed here.

## Provenance

| Field | Value |
| --- | --- |
| Full commit | `77f58c9891e0bf12be020470ba8869494ebfeb20` |
| CI run URL | [`30210110453`](https://github.com/dwickyfp/SecondBrain/actions/runs/30210110453) |
| Source repository | `https://github.com/dwickyfp/SecondBrain` |
| Desktop version | `0.1.0` |
| Tauri CLI / Rust / Node versions | `tauri-cli 2.11.4` / `rustc 1.91.1` / `v24.18.0` |
| Source state | clean; semantic tracked diff SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

## Exact Artifacts

| Platform | Primary package artifact | SHA-256 | Exact-artifact launch smoke | Signing | Notarization | Evidence JSON |
| --- | --- | --- | --- | --- | --- | --- |
| macOS | `SecondBrain_0.1.0_aarch64.dmg` | `647500b7e80108f0775d1f7adcc812da9e948016da0a953a734940cfc394b08a` | PASS: mounted and launched | unsigned | not applicable | `task-50-packaging-macos.json` |
| Windows | `SecondBrain_0.1.0_x64_en-US.msi` | `cf4f9f3cb302e84c8f4ac65a2eadd38b549df98d1223061f8e82b5f43f7677d9` | PASS: extracted and launched | unsigned | not applicable | `task-50-packaging-windows.json` |
| Linux | `SecondBrain_0.1.0_amd64.deb` | `c2e7bc661fff599ac4647bdd6b83326837df9faa58252411168110ee9c80f986` | PASS: extracted and launched | unsigned | not applicable | `task-50-packaging-linux.json` |

Allowed signing values are `unsigned`, `signed`, `not-configured`, and `failed`.
`signed` requires native verification of the exact installer. macOS
`notarized` additionally requires `xcrun stapler validate` on the exact DMG.

The automated macOS smoke mounts the exact DMG and launches its app. Windows and
Linux smoke administratively extract the exact MSI or DEB and launch the packaged
executable from that extraction; these checks do not claim installation, upgrade,
uninstall, Start Menu, registry, or package-manager semantics.

## Required Linkage

- [x] Immutable CI artifacts contain the exact package, `SHA256SUMS`, `readiness.json`, `native-memory.json`, and `evidence.json` and are linked by run `30210110453`.
- [x] All three downloaded manifests were independently verified with `node apps/desktop/scripts/verify-package.mjs <path>/SHA256SUMS` on 2026-07-26.
- [ ] Link Task 48 manual assistive-technology runs to these exact artifact names and SHA-256 values.
- [x] Signing/notarization is explicitly unclaimed: ordinary CI intentionally emits unsigned artifacts and protected credentials were not used.
- [x] No package-format alternative was needed; all three primary formats were produced.
- [x] Each `native-memory.json` has 20 raw process-tree RSS samples and is bound by `performance.sample_sha256`.
- [ ] A named reference packaged run must strictly pass `<250 MiB`; hosted CI is `recorded-not-enforced` by contract.

## Readiness And Hosted Memory

All readiness receipts report `frontend-page-loaded-and-backend-ready` and bind
the implementation commit. Hosted package RSS is diagnostic, not an absolute
budget result because GitHub-hosted machines are not named reference hardware.

| Platform | Readiness SHA-256 | Native RSS p95 | Classification |
| --- | --- | ---: | --- |
| macOS | `1ae9332d83eed428c85db31f848fb7b5971126bd18226ff615bb8bda65ecbf2c` | 96,141,312 bytes (91.7 MiB) | hosted CI, recorded-not-enforced |
| Windows | `c6e7c8afea49b8ed7352bb76739964f5ef2d64c5fdfbd5d7e0b1d03e9a5bbc2e` | 355,291,136 bytes (338.8 MiB) | hosted CI, recorded-not-enforced |
| Linux | `cf0da85274fafdd94ce0e7f063d54d8391396f2d559deeb34a0044512ec130a0` | 658,632,704 bytes (628.1 MiB) | hosted CI, recorded-not-enforced |

Windows and Linux hosted observations exceed the 250 MiB reference target.
They are not represented as passes. The target remains open until measured and
passed on the named reference configuration required by the Phase 1 contract.

## Manual AT Linkage

The checklist at `docs/evidence/task-48-manual-at-pending.md` is bound to the
exact native artifacts above and remains pending. Browser E2E, a development
binary, or an installer rebuilt from the same commit is not equivalent evidence.
