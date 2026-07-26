import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { Editor } from '@tiptap/core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import WysiwygEditor from './WysiwygEditor.svelte';
import { NoteSession } from './note-session';

const adapter = { preview: vi.fn(), apply: vi.fn() };
const createSession = (source = '# Note\n\nBody\n') => new NoteSession({
  noteId: 'note', path: 'note.md', source, revision: 1,
  actor: 'person', device: 'desktop', adapter
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('WysiwygEditor', () => {
  it('creates and destroys its TipTap Editor with the component lifecycle', () => {
    const destroy = vi.spyOn(Editor.prototype, 'destroy');
    const view = render(WysiwygEditor, { session: createSession() });
    expect(screen.getByRole('textbox', { name: 'WYSIWYG editor' })).toBeTruthy();
    view.unmount();
    expect(destroy).toHaveBeenCalledOnce();
  });

  it('updates the session and emits an explicit wysiwyg projection on edits', async () => {
    const session = createSession();
    const onSourceChange = vi.fn();
    render(WysiwygEditor, { session, onSourceChange });
    const editor = screen.getByRole('textbox', { name: 'WYSIWYG editor' });
    editor.querySelector('p')!.textContent = 'Changed';
    await fireEvent.input(editor);

    expect(session.draftSource).toContain('Changed');
    expect(onSourceChange).toHaveBeenCalledOnce();
    expect(onSourceChange.mock.calls[0][0]).toMatchObject({ origin: 'wysiwyg', source: session.draftSource });
  });

  it('projects an external session update without echoing it', async () => {
    const session = createSession();
    const onSourceChange = vi.fn();
    const view = render(WysiwygEditor, { session, draftRevision: session.draftRevision, onSourceChange });
    session.updateFromSource('# External\n\nUpdated\n');
    await view.rerender({ session, draftRevision: session.draftRevision, onSourceChange });

    expect(screen.getByRole('textbox').textContent).toContain('External');
    expect(screen.getByRole('textbox').textContent).toContain('Updated');
    expect(onSourceChange).not.toHaveBeenCalled();
  });

  it('does not emit when TipTap reports an unchanged source', async () => {
    const session = createSession('Body\n');
    const onSourceChange = vi.fn();
    const view = render(WysiwygEditor, { session, draftRevision: 0, onSourceChange });
    await view.rerender({ session, draftRevision: 1, onSourceChange });
    expect(session.draftRevision).toBe(0);
    expect(onSourceChange).not.toHaveBeenCalled();
  });

  it('preserves unsupported Markdown byte-for-byte in a visible read-only fallback', () => {
    const raw = '---\ncustom: value\n---\n\n<opaque data-x="1">keep</opaque>\n';
    const session = createSession(raw);
    render(WysiwygEditor, { session });

    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getByRole('note', { name: 'Raw Markdown preserved' }).querySelector('pre')?.textContent).toBe(raw);
    expect(session.draftSource).toBe(raw);
    expect(session.draftRevision).toBe(0);
  });
});
