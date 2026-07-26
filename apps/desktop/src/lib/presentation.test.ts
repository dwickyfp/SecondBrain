import { describe, expect, it } from 'vitest';
import { noteLabel, workspaceName } from './presentation';

describe('workspace presentation', () => {
  it('prefers a note title and falls back to its filename', () => {
    expect(noteLabel({ noteId: '1', path: 'notes/one.md', title: 'One' })).toBe('One');
    expect(noteLabel({ noteId: '2', path: 'notes/two.md', title: null })).toBe('two');
  });

  it('names Unix and Windows workspace roots', () => {
    expect(workspaceName('/vaults/research/')).toBe('research');
    expect(workspaceName('C:\\vaults\\research')).toBe('research');
  });
});
