import { cleanup, render, screen } from '@testing-library/svelte';
import { EditorView } from '@codemirror/view';
import { afterEach, describe, expect, it, vi } from 'vitest';
import SourceEditor from './SourceEditor.svelte';
import WysiwygEditor from './WysiwygEditor.svelte';
import { NoteSession } from './note-session';

const corpusModules = import.meta.glob('../../../../fixtures/markdown/**/*.md', {
  eager: true,
  import: 'default',
  query: '?raw'
}) as Record<string, string>;

function corpus() {
  return Object.entries(corpusModules)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([path, source]) => ({
      name: path.split('/fixtures/markdown/')[1],
      bytes: new TextEncoder().encode(source),
      source
    }));
}

function sessionFor(source: string, name: string) {
  return new NoteSession({
    noteId: `corpus:${name}`,
    path: name,
    source,
    revision: 0,
    actor: 'corpus-gate',
    device: 'vitest',
    adapter: { preview: vi.fn(), apply: vi.fn() }
  });
}

afterEach(cleanup);

describe('editor Markdown survival corpus gate', () => {
  it('preserves every fixture through source, WYSIWYG, split, and session no-op projections', async () => {
    const fixtures = corpus();
    expect(fixtures.length).toBeGreaterThan(0);

    for (const fixture of fixtures) {
      const session = sessionFor(fixture.source, fixture.name);
      const onSourceChange = vi.fn();
      const sourceEditor = render(SourceEditor, {
        source: session.draftSource,
        selection: session.selection,
        onChange: onSourceChange
      });
      const sourceView = EditorView.findFromDOM(document.querySelector('.cm-editor')!);
      expect(sourceView?.state.sliceDoc(0, sourceView.state.doc.length), fixture.name)
        .toBe(fixture.source);
      await sourceEditor.rerender({
        source: session.draftSource,
        selection: session.selection,
        onChange: onSourceChange
      });
      expect(onSourceChange, fixture.name).not.toHaveBeenCalled();
      sourceEditor.unmount();

      const wysiwyg = render(WysiwygEditor, {
        session,
        draftRevision: session.draftRevision,
        onSourceChange
      });
      if (screen.queryByRole('note', { name: 'Raw Markdown preserved' })) {
        expect(screen.getByRole('note').querySelector('pre')?.textContent, fixture.name)
          .toBe(fixture.source);
      }
      await wysiwyg.rerender({
        session,
        draftRevision: session.draftRevision,
        onSourceChange
      });
      wysiwyg.unmount();

      for (const mode of ['source', 'wysiwyg', 'split', 'source'] as const) {
        session.setMode(mode);
        expect(session.updateFromSource(fixture.source), fixture.name).toBeNull();
        expect(session.updateFromWysiwyg(fixture.source), fixture.name).toBeNull();
      }
      expect(onSourceChange, fixture.name).not.toHaveBeenCalled();
      expect(new TextEncoder().encode(session.draftSource), fixture.name).toEqual(fixture.bytes);
      expect(session.draftRevision, fixture.name).toBe(0);
      expect(session.dirty, fixture.name).toBe(false);
    }
  }, 30_000);
});
