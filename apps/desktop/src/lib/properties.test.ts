import { describe, expect, it } from 'vitest';
import { isPropertyValue, parsePropertyPreview } from './properties';

const transaction = {
  format: 'sb-transaction-preview-v1', workspace_id: 'workspace', note_id: 'note', path: 'note.md',
  actor: 'secondbrain-desktop', device: 'local-desktop', expected_hash: 'a', expected_version: 0,
  proposed_source_hash: 'b', review_required: false, summary: 'SetProperty x1', operations: []
};

describe('property wire contracts', () => {
  it('accepts recursive JSON-compatible values and a bound transaction preview', () => {
    const payload = {
      format: 'sb-property-preview-v1',
      edit: { action: 'set', key: 'metadata', value: { ready: true, values: [null, 2, 'three'] } },
      properties: { metadata: { ready: true, values: [null, 2, 'three'] } },
      transaction
    };
    expect(isPropertyValue(payload.properties.metadata)).toBe(true);
    expect(parsePropertyPreview(payload)).toEqual(payload);
  });

  it('rejects non-JSON numbers and malformed previews', () => {
    expect(isPropertyValue(Number.NaN)).toBe(false);
    expect(() => parsePropertyPreview({ format: 'sb-property-preview-v1', properties: {}, edit: {}, transaction })).toThrow();
  });
});
