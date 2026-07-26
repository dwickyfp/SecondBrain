import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { EditorView } from '@codemirror/view';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';

const invoke = vi.hoisted(() => vi.fn());
const chooseWorkspaceDirectory = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('./lib/dialog', () => ({ chooseWorkspaceDirectory }));

const note = { noteId: 'note-a', path: 'note.md', title: 'Note' };
const workspace = {
  root: '/vaults/edit', workspaceId: 'workspace', notes: [note], indexed: 1, brokenLinks: 0, orphans: 0
};
const context = { noteId: note.noteId, outline: [], backlinks: [] };

function transactionPreview(reviewRequired = false) {
  return {
    format: 'sb-transaction-preview-v1', workspace_id: 'workspace', note_id: note.noteId,
    path: note.path, actor: 'secondbrain-desktop', device: 'local-desktop',
    expected_hash: '0'.repeat(64), expected_version: 3, proposed_source_hash: '1'.repeat(64),
    review_required: reviewRequired, summary: reviewRequired ? 'NeedsReview x1' : 'ReplaceNode x1', operations: []
  };
}

async function openEditableWorkspace() {
  await fireEvent.input(screen.getByLabelText('Workspace folder'), { target: { value: workspace.root } });
  await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));
  await screen.findByRole('textbox', { name: 'Note WYSIWYG editor' });
}

async function switchToSource(container: HTMLElement = document.body) {
  await fireEvent.click(within(container).getByRole('button', { name: 'Source' }));
  await waitFor(() => expect(container.querySelector('.source-editor')).toBeTruthy());
  return container.querySelector('.source-editor') as HTMLElement;
}

function replaceSource(editor: HTMLElement, source: string) {
  const view = EditorView.findFromDOM(editor.querySelector('.cm-editor') as HTMLElement)!;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
}

