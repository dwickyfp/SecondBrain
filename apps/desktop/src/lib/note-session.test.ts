import { describe, expect, it, vi } from 'vitest';
import {
  NoteSession,
  resolveSelection,
  type EditorMode,
  type NoteSessionOptions
} from './note-session';
import {
  TRANSACTION_PREVIEW_FORMAT_V1,
  type TransactionAdapter,
  type TransactionApplyOutcome,
  type TransactionPreviewV1
} from './transactions';

const BASE = '# Note\n\nBody\n';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function preview(overrides: Partial<TransactionPreviewV1> = {}): TransactionPreviewV1 {
  return {
    format: TRANSACTION_PREVIEW_FORMAT_V1,
    workspace_id: 'workspace',
    note_id: 'note-a',
    path: 'notes/a.md',
    actor: 'person',
    device: 'desktop',
    expected_hash: '0'.repeat(64),
    expected_version: 7,
    proposed_source_hash: '1'.repeat(64),
    review_required: false,
    summary: 'ReplaceNode x1',
    operations: [],
    ...overrides
  };
}

function outcome(overrides: Partial<TransactionApplyOutcome> = {}): TransactionApplyOutcome {
  return {
    transaction_id: 'transaction',
    note_id: 'note-a',
    path: 'notes/a.md',
    changed: true,
    version: 8,
    index_refreshed: true,
    ...overrides
  };
}

function createSession(adapter?: Partial<TransactionAdapter>, options?: Partial<NoteSessionOptions>) {
  const completeAdapter: TransactionAdapter = {
    preview: adapter?.preview ?? vi.fn(async () => preview()),
    apply: adapter?.apply ?? vi.fn(async () => outcome())
  };
  return {
    adapter: completeAdapter,
    session: new NoteSession({
      noteId: 'note-a',
      path: 'notes/a.md',
      source: BASE,
      revision: 7,
      actor: 'person',
      device: 'desktop',
      adapter: completeAdapter,
      ...options
    })
  };
}

describe('NoteSession identity and projection synchronization', () => {
  it('is keyed by immutable note identity rather than path', () => {
    const { session } = createSession();
    expect(session.noteId).toBe('note-a');
    expect(() => new NoteSession({
      noteId: '', path: 'a.md', source: '', revision: 0,
      actor: 'person', device: 'desktop', adapter: createSession().adapter
    })).toThrow('NoteSession requires a note identity');
  });

  it('emits opposite-editor projections once and cannot echo-loop', () => {
    const { session } = createSession();
    const update = session.updateFromWysiwyg(`${BASE}Added\n`, { anchor: 20, head: 20 });

    expect(update?.origin).toBe('wysiwyg');
    expect(session.shouldProject(update!, 'source')).toBe(true);
    expect(session.shouldProject(update!, 'wysiwyg')).toBe(false);
    expect(session.updateFromSource(update!.source, update!.selection)).toBeNull();
    expect(session.draftRevision).toBe(1);
  });

  it('synchronizes deterministic edits from both origins without divergence', () => {
    const { session } = createSession();
    let seed = 0x5ec0_1d;
    let expected = BASE;
    for (let index = 0; index < 250; index += 1) {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      const offset = seed % (expected.length + 1);
      const token = String.fromCharCode(33 + (seed % 90));
      expected = `${expected.slice(0, offset)}${token}${expected.slice(offset)}`;
      const update = index % 2 === 0
        ? session.updateFromWysiwyg(expected, { anchor: offset + 1, head: offset + 1 })
        : session.updateFromSource(expected, { anchor: offset + 1, head: offset + 1 });
      expect(update?.source).toBe(expected);
      const target = update!.origin === 'source' ? 'wysiwyg' : 'source';
      expect(session.shouldProject(update!, target)).toBe(true);
      expect(target === 'source'
        ? session.updateFromSource(expected)
        : session.updateFromWysiwyg(expected)).toBeNull();
    }
    expect(session.draftSource).toBe(expected);
    expect(session.draftRevision).toBe(250);
  });
});

