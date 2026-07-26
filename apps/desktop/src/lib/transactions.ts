export const TRANSACTION_PREVIEW_FORMAT_V1 = 'sb-transaction-preview-v1' as const;
import { invoke } from './transport';

export type SemanticKind =
  | 'Root'
  | 'Heading'
  | 'Paragraph'
  | 'Code'
  | 'Math'
  | 'Blockquote'
  | 'List'
  | 'ListItem'
  | 'ThematicBreak'
  | 'Table'
  | 'TableRow'
  | 'TableCell'
  | 'Frontmatter'
  | 'Html'
  | 'InlineCode'
  | 'InlineMath'
  | 'Strong'
  | 'Emphasis'
  | 'Delete'
  | 'Link'
  | 'Image'
  | 'FootnoteDefinition'
  | 'FootnoteReference'
  | 'Definition'
  | 'Text'
  | 'Break'
  | 'Raw';

export interface SourceSpan {
  start: number;
  end: number;
}

export interface NodeAnchor {
  path: { indices: number[] };
  kind: SemanticKind;
  content_hash: string;
}

export type SemanticOperation =
  | { InsertNode: { anchor: NodeAnchor | null; content: string } }
  | { DeleteNode: { anchor: NodeAnchor } }
  | { ReplaceNode: { anchor: NodeAnchor; content: string } }
  | { MoveNode: { anchor: NodeAnchor; target_index: number; content: string } }
  | { SetProperty: { key: string; value: string } }
  | { RemoveProperty: { key: string } }
  | { NeedsReview: { reason: string; span: SourceSpan | null } };

export interface TransactionPreviewV1 {
  format: typeof TRANSACTION_PREVIEW_FORMAT_V1;
  workspace_id: string;
  note_id: string;
  path: string;
  actor: string;
  device: string;
  expected_hash: string;
  expected_version: number;
  proposed_source_hash: string;
  review_required: boolean;
  summary: string;
  operations: SemanticOperation[];
}

export interface TransactionApplyOutcome {
  transaction_id: string;
  note_id: string;
  path: string;
  changed: boolean;
  version: number;
  index_refreshed: boolean;
}

export interface TransactionPreviewRequest {
  path: string;
  proposed_source: string;
  actor: string;
  device: string;
}

export interface TransactionAdapter {
  preview(request: TransactionPreviewRequest): Promise<TransactionPreviewV1>;
  apply(preview: TransactionPreviewV1): Promise<TransactionApplyOutcome>;
}

export interface NoteCreatePreviewV1 {
  format: 'sb-note-create-preview-v1'; transaction_id: string; workspace_id: string;
  note_id: string; path: string; actor: string; device: string; source: string;
  source_hash: string; expected_absent: boolean; summary: string;
}
export interface NoteCreateOutcomeV1 {
  transaction_id: string; note_id: string; path: string; created: boolean;
  version: number; index_refreshed: boolean;
}
export type DailyNoteV1 =
  | { status: 'existing'; note_id: string; path: string }
  | { status: 'create'; preview: NoteCreatePreviewV1 };

export function parseNoteCreatePreview(value: unknown): NoteCreatePreviewV1 {
  if (!isRecord(value) || value.format !== 'sb-note-create-preview-v1'
    || typeof value.note_id !== 'string' || typeof value.path !== 'string'
    || typeof value.source !== 'string' || typeof value.source_hash !== 'string'
    || value.expected_absent !== true) throw new Error('Invalid sb-note-create-preview-v1 payload');
  return value as unknown as NoteCreatePreviewV1;
}
export function parseDailyNote(value: unknown): DailyNoteV1 {
  if (!isRecord(value) || (value.status !== 'existing' && value.status !== 'create')) throw new Error('Invalid daily note payload');
  if (value.status === 'existing' && typeof value.note_id === 'string' && typeof value.path === 'string') return value as DailyNoteV1;
  if (value.status === 'create') { parseNoteCreatePreview(value.preview); return value as DailyNoteV1; }
  throw new Error('Invalid daily note payload');
}
export const previewDailyNote = (root: string, date: string) => invoke<unknown>('daily_note', { root, date }).then(parseDailyNote);
export const applyNoteCreation = (root: string, preview: NoteCreatePreviewV1) => invoke<NoteCreateOutcomeV1>('note_create_apply', { root, preview });

export function createTauriTransactionAdapter(root: string): TransactionAdapter {
  return {
    preview: async (request) => parseTransactionPreviewV1(await invoke('transaction_preview', {
      root,
      path: request.path,
      proposedSource: request.proposed_source
    })),
    apply: async (preview) => parseTransactionApplyOutcome(await invoke('transaction_apply', {
      root,
      preview
    }))
  };
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const hasString = (value: Record<string, unknown>, key: string) => typeof value[key] === 'string';
const hasNumber = (value: Record<string, unknown>, key: string) =>
  typeof value[key] === 'number' && Number.isSafeInteger(value[key]) && (value[key] as number) >= 0;

export function isTransactionPreviewV1(value: unknown): value is TransactionPreviewV1 {
  if (!isRecord(value)) return false;
  return value.format === TRANSACTION_PREVIEW_FORMAT_V1
    && hasString(value, 'workspace_id')
    && hasString(value, 'note_id')
    && hasString(value, 'path')
    && hasString(value, 'actor')
    && hasString(value, 'device')
    && hasString(value, 'expected_hash')
    && hasNumber(value, 'expected_version')
    && hasString(value, 'proposed_source_hash')
    && typeof value.review_required === 'boolean'
    && hasString(value, 'summary')
    && Array.isArray(value.operations);
}

export function isTransactionApplyOutcome(value: unknown): value is TransactionApplyOutcome {
  if (!isRecord(value)) return false;
  return hasString(value, 'transaction_id')
    && hasString(value, 'note_id')
    && hasString(value, 'path')
    && typeof value.changed === 'boolean'
    && hasNumber(value, 'version')
    && typeof value.index_refreshed === 'boolean';
}

export function parseTransactionPreviewV1(value: unknown): TransactionPreviewV1 {
  if (!isTransactionPreviewV1(value)) throw new Error('Invalid sb-transaction-preview-v1 payload');
  return value;
}

export function parseTransactionApplyOutcome(value: unknown): TransactionApplyOutcome {
  if (!isTransactionApplyOutcome(value)) throw new Error('Invalid transaction apply outcome');
  return value;
}
