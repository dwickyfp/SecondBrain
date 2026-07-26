import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { validateEvidence, validateMemoryReceipt, validateReadiness } from './package-contract.mjs';

const manifest = resolve(process.argv[2] ?? 'SHA256SUMS');
const line = (await readFile(manifest, 'utf8')).trim();
const match = /^([a-f0-9]{64})  ([^/\\]+)$/.exec(line);
if (!match) throw new Error('SHA256SUMS must contain exactly one portable SHA-256 entry');
const artifact = join(dirname(manifest), match[2]);
const actual = createHash('sha256').update(await readFile(artifact)).digest('hex');
if (actual !== match[1]) throw new Error(`checksum mismatch for ${basename(artifact)}`);

const evidence = JSON.parse(await readFile(join(dirname(manifest), 'evidence.json'), 'utf8'));
validateEvidence(evidence, match[2], actual, { allowMutable: process.env.SB_ALLOW_MUTABLE_EVIDENCE === '1' });
const readinessBytes = await readFile(join(dirname(manifest), 'readiness.json'));
const readinessSha256 = createHash('sha256').update(readinessBytes).digest('hex');
if (readinessSha256 !== evidence.installer.readiness_sha256) throw new Error('readiness receipt checksum mismatch');
validateReadiness(JSON.parse(readinessBytes), evidence.commit, evidence.fixture_sha256, evidence.version);
const memoryBytes = await readFile(join(dirname(manifest), 'native-memory.json'));
const memorySha256 = createHash('sha256').update(memoryBytes).digest('hex');
if (memorySha256 !== evidence.performance.sample_sha256) throw new Error('native memory receipt checksum mismatch');
const memoryReceipt = JSON.parse(memoryBytes);
validateMemoryReceipt(memoryReceipt);
if (memoryReceipt.p95_bytes !== evidence.performance.p95_bytes) throw new Error('native memory receipt p95 does not match evidence');
const receiptPlatforms = { macOS: 'macos', Windows: 'windows', Linux: 'linux' };
if (memoryReceipt.platform !== receiptPlatforms[evidence.platform]) throw new Error('native memory receipt platform does not match evidence');
process.stdout.write(`verified ${match[2]} ${actual}\n`);