describe('NoteSession modes and lossless source retention', () => {
  it('makes repeated and cyclic mode switches source and revision no-ops', () => {
    const { session } = createSession();
    const modes: EditorMode[] = ['wysiwyg', 'source', 'split'];
    for (let cycle = 0; cycle < 100; cycle += 1) {
      for (const mode of modes) session.setMode(mode);
    }
    const revision = session.draftRevision;
    expect(session.setMode('split')).toBe(false);
    expect(session.baseSource).toBe(BASE);
    expect(session.draftSource).toBe(BASE);
    expect(session.revision).toBe(7);
    expect(session.draftRevision).toBe(revision);
    expect(session.status).toBe('clean');
  });

  it('retains dirty, error, and review state across all mode changes', async () => {
    const error = new Error('offline');
    const { session } = createSession({ preview: vi.fn(async () => { throw error; }) });
    session.updateFromSource(`${BASE}dirty`);
    session.setMode('split');
    expect(session.dirty).toBe(true);
    expect(await session.requestPreview()).toBeNull();
    expect(session.status).toBe('error');
    expect(session.error).toBe(error);
    session.setMode('wysiwyg');
    expect(session.status).toBe('error');
    expect(session.draftSource).toBe(`${BASE}dirty`);

    const reviewed = createSession({
      preview: vi.fn(async () => preview({ review_required: true, summary: 'NeedsReview x1' }))
    }).session;
    reviewed.updateFromWysiwyg(`${BASE}ambiguous`);
    await reviewed.requestPreview();
    reviewed.setMode('source');
    reviewed.setMode('split');
    expect(reviewed.status).toBe('review');
    expect(reviewed.preview?.review_required).toBe(true);
    expect(reviewed.dirty).toBe(true);
    await expect(reviewed.applyPreview()).rejects.toThrow('cannot be applied');
  });

  it('preserves unsupported raw Markdown byte-for-byte through no-op editor cycles', () => {
    const fixtures = [
      '<custom data-x="1">\r\n  opaque  \r\n</custom>\r\n',
      '$$\n\\unknown{α}\n$$\n\n[^odd]: untouched\n',
      '---\r\nkey: !!custom value\r\n---\r\n\r\n%% obsidian comment %%\r\n',
      '```future-language {raw=true}\n\u0000 is still a JS source byte\n```\n'
    ];
    for (const source of fixtures) {
      const { session } = createSession(undefined, { source });
      for (let cycle = 0; cycle < 50; cycle += 1) {
        session.setMode(cycle % 3 === 0 ? 'source' : cycle % 3 === 1 ? 'split' : 'wysiwyg');
        expect(session.updateFromSource(source)).toBeNull();
        expect(session.updateFromWysiwyg(source)).toBeNull();
      }
      expect(session.baseSource).toBe(source);
      expect(session.draftSource).toBe(source);
      expect(session.dirty).toBe(false);
      expect(session.draftRevision).toBe(0);
    }
  });
});

