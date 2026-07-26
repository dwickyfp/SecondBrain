import { invoke } from '@tauri-apps/api/core';

export interface NoteSummary {
  noteId: string;
  path: string;
  title: string | null;
}

export interface SearchHit extends NoteSummary {
  snippet: string;
}

export interface WorkspaceSummary {
  root: string;
  workspaceId: string;
  notes: NoteSummary[];
  indexed: number;
  brokenLinks: number;
  orphans: number;
}

export interface NoteDocument extends NoteSummary {
  source: string;
}

export const openWorkspace = (root: string) =>
  invoke<WorkspaceSummary>('open_workspace', { root });

export const searchWorkspace = (root: string, query: string) =>
  invoke<SearchHit[]>('search_workspace', { root, query });

export const readNote = (root: string, path: string) =>
  invoke<NoteDocument>('read_note', { root, path });
