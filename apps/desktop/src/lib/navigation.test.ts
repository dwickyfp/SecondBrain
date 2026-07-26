import { describe, expect, it } from 'vitest';
import {
  activeNote,
  closeSplit,
  closeTab,
  createNavigation,
  goBack,
  goForward,
  openNote,
  splitPane
} from './navigation';
import type { NoteSummary } from './workspace';

const notes: NoteSummary[] = [
  { noteId: 'a', path: 'a.md', title: 'A' },
  { noteId: 'b', path: 'b.md', title: 'B' },
  { noteId: 'c', path: 'c.md', title: 'C' }
];

describe('desktop navigation state', () => {
  it('deduplicates tabs by identity and closes the active tab onto its neighbor', () => {
    let state = createNavigation(notes[0]);
    state = openNote(state, notes[1]);
    state = openNote(state, { ...notes[0], path: 'renamed-a.md' });

    expect(state.panes[0].tabs.map((note) => note.noteId)).toEqual(['a', 'b']);
    expect(state.panes[0].tabs[0].path).toBe('renamed-a.md');

    state = closeTab(state, 'a', 'primary');
    expect(activeNote(state.panes[0])?.noteId).toBe('b');
  });

  it('keeps independent pane histories and merges tabs when the split closes', () => {
    let state = createNavigation(notes[0]);
    state = openNote(state, notes[1]);
    state = splitPane(state);
    state = openNote(state, notes[2]);

    expect(state.panes).toHaveLength(2);
    expect(activeNote(state.panes[0])?.noteId).toBe('b');
    expect(activeNote(state.panes[1])?.noteId).toBe('c');

    state = goBack(state, 'secondary');
    expect(activeNote(state.panes[1])?.noteId).toBe('b');
    expect(activeNote(state.panes[0])?.noteId).toBe('b');

    state = closeSplit(state);
    expect(state.panes).toHaveLength(1);
    expect(state.panes[0].tabs.map((note) => note.noteId)).toEqual(['a', 'b', 'c']);
  });

  it('truncates forward history after navigating back and opening another note', () => {
    let state = createNavigation(notes[0]);
    state = openNote(state, notes[1]);
    state = openNote(state, notes[2]);
    state = goBack(state, 'primary');
    state = openNote(state, notes[0]);
    state = goForward(state, 'primary');

    expect(activeNote(state.panes[0])?.noteId).toBe('a');
    expect(state.panes[0].history.map((note) => note.noteId)).toEqual(['a', 'b', 'a']);
  });
});