describe('NoteSession preview and apply lifecycle', () => {
  it('previews the exact draft and commits it only after successful apply', async () => {
    const previewCall = vi.fn(async () => preview());
    const applyCall = vi.fn(async () => outcome());
    const { session } = createSession({ preview: previewCall, apply: applyCall });
    const draft = `${BASE}\nNew paragraph\n`;
    session.updateFromSource(draft);

    expect(await session.requestPreview()).toEqual(preview());
    expect(previewCall).toHaveBeenCalledWith({
      path: 'notes/a.md', proposed_source: draft, actor: 'person', device: 'desktop'
    });
    expect(session.baseSource).toBe(BASE);
    expect(await session.applyPreview()).toEqual(outcome());
    expect(applyCall).toHaveBeenCalledWith(preview());
    expect(session.baseSource).toBe(draft);
    expect(session.draftSource).toBe(draft);
    expect(session.revision).toBe(8);
    expect(session.status).toBe('clean');
    expect(session.preview).toBeNull();
  });

  it('retains the draft and preview when apply fails', async () => {
    const failure = new Error('stale on disk');
    const { session } = createSession({ apply: vi.fn(async () => { throw failure; }) });
    session.updateFromSource(`${BASE}draft`);
    await session.requestPreview();

    expect(await session.applyPreview()).toBeNull();
    expect(session.status).toBe('error');
    expect(session.error).toBe(failure);
    expect(session.preview).toEqual(preview());
    expect(session.baseSource).toBe(BASE);
    expect(session.draftSource).toBe(`${BASE}draft`);
    expect(session.dirty).toBe(true);
  });

  it('suppresses a preview response made stale by a newer edit', async () => {
    const pending = deferred<TransactionPreviewV1>();
    const { session } = createSession({ preview: vi.fn(() => pending.promise) });
    session.updateFromSource(`${BASE}first`);
    const request = session.requestPreview();
    session.updateFromWysiwyg(`${BASE}second`);
    pending.resolve(preview());

    expect(await request).toBeNull();
    expect(session.preview).toBeNull();
    expect(session.draftSource).toBe(`${BASE}second`);
    expect(session.status).toBe('dirty');
  });

  it('suppresses out-of-order preview successes and failures', async () => {
    const first = deferred<TransactionPreviewV1>();
    const second = deferred<TransactionPreviewV1>();
    const previewCall = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const { session } = createSession({ preview: previewCall });
    session.updateFromSource(`${BASE}draft`);
    const oldRequest = session.requestPreview();
    const currentRequest = session.requestPreview();
    second.resolve(preview({ summary: 'current' }));
    expect((await currentRequest)?.summary).toBe('current');
    first.reject(new Error('obsolete failure'));
    expect(await oldRequest).toBeNull();
    expect(session.preview?.summary).toBe('current');
    expect(session.error).toBeNull();
  });

  it('suppresses an apply response when editing continues during apply', async () => {
    const pending = deferred<TransactionApplyOutcome>();
    const { session } = createSession({ apply: vi.fn(() => pending.promise) });
    session.updateFromSource(`${BASE}first`);
    await session.requestPreview();
    const apply = session.applyPreview();
    session.updateFromSource(`${BASE}second`);
    pending.resolve(outcome());

    expect(await apply).toBeNull();
    expect(session.baseSource).toBe(BASE);
    expect(session.draftSource).toBe(`${BASE}second`);
    expect(session.revision).toBe(7);
    expect(session.status).toBe('dirty');
  });

  it('rejects mismatched identities and revisions without replacing session state', async () => {
    for (const invalid of [
      preview({ note_id: 'other' }),
      preview({ path: 'other.md' }),
      preview({ expected_version: 99 })
    ]) {
      const { session } = createSession({ preview: vi.fn(async () => invalid) });
      session.updateFromSource(`${BASE}draft`);
      expect(await session.requestPreview()).toBeNull();
      expect(session.status).toBe('error');
      expect(session.preview).toBeNull();
      expect(session.draftSource).toBe(`${BASE}draft`);
    }
  });
});

describe('NoteSession recovery reconciliation', () => {
  it('reloads clean sessions and invalidates an existing preview', async () => {
    const { session } = createSession();
    await session.requestPreview();

    session.reconcileAfterRecovery('# Recovered\n', 12, 'notes/recovered.md');

    expect(session.baseSource).toBe('# Recovered\n');
    expect(session.draftSource).toBe('# Recovered\n');
    expect(session.revision).toBe(12);
    expect(session.path).toBe('notes/recovered.md');
    expect(session.preview).toBeNull();
    expect(session.status).toBe('reloaded');
  });

  it('preserves dirty drafts and exposes an explicit recovery conflict', async () => {
    const { session } = createSession();
    session.updateFromSource(`${BASE}unsaved draft`);
    await session.requestPreview();

    session.reconcileAfterRecovery('# Changed on disk\n', 12);

    expect(session.baseSource).toBe(BASE);
    expect(session.draftSource).toBe(`${BASE}unsaved draft`);
    expect(session.revision).toBe(7);
    expect(session.preview).toBeNull();
    expect(session.status).toBe('recovery-conflict');
  });
});

describe('selection fallback policy', () => {
  it('prefers a valid mapped selection', () => {
    expect(resolveSelection('abcdef', { anchor: 1, head: 2 }, { anchor: 4, head: 5 }))
      .toEqual({ anchor: 4, head: 5 });
  });

  it('clamps the previous selection when mapping is absent or invalid', () => {
    expect(resolveSelection('abc', { anchor: 8, head: -2 }, null))
      .toEqual({ anchor: 3, head: 0 });
    expect(resolveSelection('abc', { anchor: 1.8, head: Number.NaN }, { anchor: 10, head: 10 }))
      .toEqual({ anchor: 1, head: 0 });
  });

  it('falls back to document start without a usable prior selection', () => {
    expect(resolveSelection('abc')).toEqual({ anchor: 0, head: 0 });
    expect(resolveSelection('', null, { anchor: 1, head: 1 })).toEqual({ anchor: 0, head: 0 });
  });
});
