import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';

const invoke = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('desktop workspace shell', () => {
  beforeEach(() => invoke.mockReset());
  afterEach(cleanup);

  it('opens a workspace and renders the backend note summaries', async () => {
    invoke.mockResolvedValue({
      root: '/vaults/research',
      workspaceId: '01ARZ3NDEKTSV4RRFFQ69G5FAV',
      notes: [{ noteId: '01ARZ3NDEKTSV4RRFFQ69G5FAW', path: 'welcome.md', title: 'Welcome' }],
      indexed: 1,
      brokenLinks: 0,
      orphans: 0
    });
    render(App);
    expect(screen.getByRole('heading', { name: 'Open your second brain.' })).toBeTruthy();

    await fireEvent.input(screen.getByLabelText('Workspace folder'), {
      target: { value: '/vaults/research' }
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Open workspace' }));

    await screen.findByRole('heading', { name: 'research' });
    expect(screen.getByRole('navigation', { name: 'Notes' }).textContent).toContain('Welcome');
    expect(invoke).toHaveBeenCalledWith('open_workspace', { root: '/vaults/research' });
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
