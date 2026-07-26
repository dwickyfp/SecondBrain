import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const script = new URL('./locate-artifact.mjs', import.meta.url).pathname;

test('locates exactly one artifact recursively without shell globbing', () => {
  const root = mkdtempSync(join(tmpdir(), 'secondbrain-artifact-'));
  try {
    mkdirSync(join(root, 'nested'));
    const artifact = join(root, 'nested', 'SecondBrain.DMG');
    writeFileSync(artifact, 'artifact');
    assert.equal(execFileSync(process.execPath, [script, root, '.dmg'], { encoding: 'utf8' }), `SB_EXACT_INSTALLER=${artifact}\n`);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('rejects zero or multiple artifact matches', () => {
  const root = mkdtempSync(join(tmpdir(), 'secondbrain-artifact-'));
  try {
    assert.notEqual(spawnSync(process.execPath, [script, root, '.msi']).status, 0);
    writeFileSync(join(root, 'one.msi'), 'one');
    writeFileSync(join(root, 'two.msi'), 'two');
    assert.notEqual(spawnSync(process.execPath, [script, root, '.msi']).status, 0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
