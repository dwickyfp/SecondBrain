import { describe, expect, expectTypeOf, it, vi } from 'vitest';
import applyFixture from '../../../../docs/fixtures/transaction-apply-outcome-v1.json';
import previewFixture from '../../../../docs/fixtures/transaction-preview-v1.json';
import {
  TRANSACTION_PREVIEW_FORMAT_V1,
  createTauriTransactionAdapter,
  isTransactionApplyOutcome,
  isTransactionPreviewV1,
  parseTransactionApplyOutcome,
  parseTransactionPreviewV1,
  parseDailyNote,
  previewDailyNote,
  applyNoteCreation,
  type TransactionApplyOutcome,
  type TransactionPreviewV1
} from './transactions';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('transaction adapter contracts', () => {
  it('imports and validates the committed Rust preview fixture', () => {
    expect(isTransactionPreviewV1(previewFixture)).toBe(true);
    const preview = parseTransactionPreviewV1(previewFixture);

    expect(preview.format).toBe(TRANSACTION_PREVIEW_FORMAT_V1);
    expect(preview.summary).toBe('no semantic change');
    expect(preview.operations).toEqual([]);
    expectTypeOf(preview).toEqualTypeOf<TransactionPreviewV1>();
  });

  it('imports and validates the committed Rust apply fixture', () => {
    expect(isTransactionApplyOutcome(applyFixture)).toBe(true);
    const outcome = parseTransactionApplyOutcome(applyFixture);

    expect(outcome.changed).toBe(true);
    expect(outcome.index_refreshed).toBe(true);
    expectTypeOf(outcome).toEqualTypeOf<TransactionApplyOutcome>();
  });

  it('rejects unknown preview versions and incomplete outcomes', () => {
    expect(isTransactionPreviewV1({ ...previewFixture, format: 'sb-transaction-preview-v2' }))
      .toBe(false);
    expect(() => parseTransactionPreviewV1({ ...previewFixture, expected_version: -1 }))
      .toThrow('Invalid sb-transaction-preview-v1 payload');
    expect(() => parseTransactionApplyOutcome({ ...applyFixture, transaction_id: undefined }))
      .toThrow('Invalid transaction apply outcome');
  });

  it('adapts preview and apply invokes with a fixed workspace root', async () => {
    invoke.mockResolvedValueOnce(previewFixture).mockResolvedValueOnce(applyFixture);
    const adapter = createTauriTransactionAdapter('/vault');
    const preview = await adapter.preview({
      path: 'note.md', proposed_source: '# Draft\n', actor: 'ignored', device: 'ignored'
    });
    await adapter.apply(preview);

    expect(invoke).toHaveBeenNthCalledWith(1, 'transaction_preview', {
      root: '/vault', path: 'note.md', proposedSource: '# Draft\n'
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'transaction_apply', { root: '/vault', preview });
  });

  it('validates and invokes the daily-note preview then explicit creation apply', async () => {
    invoke.mockReset();
    const preview = {
      format: 'sb-note-create-preview-v1', transaction_id: 'tx', workspace_id: 'workspace',
      note_id: 'note', path: 'Daily/2026-07-26.md', actor: 'secondbrain-desktop',
      device: 'local-desktop', source: '---\nid: note\n---\n# 2026-07-26\n', source_hash: 'hash',
      expected_absent: true, summary: 'Create note at Daily/2026-07-26.md'
    } as const;
    invoke.mockResolvedValueOnce({ status: 'create', preview }).mockResolvedValueOnce({
      transaction_id: 'tx', note_id: 'note', path: preview.path, created: true, version: 0, index_refreshed: true
    });
    const daily = await previewDailyNote('/vault', '2026-07-26');
    expect(parseDailyNote(daily).status).toBe('create');
    if (daily.status === 'create') await applyNoteCreation('/vault', daily.preview);
    expect(invoke).toHaveBeenNthCalledWith(1, 'daily_note', { root: '/vault', date: '2026-07-26' });
    expect(invoke).toHaveBeenNthCalledWith(2, 'note_create_apply', { root: '/vault', preview });
    expect(() => parseDailyNote({ status: 'create', preview: { ...preview, expected_absent: false } })).toThrow();
  });
});
