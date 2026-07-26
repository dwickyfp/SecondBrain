import { cleanup, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EditorView } from '@codemirror/view';
import SourceEditor from './SourceEditor.svelte';

afterEach(cleanup);

function editor() {
  return document.querySelector('.cm-editor') as HTMLElement;
}

function view() {
  return EditorView.findFromDOM(editor())!;
}

describe('SourceEditor', () => {
  it('emits source edits with the configured origin and selection', () => {
    const onChange = vi.fn();
    render(SourceEditor, { source: '# A', onChange, origin: 'source', ariaLabel: 'Note source' });
    const instance = view();
    instance.dispatch({ changes: { from: 3, insert: '!' }, selection: { anchor: 4, head: 4 } });

    expect(onChange).toHaveBeenCalledWith({
      origin: 'source', source: '# A!', selection: { anchor: 4, head: 4 }
    });
    expect(document.querySelector('[aria-label="Note source"]')).toBeTruthy();
  });

  it('syncs an external source without echoing it and preserves/clamps selection', async () => {
    const onChange = vi.fn();
    const rendered = render(SourceEditor, { source: 'abcdef', onChange, selection: { anchor: 5, head: 6 } });
    const instance = view();
    rendered.rerender({ source: 'abc', onChange, selection: null });
    await Promise.resolve();

    expect(instance.state.doc.toString()).toBe('abc');
    expect(instance.state.selection.main.anchor).toBe(3);
    expect(instance.state.selection.main.head).toBe(3);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('does not dispatch a no-op source update and preserves CRLF, BOM, final newline, and raw syntax', async () => {
    const source = '\uFEFF<custom x="1">\r\n  raw  \r\n</custom>\r\n';
    const onChange = vi.fn();
    const rendered = render(SourceEditor, { source, onChange });
    const instance = view();
    const dispatch = vi.spyOn(instance, 'dispatch');
    rendered.rerender({ source, onChange });
    await Promise.resolve();

    expect(dispatch).not.toHaveBeenCalled();
    const exactSource = instance.state.sliceDoc(0, instance.state.doc.length);
    expect(exactSource).toBe(source);
    expect(exactSource.startsWith('\uFEFF')).toBe(true);
    expect(exactSource.endsWith('\r\n')).toBe(true);

    dispatch.mockRestore();
    instance.dispatch({
      changes: { from: instance.state.doc.length - 1, insert: '!' },
      selection: { anchor: instance.state.doc.length, head: instance.state.doc.length }
    });
    expect(onChange).toHaveBeenLastCalledWith({
      origin: 'source',
      source: '\uFEFF<custom x="1">\r\n  raw  \r\n</custom>!\r\n',
      selection: { anchor: source.length - 1, head: source.length - 1 }
    });
  });

  it('supports read-only and exposes status accessibly', () => {
    render(SourceEditor, { source: 'locked', readonly: true, status: 'review', ariaLabel: 'Source editor' });
    const content = document.querySelector('.cm-content')!;
    expect(content.getAttribute('contenteditable')).toBe('false');
    expect(content.getAttribute('aria-readonly')).toBe('true');
    expect(document.querySelector('[role="status"]')?.textContent).toContain('review');
  });

  it('destroys the CodeMirror view on unmount', () => {
    const destroy = vi.spyOn(EditorView.prototype, 'destroy');
    const rendered = render(SourceEditor, { source: 'x' });
    rendered.unmount();
    expect(destroy).toHaveBeenCalledTimes(1);
    destroy.mockRestore();
  });
});
