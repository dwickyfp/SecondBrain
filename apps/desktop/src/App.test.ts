import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
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
    invoke.mockImplementation((command: string) => command === 'open_workspace'
      ? Promise.resolve({
          root: '/vaults/research',
          workspaceId: '01ARZ3NDEKTSV4RRFFQ69G5FAV',
          notes: [{ noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW', path: 'missing.md', title: null }],
          indexed: 1,
          brokenLinks: 0,
          orphans: 0
        })
      : Promise.resolve({
          noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW',
          path: 'missing.md',
          title: null,
          source: '# Missing\n'
        }));
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
});