describe('desktop workspace shell', () => {
  beforeEach(() => {
    invoke.mockReset();
    chooseWorkspaceDirectory.mockReset();
  });
  afterEach(cleanup);

  it('selects a workspace folder and opens it from the selected path', async () => {
    chooseWorkspaceDirectory.mockResolvedValue('/vaults/chosen');
    invoke.mockResolvedValue({ root: '/vaults/chosen', workspaceId: 'workspace', notes: [], indexed: 0, brokenLinks: 0, orphans: 0 });
    render(App);

    expect((screen.getByRole('button', { name: 'Open workspace' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Review Obsidian import' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/No folder selected/)).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Choose folder' }));
    expect((screen.getByLabelText('Workspace folder') as HTMLInputElement).value).toBe('/vaults/chosen');
    expect((screen.getByRole('button', { name: 'Open workspace' }) as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByRole('button', { name: 'Review Obsidian import' }) as HTMLButtonElement).disabled).toBe(false);

    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));
    await screen.findByText('This workspace contains no Markdown notes yet.');
    expect(invoke).toHaveBeenCalledWith('open_workspace', { root: '/vaults/chosen' });
  });

  it('leaves the path unchanged when folder selection is cancelled', async () => {
    chooseWorkspaceDirectory.mockResolvedValue(null);
    render(App);
    await fireEvent.input(screen.getByLabelText('Workspace folder'), { target: { value: '/vaults/typed' } });

    await fireEvent.click(screen.getByRole('button', { name: 'Choose folder' }));

    expect((screen.getByLabelText('Workspace folder') as HTMLInputElement).value).toBe('/vaults/typed');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('reports a human-readable folder selection error', async () => {
    chooseWorkspaceDirectory.mockRejectedValue({ message: 'permission denied', code: 13 });
    render(App);

    await fireEvent.click(screen.getByRole('button', { name: 'Choose folder' }));

    expect((await screen.findByRole('alert')).textContent).toBe('Could not choose a folder: permission denied');
  });

  it('enables typed-path actions and clears stale import state when the folder changes', async () => {
    invoke.mockResolvedValue({
      format: 'sb-obsidian-import-preview-v1', root: '/vaults/first', fingerprint: 'abc',
      alreadyInitialized: false, workspaceId: null, manifestFormatVersion: null, manifestFingerprint: null,
      inventory: { markdown: [], attachments: [], obsidianConfig: [], ignored: [], unsupported: [], symlinks: [] },
      parseErrors: [], duplicateIds: [], portableCollisions: [], brokenLinks: [], ambiguousLinks: [],
      plannedWrites: { markdown: 0, attachments: 0, obsidianConfig: 0 }, canApply: true
    });
    render(App);
    const input = screen.getByLabelText('Workspace folder');

    await fireEvent.input(input, { target: { value: '/vaults/first' } });
    expect((screen.getByRole('button', { name: 'Open workspace' }) as HTMLButtonElement).disabled).toBe(false);
    await fireEvent.click(screen.getByRole('button', { name: 'Review Obsidian import' }));
    expect(await screen.findByRole('heading', { name: 'Import review' })).toBeTruthy();

    await fireEvent.input(input, { target: { value: '/vaults/second' } });
    expect(screen.queryByRole('heading', { name: 'Import review' })).toBeNull();
    expect(screen.getByText('Ready to open /vaults/second')).toBeTruthy();
  });

  it('previews in-place import, states zero source changes, applies, and opens the result', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'import_preview') return Promise.resolve({
        format: 'sb-obsidian-import-preview-v1', root: '/vaults/obsidian', fingerprint: 'abc',
        alreadyInitialized: false, workspaceId: null, manifestFormatVersion: null, manifestFingerprint: null,
        inventory: { markdown: ['inbox.md'], attachments: ['image.png'], obsidianConfig: ['.obsidian/app.json'], ignored: [], unsupported: [], symlinks: [] },
        parseErrors: [], duplicateIds: [], portableCollisions: [], brokenLinks: [], ambiguousLinks: [],
        plannedWrites: { markdown: 0, attachments: 0, obsidianConfig: 0 }, canApply: true
      });
      if (command === 'import_apply') return Promise.resolve({
        format: 'sb-obsidian-import-preview-v1', status: 'initialized', root: '/vaults/obsidian',
        workspaceId: 'workspace', fingerprint: 'abc', index: { indexed: 1, skipped: 0, brokenLinks: 0, orphans: 1 }
      });
      if (command === 'open_workspace') return Promise.resolve({ ...workspace, root: '/vaults/obsidian' });
      if (command === 'note_context') return Promise.resolve(context);
      return Promise.resolve({ ...note, source: '# Note\n', version: 0 });
    });
    render(App);
    expect(screen.getByText(/zero changes to Markdown, attachments/)).toBeTruthy();
    await fireEvent.input(screen.getByLabelText('Workspace folder'), { target: { value: '/vaults/obsidian' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Review Obsidian import' }));
    expect(await screen.findByRole('heading', { name: 'Import review' })).toBeTruthy();
    expect(screen.getByText(/1 Markdown files/)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Initialize and open vault' }));
    await screen.findByRole('heading', { name: 'obsidian' });
    expect(invoke).toHaveBeenCalledWith('import_apply', expect.objectContaining({ root: '/vaults/obsidian' }));
    expect(invoke).toHaveBeenCalledWith('open_workspace', { root: '/vaults/obsidian' });
  });

  it('offers safe initialization when open targets a folder without a manifest', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.reject('I/O operation failed: read manifest');
      if (command === 'import_preview') return Promise.resolve({
        format: 'sb-obsidian-import-preview-v1', root: '/vaults/plain', fingerprint: 'abc',
        alreadyInitialized: false, workspaceId: null, manifestFormatVersion: null, manifestFingerprint: null,
        inventory: { markdown: ['note.md'], attachments: [], obsidianConfig: [], ignored: [], unsupported: [], symlinks: [] },
        parseErrors: [], duplicateIds: [], portableCollisions: [], brokenLinks: [], ambiguousLinks: [],
        plannedWrites: { markdown: 0, attachments: 0, obsidianConfig: 0 }, canApply: true
      });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    render(App);
    await fireEvent.input(screen.getByLabelText('Workspace folder'), { target: { value: '/vaults/plain' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));

    expect(await screen.findByRole('heading', { name: 'Import review' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Initialize and open vault' })).toBeTruthy();
    expect(screen.getByRole('alert').textContent).toContain('not initialized yet');
    expect(invoke).toHaveBeenCalledWith('import_preview', { root: '/vaults/plain' });
  });

  it('opens a workspace and renders the backend note summaries', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve({
        root: '/vaults/research',
        workspaceId: '01ARZ3NDEKTSV4RRFFQ69G5FAV',
        notes: [{ noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW', path: 'welcome.md', title: 'Welcome' }],
        indexed: 1,
        brokenLinks: 0,
        orphans: 0
      });
      if (command === 'note_context') return Promise.resolve({
        noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW',
        outline: [{ level: 1, text: 'Welcome', line: 1 }],
        backlinks: []
      });
      return Promise.resolve({
        noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW',
        path: 'welcome.md',
        title: 'Welcome',
        source: '# Welcome\n\nPlain **Markdown** stays intact.\n',
        version: 0
      });
    });
    render(App);
    expect(screen.getByRole('heading', { name: 'Open your second brain.' })).toBeTruthy();

    await fireEvent.input(screen.getByLabelText('Workspace folder'), {
      target: { value: '/vaults/research' }
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));

    await screen.findByRole('heading', { name: 'research' });
    expect(screen.getByRole('navigation', { name: 'Notes' }).textContent).toContain('Welcome');
    expect((await screen.findByRole('textbox', { name: 'Welcome WYSIWYG editor' })).textContent).toContain('Plain Markdown stays intact.');
    expect(invoke).toHaveBeenCalledWith('open_workspace', { root: '/vaults/research' });
    expect(invoke).toHaveBeenCalledWith('read_note', {
      root: '/vaults/research',
      path: 'welcome.md'
    });
  });

  it('renders a note read error without discarding the open workspace', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve({
          root: '/vaults/research',
          workspaceId: '01ARZ3NDEKTSV4RRFFQ69G5FAV',
          notes: [{ noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW', path: 'missing.md', title: null }],
          indexed: 1,
          brokenLinks: 0,
          orphans: 0
        });
      if (command === 'note_context') return Promise.resolve({
        noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW', outline: [], backlinks: []
      });
      return Promise.resolve({
          noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW',
          path: 'missing.md',
          title: null,
          source: '# Missing\n',
          version: 0
        });
    });
    render(App);

    await fireEvent.input(screen.getByLabelText('Workspace folder'), {
      target: { value: '/vaults/research' }
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));

    await screen.findByRole('heading', { name: 'missing' });
    invoke.mockReset();
    invoke.mockRejectedValueOnce(new Error('note cannot be read'));
    await fireEvent.click(screen.getByRole('button', { name: /missing missing\.md/ }));

    expect((await screen.findByRole('alert')).textContent).toContain('note cannot be read');
    expect(screen.getByRole('heading', { name: 'missing' })).toBeTruthy();
  });

  it('renders an explicit empty workspace state', async () => {
    invoke.mockResolvedValue({
      root: '/vaults/empty',
      workspaceId: '01ARZ3NDEKTSV4RRFFQ69G5FAV',
      notes: [],
      indexed: 0,
      brokenLinks: 0,
      orphans: 0
    });
    render(App);

    await fireEvent.input(screen.getByLabelText('Workspace folder'), {
      target: { value: '/vaults/empty' }
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));

    expect(await screen.findByText('This workspace contains no Markdown notes yet.')).toBeTruthy();
  });

  it('opens notes in the active split pane and merges tabs when the split closes', async () => {
    const notes = [
      { noteId: 'a', path: 'a.md', title: 'Alpha' },
      { noteId: 'b', path: 'b.md', title: 'Beta' },
      { noteId: 'c', path: 'c.md', title: 'Gamma' }
    ];
    let readIndex = 0;
    invoke.mockImplementation((command: string, input?: { path?: string }) => {
      if (command === 'open_workspace') return Promise.resolve({
        root: '/vaults/split',
        workspaceId: '01ARZ3NDEKTSV4RRFFQ69G5FAV',
        notes,
        indexed: 3,
        brokenLinks: 0,
        orphans: 0
      });
      if (command === 'note_context') return Promise.resolve({
        noteId: notes.find((candidate) => candidate.path === input?.path)?.noteId ?? notes[readIndex]?.noteId ?? 'a',
        outline: [],
        backlinks: []
      });
      const note = notes.find((candidate) => candidate.path === input?.path)
        ?? notes[Math.min(readIndex++, notes.length - 1)];
      return Promise.resolve({ ...note, source: `# ${note.path}\n`, version: 0 });
    });
    render(App);

    await fireEvent.input(screen.getByLabelText('Workspace folder'), {
      target: { value: '/vaults/split' }
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));
    await screen.findByRole('textbox', { name: 'Alpha WYSIWYG editor' });

    await fireEvent.click(screen.getByRole('button', { name: /Beta b\.md/ }));
    await screen.findByRole('textbox', { name: 'Beta WYSIWYG editor' });
    await fireEvent.click(document.querySelector('.split-control') as HTMLButtonElement);
    expect(screen.getByLabelText('Secondary pane')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: /Gamma c\.md/ }));
    expect(await screen.findByRole('textbox', { name: 'Gamma WYSIWYG editor' })).toBeTruthy();
    expect(screen.getByRole('tablist', { name: 'primary pane tabs' }).textContent).not.toContain('Gamma');
    expect(screen.getByRole('tablist', { name: 'secondary pane tabs' }).textContent).toContain('Gamma');

    await fireEvent.click(screen.getAllByRole('button', { name: 'Close split' })[0]);
    expect(screen.queryByLabelText('Secondary pane')).toBeNull();
    expect(screen.getByRole('tablist', { name: 'primary pane tabs' }).textContent).toContain('Gamma');
  });

  it('renders indexed outline and opens a backlink in the active pane', async () => {
    const target = { noteId: 'target', path: 'target.md', title: 'Target' };
    const source = { noteId: 'source', path: 'source.md', title: 'Source' };
    invoke.mockImplementation((command: string, input?: { noteId?: string; path?: string }) => {
      if (command === 'open_workspace') return Promise.resolve({
        root: '/vaults/context', workspaceId: 'workspace', notes: [target, source], indexed: 2, brokenLinks: 0, orphans: 0
      });
      if (command === 'note_context') return Promise.resolve(input?.noteId === 'target'
        ? { noteId: 'target', outline: [{ level: 1, text: 'Target', line: 1 }, { level: 3, text: 'Detail', line: 4 }], backlinks: [source] }
        : { noteId: 'source', outline: [], backlinks: [] });
      const note = input?.path === 'source.md' ? source : target;
      return Promise.resolve({ ...note, source: `# ${note.title}\n`, version: 0 });
    });
    render(App);

    await fireEvent.input(screen.getByLabelText('Workspace folder'), { target: { value: '/vaults/context' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));

    expect(await screen.findByRole('heading', { name: 'Outline' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Detail' }).getAttribute('title')).toBe('Line 4');
    await fireEvent.click(within(screen.getByLabelText('Note context')).getByRole('button', { name: /Source source\.md/ }));
    expect(await screen.findByRole('textbox', { name: 'Source WYSIWYG editor' })).toBeTruthy();
    expect(screen.getByRole('tablist', { name: 'primary pane tabs' }).textContent).toContain('Source');
  });

  it('opens the command palette from the keyboard and restores focus on Escape', async () => {
    invoke.mockResolvedValue({
      root: '/vaults/empty', workspaceId: 'workspace', notes: [], indexed: 0, brokenLinks: 0, orphans: 0
    });
    render(App);
    const rootInput = screen.getByLabelText('Workspace folder');
    rootInput.focus();

    await fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    const paletteInput = await screen.findByLabelText('Find a command');
    expect(document.activeElement).toBe(paletteInput);
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(screen.queryByRole('dialog', { name: 'Command palette' })).toBeNull();
    expect(document.activeElement).toBe(rootInput);
  });

  it('switches modes, edits only the draft, previews, applies, and reloads note and context', async () => {
    let source = '# Note\n\nOriginal\n';
    let version = 3;
    let contextReads = 0;
    invoke.mockImplementation((command: string, input?: Record<string, unknown>) => {
      if (command === 'open_workspace') return Promise.resolve(workspace);
      if (command === 'read_note') return Promise.resolve({ ...note, source, version });
      if (command === 'note_context') {
        contextReads += 1;
        return Promise.resolve({ ...context, outline: source.includes('Changed') ? [{ level: 1, text: 'Changed', line: 1 }] : [] });
      }
      if (command === 'transaction_preview') return Promise.resolve(transactionPreview());
      if (command === 'transaction_apply') {
        source = '# Changed\n\nDraft\n';
        version = 4;
        return Promise.resolve({ transaction_id: 'tx', note_id: note.noteId, path: note.path, changed: true, version, index_refreshed: true });
      }
      return Promise.resolve(undefined);
    });
    render(App);
    await openEditableWorkspace();

    expect(screen.queryByRole('button', { name: 'Preview' })).toBeNull();
    const sourceEditor = await switchToSource();
    replaceSource(sourceEditor, '# Changed\n\nDraft\n');
    expect(await screen.findByRole('button', { name: 'Preview' })).toBeTruthy();
    expect(invoke).not.toHaveBeenCalledWith('transaction_preview', expect.anything());

    await fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('transaction_preview', {
      root: workspace.root, path: note.path, proposedSource: '# Changed\n\nDraft\n'
    }));
    expect(within(screen.getByLabelText('Transaction controls')).getByRole('status').textContent).toContain('ReplaceNode x1');

    await fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('transaction_apply', {
      root: workspace.root, preview: transactionPreview()
    }));
    await screen.findByRole('textbox', { name: 'Note WYSIWYG editor' });
    expect(screen.queryByRole('button', { name: 'Preview' })).toBeNull();
    expect(contextReads).toBe(2);
    expect(screen.getByRole('button', { name: 'Changed' })).toBeTruthy();
    expect(invoke.mock.calls.filter(([command]) => command === 'read_note')).toHaveLength(2);
  });

  it('blocks review-required previews and reports a stale apply without losing the draft', async () => {
    let reviewRequired = true;
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve(workspace);
      if (command === 'read_note') return Promise.resolve({ ...note, source: '# Note\n', version: 3 });
      if (command === 'note_context') return Promise.resolve(context);
      if (command === 'transaction_preview') return Promise.resolve(transactionPreview(reviewRequired));
      if (command === 'transaction_apply') return Promise.reject(new Error('stale note hash'));
      return Promise.resolve(undefined);
    });
    render(App);
    await openEditableWorkspace();
    const sourceEditor = await switchToSource();
    replaceSource(sourceEditor, '# Draft\n');

    await fireEvent.click(await screen.findByRole('button', { name: 'Preview' }));
    expect(await screen.findByText('This transaction requires review and cannot be applied.')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    expect(invoke).not.toHaveBeenCalledWith('transaction_apply', expect.anything());

    reviewRequired = false;
    await fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    await waitFor(() => expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
    expect((await screen.findByRole('alert')).textContent).toContain('stale note hash');
    expect(EditorView.findFromDOM(sourceEditor.querySelector('.cm-editor') as HTMLElement)?.state.doc.toString()).toBe('# Draft\n');
    expect(screen.getByRole('button', { name: 'Preview' })).toBeTruthy();
  });

  it('shares one note session between panes showing the same note', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve(workspace);
      if (command === 'read_note') return Promise.resolve({ ...note, source: '# Note\n', version: 3 });
      if (command === 'note_context') return Promise.resolve(context);
      return Promise.resolve(undefined);
    });
    render(App);
    await openEditableWorkspace();
    await fireEvent.click(document.querySelector('.split-control') as HTMLButtonElement);
    const primary = screen.getByLabelText('Primary pane');
    await switchToSource(primary);

    const editors = Array.from(document.querySelectorAll<HTMLElement>('.source-editor'));
    expect(editors).toHaveLength(2);
    replaceSource(editors[0], '# Shared draft\n');
    await waitFor(() => {
      const second = EditorView.findFromDOM(editors[1].querySelector('.cm-editor') as HTMLElement)!;
      expect(second.state.doc.toString()).toBe('# Shared draft\n');
    });
    expect(screen.getAllByRole('button', { name: 'Preview' })).toHaveLength(2);
  });

  it('previews and applies typed properties, reloads shared state, and locks while the editor is dirty', async () => {
    let reads = 0;
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve(workspace);
      if (command === 'read_note') return Promise.resolve({ ...note, source: '# Note\n', version: reads > 2 ? 4 : 3 });
      if (command === 'note_context') return Promise.resolve(context);
      if (command === 'properties_read') {
        reads += 1;
        return Promise.resolve({ rating: reads > 1 ? 5 : 3 });
      }
      if (command === 'property_preview') return Promise.resolve({
        format: 'sb-property-preview-v1',
        edit: { action: 'set', key: 'rating', value: 5 },
        properties: { rating: 5 },
        transaction: { ...transactionPreview(), summary: 'SetProperty x1' }
      });
      if (command === 'property_apply') return Promise.resolve({
        transaction_id: 'property-tx', note_id: note.noteId, path: note.path,
        changed: true, version: 4, index_refreshed: true
      });
      return Promise.resolve(undefined);
    });
    render(App);
    await openEditableWorkspace();

    const rating = await screen.findByLabelText('rating JSON value');
    await fireEvent.input(rating, { target: { value: '5' } });
    await fireEvent.click(within(screen.getByLabelText('Note context')).getByRole('button', { name: 'Preview' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('property_preview', {
      root: workspace.root, path: note.path, edit: { action: 'set', key: 'rating', value: 5 }
    }));
    await fireEvent.click(screen.getByRole('button', { name: 'Apply property' }));
    await waitFor(() => expect((screen.getByLabelText('rating JSON value') as HTMLInputElement).value).toBe('5'));

    const sourceEditor = await switchToSource();
    replaceSource(sourceEditor, '# Dirty\n');
    const propertyFieldset = screen.getByRole('group') as HTMLFieldSetElement;
    await waitFor(() => expect(propertyFieldset.disabled).toBe(true));
    expect(screen.getByText('Save or discard the editor draft before editing properties.')).toBeTruthy();
  });

  it('supplies the local date, requires confirmation, applies the preview, and opens the daily note', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 6, 26, 23, 30));
    const daily = { noteId: 'daily', path: 'Daily/2026-07-26.md', title: '2026-07-26' };
    const preview = {
      format: 'sb-note-create-preview-v1', transaction_id: 'tx', workspace_id: 'workspace',
      note_id: daily.noteId, path: daily.path, actor: 'secondbrain-desktop', device: 'local-desktop',
      source: '# 2026-07-26\n', source_hash: 'hash', expected_absent: true,
      summary: `Create note at ${daily.path}`
    };
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    let opened = 0;
    invoke.mockImplementation((command: string, input?: Record<string, unknown>) => {
      if (command === 'open_workspace') return Promise.resolve(opened++ ? { ...workspace, notes: [note, daily] } : workspace);
      if (command === 'read_note') {
        const current = input?.path === daily.path ? daily : note;
        return Promise.resolve({ ...current, source: '# Note\n', version: 0 });
      }
      if (command === 'note_context') return Promise.resolve({ noteId: input?.noteId, outline: [], backlinks: [] });
      if (command === 'daily_note') return Promise.resolve({ status: 'create', preview });
      if (command === 'note_create_apply') return Promise.resolve({ transaction_id: 'tx', note_id: daily.noteId, path: daily.path, created: true, version: 0, index_refreshed: true });
      return Promise.resolve({});
    });
    render(App);
    await openEditableWorkspace();
    await fireEvent.click(screen.getByRole('button', { name: 'Daily note' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('daily_note', { root: workspace.root, date: '2026-07-26' }));
    expect(window.confirm).toHaveBeenCalledWith(`Create ${daily.path}?`);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('note_create_apply', { root: workspace.root, preview }));
    expect(await screen.findByRole('heading', { name: '2026-07-26' })).toBeTruthy();
    vi.useRealTimers();
  });

  it('opens the indexed graph from the palette and opens a keyboard-operable node in the active pane', async () => {
    const graphNote = { noteId: 'graph', path: 'graph.md', title: 'Graph target' };
    invoke.mockImplementation((command: string, input?: Record<string, unknown>) => {
      if (command === 'open_workspace') return Promise.resolve({ ...workspace, notes: [note, graphNote], indexed: 2 });
      if (command === 'read_note') {
        const current = input?.path === graphNote.path ? graphNote : note;
        return Promise.resolve({ ...current, source: `# ${current.title}\n`, version: 0 });
      }
      if (command === 'note_context') return Promise.resolve({ noteId: input?.noteId, outline: [], backlinks: [] });
      if (command === 'workspace_graph') return Promise.resolve({
        format: 'sb-workspace-graph-v1',
        nodes: [
          { note_id: note.noteId, path: note.path, title: note.title, incoming_occurrences: 1, outgoing_occurrences: 0, orphan: false },
          { note_id: graphNote.noteId, path: graphNote.path, title: graphNote.title, incoming_occurrences: 0, outgoing_occurrences: 1, orphan: false }
        ],
        edges: [{ source_note_id: graphNote.noteId, target_note_id: note.noteId, occurrences: 1, self_link: false }],
        broken_links: [], ambiguous_links: []
      });
      return Promise.resolve({});
    });
    render(App);
    await openEditableWorkspace();
    await fireEvent.click(document.querySelector('.split-control') as HTMLButtonElement);
    await fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    await fireEvent.input(screen.getByLabelText('Find a command'), { target: { value: 'workspace graph' } });
    await fireEvent.keyDown(window, { key: 'Enter' });

    const table = await screen.findByRole('region', { name: 'Workspace graph table' });
    const targetButton = within(table).getByRole('button', { name: /Graph target graph\.md/ });
    targetButton.focus();
    await fireEvent.click(targetButton);

    expect(await screen.findByRole('textbox', { name: 'Graph target WYSIWYG editor' })).toBeTruthy();
    expect(screen.getByRole('tablist', { name: 'secondary pane tabs' }).textContent).toContain('Graph target');
    expect(invoke).toHaveBeenCalledWith('workspace_graph', { root: workspace.root });
  });

  it('renders graph loading, error, and empty states without fake nodes', async () => {
    let rejectGraph: ((error: Error) => void) | undefined;
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve(workspace);
      if (command === 'read_note') return Promise.resolve({ ...note, source: '# Note\n', version: 0 });
      if (command === 'note_context') return Promise.resolve(context);
      if (command === 'workspace_graph') return new Promise((_resolve, reject) => { rejectGraph = reject; });
      return Promise.resolve({});
    });
    render(App);
    await openEditableWorkspace();
    await fireEvent.click(screen.getByRole('button', { name: 'Graph' }));
    expect((await screen.findByRole('status')).textContent).toContain('Loading workspace graph');
    rejectGraph?.(new Error('index unavailable'));
    expect((await screen.findByRole('alert')).textContent).toContain('index unavailable');

    invoke.mockResolvedValueOnce({ format: 'sb-workspace-graph-v1', nodes: [], edges: [], broken_links: [], ambiguous_links: [] });
    await fireEvent.click(screen.getByRole('button', { name: 'Graph' }));
    expect(await screen.findByText('The index contains no notes to graph.')).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'Workspace graph table' })).toBeNull();
  });

  it('reloads clean sessions after recovery', async () => {
    let reads = 0;
    const diagnostics = { formatVersion: 1, workspace: workspace.root, workspaceId: workspace.workspaceId, status: 'healthy', index: { status: 'ready', path: 'index', notes: 1, links: 0, brokenLinks: 0, orphans: 1 }, transactions: { total: 0, committed: 0, aborted: 0, pending: 0, indexRepairsOutstanding: 0, reviewsPending: 0 }, issues: [] };
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve(workspace);
      if (command === 'read_note') return Promise.resolve({ ...note, source: reads++ ? '# Recovered\n' : '# Note\n', version: reads > 1 ? 4 : 3 });
      if (command === 'note_context') return Promise.resolve(context);
      if (command === 'inspect_workspace') return Promise.resolve(diagnostics);
      if (command === 'recover_workspace') return Promise.resolve({ formatVersion: 1, workspace: workspace.root, status: 'recovered', actions: [], repaired: 0, quarantined: 0, abandoned: 0, reviewRequired: 0, indexRefreshed: true, diagnostics });
      return Promise.resolve({});
    });
    render(App);
    await openEditableWorkspace();
    await fireEvent.click(await screen.findByRole('button', { name: 'healthy' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Run safe recovery' }));
    expect(await screen.findByText('Note reloaded after recovery.')).toBeTruthy();
    expect(invoke.mock.calls.filter(([command]) => command === 'read_note')).toHaveLength(2);
  });

  it('preserves dirty drafts with an explicit conflict after recovery', async () => {
    const diagnostics = { formatVersion: 1, workspace: workspace.root, workspaceId: workspace.workspaceId, status: 'healthy', index: { status: 'ready', path: 'index', notes: 1, links: 0, brokenLinks: 0, orphans: 1 }, transactions: { total: 0, committed: 0, aborted: 0, pending: 0, indexRepairsOutstanding: 0, reviewsPending: 0 }, issues: [] };
    invoke.mockImplementation((command: string) => {
      if (command === 'open_workspace') return Promise.resolve(workspace);
      if (command === 'read_note') return Promise.resolve({ ...note, source: '# On disk\n', version: 4 });
      if (command === 'note_context') return Promise.resolve(context);
      if (command === 'inspect_workspace') return Promise.resolve(diagnostics);
      if (command === 'recover_workspace') return Promise.resolve({ formatVersion: 1, workspace: workspace.root, status: 'recovered', actions: [], repaired: 0, quarantined: 0, abandoned: 0, reviewRequired: 0, indexRefreshed: true, diagnostics });
      return Promise.resolve({});
    });
    render(App);
    await openEditableWorkspace();
    const editor = await switchToSource();
    replaceSource(editor, '# Unsaved draft\n');
    await waitFor(() => expect(screen.getByRole('status', { name: 'Editor status' }).textContent).toBe('dirty'));
    await fireEvent.click(await screen.findByRole('button', { name: 'healthy' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Run safe recovery' }));
    expect((await screen.findByRole('alert')).textContent).toContain('unsaved draft was preserved');
    expect(EditorView.findFromDOM(document.querySelector('.cm-editor') as HTMLElement)?.state.doc.toString()).toBe('# Unsaved draft\n');
  });
});
