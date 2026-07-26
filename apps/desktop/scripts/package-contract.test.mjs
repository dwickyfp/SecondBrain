import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { validateEvidence, validateMemoryReceipt, validateReadiness } from './package-contract.mjs';

const commit = 'a'.repeat(40);
const hash = 'b'.repeat(64);
const readiness = { schema: 'secondbrain.desktop.readiness.v1', version: '0.1.0', commit, platform: 'macos', fixture_sha256: hash, diagnostics: 'frontend-page-loaded-and-backend-ready' };
const memory = { schema: 'secondbrain.native-process-tree-rss.v1', version: 1, platform: 'macos', root_pid: 123, sample_interval_ms: 500, sample_count: 5, methodology: 'test parent and descendants', raw_rss_bytes: [100, 200, 300, 400, 500], raw_process_counts: [3, 3, 3, 3, 3], p50_bytes: 300, p95_bytes: 500, p99_bytes: 500 };
const evidence = { schema: 'secondbrain.packaging-evidence.v1', version: '0.1.0', commit, platform: 'macOS', installer: { name: 'SecondBrain.dmg', kind: 'macOS-DMG', extension: '.dmg', sha256: hash, smoke: 'passed-exact-artifact-mount-and-launch', readiness_sha256: hash }, performance: { sample_sha256: hash, p95_bytes: 200 * 1024 * 1024, budget_bytes: 250 * 1024 * 1024, status: 'passed', mode: 'named-reference' }, fixture_sha256: hash, tools: { tauri_cli: 'tauri-cli 2.11.4', rustc: 'rustc 1.91.1', node: 'v24.0.0' }, signing: { status: 'unsigned', notarization_status: 'not-applicable', reason: 'ordinary CI' }, inputs: { root_cargo_lock_sha256: hash, desktop_cargo_lock_sha256: hash, npm_lock_sha256: hash, budget_sha256: hash, source_sha256: hash }, source: { dirty: false, diff_sha256: hash, mutable: false }, ci: { run_url: 'https://github.com/org/repo/actions/runs/123' } };

test('accepts strict bound readiness and package evidence', () => {
  assert.doesNotThrow(() => validateReadiness(readiness, commit, hash));
  assert.doesNotThrow(() => validateMemoryReceipt(memory));
  assert.doesNotThrow(() => validateEvidence(evidence, 'SecondBrain.dmg', hash));
});

for (const [name, mutate] of [
  ['unknown field', (value) => { value.extra = true; }],
  ['kind mismatch', (value) => { value.installer.kind = 'Windows-MSI'; }],
  ['extension mismatch', (value) => { value.installer.extension = '.msi'; }],
  ['mutable source', (value) => { value.source.dirty = true; value.source.mutable = true; }],
  ['missing tool metadata', (value) => { value.tools.rustc = 'unknown'; }],
  ['invalid CI URL', (value) => { value.ci.run_url = 'local'; }],
  ['memory budget equality', (value) => { value.performance.p95_bytes = value.performance.budget_bytes; }],
  ['false hosted pass', (value) => { value.performance = { ...value.performance, mode: 'hosted-ci-smoke-non-reference', status: 'passed', budget_bytes: 250 * 1024 * 1024 }; }],
  ['invalid memory hash', (value) => { value.performance.sample_sha256 = 'nope'; }],
  ['unnotarized signed macOS', (value) => { value.signing.status = 'signed'; }],
  ['unknown signing status', (value) => { value.signing.status = 'pending'; }],
  ['unknown notarization status', (value) => { value.signing.notarization_status = 'pending'; }],
  ['failed signing with notarized macOS', (value) => { value.signing.status = 'failed'; value.signing.notarization_status = 'notarized'; }],
  ['notarized Windows package', (value) => { value.platform = 'Windows'; value.installer = { ...value.installer, name: 'SecondBrain.msi', kind: 'Windows-MSI', extension: '.msi', smoke: 'passed-exact-artifact-extraction-and-launch' }; value.signing.notarization_status = 'notarized'; }]
]) test(`rejects ${name}`, () => {
  const value = structuredClone(evidence);
  mutate(value);
  assert.throws(() => validateEvidence(value, value.installer.name, hash));
});

