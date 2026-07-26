import type {
  TransactionAdapter,
  TransactionApplyOutcome,
  TransactionPreviewV1
} from './transactions';

export type EditorMode = 'wysiwyg' | 'source' | 'split';
export type ProjectionOrigin = 'wysiwyg' | 'source';
export type NoteSessionStatus = 'clean' | 'dirty' | 'previewing' | 'review' | 'applying' | 'error' | 'reloaded' | 'recovery-conflict';

export interface TextSelection {
  anchor: number;
  head: number;
}

export interface ProjectionUpdate {
  origin: ProjectionOrigin;
  source: string;
  selection: TextSelection;
  draftRevision: number;
}

export interface NoteSessionOptions {
  noteId: string;
  path: string;
  source: string;
  revision: number;
  adapter: TransactionAdapter;
  actor: string;
  device: string;
  mode?: EditorMode;
}

export function resolveSelection(
  source: string,
  previous?: TextSelection | null,
  mapped?: TextSelection | null
): TextSelection {
  const valid = (selection: TextSelection | null | undefined) => selection
    && Number.isSafeInteger(selection.anchor)
    && Number.isSafeInteger(selection.head)
    && selection.anchor >= 0
    && selection.head >= 0
    && selection.anchor <= source.length
    && selection.head <= source.length;
  if (valid(mapped)) return { ...mapped! };
  if (!previous) return { anchor: 0, head: 0 };
  const clamp = (offset: number) => Number.isFinite(offset)
    ? Math.max(0, Math.min(source.length, Math.trunc(offset)))
    : 0;
  return { anchor: clamp(previous.anchor), head: clamp(previous.head) };
}

export class NoteSession {
  readonly noteId: string;
  path: string;
  baseSource: string;
  draftSource: string;
  mode: EditorMode;
  status: NoteSessionStatus = 'clean';
  revision: number;
  preview: TransactionPreviewV1 | null = null;
  error: unknown = null;
  selection: TextSelection = { anchor: 0, head: 0 };

  readonly #adapter: TransactionAdapter;
  readonly #actor: string;
  readonly #device: string;
  #draftRevision = 0;
  #request = 0;

  constructor(options: NoteSessionOptions) {
    if (!options.noteId) throw new Error('NoteSession requires a note identity');
    this.noteId = options.noteId;
    this.path = options.path;
    this.baseSource = options.source;
    this.draftSource = options.source;
    this.revision = options.revision;
    this.mode = options.mode ?? 'wysiwyg';
    this.#adapter = options.adapter;
    this.#actor = options.actor;
    this.#device = options.device;
  }

  get dirty(): boolean {
    return this.draftSource !== this.baseSource;
  }

  get draftRevision(): number {
    return this.#draftRevision;
  }

  setMode(mode: EditorMode): boolean {
    if (mode === this.mode) return false;
    this.mode = mode;
    return true;
  }

  reconcileAfterRecovery(source: string, revision: number, path = this.path): void {
    this.#request += 1;
    this.preview = null;
    this.error = null;
    if (this.dirty) {
      this.status = 'recovery-conflict';
      return;
    }
    this.path = path;
    this.baseSource = source;
    this.draftSource = source;
    this.revision = revision;
    this.selection = resolveSelection(source, this.selection);
    this.#draftRevision += 1;
    this.status = 'reloaded';
  }

  markRecoveryConflict(message: string): void {
    this.#request += 1;
    this.preview = null;
    this.error = new Error(message);
    this.status = 'recovery-conflict';
  }

  updateFromWysiwyg(source: string, selection?: TextSelection | null): ProjectionUpdate | null {
    return this.#updateFrom('wysiwyg', source, selection);
  }

  updateFromSource(source: string, selection?: TextSelection | null): ProjectionUpdate | null {
    return this.#updateFrom('source', source, selection);
  }

  #updateFrom(
    origin: ProjectionOrigin,
    source: string,
    selection?: TextSelection | null
  ): ProjectionUpdate | null {
    if (source === this.draftSource) return null;
    this.draftSource = source;
    this.selection = resolveSelection(source, this.selection, selection);
    this.#draftRevision += 1;
    this.#request += 1;
    this.preview = null;
    this.error = null;
    this.status = this.dirty ? 'dirty' : 'clean';
    return {
      origin,
      source,
      selection: { ...this.selection },
      draftRevision: this.#draftRevision
    };
  }

  shouldProject(update: ProjectionUpdate, target: ProjectionOrigin): boolean {
    return update.origin !== target;
  }

  async requestPreview(): Promise<TransactionPreviewV1 | null> {
    const request = ++this.#request;
    const draftRevision = this.#draftRevision;
    const proposedSource = this.draftSource;
    this.status = 'previewing';
    this.error = null;
    try {
      const preview = await this.#adapter.preview({
        path: this.path,
        proposed_source: proposedSource,
        actor: this.#actor,
        device: this.#device
      });
      if (request !== this.#request || draftRevision !== this.#draftRevision) return null;
      if (preview.note_id !== this.noteId) throw new Error('Preview note identity does not match session');
      if (preview.path !== this.path) throw new Error('Preview path does not match session');
      if (preview.expected_version !== this.revision) throw new Error('Preview version does not match session');
      this.preview = preview;
      this.status = preview.review_required ? 'review' : this.dirty ? 'dirty' : 'clean';
      return preview;
    } catch (error) {
      if (request !== this.#request || draftRevision !== this.#draftRevision) return null;
      this.error = error;
      this.status = 'error';
      return null;
    }
  }

  async applyPreview(): Promise<TransactionApplyOutcome | null> {
    if (!this.preview) throw new Error('A current transaction preview is required');
    if (this.preview.review_required) throw new Error('A review-required preview cannot be applied');
    const preview = this.preview;
    const request = ++this.#request;
    const draftRevision = this.#draftRevision;
    const appliedSource = this.draftSource;
    this.status = 'applying';
    this.error = null;
    try {
      const outcome = await this.#adapter.apply(preview);
      if (request !== this.#request || draftRevision !== this.#draftRevision) return null;
      if (outcome.note_id !== this.noteId) throw new Error('Apply outcome note identity does not match session');
      this.baseSource = appliedSource;
      this.revision = outcome.version;
      this.path = outcome.path;
      this.preview = null;
      this.status = this.dirty ? 'dirty' : 'clean';
      return outcome;
    } catch (error) {
      if (request !== this.#request || draftRevision !== this.#draftRevision) return null;
      this.error = error;
      this.status = 'error';
      return null;
    }
  }
}
