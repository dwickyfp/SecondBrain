import type { NoteSummary } from './workspace';

export function noteLabel(note: NoteSummary): string {
  return note.title?.trim() || note.path.split('/').at(-1)?.replace(/\.md$/i, '') || note.path;
}

export function workspaceName(root: string): string {
  const normalized = root.replace(/[\\/]+$/, '');
  return normalized.split(/[\\/]/).at(-1) || root;
}