test('rejects a changed readiness identity and unknown receipt fields', () => {
  assert.throws(() => validateReadiness({ ...readiness, commit: 'c'.repeat(40) }, commit, hash));
  assert.throws(() => validateReadiness({ ...readiness, extra: true }, commit, hash));
  assert.throws(() => validateReadiness({ ...readiness, diagnostics: 'tauri-startup-complete' }, commit, hash));
  assert.throws(() => validateReadiness(readiness, commit, hash, '0.2.0'));
});

test('accepts hosted memory smoke only without an absolute result', () => {
  const value = structuredClone(evidence);
  value.performance = { ...value.performance, mode: 'hosted-ci-smoke-non-reference', status: 'recorded-not-enforced', budget_bytes: null };
  assert.doesNotThrow(() => validateEvidence(value, 'SecondBrain.dmg', hash));
});

test('accepts local mutable evidence only with explicit opt-in', () => {
  const value = structuredClone(evidence);
  value.source.dirty = true;
  value.source.mutable = true;
  value.ci.run_url = 'local';
  assert.throws(() => validateEvidence(value, 'SecondBrain.dmg', hash));
  assert.doesNotThrow(() => validateEvidence(value, 'SecondBrain.dmg', hash, { allowMutable: true }));
});

test('rejects malformed or falsified raw memory receipts', () => {
  assert.throws(() => validateMemoryReceipt({ ...memory, p95_bytes: 400 }));
  assert.throws(() => validateMemoryReceipt({ ...memory, sample_count: 4 }));
  assert.throws(() => validateMemoryReceipt({ ...memory, raw_rss_bytes: [] }));
  assert.throws(() => validateMemoryReceipt({ ...memory, extra: true }));
});

test('verifier binds the raw memory receipt by SHA-256', () => {
  const directory = mkdtempSync(join(tmpdir(), 'secondbrain-package-contract-'));
  try {
    const artifactName = 'SecondBrain.dmg';
    const artifactBytes = Buffer.from('exact artifact');
    const readinessBytes = Buffer.from(`${JSON.stringify(readiness)}\n`);
    const memoryBytes = Buffer.from(`${JSON.stringify(memory)}\n`);
    const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
    const value = structuredClone(evidence);
    value.installer.sha256 = digest(artifactBytes);
    value.installer.readiness_sha256 = digest(readinessBytes);
    value.performance.sample_sha256 = digest(memoryBytes);
    value.performance.p95_bytes = memory.p95_bytes;
    writeFileSync(join(directory, artifactName), artifactBytes);
    writeFileSync(join(directory, 'SHA256SUMS'), `${value.installer.sha256}  ${artifactName}\n`);
    writeFileSync(join(directory, 'readiness.json'), readinessBytes);
    writeFileSync(join(directory, 'native-memory.json'), memoryBytes);
    writeFileSync(join(directory, 'evidence.json'), `${JSON.stringify(value)}\n`);
    assert.match(execFileSync(process.execPath, [new URL('./verify-package.mjs', import.meta.url).pathname, join(directory, 'SHA256SUMS')], { encoding: 'utf8' }), /^verified /);
    writeFileSync(join(directory, 'native-memory.json'), `${JSON.stringify({ ...memory, p95_bytes: 400 })}\n`);
    const result = spawnSync(process.execPath, [new URL('./verify-package.mjs', import.meta.url).pathname, join(directory, 'SHA256SUMS')], { encoding: 'utf8' });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /native memory receipt checksum mismatch/);
    writeFileSync(join(directory, 'native-memory.json'), memoryBytes);
    const mismatchedReadiness = Buffer.from(`${JSON.stringify({ ...readiness, version: '9.9.9' })}\n`);
    value.installer.readiness_sha256 = digest(mismatchedReadiness);
    writeFileSync(join(directory, 'readiness.json'), mismatchedReadiness);
    writeFileSync(join(directory, 'evidence.json'), `${JSON.stringify(value)}\n`);
    const mismatch = spawnSync(process.execPath, [new URL('./verify-package.mjs', import.meta.url).pathname, join(directory, 'SHA256SUMS')], { encoding: 'utf8' });
    assert.notEqual(mismatch.status, 0);
    assert.match(mismatch.stderr, /readiness fields are invalid/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
