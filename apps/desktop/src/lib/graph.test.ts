import { describe, expect, it } from 'vitest';
import { parseWorkspaceGraph } from './graph';

const valid = {
  format: 'sb-workspace-graph-v1',
  nodes: [{ note_id: 'a', path: 'a.md', title: 'A', incoming_occurrences: 1, outgoing_occurrences: 0, orphan: false }],
  edges: [], broken_links: [], ambiguous_links: []
};

describe('workspace graph boundary', () => {
  it('validates and maps the Rust contract', () => {
    expect(parseWorkspaceGraph(valid).nodes[0]).toEqual({ noteId: 'a', path: 'a.md', title: 'A', incoming_occurrences: 1, outgoing_occurrences: 0, orphan: false });
  });

  it('rejects unknown versions, unsafe counts, and extra fields', () => {
    expect(() => parseWorkspaceGraph({ ...valid, format: 'sb-workspace-graph-v2' })).toThrow('unsupported');
    expect(() => parseWorkspaceGraph({ ...valid, nodes: [{ ...valid.nodes[0], incoming_occurrences: -1 }] })).toThrow('non-negative integer');
    expect(() => parseWorkspaceGraph({ ...valid, surprise: true })).toThrow('unexpected fields');
  });
});
