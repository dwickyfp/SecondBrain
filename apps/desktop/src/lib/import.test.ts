import { describe, expect, it } from 'vitest';
import { IMPORT_PREVIEW_FORMAT, validateImportOutcome, validateImportPreview } from './import';

const preview = () => ({
  format: IMPORT_PREVIEW_FORMAT, root: '/vault', fingerprint: 'abc', alreadyInitialized: false, workspaceId: null,
  manifestFormatVersion: null, manifestFingerprint: null,
  inventory: { markdown: ['a.md'], attachments: [], obsidianConfig: ['.obsidian/app.json'], ignored: [], unsupported: [], symlinks: [] },
  parseErrors: [], duplicateIds: [], portableCollisions: [], brokenLinks: [], ambiguousLinks: [],
  plannedWrites: { markdown: 0, attachments: 0, obsidianConfig: 0 }, canApply: true
});

describe('import boundary validators', () => {
  it('accepts the complete v1 preview and outcome', () => {
    expect(validateImportPreview(preview()).inventory.markdown).toEqual(['a.md']);
    expect(validateImportOutcome({
      format: IMPORT_PREVIEW_FORMAT, status: 'initialized', root: '/vault', workspaceId: 'workspace', fingerprint: 'abc',
      index: { indexed: 1, skipped: 0, brokenLinks: 0, orphans: 1 }
    }).status).toBe('initialized');
  });

  it('accepts an initialized preview bound to its exact manifest', () => {
    const result = validateImportPreview({
      ...preview(), alreadyInitialized: true, workspaceId: 'workspace',
      manifestFormatVersion: 1, manifestFingerprint: 'f'.repeat(64)
    });
    expect(result.manifestFormatVersion).toBe(1);
    expect(result.manifestFingerprint).toBe('f'.repeat(64));
  });

  it('fails closed on wrong types, versions, and any planned source change', () => {
    expect(() => validateImportPreview({ ...preview(), format: 'v2' })).toThrow('unsupported');
    expect(() => validateImportPreview({ ...preview(), canApply: 'yes' })).toThrow('boolean');
    expect(() => validateImportPreview({ ...preview(), surprise: true })).toThrow('unknown fields');
    expect(() => validateImportPreview({ ...preview(), plannedWrites: { markdown: 1, attachments: 0, obsidianConfig: 0 } })).toThrow('zero source');
    expect(() => validateImportOutcome({ format: IMPORT_PREVIEW_FORMAT, status: 'maybe' })).toThrow('status');
  });
});
