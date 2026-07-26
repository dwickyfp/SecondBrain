import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { validateEvidence, validateMemoryReceipt, validateReadiness } from './package-contract.mjs';

const [artifactArg, outputArg, readinessArg, memoryArg] = process.argv.slice(2);
if (!artifactArg || !outputArg || !readinessArg || !memoryArg) throw new Error('usage: node package-evidence.mjs <exact-installer> <output-directory> <readiness-marker> <memory-receipt>');

const artifact = resolve(artifactArg);
const output = resolve(outputArg);
const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const name = basename(artifact);
const packageJson = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
const bytes = await readFile(artifact);
const sha256 = createHash('sha256').update(bytes).digest('hex');
const required = ['SB_BUILD_COMMIT', 'SB_FIXTURE_SHA256', 'SB_PLATFORM', 'SB_INSTALLER_KIND', 'SB_CI_RUN_URL', 'SB_PERFORMANCE_MODE'];
for (const key of required) {
  if (!process.env[key]) throw new Error(`${key} is required`);
}
const git = (args) => execFileSync('git', args, { cwd: repositoryRoot, encoding: 'utf8' }).trim();
const revision = git(['rev-parse', 'HEAD']);
const sourceStatus = git(['status', '--porcelain']);
const sourceDiff = git(['diff', '--binary']);
if (revision !== process.env.SB_BUILD_COMMIT) throw new Error('SB_BUILD_COMMIT is not the checked out revision');

await mkdir(output, { recursive: true });
await copyFile(artifact, join(output, name));
await writeFile(join(output, 'SHA256SUMS'), `${sha256}  ${name}\n`);
const readinessBytes = await readFile(resolve(readinessArg));
const readiness = JSON.parse(readinessBytes);
validateReadiness(readiness, process.env.SB_BUILD_COMMIT, process.env.SB_FIXTURE_SHA256, packageJson.version);
await writeFile(join(output, 'readiness.json'), readinessBytes);
const memoryBytes = await readFile(resolve(memoryArg));
const memoryReceipt = JSON.parse(memoryBytes);
validateMemoryReceipt(memoryReceipt);
const receiptPlatforms = { macOS: 'macos', Windows: 'windows', Linux: 'linux' };
if (memoryReceipt.platform !== receiptPlatforms[process.env.SB_PLATFORM]) throw new Error('memory receipt platform does not match package platform');
await writeFile(join(output, 'native-memory.json'), memoryBytes);
const hashFile = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');
const kind = process.env.SB_INSTALLER_KIND;
const expected = { 'macOS-DMG': ['macOS', '.dmg'], 'Windows-MSI': ['Windows', '.msi'], 'Linux-DEB': ['Linux', '.deb'] }[kind];
if (!expected || process.env.SB_PLATFORM !== expected[0] || !name.toLowerCase().endsWith(expected[1])) throw new Error('platform, installer kind, and extension do not agree');
if (!['named-reference', 'hosted-ci-smoke-non-reference'].includes(process.env.SB_PERFORMANCE_MODE)) throw new Error('SB_PERFORMANCE_MODE is invalid');
const evidence = {
  schema: 'secondbrain.packaging-evidence.v1',
  version: packageJson.version,
  commit: revision,
  source: { dirty: Boolean(sourceStatus), diff_sha256: createHash('sha256').update(sourceDiff).digest('hex'), mutable: Boolean(sourceStatus) },
  platform: process.env.SB_PLATFORM,
  installer: { name, kind, extension: expected[1], sha256, smoke: process.env.SB_PLATFORM === 'macOS' ? 'passed-exact-artifact-mount-and-launch' : 'passed-exact-artifact-extraction-and-launch', readiness_sha256: createHash('sha256').update(readinessBytes).digest('hex') },
  performance: process.env.SB_PERFORMANCE_MODE === 'named-reference' ? {
    sample_sha256: createHash('sha256').update(memoryBytes).digest('hex'),
    p95_bytes: memoryReceipt.p95_bytes,
    budget_bytes: 250 * 1024 * 1024,
    status: memoryReceipt.p95_bytes < 250 * 1024 * 1024 ? 'passed' : 'failed',
    mode: 'named-reference'
  } : {
    sample_sha256: createHash('sha256').update(memoryBytes).digest('hex'),
    p95_bytes: memoryReceipt.p95_bytes,
    budget_bytes: null,
    status: 'recorded-not-enforced',
    mode: process.env.SB_PERFORMANCE_MODE
  },
  fixture_sha256: process.env.SB_FIXTURE_SHA256,
  tools: {
    tauri_cli: process.env.SB_TAURI_CLI ?? 'unknown',
    rustc: process.env.SB_RUSTC ?? 'unknown',
    node: process.version
  },
  signing: {
    status: process.env.SB_SIGNING_STATUS ?? 'unsigned',
    notarization_status: process.env.SB_NOTARIZATION_STATUS ?? 'not-applicable',
    reason: process.env.SB_SIGNING_REASON ?? 'ordinary CI intentionally produces unsigned installers'
  },
  inputs: { root_cargo_lock_sha256: hashFile(join(repositoryRoot, 'Cargo.lock')), desktop_cargo_lock_sha256: hashFile(join(repositoryRoot, 'apps/desktop/src-tauri/Cargo.lock')), npm_lock_sha256: hashFile(join(repositoryRoot, 'apps/desktop/package-lock.json')), budget_sha256: hashFile(join(repositoryRoot, 'apps/desktop/performance-budgets.json')), source_sha256: hashFile(join(repositoryRoot, 'apps/desktop/src-tauri/src/lib.rs')) },
  ci: { run_url: process.env.SB_CI_RUN_URL }
};
validateEvidence(evidence, name, sha256, { allowMutable: process.env.SB_ALLOW_MUTABLE_EVIDENCE === '1' });
await writeFile(join(output, 'evidence.json'), `${JSON.stringify(evidence, null, 2)}\n`);
