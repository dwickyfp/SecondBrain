import type { NoteSummary } from './workspace';

export type PaneId = 'primary' | 'secondary';

export interface PaneState {
  id: PaneId;
  tabs: NoteSummary[];
  activeNoteId: string | null;
  history: NoteSummary[];
  historyIndex: number;
}

export interface NavigationState {
  panes: PaneState[];
  activePaneId: PaneId;
}

function createPane(id: PaneId, note?: NoteSummary): PaneState {
  return {
    id,
    tabs: note ? [note] : [],
    activeNoteId: note?.noteId ?? null,
    history: note ? [note] : [],
    historyIndex: note ? 0 : -1
  };
}

export function createNavigation(note?: NoteSummary): NavigationState {
  return { panes: [createPane('primary', note)], activePaneId: 'primary' };
}

export function activeNote(pane: PaneState): NoteSummary | null {
  return pane.tabs.find((note) => note.noteId === pane.activeNoteId) ?? null;
}

function updatePane(
  state: NavigationState,
  paneId: PaneId,
  update: (pane: PaneState) => PaneState
): NavigationState {
  return {
    ...state,
    panes: state.panes.map((pane) => pane.id === paneId ? update(pane) : pane)
  };
}

export function focusPane(state: NavigationState, paneId: PaneId): NavigationState {
  return state.panes.some((pane) => pane.id === paneId)
    ? { ...state, activePaneId: paneId }
    : state;
}

export function openNote(
  state: NavigationState,
  note: NoteSummary,
  paneId: PaneId = state.activePaneId
): NavigationState {
  const focused = focusPane(state, paneId);
  return updatePane(focused, paneId, (pane) => {
    const tabs = pane.tabs.some((tab) => tab.noteId === note.noteId)
      ? pane.tabs.map((tab) => tab.noteId === note.noteId ? note : tab)
      : [...pane.tabs, note];
    const current = pane.history[pane.historyIndex];
    const history = current?.noteId === note.noteId
      ? pane.history
      : [...pane.history.slice(0, pane.historyIndex + 1), note];

    return {
      ...pane,
      tabs,
      activeNoteId: note.noteId,
      history,
      historyIndex: history.length - 1
    };
  });
}

export function closeTab(
  state: NavigationState,
  noteId: string,
  paneId: PaneId
): NavigationState {
  return updatePane(state, paneId, (pane) => {
    const index = pane.tabs.findIndex((note) => note.noteId === noteId);
    if (index < 0) return pane;
    const tabs = pane.tabs.filter((note) => note.noteId !== noteId);
    const fallback = tabs[Math.min(index, tabs.length - 1)] ?? null;
    return {
      ...pane,
      tabs,
      activeNoteId: pane.activeNoteId === noteId ? fallback?.noteId ?? null : pane.activeNoteId
    };
  });
}

export function splitPane(state: NavigationState): NavigationState {
  if (state.panes.length === 2) return state;
  const source = state.panes.find((pane) => pane.id === state.activePaneId);
  const note = source ? activeNote(source) ?? undefined : undefined;
  return {
    panes: [...state.panes, createPane('secondary', note)],
    activePaneId: 'secondary'
  };
}

export function closeSplit(state: NavigationState): NavigationState {
  if (state.panes.length === 1) return state;
  const primary = state.panes.find((pane) => pane.id === 'primary') ?? state.panes[0];
  const secondary = state.panes.find((pane) => pane.id === 'secondary');
  if (!secondary) return { panes: [primary], activePaneId: 'primary' };

  const tabs = [...primary.tabs];
  for (const note of secondary.tabs) {
    if (!tabs.some((tab) => tab.noteId === note.noteId)) tabs.push(note);
  }
  const activeNoteId = state.activePaneId === 'secondary'
    ? secondary.activeNoteId
    : primary.activeNoteId;
  return {
    panes: [{ ...primary, tabs, activeNoteId }],
    activePaneId: 'primary'
  };
}

export function goBack(state: NavigationState, paneId: PaneId): NavigationState {
  return moveHistory(state, paneId, -1);
}

export function goForward(state: NavigationState, paneId: PaneId): NavigationState {
  return moveHistory(state, paneId, 1);
}

function moveHistory(state: NavigationState, paneId: PaneId, delta: -1 | 1): NavigationState {
  const focused = focusPane(state, paneId);
  return updatePane(focused, paneId, (pane) => {
    const historyIndex = pane.historyIndex + delta;
    const note = pane.history[historyIndex];
    if (!note) return pane;
    const tabs = pane.tabs.some((tab) => tab.noteId === note.noteId)
      ? pane.tabs
      : [...pane.tabs, note];
    return { ...pane, tabs, activeNoteId: note.noteId, historyIndex };
  });
}
