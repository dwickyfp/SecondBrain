import { invoke } from './transport';
import {
  isTransactionPreviewV1,
  parseTransactionApplyOutcome,
  type TransactionApplyOutcome,
  type TransactionPreviewV1
} from './transactions';

export type PropertyValue = null | boolean | number | string | PropertyValue[] | { [key: string]: PropertyValue };
export type PropertyEdit =
  | { action: 'set'; key: string; value: PropertyValue }
  | { action: 'remove'; key: string };

export interface PropertyPreviewV1 {
  format: 'sb-property-preview-v1';
  edit: PropertyEdit;
  properties: Record<string, PropertyValue>;
  transaction: TransactionPreviewV1;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

export function isPropertyValue(value: unknown): value is PropertyValue {
  return value === null
    || typeof value === 'boolean'
    || (typeof value === 'number' && Number.isFinite(value))
    || typeof value === 'string'
    || (Array.isArray(value) && value.every(isPropertyValue))
    || (isRecord(value) && Object.values(value).every(isPropertyValue));
}

export function parsePropertyPreview(value: unknown): PropertyPreviewV1 {
  if (!isRecord(value) || value.format !== 'sb-property-preview-v1' || !isRecord(value.edit)
    || !isRecord(value.properties) || !Object.values(value.properties).every(isPropertyValue)
    || !isTransactionPreviewV1(value.transaction)) {
    throw new Error('Invalid sb-property-preview-v1 payload');
  }
  const edit = value.edit;
  if (typeof edit.key !== 'string'
    || (edit.action !== 'remove' && (edit.action !== 'set' || !isPropertyValue(edit.value)))) {
    throw new Error('Invalid sb-property-preview-v1 edit');
  }
  return value as unknown as PropertyPreviewV1;
}

export const readProperties = (root: string, path: string) =>
  invoke<Record<string, PropertyValue>>('properties_read', { root, path });

export const previewProperty = async (root: string, path: string, edit: PropertyEdit) =>
  parsePropertyPreview(await invoke('property_preview', { root, path, edit }));

export const applyProperty = async (root: string, preview: PropertyPreviewV1): Promise<TransactionApplyOutcome> =>
  parseTransactionApplyOutcome(await invoke('property_apply', { root, preview }));
