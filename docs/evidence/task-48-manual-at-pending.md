# Task 48 Manual Assistive-Technology Evidence

Status: **PENDING - this template is not completion evidence.**

## Artifact Identity

- Commit: `77f58c9891e0bf12be020470ba8869494ebfeb20`
- CI run URL: `https://github.com/dwickyfp/SecondBrain/actions/runs/30210110453`

| Platform | Exact installer | Installer SHA-256 | OS/tool version | Executable SHA-256 |
| --- | --- | --- | --- | --- |
| macOS | `SecondBrain_0.1.0_aarch64.dmg` | `647500b7e80108f0775d1f7adcc812da9e948016da0a953a734940cfc394b08a` | pending manual run | pending manual run |
| Windows | `SecondBrain_0.1.0_x64_en-US.msi` | `cf4f9f3cb302e84c8f4ac65a2eadd38b549df98d1223061f8e82b5f43f7677d9` | pending manual run | pending manual run |
| Linux | `SecondBrain_0.1.0_amd64.deb` | `c2e7bc661fff599ac4647bdd6b83326837df9faa58252411168110ee9c80f986` | pending manual run | pending manual run |

## Required Manual Runs

| Platform | Tool | Version | Status | Tester | Date | Notes / Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| macOS | VoiceOver | pending | PENDING | pending | pending | pending |
| Windows | NVDA | pending | PENDING | pending | pending | pending |
| Linux | Orca or named equivalent | pending | PENDING | pending | pending | pending |

## Checklist

- [ ] Open an initialized real workspace using only the named assistive technology and keyboard.
- [ ] Navigate notes, tabs, split panes, editor modes, graph nodes, and context.
- [ ] Open and dismiss the command palette; verify announcement, focus containment, Escape, and focus restoration.
- [ ] Preview and apply an edit; verify busy, success, review-required, and error announcements.
- [ ] Inspect health and run recovery against a disposable real workspace; verify repaired, quarantined, abandoned, and review-required outcomes are spoken without implying data was discarded.
- [ ] Verify the exact packaged executable identified above, not a Vite or mocked browser build.
- [ ] Attach logs/screenshots/recordings and document every failure, waiver, or unsupported tool explicitly.
