import { invoke } from './transport';
import type { NoteSummary } from './workspace';

export interface WorkspaceGraphNode extends NoteSummary {
  incoming_occurrences: number;
  outgoing_occurrences: number;
  orphan: boolean;
}

export interface WorkspaceGraphEdge {
  source_note_id: string;
  target_note_id: string;
  occurrences: number;
  self_link: boolean;
}

export interface WorkspaceGraphDiagnostic {
  source_note_id: string;
  source_path: string;
  target: string;
  occurrences: number;
}

export interface WorkspaceGraphAmbiguousLink extends WorkspaceGraphDiagnostic {
  candidates: NoteSummary[];
}

export interface WorkspaceGraphV1 {
  format: 'sb-workspace-graph-v1';
  nodes: WorkspaceGraphNode[];
  edges: WorkspaceGraphEdge[];
  broken_links: WorkspaceGraphDiagnostic[];
  ambiguous_links: WorkspaceGraphAmbiguousLink[];
}

function record(value: unknown, at: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${at} must be an object`);
  return value as Record<string, unknown>;
}

function exact(value: Record<string, unknown>, keys: readonly string[], at: string) {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${at} has unexpected fields`);
  }
}

function text(value: unknown, at: string): string {
  if (typeof value !== 'string') throw new Error(`${at} must be a string`);
  return value;
}

function count(value: unknown, at: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) throw new Error(`${at} must be a non-negative integer`);
  return value as number;
}

function summary(value: unknown, at: string): NoteSummary {
  const item = record(value, at);
  exact(item, ['note_id', 'path', 'title'], at);
  if (item.title !== null && typeof item.title !== 'string') throw new Error(`${at}.title must be a string or null`);
  return { noteId: text(item.note_id, `${at}.note_id`), path: text(item.path, `${at}.path`), title: item.title as string | null };
}

function diagnostic(value: unknown, at: string): WorkspaceGraphDiagnostic {
  const item = record(value, at);
  exact(item, ['source_note_id', 'source_path', 'target', 'occurrences'], at);
  return {
    source_note_id: text(item.source_note_id, `${at}.source_note_id`),
    source_path: text(item.source_path, `${at}.source_path`),
    target: text(item.target, `${at}.target`),
    occurrences: count(item.occurrences, `${at}.occurrences`)
  };
}

export function parseWorkspaceGraph(value: unknown): WorkspaceGraphV1 {
  const graph = record(value, 'graph');
  exact(graph, ['format', 'nodes', 'edges', 'broken_links', 'ambiguous_links'], 'graph');
  if (graph.format !== 'sb-workspace-graph-v1') throw new Error(`unsupported workspace graph format: ${String(graph.format)}`);
  if (!Array.isArray(graph.nodes) || !Array.isArray(graph.edges) || !Array.isArray(graph.broken_links) || !Array.isArray(graph.ambiguous_links)) {
    throw new Error('workspace graph collections must be arrays');
  }
  const nodes = graph.nodes.map((value, index) => {
    const item = record(value, `nodes[${index}]`);
    exact(item, ['note_id', 'path', 'title', 'incoming_occurrences', 'outgoing_occurrences', 'orphan'], `nodes[${index}]`);
    if (typeof item.orphan !== 'boolean') throw new Error(`nodes[${index}].orphan must be a boolean`);
    return {
      noteId: text(item.note_id, `nodes[${index}].note_id`), path: text(item.path, `nodes[${index}].path`),
      title: item.title === null ? null : text(item.title, `nodes[${index}].title`),
      incoming_occurrences: count(item.incoming_occurrences, `nodes[${index}].incoming_occurrences`),
      outgoing_occurrences: count(item.outgoing_occurrences, `nodes[${index}].outgoing_occurrences`), orphan: item.orphan
    };
  });
  const edges = graph.edges.map((value, index) => {
    const item = record(value, `edges[${index}]`);
    exact(item, ['source_note_id', 'target_note_id', 'occurrences', 'self_link'], `edges[${index}]`);
    if (typeof item.self_link !== 'boolean') throw new Error(`edges[${index}].self_link must be a boolean`);
    return { source_note_id: text(item.source_note_id, `edges[${index}].source_note_id`), target_note_id: text(item.target_note_id, `edges[${index}].target_note_id`), occurrences: count(item.occurrences, `edges[${index}].occurrences`), self_link: item.self_link };
  });
  const ambiguous_links = graph.ambiguous_links.map((value, index) => {
    const item = record(value, `ambiguous_links[${index}]`);
    exact(item, ['source_note_id', 'source_path', 'target', 'occurrences', 'candidates'], `ambiguous_links[${index}]`);
    if (!Array.isArray(item.candidates)) throw new Error(`ambiguous_links[${index}].candidates must be an array`);
    return { source_note_id: text(item.source_note_id, `ambiguous_links[${index}].source_note_id`), source_path: text(item.source_path, `ambiguous_links[${index}].source_path`), target: text(item.target, `ambiguous_links[${index}].target`), occurrences: count(item.occurrences, `ambiguous_links[${index}].occurrences`), candidates: item.candidates.map((candidate, candidateIndex) => summary(candidate, `ambiguous_links[${index}].candidates[${candidateIndex}]`)) };
  });
  return { format: graph.format, nodes, edges, broken_links: graph.broken_links.map((value, index) => diagnostic(value, `broken_links[${index}]`)), ambiguous_links };
}

export async function readWorkspaceGraph(root: string): Promise<WorkspaceGraphV1> {
  return parseWorkspaceGraph(await invoke<unknown>('workspace_graph', { root }));
}
