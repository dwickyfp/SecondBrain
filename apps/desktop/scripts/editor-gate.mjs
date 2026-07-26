import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { arch, cpus, platform, release, totalmem } from 'node:os';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';
import { NoteSession } from '../src/lib/note-session.ts';

const here = dirname(fileURLToPath(import.meta.url));
const desktopRoot = resolve(here, '..');
const repositoryRoot = resolve(desktopRoot, '../..');
const corpusRoot = resolve(repositoryRoot, 'fixtures/markdown');
const output = process.argv[2];
const environmentName = process.env.SECONDBRAIN_BENCH_ENV;
const samples = Number(process.env.SECONDBRAIN_BENCH_SAMPLES ?? 100);
const git = (args) => execFileSync('git', args, { cwd: repositoryRoot, encoding: 'utf8' }).trim();
const gitRevision = git(['rev-parse', 'HEAD']);
const gitStatus = git(['status', '--porcelain']);
const fileSha256 = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');

if (!output || !environmentName) {
  console.error('usage: SECONDBRAIN_BENCH_ENV=<named-machine> npm run editor:benchmark -- <output.json>');
  process.exit(2);
}
if (!Number.isSafeInteger(samples) || samples < 20) throw new Error('SECONDBRAIN_BENCH_SAMPLES must be an integer >= 20');

execFileSync(platform() === 'win32' ? 'npm.cmd' : 'npm', ['run', 'editor:gate'], {
  cwd: desktopRoot,
  stdio: 'inherit'
});

const fixtures = readdirSync(corpusRoot, { recursive: true, withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith('.md'))
  .map((entry) => resolve(entry.parentPath, entry.name))
  .sort()
  .map((path) => {
    const bytes = readFileSync(path);
    return {
      path: relative(repositoryRoot, path),
      bytes: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex')
    };
  });

const source = readFileSync(resolve(corpusRoot, 'obsidian/combined.md'), 'utf8');
const session = new NoteSession({
  noteId: 'benchmark-note', path: 'benchmark.md', source, revision: 0,
  actor: 'benchmark', device: environmentName,
  adapter: { preview: async () => { throw new Error('unused'); }, apply: async () => { throw new Error('unused'); } }
});
const keystrokeSamplesMs = [];
let projected = source;
for (let index = 0; index < samples; index += 1) {
  const offset = (index * 7919) % (projected.length + 1);
  projected = `${projected.slice(0, offset)}x${projected.slice(offset)}`;
  const start = performance.now();
  const update = index % 2
    ? session.updateFromSource(projected, { anchor: offset + 1, head: offset + 1 })
    : session.updateFromWysiwyg(projected, { anchor: offset + 1, head: offset + 1 });
  if (!update || session.draftSource !== projected) throw new Error(`session synchronization failed at sample ${index}`);
  const targetEcho = update.origin === 'source'
    ? session.updateFromWysiwyg(update.source, update.selection)
    : session.updateFromSource(update.source, update.selection);
  if (targetEcho !== null) throw new Error(`projection echo at sample ${index}`);
  keystrokeSamplesMs.push(performance.now() - start);
}

const rustOutput = execFileSync('cargo', [
  'run', '--locked', '--release', '--features', 'benchmark-binaries', '--bin', 'editor-gate-bench', '--', String(samples)
], { cwd: resolve(desktopRoot, 'src-tauri'), encoding: 'utf8' });
const rustSamples = rustOutput.trim().split('\n').map((line) => {
  const [previewUs, materializationUs] = line.split(',').map(Number);
  if (!Number.isFinite(previewUs) || !Number.isFinite(materializationUs)) throw new Error(`invalid Rust sample: ${line}`);
  return { preview_ms: previewUs / 1000, materialization_ms: materializationUs / 1000 };
});

function percentile(values, percentileValue) {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.ceil((percentileValue / 100) * sorted.length) - 1];
}
function metric(raw, budgetMs) {
  const percentilesMs = { p50: percentile(raw, 50), p95: percentile(raw, 95), p99: percentile(raw, 99), max: Math.max(...raw) };
  return {
    ...(budgetMs === null ? {} : { budget_ms_exclusive: budgetMs, budget_observation_passed: percentilesMs.p95 < budgetMs }),
    percentiles_ms: percentilesMs,
    raw_samples_ms: raw
  };
}

const evidence = {
  schema: 'secondbrain-editor-gate-evidence-v1',
  generated_at: new Date().toISOString(),
  provenance: {
    git_revision: gitRevision,
    dirty: Boolean(gitStatus),
    diff_sha256: createHash('sha256').update(git(['diff', '--binary'])).digest('hex'),
    classification: gitStatus ? 'mutable-local-non-immutable-not-completion' : 'immutable-source',
    cargo_lock_sha256: fileSha256(resolve(repositoryRoot, 'Cargo.lock')),
    npm_lock_sha256: fileSha256(resolve(desktopRoot, 'package-lock.json')),
    performance_budget_sha256: fileSha256(resolve(desktopRoot, 'performance-budgets.json')),
    source_sha256: fileSha256(resolve(desktopRoot, 'src/lib/note-session.ts'))
  },
  environment: {
    name: environmentName, platform: platform(), release: release(), arch: arch(),
    cpu: cpus()[0]?.model ?? 'unknown', logical_cpus: cpus().length, total_memory_bytes: totalmem(),
    node: process.version,
    rustc: execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim()
  },
  commands: {
    correctness: 'npm run editor:gate',
    benchmark_argv: ['npm', 'run', 'editor:benchmark', '--', relative(desktopRoot, resolve(output))],
    benchmark_environment: { SECONDBRAIN_BENCH_ENV: environmentName }
  },
  correctness: {
    passed: true,
    fixture_count: fixtures.length,
    fixture_manifest_sha256: createHash('sha256').update(JSON.stringify(fixtures)).digest('hex'),
    frontend_projection_gate: { passed: true, fixtures: fixtures.length },
    rust_preview_apply_gate: {
      passed: true,
      no_op_fixtures: fixtures.length - 2,
      expected_index_rejections: [
        { path: 'frontmatter/duplicate_id.md', reason: 'duplicate YAML entry' },
        { path: 'frontmatter/malformed.md', reason: 'malformed YAML frontmatter' }
      ],
      rejected_fixture_bytes_untouched: true
    },
    fixtures
  },
  measurements: {
    local_keystroke_session_synchronization: metric(keystrokeSamplesMs, 16),
    rust_transaction_preview: metric(rustSamples.map((sample) => sample.preview_ms), null),
    rust_transaction_materialization: metric(rustSamples.map((sample) => sample.materialization_ms), 100)
  },
  release_budget_caveat: 'These wall-clock observations are a local named-environment baseline, not a hard release gate. Correctness is enforced in CI; absolute latency release budgets require repeated measurements on named reference hardware without hosted-runner contention.'
};
writeFileSync(output, `${JSON.stringify(evidence, null, 2)}\n`);
console.log(JSON.stringify({ output, samples, fixture_count: fixtures.length, measurements: evidence.measurements }, null, 2));
