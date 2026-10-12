import type { ReactNode } from 'react';

export type TemplateText = string | { zh: string; en: string; ja?: string };
export type TemplateConfigField = {
  path: readonly string[];
  type: 'boolean' | 'string' | 'number' | 'select' | 'string-list' | 'json' | 'custom';
  label: TemplateText;
  description?: TemplateText;
  defaultValue?: unknown;
  options?: readonly { value: string | number | boolean; label: TemplateText }[];
  min?: number;
  max?: number;
  step?: number;
  placeholder?: string;
  sensitive?: boolean;
  link?: { url: string; label: TemplateText; hint?: TemplateText };
  directory?: boolean;
  rows?: number;
  restart?: boolean;
  render?: (props: { value: unknown; onChange: (value: unknown) => void; disabled: boolean; id: string }) => ReactNode;
  validate?: (value: unknown) => TemplateText | null;
};
export type TemplateConfigGroup = {
  id: string;
  title: TemplateText;
  description?: TemplateText;
  fields: readonly TemplateConfigField[];
  validate?: (config: Record<string, unknown>) => TemplateText | null;
};
export type TemplateConfigChange = {
  path: string[];
  value: unknown;
  remove: boolean;
  expected: unknown;
  expectedExists: boolean;
};
export type TemplateConfigSaveResult = { config: Record<string, unknown>; restartRequired: boolean };
export type TemplateConfigDraft = Omit<TemplateConfigChange, 'path'>;

export const templateFieldKey = (path: readonly string[]) => JSON.stringify(path);
export const sameTemplateValue = (left: unknown, right: unknown): boolean => {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) && Array.isArray(right)) return left.length === right.length && left.every((item, i) => sameTemplateValue(item, right[i]));
  if (left && right && typeof left === 'object' && typeof right === 'object' && !Array.isArray(left) && !Array.isArray(right)) {
    const keys = Object.keys(left);
    return keys.length === Object.keys(right).length && keys.every((key) => Object.prototype.hasOwnProperty.call(right, key) && sameTemplateValue((left as Record<string, unknown>)[key], (right as Record<string, unknown>)[key]));
  }
  return false;
};

export function readTemplatePath(config: unknown, path: readonly string[]): { exists: boolean; value: unknown } {
  let value = config;
  for (const segment of path) {
    if (!value || typeof value !== 'object' || Array.isArray(value) || !Object.prototype.hasOwnProperty.call(value, segment)) return { exists: false, value: null };
    value = (value as Record<string, unknown>)[segment];
  }
  return { exists: true, value };
}

export function applyTemplateChanges(config: Record<string, unknown>, changes: readonly TemplateConfigChange[]): Record<string, unknown> {
  const next = structuredClone(config);
  for (const change of changes) {
    if (!change.path.length || change.path.some((key) => ['__proto__', 'prototype', 'constructor'].includes(key))) throw new Error('Invalid configuration path');
    let cursor = next;
    let skip = false;
    for (const segment of change.path.slice(0, -1)) {
      const child = cursor[segment];
      if (!child || typeof child !== 'object' || Array.isArray(child)) {
        if (change.remove) { skip = true; break; }
        cursor[segment] = {};
      }
      cursor = cursor[segment] as Record<string, unknown>;
    }
    if (skip) continue;
    const key = change.path[change.path.length - 1];
    if (change.remove) delete cursor[key];
    else cursor[key] = structuredClone(change.value);
  }
  return next;
}

export function draftTemplateField(field: TemplateConfigField, config: Record<string, unknown>, previous: TemplateConfigDraft | undefined, value: unknown, remove = false): TemplateConfigDraft | undefined {
  const original = previous ? { exists: previous.expectedExists, value: previous.expected } : readTemplatePath(config, field.path);
  if (remove ? !original.exists : original.exists && sameTemplateValue(original.value, value)) return undefined;
  return { value, remove, expectedExists: original.exists, expected: original.value ?? null };
}

export function templateFieldValidation(field: TemplateConfigField, value: unknown): TemplateText | null {
  if (field.type === 'number' && (typeof value !== 'number' || !Number.isFinite(value) || ((field.step ?? 1) === 1 && !Number.isInteger(value)) || (field.min !== undefined && value < field.min) || (field.max !== undefined && value > field.max))) {
    return { zh: `请输入有效数值${field.min !== undefined ? `，最小 ${field.min}` : ''}${field.max !== undefined ? `，最大 ${field.max}` : ''}`, en: `Enter a valid number${field.min !== undefined ? `, minimum ${field.min}` : ''}${field.max !== undefined ? `, maximum ${field.max}` : ''}`, ja: `有効な数値を入力してください${field.min !== undefined ? `（最小 ${field.min}）` : ''}${field.max !== undefined ? `（最大 ${field.max}）` : ''}` };
  }
  if (field.type === 'select' && !field.options?.some((option) => sameTemplateValue(option.value, value))) return { zh: '请选择有效选项', en: 'Select a valid option', ja: '有効な項目を選択してください' };
  if (field.type === 'json' && typeof value === 'string') {
    try { JSON.parse(value); } catch { return { zh: '请输入有效 JSON', en: 'Enter valid JSON', ja: '有効な JSON を入力してください' }; }
  }
  return field.validate?.(value) ?? null;
}

export function templateFieldSaveValue(field: TemplateConfigField, value: unknown): unknown {
  if (field.type === 'string-list') return Array.isArray(value) ? value.map(String).map((item) => item.trim()).filter(Boolean) : [];
  if (field.type === 'json' && typeof value === 'string') return JSON.parse(value);
  return value;
}
