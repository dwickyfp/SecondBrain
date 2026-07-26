import { invoke } from './transport';

export const IMPORT_PREVIEW_FORMAT = 'sb-obsidian-import-preview-v1';

export interface ImportIssue { path: string; message: string }
export interface ImportInventory {
  markdown: string[];
  attachments: string[];
  obsidianConfig: string[];
  ignored: string[];
  unsupported: string[];
  symlinks: string[];
}
export interface ImportPreviewV1 {
  format: typeof IMPORT_PREVIEW_FORMAT;
  root: string;
  fingerprint: string;
  alreadyInitialized: boolean;
  workspaceId: string | null;
  manifestFormatVersion: number | null;
  manifestFingerprint: string | null;
  inventory: ImportInventory;
  parseErrors: ImportIssue[];
  duplicateIds: ImportIssue[];
  portableCollisions: ImportIssue[];
  brokenLinks: ImportIssue[];
  ambiguousLinks: ImportIssue[];
  plannedWrites: { markdown: 0; attachments: 0; obsidianConfig: 0 };
  canApply: boolean;
}
export interface ImportApplyOutcomeV1 {
  format: typeof IMPORT_PREVIEW_FORMAT;
  status: 'initialized' | 'already_initialized';
  root: string;
  workspaceId: string;
  fingerprint: string;
  index: { indexed: number; skipped: number; brokenLinks: number; orphans: number };
}

const record = (value: unknown, name: string): Record<string, unknown> => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${name} must be an object`);
  return value as Record<string, unknown>;
};
const exact = (value: Record<string, unknown>, keys: string[], name: string) => {
  const unknown = Object.keys(value).filter((key) => !keys.includes(key));
  if (unknown.length) throw new Error(`${name} has unknown fields: ${unknown.join(', ')}`);
};
const string = (value: unknown, name: string): string => {
  if (typeof value !== 'string') throw new Error(`${name} must be a string`);
  return value;
};
const boolean = (value: unknown, name: string): boolean => {
  if (typeof value !== 'boolean') throw new Error(`${name} must be a boolean`);
  return value;
};
const count = (value: unknown, name: string): number => {
  if (!Number.isSafeInteger(value) || (value as number) < 0) throw new Error(`${name} must be a non-negative integer`);
  return value as number;
};
const strings = (value: unknown, name: string): string[] => {
  if (!Array.isArray(value)) throw new Error(`${name} must be an array`);
  return value.map((item, index) => string(item, `${name}[${index}]`));
};
const issues = (value: unknown, name: string): ImportIssue[] => {
  if (!Array.isArray(value)) throw new Error(`${name} must be an array`);
  return value.map((item, index) => {
    const issue = record(item, `${name}[${index}]`);
    exact(issue, ['path', 'message'], `${name}[${index}]`);
    return { path: string(issue.path, `${name}[${index}].path`), message: string(issue.message, `${name}[${index}].message`) };
  });
};

export function validateImportPreview(value: unknown): ImportPreviewV1 {
  const input = record(value, 'import preview');
  exact(input, ['format', 'root', 'fingerprint', 'alreadyInitialized', 'workspaceId', 'manifestFormatVersion', 'manifestFingerprint', 'inventory', 'parseErrors', 'duplicateIds', 'portableCollisions', 'brokenLinks', 'ambiguousLinks', 'plannedWrites', 'canApply'], 'import preview');
  if (input.format !== IMPORT_PREVIEW_FORMAT) throw new Error('unsupported import preview format');
  const inventory = record(input.inventory, 'inventory');
  exact(inventory, ['markdown', 'attachments', 'obsidianConfig', 'ignored', 'unsupported', 'symlinks'], 'inventory');
  const writes = record(input.plannedWrites, 'plannedWrites');
  exact(writes, ['markdown', 'attachments', 'obsidianConfig'], 'plannedWrites');
  const plannedWrites = {
    markdown: count(writes.markdown, 'plannedWrites.markdown'),
    attachments: count(writes.attachments, 'plannedWrites.attachments'),
    obsidianConfig: count(writes.obsidianConfig, 'plannedWrites.obsidianConfig')
  };
  if (plannedWrites.markdown !== 0 || plannedWrites.attachments !== 0 || plannedWrites.obsidianConfig !== 0) {
    throw new Error('in-place import must plan zero source, attachment, and Obsidian config writes');
  }
  return {
    format: IMPORT_PREVIEW_FORMAT,
    root: string(input.root, 'root'), fingerprint: string(input.fingerprint, 'fingerprint'),
    alreadyInitialized: boolean(input.alreadyInitialized, 'alreadyInitialized'),
    workspaceId: input.workspaceId === null ? null : string(input.workspaceId, 'workspaceId'),
    manifestFormatVersion: input.manifestFormatVersion === null ? null : count(input.manifestFormatVersion, 'manifestFormatVersion'),
    manifestFingerprint: input.manifestFingerprint === null ? null : string(input.manifestFingerprint, 'manifestFingerprint'),
    inventory: {
      markdown: strings(inventory.markdown, 'inventory.markdown'), attachments: strings(inventory.attachments, 'inventory.attachments'),
      obsidianConfig: strings(inventory.obsidianConfig, 'inventory.obsidianConfig'), ignored: strings(inventory.ignored, 'inventory.ignored'),
      unsupported: strings(inventory.unsupported, 'inventory.unsupported'), symlinks: strings(inventory.symlinks, 'inventory.symlinks')
    },
    parseErrors: issues(input.parseErrors, 'parseErrors'), duplicateIds: issues(input.duplicateIds, 'duplicateIds'),
    portableCollisions: issues(input.portableCollisions, 'portableCollisions'), brokenLinks: issues(input.brokenLinks, 'brokenLinks'),
    ambiguousLinks: issues(input.ambiguousLinks, 'ambiguousLinks'), plannedWrites: plannedWrites as ImportPreviewV1['plannedWrites'],
    canApply: boolean(input.canApply, 'canApply')
  };
}

export function validateImportOutcome(value: unknown): ImportApplyOutcomeV1 {
  const input = record(value, 'import outcome');
  exact(input, ['format', 'status', 'root', 'workspaceId', 'fingerprint', 'index'], 'import outcome');
  if (input.format !== IMPORT_PREVIEW_FORMAT) throw new Error('unsupported import outcome format');
  if (input.status !== 'initialized' && input.status !== 'already_initialized') throw new Error('invalid import status');
  const index = record(input.index, 'index');
  exact(index, ['indexed', 'skipped', 'brokenLinks', 'orphans'], 'index');
  return {
    format: IMPORT_PREVIEW_FORMAT, status: input.status, root: string(input.root, 'root'),
    workspaceId: string(input.workspaceId, 'workspaceId'), fingerprint: string(input.fingerprint, 'fingerprint'),
    index: { indexed: count(index.indexed, 'index.indexed'), skipped: count(index.skipped, 'index.skipped'), brokenLinks: count(index.brokenLinks, 'index.brokenLinks'), orphans: count(index.orphans, 'index.orphans') }
  };
}

export const previewVaultImport = async (root: string) => validateImportPreview(await invoke('import_preview', { root }));
export const applyVaultImport = async (root: string, preview: ImportPreviewV1) => validateImportOutcome(await invoke('import_apply', { root, preview }));
