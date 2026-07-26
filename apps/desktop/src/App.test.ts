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
    expect((await screen.findByLabelText('Markdown source')).textContent).toContain('Plain **Markdown** stays intact.');
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
});
