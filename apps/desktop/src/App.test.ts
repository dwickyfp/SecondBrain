import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';

const invoke = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('desktop workspace shell', () => {
  beforeEach(() => invoke.mockReset());
  afterEach(cleanup);

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
        source: '# Welcome\n\nPlain **Markdown** stays intact.\n'
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
    expect((await screen.findByLabelText('Welcome Markdown source')).textContent).toContain('Plain **Markdown** stays intact.');
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
          source: '# Missing\n'
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
      return Promise.resolve({ ...note, source: `# ${note.path}\n` });
    });
    render(App);

    await fireEvent.input(screen.getByLabelText('Workspace folder'), {
      target: { value: '/vaults/split' }
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));
    await screen.findByLabelText('Alpha Markdown source');

    await fireEvent.click(screen.getByRole('button', { name: /Beta b\.md/ }));
    await screen.findByLabelText('Beta Markdown source');
    await fireEvent.click(screen.getByRole('button', { name: 'Split' }));
    expect(screen.getByLabelText('Secondary pane')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: /Gamma c\.md/ }));
    expect(await screen.findByLabelText('Gamma Markdown source')).toBeTruthy();
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
      return Promise.resolve({ ...note, source: `# ${note.title}\n` });
    });
    render(App);

    await fireEvent.input(screen.getByLabelText('Workspace folder'), { target: { value: '/vaults/context' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));

    expect(await screen.findByRole('heading', { name: 'Outline' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Detail' }).getAttribute('title')).toBe('Line 4');
    await fireEvent.click(within(screen.getByLabelText('Note context')).getByRole('button', { name: /Source source\.md/ }));
    expect(await screen.findByLabelText('Source Markdown source')).toBeTruthy();
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
});
