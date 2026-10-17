import { useConfirmation } from '../components/ConfirmationDialog';
import { AdvancedSettingsGroup } from '../components/api-access/AdvancedSettingsGroup';
import { normalizeProviderModels } from '../services/providerModels';
import { ModelSelectionPanel } from '../components/ModelSelectionPanel';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import {
  type CSSProperties,
  FormEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  sortableKeyboardCoordinates,
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS as DndCss } from '@dnd-kit/utilities';
import {
  Edit3,
  Filter,
  GripVertical,
  LoaderCircle,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import claudeIcon from '../assets/icons/claude.svg';
import codexIcon from '../assets/icons/codex.svg';
import deepseekIcon from '../assets/icons/deepseek.svg';
import geminiIcon from '../assets/icons/gemini.svg';
import metaIcon from '../assets/icons/meta.svg';
import openaiIcon from '../assets/icons/openai-light.svg';
import vertexIcon from '../assets/icons/vertex.svg';
import grokIcon from '../assets/icons/grok.svg';
import {
  isRecord,
  managementApi,
  maskSecret,
  readBoolean,
  readNumber,
  readString,
} from '../services/managementApi';
import {
  DEEPSEEK_BASE_URL,
  fetchModels,
  mergeModelOptions,
  modelSearchText,
  modelsFromRecord,
  normalizeBaseUrl,
  reconcileModelSelection,
  usableModelAlias,
  type ModelOption,
  type ModelProvider,
} from '../services/modelService';
import {
  checkProviderModelHealth,
  checkProviderModelsHealth,
  mergeProviderHealthModels,
  PROVIDER_HEALTH_TIMEOUT_MS,
  type ProviderHealthCheckOptions,
  type ProviderModelHealthResult,
} from '../services/providerHealthCheck';
import { modelMatchesRule } from '../services/oauthModels';
import { normalizeProviderProxyUrl } from '../services/providerProxy';
import { ProviderGroupKeysEditor } from '../components/ProviderGroupKeysEditor';
import { ProviderGroupTemplateFields, ProviderKeyTemplateFields, ProviderModelFields } from '../components/ProviderTemplateFields';
import { providerText } from '../i18n/providerTemplate';
import {
  cleanProviderGroup, effectiveProviderKey, providerGroupIdentity, providerGroupKeys, providerGroupStatus,
  providerKeyDraft, providerKeyIsDisabled, serializeProviderKey, validateProviderGroupKeys, validateProviderTemplateRecord, type ProviderKeyDraft,
} from '../services/providerGroups';
import { providerGroupsApi } from '../services/managementApi';
import {
  appendProviderEntry, locateProviderEntry, providerEntries, removeProviderEntry,
  reorderProviderEntries, updateProviderEntry, type ProviderEntrySource,
} from '../services/providerEntries';
import { getCurrentLocale, translate, useI18n } from '../i18n';
import type { MessageKey } from '../i18n/resources';
import { MessageNotice, FloatingNotice, useAppNotice } from '../appNotice';

export type ProviderSection =
  | 'gemini-api-key'
  | 'interactions-api-key'
  | 'vertex-api-key'
  | 'xai-api-key'
  | 'meta-api-key'
  | 'codex-api-key'
  | 'claude-api-key'
  | 'openai-compatibility';

export type ProviderCategory = ProviderSection | 'deepseek';

export const OPENAI_THINKING_LEVELS = ['low', 'medium', 'high', 'xhigh'] as const;
export { DEEPSEEK_BASE_URL };

type ProviderDefinition = {
  id: ProviderCategory;
  section: ProviderSection;
  responseKey: string;
  labelKey: MessageKey;
  label?: string;
  icon: string;
  openAi: boolean;
};

type ProviderRow = {
  source?: ProviderEntrySource;
  category?: ProviderCategory;
  section: ProviderSection;
  index: number;
  record: Record<string, unknown>;
  name: string;
  apiKey: string;
  apiKeys: string[];
  baseUrl: string;
  models: ModelOption[];
  disabled: boolean;
  priority: number | null;
  authIndex: string;
  remark: string;
};

export type ApiAccessRemarkLocator = {
  providerName: string;
  baseUrl: string;
  apiKeys: string[];
  configIdentity?: string;
};

export const providerDragId = (
  row: Pick<ProviderRow, 'section' | 'index' | 'name' | 'apiKey' | 'baseUrl'>,
) => {
  const identity = `${row.section}\u0000${row.name}\u0000${row.apiKey}\u0000${row.baseUrl}`;
  let hash = 2166136261;
  for (let index = 0; index < identity.length; index += 1) {
    hash ^= identity.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `${row.section}:${(hash >>> 0).toString(36)}:${row.index}`;
};

function SortableProviderRow({
  row,
  disabled,
  dragLabel,
  isDragOver,
  children,
}: {
  row: ProviderRow;
  disabled: boolean;
  dragLabel: string;
  isDragOver: boolean;
  children: ReactNode;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: providerDragId(row),
    disabled,
    transition: {
      duration: 220,
      easing: 'cubic-bezier(0.2, 0, 0, 1)',
    },
  });
  const renderedTransform = transform
    ? {
      ...transform,
      scaleX: isDragging ? 1.012 : transform.scaleX,
      scaleY: isDragging ? 1.012 : transform.scaleY,
    }
    : transform;
  const style: CSSProperties = {
    position: 'relative',
    zIndex: isDragging ? 2 : undefined,
    transform: DndCss.Transform.toString(renderedTransform),
    transition: isDragging ? undefined : transition,
  };

  return (
    <article
      ref={setNodeRef}
      style={style}
      className={`real-provider-row${isDragging ? ' dragging' : ''}${isDragOver && !isDragging ? ' drag-over' : ''}${row.disabled ? ' disabled' : ''}`}
    >
      <button
        type="button"
        className="icon-button quiet provider-drag-handle"
        disabled={disabled}
        aria-label={dragLabel}
        title={dragLabel}
        {...attributes}
        {...listeners}
      >
        <GripVertical size={17} aria-hidden="true" />
      </button>
      {children}
    </article>
  );
}

type ProviderSaveResult =
  | { saved: true }
  | { saved: false; error: string; target: 'form' | 'models' };

const requestErrorMessage = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export type ProviderDraft = {
  weight?: string;
  templateFields?: Record<string, unknown>;
  groupKeys?: ProviderKeyDraft[];
  name: string;
  apiKey: string;
  remark: string;
  baseUrl: string;
  proxyUrl?: string;
  proxyUrlEdited?: boolean;
  proxyUrlMixed?: boolean;
  priority: string;
  models: ModelOption[];
  modelSelectionCatalog?: ModelOption[];
  prefix?: string;
  headersText?: string;
  excludedModelsText?: string;
  disableCooling?: boolean | null;
  websockets?: boolean;
  thinkingLevels?: string[];
  thinkingLevelsEdited?: boolean;
  disabled?: boolean;
  cloakMode?: string;
  cloakStrictMode?: boolean;
  cloakSensitiveWordsText?: string;
  cloakCacheUserId?: boolean | null;
};

const providerDefinitions: ProviderDefinition[] = [
  { id: 'codex-api-key', section: 'codex-api-key', responseKey: 'codex-api-key', labelKey: 'apiAccess.provider.codex', icon: codexIcon, openAi: false },
  {
    id: 'openai-compatibility',
    section: 'openai-compatibility',
    responseKey: 'openai-compatibility',
    labelKey: 'aliases.source.openAiCompatible',
    icon: openaiIcon,
    openAi: true,
  },
  {
    id: 'deepseek',
    section: 'codex-api-key',
    responseKey: 'codex-api-key',
    labelKey: 'apiAccess.provider.deepseek',
    icon: deepseekIcon,
    openAi: false,
  },
  { id: 'claude-api-key', section: 'claude-api-key', responseKey: 'claude-api-key', labelKey: 'apiAccess.provider.claude', icon: claudeIcon, openAi: false },
  { id: 'gemini-api-key', section: 'gemini-api-key', responseKey: 'gemini-api-key', labelKey: 'apiAccess.provider.gemini', icon: geminiIcon, openAi: false },
  { id: 'interactions-api-key', section: 'interactions-api-key', responseKey: 'interactions-api-key', labelKey: 'apiAccess.provider.gemini', label: 'Gemini Interactions', icon: geminiIcon, openAi: false },
  { id: 'vertex-api-key', section: 'vertex-api-key', responseKey: 'vertex-api-key', labelKey: 'apiAccess.provider.gemini', label: 'Vertex AI', icon: vertexIcon, openAi: false },
  { id: 'xai-api-key', section: 'xai-api-key', responseKey: 'xai-api-key', labelKey: 'apiAccess.provider.codex', label: 'xAI', icon: grokIcon, openAi: false },
  { id: 'meta-api-key', section: 'meta-api-key', responseKey: 'meta-api-key', labelKey: 'apiAccess.provider.codex', label: 'Meta', icon: metaIcon, openAi: false },
];

const providerLabel = (definition: ProviderDefinition) => definition.label ?? translate(getCurrentLocale(), definition.labelKey);

const SELECTED_CATEGORY_KEY = 'cpa-gui.api-access-selected-category.v1';

const readSelectedCategory = (): ProviderCategory => {
  const fallback = providerDefinitions[0].id;
  if (typeof window === 'undefined') return fallback;
  try {
    const saved = window.localStorage.getItem(SELECTED_CATEGORY_KEY);
    return providerDefinitions.find((definition) => definition.id === saved)?.id ?? fallback;
  } catch {
    return fallback;
  }
};

const writeSelectedCategory = (category: ProviderCategory) => {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(SELECTED_CATEGORY_KEY, category);
  } catch { /* Keep category navigation available when storage is unavailable. */ }
};

export const providerSectionOrder = providerDefinitions.map((definition) => definition.id);

const providerLoadDefinitions = providerDefinitions.filter(
  (definition, index, definitions) =>
    definitions.findIndex((item) => item.section === definition.section) === index,
);

const emptyRecords = (): Record<ProviderSection, Record<string, unknown>[]> => ({
  'gemini-api-key': [],
  'interactions-api-key': [],
  'vertex-api-key': [],
  'xai-api-key': [],
  'meta-api-key': [],
  'codex-api-key': [],
  'claude-api-key': [],
  'openai-compatibility': [],
});

const definitionFor = (category: ProviderCategory) =>
  providerDefinitions.find((item) => item.id === category) ?? providerDefinitions[0];

const isDeepSeekRecord = (record: Record<string, unknown>) => {
  const name = readString(record, 'name').trim().toLowerCase();
  const baseUrl = readString(record, 'base-url', 'baseUrl').trim().toLowerCase();
  return name.includes('deepseek') || /^https?:\/\/api\.deepseek\.com(?:\/|$)/i.test(baseUrl);
};

export const providerCategoryMatchesRecord = (
  category: ProviderCategory,
  record: Record<string, unknown>,
  section: ProviderSection = definitionFor(category).section,
) => {
  if (category === 'deepseek') {
    return section === 'codex-api-key' && isDeepSeekRecord(record);
  }
  if (category === 'codex-api-key') {
    return section === 'codex-api-key' && !isDeepSeekRecord(record);
  }
  if (category === 'openai-compatibility') {
    return section === 'openai-compatibility';
  }
  return true;
};

export const sectionRecordsFromConfig = (payload: unknown, section: ProviderSection) =>
  isRecord(payload) && Array.isArray(payload[section])
    ? payload[section].filter(isRecord)
    : [];

const rowFromRecord = (
  section: ProviderSection,
  record: Record<string, unknown>,
  index: number,
): ProviderRow => {
  const grouped = Array.isArray(record.keys);
  const entries = grouped ? providerGroupKeys(record) : definitionFor(section).openAi && Array.isArray(record['api-key-entries'])
    ? record['api-key-entries'].filter(isRecord)
    : [];
  const entry = entries[0] ?? null;
  const apiKeys = entries
    .map((item) => readString(item, 'api-key', 'apiKey'))
    .filter(Boolean);
  const singleApiKey = readString(record, 'api-key', 'apiKey');
  return {
    section,
    index,
    record,
    name: grouped || definitionFor(section).openAi
      ? readString(record, 'name') || translate(getCurrentLocale(), 'apiAccess.compatibleName', { number: index + 1 })
      : providerLabel(definitionFor(section)),
    apiKey: entry ? readString(entry, 'api-key', 'apiKey') : singleApiKey,
    apiKeys: entry ? apiKeys : singleApiKey ? [singleApiKey] : [],
    baseUrl: readString(record, 'base-url', 'baseUrl'),
    models: Array.isArray(record.models)
      ? record.models.flatMap((model) => modelsFromRecord([model], true)) : modelsFromRecord(record.models, true),
    disabled: grouped ? providerGroupStatus(record) === 'disabled' : providerKeyIsDisabled(record, record),
    priority: record.priority == null ? null : readNumber(record, 'priority'),
    authIndex: entry
      ? readString(entry, 'auth-index', 'authIndex')
      : readString(record, 'auth-index', 'authIndex'),
    remark: '',
  };
};

export const providerRowsFromGroups = (section: ProviderSection, groups: Record<string, unknown>[]): ProviderRow[] =>
  providerEntries(section, groups).map(({ record, source }, index) => {
    const deepSeek = section === 'codex-api-key' && (isDeepSeekRecord(record) || isDeepSeekRecord(source.group));
    const row = rowFromRecord(section, record, index);
    const key = source.keyIndex !== undefined && Array.isArray(source.group.keys)
      ? source.group.keys[source.keyIndex]
      : undefined;
    return {
      ...row,
      source,
      category: deepSeek ? 'deepseek' : section,
      name: deepSeek ? 'DeepSeek' : row.name,
      disabled: isRecord(key) ? providerKeyIsDisabled(source.group, key) : row.disabled,
    };
  });

export const providerRemarkIdentity = (
  section: ProviderSection,
  locator: ApiAccessRemarkLocator,
) => JSON.stringify([section, locator.providerName, locator.baseUrl, locator.apiKeys, locator.configIdentity ?? '']);

export const apiAccessRemarkLocatorFromRecord = (
  section: ProviderSection,
  record: Record<string, unknown>,
): ApiAccessRemarkLocator => {
  const row = rowFromRecord(section, record, 0);
  return {
    providerName: readString(record, 'name'),
    baseUrl: row.baseUrl,
    apiKeys: row.apiKeys,
    configIdentity: providerRemarkConfigIdentity(record),
  };
};

const apiAccessRemarkLocatorFromRow = (row: ProviderRow): ApiAccessRemarkLocator =>
  apiAccessRemarkLocatorFromRecord(row.section, row.record);

const providerHealthIdentity = (row: ProviderRow) => [
  row.section,
  row.index,
  row.name,
  row.baseUrl,
  row.authIndex,
  row.apiKeys.join('\u0000'),
  row.models.map((model) => model.name).join('\u0000'),
].join('\u0001');

const providerModelType = (
  section: ProviderSection,
  record?: Record<string, unknown>,
): ModelProvider => {
  if (section === 'gemini-api-key') return 'gemini';
  if (section === 'interactions-api-key') return 'interactions';
  if (section === 'vertex-api-key') return 'vertex';
  if (section === 'xai-api-key') return 'xai';
  if (section === 'meta-api-key') return 'meta';
  if (section === 'claude-api-key') return 'claude';
  if (section === 'codex-api-key' && record && isDeepSeekRecord(record)) return 'deepseek';
  if (section === 'codex-api-key') return 'codex';
  return 'openai';
};

const providerHeadersFromRecord = (record: Record<string, unknown>) =>
  isRecord(record.headers)
    ? Object.fromEntries(Object.entries(record.headers).map(([key, value]) => [key, String(value)]))
    : {};

export const stripResponseFields = (record: Record<string, unknown>) => {
  if (Array.isArray(record.keys)) return cleanProviderGroup(record);
  const next = normalizeProviderModels(record);
  delete next['test-model'];
  delete next.testModel;
  delete next['auth-index'];
  delete next.authIndex;
  delete next.auth_index;
  if (Array.isArray(next['api-key-entries'])) {
    next['api-key-entries'] = next['api-key-entries']
      .filter(isRecord)
      .map((entry) => {
        const clean = { ...entry };
        delete clean['auth-index'];
        delete clean.authIndex;
        delete clean.auth_index;
        return clean;
      });
  }
  return next;
};

const normalizeProviderIdentity = (value: unknown): unknown => {
  if (value == null || value === '') return undefined;
  if (Array.isArray(value)) return value.length ? value.map(normalizeProviderIdentity) : undefined;
  if (isRecord(value)) {
    const entries = Object.keys(value).sort().flatMap((key) => {
      const normalized = normalizeProviderIdentity(value[key]);
      return normalized === undefined ? [] : [[key, normalized]];
    });
    return entries.length ? Object.fromEntries(entries) : undefined;
  }
  return value;
};

const providerConfigIdentity = (record: Record<string, unknown>) => {
  if (Array.isArray(record.keys)) return providerGroupIdentity(record);
  const config = stripResponseFields(record);
  if (config.priority === 0) delete config.priority;
  if (config.websockets === false) delete config.websockets;
  if (config.disabled === false) delete config.disabled;
  return JSON.stringify(normalizeProviderIdentity(config) ?? {});
};

const providerRemarkConfigIdentity = (record: Record<string, unknown>) => {
  const config = stripResponseFields(record);
  for (const key of ['name', 'api-key', 'apiKey', 'api-key-entries', 'keys', 'base-url', 'baseUrl', 'disabled']) {
    delete config[key];
  }
  if (Array.isArray(config['excluded-models'])) {
    config['excluded-models'] = config['excluded-models'].filter((model) => String(model).trim() !== '*');
  }
  return providerConfigIdentity(config);
};

export const hasDuplicateProviderRecord = (
  section: ProviderSection,
  records: Record<string, unknown>[],
  candidates: Record<string, unknown>[],
  targetIndex = -1,
) => records.some((record, index) => index !== targetIndex && candidates.some((candidate) => {
  if (Array.isArray(candidate.keys)) return readString(record, 'name') === readString(candidate, 'name');
  if (definitionFor(section).openAi) return readString(record, 'name') === readString(candidate, 'name');
  if (section === 'gemini-api-key') {
    return readString(record, 'api-key', 'apiKey') === readString(candidate, 'api-key', 'apiKey')
      && readString(record, 'base-url', 'baseUrl') === readString(candidate, 'base-url', 'baseUrl');
  }
  const { name: _recordName, ...recordConfig } = record;
  const { name: _candidateName, ...candidateConfig } = candidate;
  return providerConfigIdentity(recordConfig) === providerConfigIdentity(candidateConfig);
}));

const mergeModelRecords = (current: unknown, selected: ModelOption[]) => {
  const existing = Array.isArray(current) ? current : [];
  const consumedExistingIndexes = new Set<number>();
  const selectedNames = new Set<string>();
  const seen = new Set<string>();
  const models = selected.reduce<Record<string, unknown>[]>((result, model) => {
    const name = model.name.trim();
    const key = name.toLowerCase();
    if (!name || seen.has(key)) return result;
    seen.add(key);
    selectedNames.add(key);
    const requested = (model.alias ?? '').trim();
    const requestedAlias = requested.toLowerCase();
    let matchedIndex = existing.findIndex((item, index) => (
      !consumedExistingIndexes.has(index)
      && isRecord(item)
      && readString(item, 'name').toLowerCase() === key
      && readString(item, 'alias').toLowerCase() === requestedAlias
    ));
    if (matchedIndex < 0) {
      matchedIndex = existing.findIndex((item, index) => (
        !consumedExistingIndexes.has(index)
        && isRecord(item)
        && readString(item, 'name').toLowerCase() === key
      ));
    }
    if (matchedIndex >= 0) consumedExistingIndexes.add(matchedIndex);
    const matched = matchedIndex >= 0 ? existing[matchedIndex] : undefined;
    const next: Record<string, unknown> = model.config ? { ...model.config } : isRecord(matched) ? { ...matched } : {};
    next.name = name;
    const storedAlias = readString(next, 'alias');
    const alias = usableModelAlias(requested);
    if (alias && alias !== name) {
      next.alias = alias;
    } else if (
      requested
      && storedAlias
      && requested.toLowerCase() === storedAlias.toLowerCase()
      && requested.toLowerCase() !== name.toLowerCase()
    ) {
      next.alias = storedAlias;
    } else if (requested && requested !== name) {
      throw new Error(translate(getCurrentLocale(), 'aliases.error.invalidAlias'));
    } else {
      delete next.alias;
    }
    if (model.thinking) next.thinking = { ...model.thinking };
    result.push(next);
    return result;
  }, []);

  existing.forEach((item, index) => {
    if (
      consumedExistingIndexes.has(index)
      || !isRecord(item)
      || !selectedNames.has(readString(item, 'name').toLowerCase())
    ) return;
    models.push({ ...item });
  });

  return models;
};

export const exclusionsForModelSelection = (
  currentText: string,
  discoveredModels: ModelOption[],
  selectedModels: ModelOption[],
) => {
  const discovered = new Map<string, string>();
  discoveredModels.forEach((model) => {
    const name = model.name.trim();
    if (name && !discovered.has(name.toLowerCase())) discovered.set(name.toLowerCase(), name);
  });
  const selected = new Set(
    selectedModels.map((model) => model.name.trim().toLowerCase()).filter(Boolean),
  );
  const selectedClientNames = new Set(
    selectedModels
      .filter((model) => model.name.trim())
      .map((model) => (model.alias?.trim() || model.name.trim()).toLowerCase()),
  );
  const rules = currentText
    .split(/[,\n]/)
    .map((value) => value.trim())
    .filter(Boolean);
  const next = rules.filter((rule) => !discovered.has(rule.toLowerCase()));

  if (selected.size > 0) {
    discovered.forEach((name, key) => {
      if (!selected.has(key) && !selectedClientNames.has(key)) next.push(name);
    });
  }

  return next
    .filter((rule, index, values) =>
      values.findIndex((value) => value.toLowerCase() === rule.toLowerCase()) === index,
    )
    .join('\n');
};

export const modelSelectionForDiscovery = (
  configuredModels: ModelOption[],
  discoveredModels: ModelOption[],
  excludedModelsText: string,
) => {
  const configured = new Set(
    configuredModels.map((model) => model.name.trim().toLowerCase()).filter(Boolean),
  );
  if (configured.size > 0) return configured;

  const excludedRules = excludedModelsText
    .split(/[,\n]/)
    .map((rule) => rule.trim())
    .filter(Boolean);
  return new Set(
    discoveredModels
      .filter((model) => !excludedRules.some((rule) => modelMatchesRule(model.name, rule)))
      .map((model) => model.name.toLowerCase()),
  );
};

export const parseProviderApiKeys = (value: string) => value
  .split(/\r?\n/)
  .map((item) => item.trim())
  .filter((item, index, values) => item && values.indexOf(item) === index);

const mergeOpenAiApiKeyEntries = (current: unknown, apiKey: string) => {
  const entries = Array.isArray(current) ? current.filter(isRecord) : [];
  const keys = parseProviderApiKeys(apiKey);
  const usedIndexes = new Set<number>();
  return keys.map((key, index) => {
    let matchedIndex = entries.findIndex(
      (entry, entryIndex) =>
        !usedIndexes.has(entryIndex) && readString(entry, 'api-key', 'apiKey') === key,
    );
    if (matchedIndex < 0 && entries[index] && !usedIndexes.has(index)) matchedIndex = index;
    if (matchedIndex >= 0) usedIndexes.add(matchedIndex);
    const next = matchedIndex >= 0 ? stripResponseFields(entries[matchedIndex]) : {};
    next['api-key'] = key;
    return next;
  });
};

const thinkingLevelsFromModels = (models: ModelOption[]): string[] => {
  const levels: string[] = [];
  models.forEach((model) => {
    const configured = model.thinking?.levels;
    if (!Array.isArray(configured)) return;
    configured.forEach((level) => {
      const normalized = String(level).trim().toLowerCase();
      if (normalized && !levels.includes(normalized)) {
        levels.push(normalized);
      }
    });
  });
  return levels;
};

export const providerProxyDraftFromRecord = (
  section: ProviderSection,
  record: Record<string, unknown>,
): Pick<ProviderDraft, 'proxyUrl' | 'proxyUrlEdited' | 'proxyUrlMixed'> => {
  if (Array.isArray(record.keys)) return { proxyUrl: readString(record, 'proxy-url'), proxyUrlEdited: false, proxyUrlMixed: false };
  const entries = definitionFor(section).openAi && Array.isArray(record['api-key-entries'])
    ? record['api-key-entries'].filter(isRecord)
    : [];
  const proxyUrlMixed = entries.some((entry) =>
    readString(entry, 'proxy-url', 'proxyUrl') !== readString(entries[0], 'proxy-url', 'proxyUrl'));
  return {
    proxyUrl: definitionFor(section).openAi
      ? proxyUrlMixed ? '' : readString(entries[0], 'proxy-url', 'proxyUrl')
      : readString(record, 'proxy-url', 'proxyUrl'),
    proxyUrlEdited: false,
    proxyUrlMixed,
  };
};

const draftFromRow = (row: ProviderRow): ProviderDraft => {
  const definition = definitionFor(row.section);
  const isDeepSeek = row.category === 'deepseek' || (row.section === 'codex-api-key' && isDeepSeekRecord(row.record));
  return {
    weight: row.record.weight == null ? '' : String(row.record.weight),
    templateFields: Object.fromEntries(['request-retry', 'request-scoped-errors', 'support-prompt-cache-key', 'disable-codex-cloaking', 'alpha-search', 'rebuild-mid-system-message', 'fingerprint-profile', 'experimental-cch-signing']
      .filter((field) => Object.prototype.hasOwnProperty.call(row.record, field)).map((field) => [field, structuredClone(row.record[field])])),
    ...(Array.isArray(row.record.keys) ? { groupKeys: providerGroupKeys(row.record).map(providerKeyDraft) } : {}),
    name: Array.isArray(row.record.keys) ? row.name : isDeepSeek ? 'DeepSeek' : row.name,
    apiKey: definition.openAi ? row.apiKeys.join('\n') : row.apiKey,
    remark: row.remark || (!Array.isArray(row.record.keys) && definition.openAi && !isDeepSeek ? row.name : ''),
    baseUrl: row.baseUrl,
    ...providerProxyDraftFromRecord(row.section, row.record),
    priority: row.priority === null ? '' : String(row.priority),
    models: row.models,
    prefix: readString(row.record, 'prefix'),
    headersText: isRecord(row.record.headers)
      ? Object.entries(row.record.headers)
        .map(([key, value]) => `${key}: ${String(value)}`)
        .join('\n')
      : '',
    excludedModelsText: Array.isArray(row.record['excluded-models'])
      ? row.record['excluded-models'].map(String).filter((model) => model.trim() !== '*').join('\n')
      : '',
    disableCooling: typeof row.record['disable-cooling'] === 'boolean' ? row.record['disable-cooling'] : null,
    websockets: readBoolean(row.record, 'websockets'),
    thinkingLevels: definition.openAi
      ? thinkingLevelsFromModels(row.models)
      : undefined,
    thinkingLevelsEdited: false,
    disabled: row.disabled,
    cloakMode: isRecord(row.record.cloak) ? readString(row.record.cloak, 'mode') : '',
    cloakStrictMode: isRecord(row.record.cloak)
      ? readBoolean(row.record.cloak, 'strict-mode', 'strictMode')
      : false,
    cloakSensitiveWordsText:
      isRecord(row.record.cloak) && Array.isArray(row.record.cloak['sensitive-words'])
        ? row.record.cloak['sensitive-words'].map(String).join('\n')
        : '',
    cloakCacheUserId: isRecord(row.record.cloak) && typeof row.record.cloak['cache-user-id'] === 'boolean'
      ? row.record.cloak['cache-user-id'] : null,
  };
};

export const providerDraftFromRecord = (section: ProviderSection, record: Record<string, unknown>) =>
  draftFromRow(rowFromRecord(section, record, 0));

const emptyProviderDraft = (): ProviderDraft => ({
  templateFields: {},
  name: '',
  apiKey: '',
  remark: '',
  baseUrl: '',
  proxyUrl: '',
  priority: '',
  models: [],
  prefix: '',
  headersText: '',
  excludedModelsText: '',
  disableCooling: null,
  websockets: false,
  disabled: false,
  cloakMode: '',
  cloakStrictMode: false,
  cloakSensitiveWordsText: '',
  cloakCacheUserId: null,
});

export const createProviderDraft = (category: ProviderCategory): ProviderDraft => {
  const draft = emptyProviderDraft();
  if (category === 'openai-compatibility') return { ...draft, thinkingLevels: [], thinkingLevelsEdited: false };
  if (category !== 'deepseek') return draft;
  return {
    ...draft,
    name: 'DeepSeek',
    remark: '',
    baseUrl: DEEPSEEK_BASE_URL,
  };
};

export const applyProviderRemarkIdentity = (
  category: ProviderCategory,
  draft: ProviderDraft,
): ProviderDraft => draft.groupKeys ? draft : category === 'deepseek'
  ? { ...draft, name: draft.name.trim() || 'DeepSeek' }
  : definitionFor(category).openAi
    ? { ...draft, name: draft.remark.trim() }
    : draft;

export const applyProviderPreset = (
  category: ProviderCategory,
  draft: ProviderDraft,
): ProviderDraft => {
  if (!definitionFor(category).openAi || draft.thinkingLevels === undefined || draft.thinkingLevelsEdited === false) return draft;
  const levels = draft.thinkingLevels;
  return {
    ...draft,
    models: draft.models.map((model) => {
      const thinking = { ...model.thinking };
      if (levels.length > 0) thinking.levels = [...levels];
      else delete thinking.levels;
      const { thinking: _thinking, ...withoutThinking } = model;
      return Object.keys(thinking).length > 0
        ? { ...withoutThinking, thinking }
        : withoutThinking;
    }),
  };
};

export const parseProviderHeaders = (value: string): Record<string, string> => {
  const headers: Record<string, string> = {};
  value.split(/\r?\n/).forEach((line, index) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    const separator = trimmed.indexOf(':');
    if (separator <= 0) throw new Error(translate(getCurrentLocale(), 'apiAccess.error.headerMissingColon', { number: index + 1 }));
    const key = trimmed.slice(0, separator).trim();
    const headerValue = trimmed.slice(separator + 1).trim();
    if (!/^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/.test(key) || !headerValue) {
      throw new Error(translate(getCurrentLocale(), 'apiAccess.error.headerInvalid', { number: index + 1 }));
    }
    const duplicateKey = Object.keys(headers).find(
      (current) => current.toLowerCase() === key.toLowerCase(),
    );
    if (duplicateKey) delete headers[duplicateKey];
    headers[key] = headerValue;
  });
  return headers;
};

const applyAdvancedFields = (
  next: Record<string, unknown>,
  section: ProviderSection,
  draft: ProviderDraft,
) => {
  if (draft.prefix !== undefined) {
    const prefix = draft.prefix.trim();
    if (prefix) next.prefix = prefix;
    else delete next.prefix;
  }
  if (draft.headersText !== undefined) {
    const headers = parseProviderHeaders(draft.headersText);
    if (Object.keys(headers).length > 0) next.headers = headers;
    else delete next.headers;
  }
  if (draft.excludedModelsText !== undefined && section !== 'openai-compatibility') {
    const excludedModelsText = draft.modelSelectionCatalog && draft.models.some((model) => model.name.trim())
      ? exclusionsForModelSelection(
          draft.excludedModelsText,
          draft.modelSelectionCatalog,
          (Array.isArray(next.models) ? next.models : []).filter(isRecord).map((model) => ({
            name: readString(model, 'name'),
            alias: readString(model, 'alias'),
          })),
        )
      : draft.excludedModelsText;
    const excludedModels = excludedModelsText
      .split(/[,\n]/)
      .map((value) => value.trim())
      .filter((value, index, values) =>
        value && values.findIndex((item) => item.toLowerCase() === value.toLowerCase()) === index,
      );
    if (draft.disabled && !excludedModels.includes('*')) excludedModels.push('*');
    if (excludedModels.length > 0) next['excluded-models'] = excludedModels;
    else delete next['excluded-models'];
  }
  if (draft.disableCooling !== undefined) {
    if (draft.disableCooling === null) delete next['disable-cooling'];
    else next['disable-cooling'] = draft.disableCooling;
  }
  if (draft.websockets !== undefined && (section === 'codex-api-key' || section === 'xai-api-key')) {
    next.websockets = draft.websockets;
  }
  if (
    section === 'claude-api-key'
    && (
      draft.cloakMode !== undefined
      || draft.cloakStrictMode !== undefined
      || draft.cloakSensitiveWordsText !== undefined
      || draft.cloakCacheUserId !== undefined
    )
  ) {
    const cloak: Record<string, unknown> = isRecord(next.cloak) ? { ...next.cloak } : {};
    if (draft.cloakMode !== undefined) {
      const mode = draft.cloakMode.trim();
      if (mode) cloak.mode = mode;
      else delete cloak.mode;
    }
    if (draft.cloakStrictMode !== undefined) {
      delete cloak.strictMode;
      if (draft.cloakStrictMode) cloak['strict-mode'] = true;
      else delete cloak['strict-mode'];
    }
    if (draft.cloakSensitiveWordsText !== undefined) {
      const sensitiveWords = draft.cloakSensitiveWordsText.split(/[,\n]/)
        .map((value) => value.trim())
        .filter((value, index, values) => value && values.indexOf(value) === index);
      delete cloak.sensitiveWords;
      if (sensitiveWords.length > 0) cloak['sensitive-words'] = sensitiveWords;
      else delete cloak['sensitive-words'];
    }
    if (draft.cloakCacheUserId !== undefined) {
      delete cloak.cacheUserId;
      if (draft.cloakCacheUserId === null) delete cloak['cache-user-id'];
      else cloak['cache-user-id'] = draft.cloakCacheUserId;
    }
    if (Object.keys(cloak).length > 0) next.cloak = cloak;
    else delete next.cloak;
  }
  return next;
};

export const buildProviderRecord = (
  section: ProviderSection,
  draft: ProviderDraft,
  current?: Record<string, unknown>,
) => {
  const record = current ? stripResponseFields(current) : {};
  const priorityText = draft.priority.trim();
  const priority = priorityText ? Number(priorityText) : null;
  if (priority !== null && !Number.isSafeInteger(priority)) {
    throw new Error(translate(getCurrentLocale(), 'apiAccess.error.priorityInteger'));
  }
  const models = mergeModelRecords(record.models, draft.models);
  if (definitionFor(section).openAi) {
    if (draft.thinkingLevels !== undefined && draft.thinkingLevelsEdited !== false) {
      for (const model of models) {
        const thinking = isRecord(model.thinking) ? { ...model.thinking } : {};
        if (draft.thinkingLevels.length) thinking.levels = [...draft.thinkingLevels];
        else delete thinking.levels;
        if (Object.keys(thinking).length) model.thinking = thinking;
        else delete model.thinking;
      }
    }
    const entries = mergeOpenAiApiKeyEntries(record['api-key-entries'], draft.apiKey.trim());
    if (draft.proxyUrlEdited !== false && draft.proxyUrl !== undefined) {
      const proxyUrl = draft.proxyUrl.trim();
      entries.forEach((entry) => {
        if (proxyUrl) entry['proxy-url'] = proxyUrl;
        else delete entry['proxy-url'];
      });
    }
    const next: Record<string, unknown> = {
      ...record,
      name: draft.name.trim(),
      'base-url': draft.baseUrl.trim(),
      'api-key-entries': entries,
      models,
    };
    if (priority !== null && Number.isFinite(priority)) next.priority = priority;
    else delete next.priority;
    return applyAdvancedFields(next, section, draft);
  }

  const next: Record<string, unknown> = {
    ...record,
    'api-key': draft.apiKey.trim(),
    models,
  };
  if (section === 'codex-api-key' && draft.name.trim().toLowerCase() === 'deepseek') {
    next.name = 'DeepSeek';
  }
  if (draft.baseUrl.trim()) next['base-url'] = draft.baseUrl.trim();
  else delete next['base-url'];
  if (draft.proxyUrl !== undefined && draft.proxyUrlEdited !== false) {
    const proxyUrl = draft.proxyUrl.trim();
    if (proxyUrl) next['proxy-url'] = proxyUrl;
    else delete next['proxy-url'];
  }
  if (priority !== null && Number.isFinite(priority)) next.priority = priority;
  else delete next.priority;
  return applyAdvancedFields(next, section, draft);
};

// The form edits one effective credential. Preserve every untouched persisted
// value so displaying inherited defaults never turns them into key overrides.
export const buildProviderKeyRecord = (
  section: ProviderSection, draft: ProviderDraft, current?: Record<string, unknown>,
): Record<string, unknown> => {
  const next = buildProviderRecord(section, draft, current);
  if (current && Object.prototype.hasOwnProperty.call(current, 'name')) next.name = current.name;
  else delete next.name;
  const baseline = current ? providerDraftFromRecord(section, current) : emptyProviderDraft();
  const fields: [keyof ProviderDraft, string][] = [
    ['apiKey', 'api-key'], ['baseUrl', 'base-url'], ['proxyUrl', 'proxy-url'], ['priority', 'priority'],
    ['prefix', 'prefix'], ['headersText', 'headers'], ['excludedModelsText', 'excluded-models'],
    ['disableCooling', 'disable-cooling'], ['websockets', 'websockets'], ['models', 'models'],
  ];
  if (JSON.stringify(draft.models) !== JSON.stringify(baseline.models)) {
    // Keep each alias row independently editable, including repeated upstream IDs.
    next.models = draft.models.filter((model) => model.name.trim()).flatMap((model) => mergeModelRecords([], [model]));
  }
  for (const [field, configField] of fields) {
    if (field === 'excludedModelsText' && draft.modelSelectionCatalog) continue;
    if (JSON.stringify(draft[field]) !== JSON.stringify(baseline[field])) continue;
    if (current && Object.prototype.hasOwnProperty.call(current, configField)) next[configField] = structuredClone(current[configField]);
    else delete next[configField];
  }
  if (['cloakMode', 'cloakStrictMode', 'cloakSensitiveWordsText', 'cloakCacheUserId'].every((field) =>
    JSON.stringify(draft[field as keyof ProviderDraft]) === JSON.stringify(baseline[field as keyof ProviderDraft]))) {
    if (current && Object.prototype.hasOwnProperty.call(current, 'cloak')) next.cloak = structuredClone(current.cloak);
    else delete next.cloak;
  } else if (isRecord(current?.cloak)) {
    const cloak = isRecord(next.cloak) ? { ...next.cloak } : {};
    const cloakFields: [keyof ProviderDraft, string][] = [
      ['cloakMode', 'mode'], ['cloakStrictMode', 'strict-mode'],
      ['cloakSensitiveWordsText', 'sensitive-words'], ['cloakCacheUserId', 'cache-user-id'],
    ];
    for (const [field, configField] of cloakFields) {
      if (JSON.stringify(draft[field]) !== JSON.stringify(baseline[field])) continue;
      if (Object.prototype.hasOwnProperty.call(current.cloak, configField)) cloak[configField] = structuredClone(current.cloak[configField]);
      else delete cloak[configField];
    }
    if (Object.keys(cloak).length) next.cloak = cloak; else delete next.cloak;
  }
  const weight = draft.weight?.trim() ?? '';
  if (weight && (!Number.isSafeInteger(Number(weight)) || Number(weight) > 1_000_000)) {
    throw new Error(translate(getCurrentLocale(), 'apiAccess.groups.invalidKey', { field: 'weight' }));
  }
  if (weight !== (baseline.weight?.trim() ?? '')) {
    if (weight) next.weight = Number(weight); else delete next.weight;
  }
  for (const field of new Set([...Object.keys(baseline.templateFields ?? {}), ...Object.keys(draft.templateFields ?? {})])) {
    if (JSON.stringify(baseline.templateFields?.[field]) === JSON.stringify(draft.templateFields?.[field])) continue;
    if (Object.prototype.hasOwnProperty.call(draft.templateFields ?? {}, field)) next[field] = structuredClone(draft.templateFields![field]);
    else delete next[field];
  }
  const invalid = validateProviderTemplateRecord(next);
  if (invalid) throw new Error(translate(getCurrentLocale(), 'apiAccess.groups.invalidKey', { field: invalid }));
  return next;
};

export const providerDraftConnection = (
  draft: Pick<ProviderDraft, 'headersText' | 'proxyUrl'>, key?: ProviderKeyDraft,
) => effectiveProviderKey({
  headers: parseProviderHeaders(draft.headersText ?? ''), 'proxy-url': draft.proxyUrl,
}, key ? serializeProviderKey(key) : {});

export const buildProviderGroupRecord = (
  section: ProviderSection, draft: ProviderDraft, current?: Record<string, unknown>,
): Record<string, unknown> => {
  const sharedDraft = { ...draft, apiKey: '', proxyUrlEdited: true,
    websockets: undefined, cloakMode: undefined, cloakStrictMode: undefined,
    cloakSensitiveWordsText: undefined, cloakCacheUserId: undefined,
    disabled: Array.isArray(current?.['excluded-models']) && current['excluded-models'].includes('*'),
  };
  const next = buildProviderRecord(section, sharedDraft, current);
  // Native groups expose every mapping, including multiple aliases of the same
  // upstream ID. Match each row independently so edits/removals are explicit.
  const existing = Array.isArray(current?.models) ? current.models : [];
  const used = new Set<number>();
  next.models = draft.models.filter((model) => model.name.trim()).flatMap((model) => {
    let index = existing.findIndex((item, index) => !used.has(index) && isRecord(item)
      && readString(item, 'name') === model.name && readString(item, 'alias') === (model.alias ?? ''));
    if (index < 0) index = existing.findIndex((item, index) => !used.has(index) && isRecord(item) && readString(item, 'name') === model.name);
    if (index >= 0) used.add(index);
    return mergeModelRecords(index >= 0 ? [existing[index]] : [], [model]);
  });
  if (section === 'openai-compatibility' && draft.thinkingLevelsEdited) {
    for (const model of next.models as Record<string, unknown>[]) {
      const thinking = isRecord(model.thinking) ? { ...model.thinking } : {};
      if (draft.thinkingLevels?.length) thinking.levels = draft.thinkingLevels;
      else delete thinking.levels;
      if (Object.keys(thinking).length) model.thinking = thinking; else delete model.thinking;
    }
  }
  applyAdvancedFields(next, section, sharedDraft);
  delete next['api-key'];
  delete next['api-key-entries'];
  next.name = draft.name.trim();
  if (draft.proxyUrl?.trim()) next['proxy-url'] = draft.proxyUrl.trim();
  else delete next['proxy-url'];
  next.keys = (draft.groupKeys ?? []).map(serializeProviderKey);
  if (draft.templateFields) {
    for (const field of ['request-retry', 'request-scoped-errors', 'support-prompt-cache-key']) {
      if (Object.prototype.hasOwnProperty.call(draft.templateFields, field)) next[field] = structuredClone(draft.templateFields[field]);
      else delete next[field];
    }
  }
  // Preserve absent/null/empty fields when the corresponding shared setting was
  // not edited. In v8 these values can have different inheritance semantics.
  if (current) {
    const baseline = draftFromRow(rowFromRecord(section, current, 0));
    const fields: [keyof ProviderDraft, string][] = [
      ['baseUrl', 'base-url'], ['proxyUrl', 'proxy-url'], ['priority', 'priority'],
      ['prefix', 'prefix'], ['headersText', 'headers'], ['excludedModelsText', 'excluded-models'],
      ['disableCooling', 'disable-cooling'], ['models', 'models'],
    ];
    for (const [field, configField] of fields) {
      if (field === 'models' && draft.thinkingLevelsEdited) continue;
      if (field === 'excludedModelsText' && draft.modelSelectionCatalog) continue;
      if (JSON.stringify(draft[field]) !== JSON.stringify(baseline[field])) continue;
      if (Object.prototype.hasOwnProperty.call(current, configField)) next[configField] = structuredClone(current[configField]);
      else delete next[configField];
    }
  }
  const invalid = validateProviderTemplateRecord(next);
  if (invalid) throw new Error(`${providerText('invalid')}${invalid}`);
  return next;
};

const providerIdentityMatches = (
  row: ProviderRecordIdentity,
  record: Record<string, unknown>,
) => {
  if (Array.isArray(record.keys)) return readString(record, 'name') === row.name;
  if (definitionFor(row.section).openAi) {
    return readString(record, 'name') === row.name;
  }
  return (
    readString(record, 'api-key', 'apiKey') === row.apiKey
    && readString(record, 'base-url', 'baseUrl') === row.baseUrl
  );
};

export type ProviderRecordIdentity = Pick<
  ProviderRow,
  'section' | 'index' | 'name' | 'apiKey' | 'baseUrl'
> & Partial<Pick<ProviderRow, 'record'>>;

const providerPrimaryIdentityMatches = (
  row: ProviderRecordIdentity,
  record: Record<string, unknown>,
) => Array.isArray(record.keys) || definitionFor(row.section).openAi
  ? readString(record, 'name') === row.name
  : readString(record, 'api-key', 'apiKey') === row.apiKey;

export const resolveProviderRecordIndex = (
  records: Record<string, unknown>[],
  row: ProviderRecordIdentity,
) => {
  if (row.record) {
    const identity = providerConfigIdentity(row.record);
    const matches = records.flatMap((record, index) => (
      providerConfigIdentity(record) === identity ? [index] : []
    ));
    if (matches.length === 1) return matches[0];
    if (matches.length > 1) return matches.includes(row.index) ? row.index : -1;

    const withoutBaseUrl = (record: Record<string, unknown>) => {
      const { 'base-url': _baseUrl, baseUrl: _camelBaseUrl, ...rest } = record;
      return providerConfigIdentity(rest);
    };
    const defaultUrlMatches = records.flatMap((record, index) => (
      providerPrimaryIdentityMatches(row, record)
      && (!readString(record, 'base-url', 'baseUrl') || !row.baseUrl)
      && withoutBaseUrl(record) === withoutBaseUrl(row.record!) ? [index] : []
    ));
    return defaultUrlMatches.length === 1 ? defaultUrlMatches[0] : -1;
  }

  const exactMatches = records.flatMap((record, index) => providerIdentityMatches(row, record) ? [index] : []);
  if (exactMatches.length === 1) return exactMatches[0];
  if (exactMatches.length > 1) return -1;

  const indexedRecord = records[row.index];
  if (indexedRecord && providerPrimaryIdentityMatches(row, indexedRecord)) {
    return row.index;
  }

  const primaryMatches = records
    .map((record, index) => ({ record, index }))
    .filter(({ record }) => providerPrimaryIdentityMatches(row, record));
  return primaryMatches.length === 1 ? primaryMatches[0].index : -1;
};

export const reorderProviderRecords = (
  records: Record<string, unknown>[],
  scopeRows: ProviderRecordIdentity[],
  source: ProviderRecordIdentity,
  target: ProviderRecordIdentity,
) => {
  const sourceIndex = resolveProviderRecordIndex(records, source);
  const targetIndex = resolveProviderRecordIndex(records, target);
  if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return null;

  const scopedIndexes = scopeRows
    .map((row) => resolveProviderRecordIndex(records, row))
    .filter((index, position, indexes) => index >= 0 && indexes.indexOf(index) === position)
    .sort((left, right) => left - right);
  const sourcePosition = scopedIndexes.indexOf(sourceIndex);
  const targetPosition = scopedIndexes.indexOf(targetIndex);
  if (sourcePosition < 0 || targetPosition < 0) return null;

  const reordered = scopedIndexes.map((index) => stripResponseFields(records[index]));
  const [moved] = reordered.splice(sourcePosition, 1);
  reordered.splice(targetPosition, 0, moved);

  const next = records.map(stripResponseFields);
  scopedIndexes.forEach((recordIndex, position) => {
    next[recordIndex] = reordered[position];
  });
  return next;
};

export const providerRecordWithDisabledState = (
  section: ProviderSection,
  record: Record<string, unknown>,
  disabled: boolean,
) => {
  const nextRecord = stripResponseFields(record);
  // Only OpenAI-compatible providers have a native disabled field. Codex, Claude,
  // Gemini, and the other key types reject it during YAML unmarshal.
  if (definitionFor(section).openAi) {
    nextRecord.disabled = disabled;
    return nextRecord;
  }
  delete nextRecord.disabled;

  const excludedModels = Array.isArray(nextRecord['excluded-models'])
    ? nextRecord['excluded-models'].map(String).filter((model) => model.trim() !== '*')
    : [];
  if (disabled) excludedModels.push('*');
  if (excludedModels.length > 0) nextRecord['excluded-models'] = excludedModels;
  else delete nextRecord['excluded-models'];
  return nextRecord;
};

export function ApiAccessPage() {
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const { t } = useI18n();
  const [records, setRecords] = useState(emptyRecords);
  const [activeCategory, setActiveCategory] = useState<ProviderCategory>(readSelectedCategory);
  const [filter, setFilter] = useState('');
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const feedback = useAppNotice();
  const { showNotice: setNotice } = feedback;
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingRow, setEditingRow] = useState<ProviderRow | null>(null);
  const [dialogDraft, setDialogDraft] = useState<ProviderDraft>(emptyProviderDraft);
  const [apiAccessRemarks, setApiAccessRemarks] = useState<Record<string, string>>({});
  const [healthDialogRow, setHealthDialogRow] = useState<ProviderRow | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const activeDefinition = definitionFor(activeCategory);
  const activeSection = activeDefinition.section;
  const dragSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const loadProviders = useCallback(async (showLoading = true) => {
    if (showLoading) setLoading(true);
    setError('');
    try {
      const responses = await Promise.allSettled(
        providerLoadDefinitions.map(async (definition) => ({
          section: definition.section,
          records: await providerGroupsApi.get(definition.section),
        })),
      );
      const failures: string[] = [];
      setRecords((current) => {
        const next = { ...current };
        responses.forEach((result, index) => {
          const definition = providerLoadDefinitions[index];
          if (result.status === 'fulfilled') {
            next[result.value.section] = result.value.records;
          } else {
            failures.push(`${providerLabel(definition)}: ${String(result.reason)}`);
          }
        });
        return next;
      });
      if (failures.length > 0) {
        setError(t('apiAccess.error.partialLoad', { errors: failures.join('; ') }));
      }
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      if (showLoading) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadProviders();
  }, [loadProviders]);

  useEffect(() => {
    const providerRows = (Object.entries(records) as [ProviderSection, Record<string, unknown>[]][])
      .flatMap(([section, items]) => providerRowsFromGroups(section, items));
    if (providerRows.length === 0) {
      setApiAccessRemarks({});
      return;
    }
    let disposed = false;
    void invoke<string[]>('resolve_api_access_remarks', {
      queries: providerRows.flatMap((row) => [{
        providerSection: row.section,
        ...apiAccessRemarkLocatorFromRow(row),
      }, {
        providerSection: row.section,
        ...apiAccessRemarkLocatorFromRecord(row.section, row.source?.group ?? row.record),
      }]),
    }).then((remarks) => {
      if (disposed) return;
      setApiAccessRemarks(Object.fromEntries(providerRows.map((row, index) => [
        providerRemarkIdentity(row.section, apiAccessRemarkLocatorFromRow(row)),
        remarks[index * 2] || remarks[index * 2 + 1] || '',
      ])));
    }).catch(() => {
      if (!disposed) setApiAccessRemarks({});
    });
    return () => {
      disposed = true;
    };
  }, [records]);

  const rows = useMemo(
    () =>
      providerRowsFromGroups(activeSection, records[activeSection])
        .map((row) => ({
          ...row,
          remark: apiAccessRemarks[
            providerRemarkIdentity(row.section, apiAccessRemarkLocatorFromRow(row))
          ] ?? '',
        }))
        .filter((row) => row.category === activeCategory)
        .filter((row) => {
          const query = filter.trim().toLowerCase();
          if (!query) return true;
          return [row.remark, row.name, ...row.apiKeys, row.baseUrl, row.models.map(modelSearchText).join(' ')]
            .join(' ')
            .toLowerCase()
            .includes(query);
        }),
    [activeCategory, activeSection, apiAccessRemarks, filter, records, t],
  );

  const openCreate = () => {
    feedback.clearNotice();
    setError('');
    setEditingRow(null);
    setDialogDraft({ ...createProviderDraft(activeCategory), ...(activeDefinition.openAi ? { groupKeys: [providerKeyDraft({ 'api-key': '' })] } : {}) });
    setDialogOpen(true);
  };

  const openEdit = (row: ProviderRow) => {
    feedback.clearNotice();
    setError('');
    setEditingRow(row);
    const draft = draftFromRow(row);
    setDialogDraft(draft);
    setDialogOpen(true);
  };

  const persistEntryRemarks = async (
    section: ProviderSection, previous: Record<string, unknown>[], next: Record<string, unknown>[],
    edited?: { row: ProviderRow; remark: string },
  ) => {
    const previousRows = providerRowsFromGroups(section, previous);
    const previousLocators = previousRows.flatMap((row) => [
      apiAccessRemarkLocatorFromRow(row), apiAccessRemarkLocatorFromRecord(section, row.source!.group),
    ]);
    const resolved = await invoke<string[]>('resolve_api_access_remarks', {
      queries: previousLocators.map((locator) => ({ providerSection: section, ...locator })),
    });
    const previousRemarks = new Map(previousRows.map((row, index) => [
      providerRemarkIdentity(section, apiAccessRemarkLocatorFromRow(row)), resolved[index * 2] || resolved[index * 2 + 1] || '',
    ]));
    const nextRows = providerRowsFromGroups(section, next);
    const allRecords = nextRows.map(apiAccessRemarkLocatorFromRow);
    const byRemark = new Map<string, ApiAccessRemarkLocator[]>();
    for (const row of nextRows) {
      const locator = apiAccessRemarkLocatorFromRow(row);
      const identity = providerRemarkIdentity(section, locator);
      const isEdited = edited && row.source?.groupIndex === edited.row.source?.groupIndex && row.source?.keyIndex === edited.row.source?.keyIndex;
      const remark = isEdited ? edited.remark : previousRemarks.get(identity) ?? '';
      byRemark.set(remark, [...(byRemark.get(remark) ?? []), locator]);
    }
    if (!byRemark.size) byRemark.set('', []);
    let first = true;
    for (const [remark, updatedRecords] of byRemark) {
      await invoke('save_api_access_remark', { update: {
        providerSection: section, previousRecords: first ? previousLocators : [],
        records: updatedRecords, allRecords, remark,
      } });
      first = false;
    }
  };

  const saveProvider = async (
    nextDraft: ProviderDraft,
  ): Promise<ProviderSaveResult> => {
    const definition = activeDefinition;
    const preparedDraft = applyProviderRemarkIdentity(
      activeCategory,
      applyProviderPreset(activeCategory, nextDraft),
    );
    const preparedDraftForSave = {
      ...preparedDraft,
      models: preparedDraft.models.filter((model) => model.name.trim()),
    };
    const baseUrlRequired = definition.openAi || definition.section === 'codex-api-key';
    const parsedApiKeys = definition.openAi
      ? (preparedDraft.groupKeys ?? []).map(({ value }) => readString(value, 'api-key').trim())
      : parseProviderApiKeys(preparedDraft.apiKey);
    if (definition.openAi && (!preparedDraft.name.trim() || preparedDraft.name.trim().length > 80 || /[\u0000-\u001f\u007f]/.test(preparedDraft.name))) {
      return { saved: false, target: 'form', error: t('apiAccess.entries.nameRequired') };
    }
    const invalidKey = validateProviderGroupKeys(preparedDraft.groupKeys ?? []);
    if (invalidKey) return { saved: false, target: 'form', error: t('apiAccess.groups.invalidKey', { field: invalidKey }) };
    if (
      (!definition.openAi && parsedApiKeys.length !== 1)
      || (baseUrlRequired && !preparedDraft.baseUrl.trim())
    ) {
      return {
        saved: false,
        target: 'form',
        error: definition.openAi
          ? t('apiAccess.error.requiredAll')
          : baseUrlRequired
            ? t('apiAccess.error.requiredBaseKey')
            : t('apiAccess.error.requiredKey'),
      };
    }
    if (
      activeCategory === 'deepseek'
      && preparedDraftForSave.models.length === 0
    ) {
      return {
        saved: false,
        target: 'models',
        error: t('apiAccess.error.modelsRequired'),
      };
    }
    if (Array.from(preparedDraft.remark.trim()).length > 80 || /[\u0000-\u001f\u007f]/.test(preparedDraft.remark)) {
      return { saved: false, target: 'form', error: t('apiAccess.error.remarkInvalid') };
    }
    let baseUrl = preparedDraft.baseUrl.trim();
    let proxyUrl = preparedDraft.proxyUrl ?? '';
    let providerHeaders: Record<string, string> = {};
    try {
      if (baseUrl) baseUrl = normalizeBaseUrl(baseUrl);
      if (preparedDraft.proxyUrlEdited !== false) {
        try {
          proxyUrl = normalizeProviderProxyUrl(proxyUrl);
        } catch {
          throw new Error(t('apiAccess.error.proxyUrlInvalid'));
        }
      }
      if (baseUrlRequired && !baseUrl) throw new Error(t('apiAccess.error.baseRequired', { provider: providerLabel(definition) }));
      providerHeaders = parseProviderHeaders(preparedDraft.headersText ?? '');
    } catch (requestError) {
      return { saved: false, target: 'form', error: requestErrorMessage(requestError) };
    }
    setBusy(true);
    setError('');
    try {
      let draftToSave: ProviderDraft = { ...preparedDraftForSave, baseUrl, proxyUrl };
      if (
        definition.openAi
        && activeCategory !== 'deepseek'
        && !editingRow && draftToSave.models.length === 0
      ) {
        let fetchedModels: ModelOption[];
        try {
          fetchedModels = await fetchModels(
            'openai',
            baseUrl,
            parsedApiKeys[0],
            undefined,
            preparedDraft.groupKeys?.[0]
              ? providerHeadersFromRecord(effectiveProviderKey({ headers: providerHeaders }, serializeProviderKey(preparedDraft.groupKeys[0])))
              : providerHeaders,
            undefined,
            readString(providerDraftConnection(draftToSave, preparedDraft.groupKeys?.[0]), 'proxy-url'),
          );
          if (fetchedModels.length === 0) {
            throw new Error(t('apiAccess.error.noModels'));
          }
        } catch (requestError) {
          return {
            saved: false,
            target: 'models',
            error: requestErrorMessage(requestError),
          };
        }
        draftToSave = applyProviderPreset(activeCategory, {
          ...draftToSave,
          models: fetchedModels,
        });
      }
      const current = await providerGroupsApi.get(activeSection);
      let nextList: Record<string, unknown>[];
      const location = editingRow?.source ? locateProviderEntry(current, editingRow.source) : undefined;
      const currentRecord = editingRow?.record;
      const savedRecord = definition.openAi
        ? buildProviderGroupRecord(activeSection, draftToSave, currentRecord)
        : buildProviderKeyRecord(activeSection, draftToSave, currentRecord);
      const currentEntries = providerRowsFromGroups(activeSection, current);
      const targetIndex = location ? currentEntries.findIndex((row) => {
        return row.source?.groupIndex === location.groupIndex && row.source?.keyIndex === location.keyIndex;
      }) : -1;
      if (hasDuplicateProviderRecord(activeSection, currentEntries.map((row) => row.record), [savedRecord], targetIndex)) {
        throw new Error(t('apiAccess.error.duplicate'));
      }
      nextList = editingRow?.source
        ? updateProviderEntry(current, editingRow.source, editingRow.record, savedRecord)
        : appendProviderEntry(current, activeCategory === 'deepseek' ? 'deepseek' : activeSection, savedRecord);

      await providerGroupsApi.put(activeSection, nextList.map(stripResponseFields));
      const detached = location && nextList.length > current.length;
      const savedGroupIndex = location ? location.groupIndex + (detached ? 1 : 0) : nextList.length - 1;
      const savedKeyIndex = definition.openAi ? undefined : location && !detached ? location.keyIndex : 0;
      const savedRow = providerRowsFromGroups(activeSection, nextList).find((row) => row.source?.groupIndex === savedGroupIndex && row.source?.keyIndex === savedKeyIndex)!;
      await persistEntryRemarks(activeSection, current, nextList, { row: savedRow, remark: draftToSave.remark });
      setNotice(editingRow ? t('apiAccess.notice.updated') : t('apiAccess.notice.added'));
      await loadProviders();
      return { saved: true };
    } catch (requestError) {
      return { saved: false, target: 'form', error: requestErrorMessage(requestError) };
    } finally {
      setBusy(false);
    }
  };

  const deleteRow = async (row: ProviderRow) => {
    if (!await askConfirmation({ title: t('common.delete'), message: t('apiAccess.deleteConfirm', { remark: row.remark || row.name }), confirmText: t('common.delete'), variant: 'danger' })) return;
    feedback.clearNotice();
    setBusy(true);
    setError('');
    try {
      const current = await providerGroupsApi.get(row.section);
      if (!row.source) throw new Error(t('apiAccess.error.stale'));
      const remainingRecords = removeProviderEntry(current, row.source);
      await providerGroupsApi.put(row.section, remainingRecords);
      await persistEntryRemarks(row.section, current, remainingRecords);
      setNotice({ key: 'apiAccess.notice.deleted' });
      await loadProviders();
    } catch (requestError) {
      setNotice(requestErrorMessage(requestError), 'error');
    } finally {
      setBusy(false);
    }
  };

  const toggleProvider = async (row: ProviderRow) => {
    setBusy(true);
    setError('');
    setNotice('');
    try {
      const latestRows = await providerGroupsApi.get(row.section);
      if (!row.source) throw new Error(t('apiAccess.error.stale'));
      const nextRecord = providerRecordWithDisabledState(row.section, row.record, !row.disabled);
      const nextRows = updateProviderEntry(latestRows, row.source, row.record, nextRecord);
      await providerGroupsApi.put(row.section, nextRows);
      await persistEntryRemarks(row.section, latestRows, nextRows);
      await loadProviders(false);
    } catch (requestError) {
      setNotice(requestErrorMessage(requestError), 'error');
    } finally {
      setBusy(false);
    }
  };

  const reorderProviders = async (source: ProviderRow, target: ProviderRow) => {
    if (source.section !== target.section || source.index === target.index) return;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      const latestRows = await providerGroupsApi.get(source.section);
      if (!source.source || !target.source) throw new Error(t('apiAccess.error.stale'));
      const nextRows = reorderProviderEntries(latestRows, source.section, source.source, target.source, rows.map((row) => row.source!));
      await providerGroupsApi.put(source.section, nextRows);
      await persistEntryRemarks(source.section, latestRows, nextRows);
      await loadProviders(false);
    } catch (requestError) {
      await loadProviders(false);
      setNotice(requestErrorMessage(requestError), 'error');
    } finally {
      setDragOverId(null);
      setBusy(false);
    }
  };

  const handleDragEnd = (event: DragEndEvent) => {
    setDragOverId(null);
    const source = rows.find((row) => providerDragId(row) === String(event.active.id));
    const target = rows.find((row) => providerDragId(row) === String(event.over?.id ?? ''));
    if (!source || !target || source.index === target.index) return;

    void reorderProviders(source, target);
  };

  const countForDefinition = (definition: ProviderDefinition) =>
    providerRowsFromGroups(definition.section, records[definition.section]).filter((row) => row.category === definition.id).length;

  return (
    <section className="page management-page api-access-page">
      {confirmationDialog}
      {error ? <MessageNotice message={error} onDismiss={() => setError('')} /> : null}
      <div className="provider-workbench real-provider-workbench">
        <aside className="panel provider-category-panel">
          {providerDefinitions.map((definition) => (
            <button
              type="button"
              key={definition.id}
              className={definition.id === activeCategory ? 'active' : ''}
              aria-pressed={definition.id === activeCategory}
              onClick={() => {
                setActiveCategory(definition.id);
                writeSelectedCategory(definition.id);
                feedback.clearNotice();
              }}
              disabled={busy}
            >
              <img src={definition.icon} alt="" className="provider-logo" />
              <span title={providerLabel(definition)}>{providerLabel(definition)}</span>
              <strong>{countForDefinition(definition)}</strong>
            </button>
          ))}
        </aside>

        <section className="panel provider-resource-panel">
          <div className="management-panel-heading">
            <div>
              <h2 title={providerLabel(activeDefinition)}>{providerLabel(activeDefinition)}</h2>
              <span>{t(activeDefinition.openAi ? 'apiAccess.entries.summaryProviders' : 'apiAccess.entries.summaryKeys', { count: rows.length })}</span>
            </div>
            <div className="management-toolbar compact-toolbar api-access-search">
              <Search size={16} aria-hidden="true" />
              <input value={filter} onChange={(event) => setFilter(event.currentTarget.value)} placeholder={t('apiAccess.search')} aria-label={t('apiAccess.search')} />
            </div>
            <div className="api-access-heading-tools">
              <button type="button" className="secondary-button compact-button" onClick={() => void loadProviders()} disabled={loading || busy}>
                <RefreshCw size={16} aria-hidden="true" />
                {t('common.refresh')}
              </button>
              <button type="button" className="primary-button compact-button" onClick={openCreate} disabled={loading || busy}>
                <Plus size={16} aria-hidden="true" />
                {t(activeDefinition.openAi ? 'apiAccess.entries.addProvider' : 'apiAccess.entries.addKey')}
              </button>
            </div>
          </div>

          {loading ? (
            <div className="management-loading"><LoaderCircle size={20} className="spin" />{t('apiAccess.loading')}</div>
          ) : rows.length === 0 ? (
            <div className="management-empty">
              <Filter size={24} aria-hidden="true" />
              <strong>{filter ? t('apiAccess.empty.filtered') : t('apiAccess.empty.none')}</strong>
              <span>{filter ? t('apiAccess.empty.tryKeyword') : t('apiAccess.empty.addFirst', {
                action: t(activeDefinition.openAi ? 'apiAccess.entries.addProvider' : 'apiAccess.entries.addKey'),
              })}</span>
            </div>
          ) : (
            <DndContext
              sensors={dragSensors}
              collisionDetection={closestCenter}
              onDragStart={() => setNotice('')}
              onDragOver={({ over }) => setDragOverId(over ? String(over.id) : null)}
              onDragCancel={() => setDragOverId(null)}
              onDragEnd={handleDragEnd}
            >
              <SortableContext
                items={rows.map(providerDragId)}
                strategy={verticalListSortingStrategy}
              >
                <div className="real-provider-list">
                  {rows.map((row) => (
                    <SortableProviderRow
                      key={providerDragId(row)}
                      row={row}
                      disabled={busy || rows.length < 2}
                      dragLabel={t('apiAccess.dragHandle', { remark: row.remark || row.name })}
                      isDragOver={dragOverId === providerDragId(row)}
                    >
                  <div className="provider-row-main">
                    <div className="provider-row-title">
                      <strong title={row.remark || row.name}>{row.remark || row.name}</strong>
                      {Array.isArray(row.record.keys) && providerGroupStatus(row.record) === 'partial'
                        ? <span className="state-pill">{t('apiAccess.groups.partial')}</span>
                        : null}
                    </div>
                    <span className="provider-row-url" title={row.baseUrl || undefined}>{row.baseUrl || t('apiAccess.defaultUrl')}</span>
                    <div className="provider-row-meta">
                      <code title={t('apiAccess.keys.count', { count: row.apiKeys.length })}>
                        {row.apiKeys.length > 1
                          ? t('apiAccess.keys.summary', { key: maskSecret(row.apiKey), count: row.apiKeys.length })
                          : maskSecret(row.apiKey)}
                      </code>
                      {row.models.length > 0 ? <span className="provider-row-models">{t('apiAccess.models.summary', { count: row.models.length })}</span> : null}
                      {row.priority === null ? null : <span>{t('apiAccess.priorityValue', { priority: row.priority })}</span>}
                    </div>
                  </div>
                  <div className="provider-row-actions">
                    <button
                      type="button"
                      className="secondary-button provider-health-button"
                      onClick={() => setHealthDialogRow(row)}
                      disabled={busy}
                    >
                      {t('apiAccess.health.action')}
                    </button>
                    <label className="provider-enabled-control">
                      <span>{t(row.disabled ? 'apiAccess.status.disabled' : 'apiAccess.status.enabled')}</span>
                      <span className="switch-control">
                        <input
                          type="checkbox"
                          checked={!row.disabled}
                          onChange={() => void toggleProvider(row)}
                          disabled={busy}
                          aria-label={t('apiAccess.toggleAria', { remark: row.remark || row.name, action: row.disabled ? t('common.enable') : t('common.disable') })}
                        />
                        <span className="switch-track" />
                      </span>
                    </label>
                    <button type="button" className="icon-button quiet" onClick={() => openEdit(row)} disabled={busy} title={t('common.edit')} aria-label={t('common.edit')}>
                      <Edit3 size={16} aria-hidden="true" />
                    </button>
                    <button type="button" className="icon-button danger" onClick={() => void deleteRow(row)} disabled={busy} title={t('common.delete')} aria-label={t('common.delete')}>
                      <Trash2 size={16} aria-hidden="true" />
                    </button>
                  </div>
                    </SortableProviderRow>
                  ))}
                </div>
              </SortableContext>
            </DndContext>
          )}
        </section>
      </div>

      {dialogOpen ? (
        <ApiProviderDialog
          activeCategory={activeCategory}
          editingRow={editingRow}
          initialDraft={dialogDraft}
          busy={busy}
          onClose={() => setDialogOpen(false)}
          onSave={saveProvider}
        />
      ) : null}
      {healthDialogRow ? (
        <ProviderGroupHealthDialog
          key={providerHealthIdentity(healthDialogRow)}
          row={healthDialogRow}
          onClose={() => setHealthDialogRow(null)}
        />
      ) : null}
      <FloatingNotice key={feedback.revision} notice={feedback.notice} onDismiss={feedback.clearNotice} />
    </section>
  );
}

type ProviderHealthDialogProps = {
  row: ProviderRow;
  onClose: () => void;
  keySelector?: ReactNode;
};

function ProviderGroupHealthDialog({ row, onClose }: ProviderHealthDialogProps) {
  const { t } = useI18n();
  const keys = providerGroupKeys(row.record);
  const [selected, setSelected] = useState(0);
  const key = keys[selected];
  const effectiveRow = useMemo(() => {
    if (!key) return row;
    const record = effectiveProviderKey(row.record, key);
    return { ...row, record, apiKey: readString(key, 'api-key'), apiKeys: [readString(key, 'api-key')],
      authIndex: readString(key, 'auth-index', 'authIndex'), models: modelsFromRecord(record.models) };
  }, [row, key]);
  return <ProviderHealthDialog key={selected} row={effectiveRow} onClose={onClose} keySelector={keys.length ?
    <label className="provider-health-key-selector"><span>{t('apiAccess.groups.healthKey')}</span>
      <select value={selected} onChange={(event) => setSelected(Number(event.currentTarget.value))}>
        {keys.map((entry, index) => <option key={index} value={index}>{t('apiAccess.groups.keyNumber', { number: index + 1 })} · {maskSecret(readString(entry, 'api-key'))}</option>)}
      </select></label> : undefined} />;
}

type ProviderModelHealthState = { status: 'checking' } | ProviderModelHealthResult;

function ProviderHealthDialog({ row, onClose, keySelector }: ProviderHealthDialogProps) {
  const { t } = useI18n();
  const configuredModels = useMemo(
    () => mergeProviderHealthModels([], row.models),
    [row.models],
  );
  const [models, setModels] = useState<ModelOption[]>(configuredModels);
  const [modelLoading, setModelLoading] = useState(true);
  const [modelError, setModelError] = useState('');
  const [search, setSearch] = useState('');
  const [results, setResults] = useState<Record<string, ProviderModelHealthState>>({});
  const [checkingAll, setCheckingAll] = useState(false);
  const healthControllerRef = useRef<AbortController | null>(null);
  const dialogRef = useDialogFocusTrap<HTMLElement>({ onEscape: onClose });

  useEffect(() => {
    const controller = new AbortController();
    healthControllerRef.current = controller;
    return () => controller.abort();
  }, []);

  const healthOptions = useMemo<ProviderHealthCheckOptions>(() => ({
    provider: row.category === 'deepseek' ? 'deepseek' : providerModelType(row.section, row.record),
    baseUrl: row.baseUrl,
    apiKeys: row.apiKeys,
    authIndex: row.authIndex,
    customHeaders: providerHeadersFromRecord(row.record),
    proxyUrl: readString(row.record, 'proxy-url'),
    timeoutMs: PROVIDER_HEALTH_TIMEOUT_MS,
  }), [row]);

  useEffect(() => {
    let disposed = false;
    setModelLoading(true);
    setModelError('');
    void fetchModels(
      healthOptions.provider,
      healthOptions.baseUrl,
      row.apiKeys.find((key) => key.trim()) ?? '',
      healthOptions.authIndex,
      healthOptions.customHeaders,
      healthOptions.timeoutMs,
      healthOptions.proxyUrl,
    ).then((discovered) => {
      if (!disposed) setModels(mergeProviderHealthModels(discovered, row.models));
    }).catch((requestError) => {
      if (!disposed) setModelError(String(requestError).replace(/^Error:\s*/i, ''));
    }).finally(() => {
      if (!disposed) setModelLoading(false);
    });
    return () => {
      disposed = true;
    };
  }, [healthOptions, row.apiKeys, row.models]);

  const visibleModels = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return models;
    return models.filter((model) => modelSearchText(model).includes(query));
  }, [models, search]);

  const resultValues = Object.values(results);
  const checkedCount = resultValues.filter((result) => result.status !== 'checking').length;
  const healthyCount = resultValues.filter((result) => result.status === 'healthy').length;
  const failedCount = resultValues.filter((result) => result.status === 'failed').length;
  const hasCheckingModel = resultValues.some((result) => result.status === 'checking');

  const saveResult = (result: ProviderModelHealthResult) => {
    setResults((current) => ({
      ...current,
      [result.model.toLowerCase()]: result,
    }));
  };

  const checkOneModel = async (model: ModelOption) => {
    const signal = healthControllerRef.current?.signal;
    if (!signal || signal.aborted) return;
    const key = model.name.toLowerCase();
    setResults((current) => ({ ...current, [key]: { status: 'checking' } }));
    const result = await checkProviderModelHealth(healthOptions, model.name);
    if (!signal.aborted) saveResult(result);
  };

  const checkAllModels = async () => {
    const signal = healthControllerRef.current?.signal;
    if (models.length === 0 || checkingAll || !signal || signal.aborted) return;
    setCheckingAll(true);
    setResults(Object.fromEntries(models.map((model) => [
      model.name.toLowerCase(),
      { status: 'checking' } satisfies ProviderModelHealthState,
    ])));
    try {
      await checkProviderModelsHealth(healthOptions, models, saveResult, undefined, signal);
    } finally {
      if (!signal.aborted) setCheckingAll(false);
    }
  };

  const statusLabel = (state: ProviderModelHealthState | undefined) => {
    if (!state) return t('apiAccess.health.notChecked');
    if (state.status === 'checking') return t('apiAccess.health.checking');
    return state.status === 'healthy'
      ? t('apiAccess.health.healthy')
      : t('apiAccess.health.failed');
  };

  return (
    <div className="model-discovery-backdrop" onMouseDown={(event) => event.currentTarget === event.target && onClose()}>
      <section ref={dialogRef} className="model-discovery-dialog provider-health-dialog" role="dialog" aria-modal="true" aria-labelledby="provider-health-title">
        <div className="model-discovery-header">
          <div>
            <h2 id="provider-health-title">{t('apiAccess.health.title')}</h2>
            <span>{t('apiAccess.health.description', { provider: row.remark || row.name })}</span>
          </div>
          <button type="button" className="icon-button quiet" onClick={onClose} title={t('common.close')} aria-label={t('common.close')}>
            <X size={18} aria-hidden="true" />
          </button>
        </div>

        {keySelector}
        <div className="model-discovery-search">
          <Search size={16} aria-hidden="true" />
          <input value={search} onChange={(event) => setSearch(event.currentTarget.value)} placeholder={t('apiAccess.health.search')} />
        </div>

        <div className="provider-health-overview">
          <p>{t('apiAccess.health.warning')}</p>
          <span>{t('apiAccess.health.summary', {
            checked: checkedCount,
            total: models.length,
            healthy: healthyCount,
            failed: failedCount,
          })}</span>
          {modelError ? (
            <MessageNotice message={t('apiAccess.health.modelLoadFailed', { error: modelError })} onDismiss={() => setModelError('')} />
          ) : modelLoading ? (
            <small>{t('apiAccess.health.loadingModels')}</small>
          ) : null}
        </div>

        <div className="model-discovery-list provider-health-model-list">
          {modelLoading && models.length === 0 ? (
            <div className="model-discovery-message"><LoaderCircle size={20} className="spin" /><strong>{t('apiAccess.health.loadingModels')}</strong></div>
          ) : visibleModels.length === 0 ? (
            <div className="model-discovery-message"><strong>{models.length ? t('apiAccess.health.noMatch') : t('apiAccess.health.noModel')}</strong></div>
          ) : visibleModels.map((model) => {
            const state = results[model.name.toLowerCase()];
            const checked = state && state.status !== 'checking' ? state : null;
            const error = checked?.status === 'failed'
              ? checked.errorCode === 'missing-direct-key'
                ? t('apiAccess.health.missingDirectKey')
                : checked.timedOut
                  ? t('apiAccess.health.timeout')
                  : checked.error || t('apiAccess.health.failed')
              : '';
            const latencyTitle = checked?.firstTokenLatencyMs !== undefined
              ? t('apiAccess.health.firstTokenLatencyResult', { latency: checked.firstTokenLatencyMs })
              : checked?.responseLatencyMs !== undefined
                ? t('apiAccess.health.responseLatencyResult', { latency: checked.responseLatencyMs })
                : error || undefined;
            const latencyInline = checked?.firstTokenLatencyMs !== undefined
              ? t('apiAccess.health.firstTokenInline', { latency: checked.firstTokenLatencyMs })
              : checked?.responseLatencyMs !== undefined
                ? t('apiAccess.health.responseInline', { latency: checked.responseLatencyMs })
                : '';
            return (
              <div className={`provider-health-model-row ${checked?.status === 'failed' ? 'failed' : ''}`} key={model.name}>
                <div className="provider-health-model-name">
                  <strong title={model.name}>{model.name}</strong>
                  {error
                    ? <small className="error" title={error}>{error}</small>
                    : model.alias || model.displayName
                      ? <small title={model.alias || model.displayName}>{model.alias || model.displayName}</small>
                      : null}
                </div>
                <span
                  className={`state-pill provider-health-pill ${state?.status === 'checking' ? 'checking' : checked?.status === 'healthy' ? 'success' : checked?.status === 'failed' ? 'error' : ''}`}
                  title={latencyTitle}
                >
                  {statusLabel(state)}
                  {latencyInline ? ` · ${latencyInline}` : ''}
                </span>
                <button
                  type="button"
                  className="secondary-button compact-button provider-health-single-button"
                  onClick={() => void checkOneModel(model)}
                  disabled={modelLoading || checkingAll || state?.status === 'checking'}
                >
                  {checked ? t('apiAccess.health.retryOne') : t('apiAccess.health.checkOne')}
                </button>
              </div>
            );
          })}
        </div>

        <div className="model-discovery-actions provider-health-actions">
          <button type="button" className="secondary-button compact-button" onClick={onClose}>{t('common.close')}</button>
          <button
            type="button"
            className="primary-button compact-button"
            onClick={() => void checkAllModels()}
            disabled={modelLoading || models.length === 0 || hasCheckingModel}
          >
            {checkingAll
              ? t('apiAccess.health.checkingProgress', { checked: checkedCount, total: models.length })
              : t('apiAccess.health.checkAll')}
          </button>
        </div>
      </section>
    </div>
  );
}

type ApiProviderDialogProps = {
  activeCategory: ProviderCategory;
  editingRow: ProviderRow | null;
  initialDraft: ProviderDraft;
  busy: boolean;
  onClose: () => void;
  onSave: (draft: ProviderDraft) => Promise<ProviderSaveResult>;
};

export function ApiProviderDialog({
  activeCategory,
  editingRow,
  initialDraft,
  busy,
  onClose,
  onSave,
}: ApiProviderDialogProps) {
  const { t } = useI18n();
  const definition = definitionFor(activeCategory);
  const activeSection = definition.section;
  const [draft, setDraft] = useState<ProviderDraft>(initialDraft);
  const grouped = Boolean(draft.groupKeys);
  const [keyVisible, setKeyVisible] = useState(false);
  const [discoveryKeyId, setDiscoveryKeyId] = useState(initialDraft.groupKeys?.[0]?.id ?? '');
  const [modelLoading, setModelLoading] = useState(false);
  const [modelError, setModelError] = useState('');
  const [formError, setFormError] = useState('');
  const [discoveredModels, setDiscoveredModels] = useState<ModelOption[]>([]);
  const [modelDiscoveryOpen, setModelDiscoveryOpen] = useState(false);
  const [thinkingLevelInput, setThinkingLevelInput] = useState('');
  const [selectedModelNames, setSelectedModelNames] = useState<Set<string>>(
    () => new Set(mergeModelOptions(initialDraft.models).map((model) => model.name.toLowerCase())),
  );
  const modelCardRef = useRef<HTMLDivElement>(null);
  const discoverySelectionInitializedRef = useRef(false);
  const discoveryRequestRef = useRef(0);

  const closeModelDiscovery = useCallback(() => {
    discoveryRequestRef.current += 1;
    setModelLoading(false);
    setModelDiscoveryOpen(false);
  }, []);
  const providerDialogRef = useDialogFocusTrap<HTMLFormElement>({
    onEscape: busy ? undefined : onClose,
    preventEscape: busy,
  });
  const modelDialogRef = useDialogFocusTrap<HTMLElement>({
    active: modelDiscoveryOpen,
    onEscape: closeModelDiscovery,
  });

  useEffect(() => () => { discoveryRequestRef.current += 1; }, []);

  const modelOptions = useMemo(
    () => mergeModelOptions(discoveredModels, draft.models),
    [discoveredModels, draft.models],
  );

  const configuredModels = useMemo(
    () => draft.models.filter((model) => model.name.trim()),
    [draft.models],
  );

  const selectedModels = useMemo(() => modelOptions.filter((model) =>
    selectedModelNames.has(model.name.toLowerCase()),
  ), [modelOptions, selectedModelNames]);
  const unselectedModels = useMemo(() => modelOptions.filter((model) =>
    !selectedModelNames.has(model.name.toLowerCase()),
  ), [modelOptions, selectedModelNames]);

  const updateTextField = (
    field: 'name' | 'apiKey' | 'remark' | 'baseUrl' | 'proxyUrl' | 'priority' | 'prefix' | 'headersText' | 'excludedModelsText' | 'cloakMode' | 'cloakSensitiveWordsText',
    value: string,
  ) => {
    setFormError('');
    if (field === 'apiKey' || field === 'baseUrl' || field === 'headersText') {
      discoveryRequestRef.current += 1;
      setModelLoading(false);
      setModelError('');
    }
    setDraft((current) => ({
      ...current,
      [field]: value,
      ...(field === 'proxyUrl' ? { proxyUrlEdited: true, proxyUrlMixed: false } : {}),
      ...(field === 'excludedModelsText' ? { modelSelectionCatalog: undefined } : {}),
    }));
  };

  const updateBooleanField = (
    field: 'websockets' | 'cloakStrictMode',
    value: boolean,
  ) => {
    setFormError('');
    setDraft((current) => ({ ...current, [field]: value }));
  };

  const updateOptionalBooleanField = (field: 'disableCooling' | 'cloakCacheUserId', value: string) => {
    setFormError('');
    setDraft((current) => ({ ...current, [field]: value === '' ? null : value === 'true' }));
  };

  const updateModels = (update: (models: ModelOption[]) => ModelOption[]) => {
    setDraft((current) => {
      const models = update(current.models);
      return {
        ...current,
        models,
        excludedModelsText: current.modelSelectionCatalog && models.some((model) => model.name.trim())
          ? exclusionsForModelSelection(current.excludedModelsText ?? '', current.modelSelectionCatalog, models)
          : current.excludedModelsText,
      };
    });
  };

  const updateModel = (index: number, patch: Partial<ModelOption>) => {
    updateModels((models) => models.map((model, modelIndex) =>
      modelIndex === index ? { ...model, ...patch } : model,
    ));
  };

  const addModel = () => {
    updateModels((models) => [...models, { name: '', alias: '' }]);
  };

  const removeModel = (index: number) => {
    updateModels((models) => models.filter((_, modelIndex) => modelIndex !== index));
  };

  const addThinkingLevel = () => {
    const level = thinkingLevelInput.trim().toLowerCase();
    if (!level) return;
    setDraft((current) => {
      const levels = current.thinkingLevels ?? [];
      if (levels.some((item) => item.toLowerCase() === level)) return current;
      return {
        ...current,
        thinkingLevels: [...levels, level],
        thinkingLevelsEdited: true,
      };
    });
    setThinkingLevelInput('');
  };

  const removeThinkingLevel = (level: string) => {
    setDraft((current) => ({
      ...current,
      thinkingLevels: (current.thinkingLevels ?? []).filter((item) => item !== level),
      thinkingLevelsEdited: true,
    }));
  };

  const discoverModels = async () => {
    const baseUrlRequired =
      activeSection === 'codex-api-key' || activeSection === 'openai-compatibility';
    if (baseUrlRequired && !draft.baseUrl.trim()) {
      setModelError(t('apiAccess.error.enterBaseUrl'));
      return;
    }
    const requestId = ++discoveryRequestRef.current;
    setModelLoading(true);
    setModelError('');
    try {
      const provider: ModelProvider = activeCategory === 'deepseek' ? 'deepseek' : providerModelType(definition.section);
      const selectedKey = draft.groupKeys?.find((key) => key.id === discoveryKeyId) ?? draft.groupKeys?.[0];
      const connection = selectedKey ? providerDraftConnection(draft, selectedKey) : null;
      const modelApiKey = connection ? readString(connection, 'api-key') : draft.apiKey.split(/\r?\n/).map((value) => value.trim()).find(Boolean) ?? '';
      const fetchedModels = await fetchModels(
        provider,
        draft.baseUrl,
        modelApiKey,
        connection ? readString(selectedKey!.value, 'auth-index', 'authIndex') || undefined : editingRow?.authIndex,
        connection ? providerHeadersFromRecord(connection) : parseProviderHeaders(draft.headersText ?? ''),
        undefined,
        connection ? readString(connection, 'proxy-url') : draft.proxyUrl,
      );
      if (requestId !== discoveryRequestRef.current) return;
      const models = applyProviderPreset(
        activeCategory,
        { ...draft, models: fetchedModels },
      ).models;
      const initialized = discoverySelectionInitializedRef.current;
      setDiscoveredModels(models);
      setSelectedModelNames((current) =>
        initialized
          ? reconcileModelSelection(models, draft.models, current, 'refresh')
          : modelSelectionForDiscovery(draft.models, models, draft.excludedModelsText ?? ''));
      discoverySelectionInitializedRef.current = true;
      if (!models.length) setModelError(t('apiAccess.error.noAvailableModels'));
    } catch (requestError) {
      if (requestId === discoveryRequestRef.current) setModelError(requestErrorMessage(requestError));
    } finally {
      if (requestId === discoveryRequestRef.current) setModelLoading(false);
    }
  };

  const openModelDiscovery = () => {
    const baseUrlRequired =
      activeSection === 'codex-api-key' || activeSection === 'openai-compatibility';
    if (baseUrlRequired && !draft.baseUrl.trim()) {
      setModelError(t('apiAccess.error.baseBeforeModels'));
      return;
    }
    discoverySelectionInitializedRef.current = false;
    setSelectedModelNames(modelSelectionForDiscovery(draft.models, discoveredModels, draft.excludedModelsText ?? ''));
    setModelDiscoveryOpen(true);
    void discoverModels();
  };

  const moveModels = (models: ModelOption[], selected: boolean) => {
    discoverySelectionInitializedRef.current = true;
    setSelectedModelNames((current) => {
      const next = new Set(current);
      models.forEach((model) => {
        const key = model.name.toLowerCase();
        if (selected) next.add(key);
        else next.delete(key);
      });
      return next;
    });
  };

  const applyModelSelection = () => {
    if (modelLoading || selectedModels.length === 0) return;
    setDraft((current) => ({
      ...current,
      models: selectedModels.flatMap((selected) => {
        const existing = current.models.filter((model) => model.name.toLowerCase() === selected.name.toLowerCase());
        return existing.length ? existing : [selected];
      }),
      modelSelectionCatalog: activeSection === 'openai-compatibility' ? undefined : discoveredModels,
      excludedModelsText: activeSection === 'openai-compatibility'
        ? current.excludedModelsText
        : exclusionsForModelSelection(
            current.excludedModelsText ?? '',
            discoveredModels,
            selectedModels,
          ),
    }));
    closeModelDiscovery();
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setFormError('');
    const result = await onSave(draft);
    if (result.saved) {
      onClose();
      return;
    }
    if (result.target === 'models') {
      setModelError(result.error);
      window.requestAnimationFrame(() => {
        modelCardRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
      });
      return;
    }
    setFormError(result.error);
  };

  const hasModelExclusions = activeSection !== 'openai-compatibility'
    && Boolean(draft.excludedModelsText?.trim());
  const modelSummaryTitle = configuredModels.length > 0
    ? t('apiAccess.models.selected', { count: configuredModels.length })
    : activeCategory === 'deepseek'
      ? t('apiAccess.models.selectionRequired')
    : hasModelExclusions
      ? t('apiAccess.models.restricted')
      : activeSection === 'openai-compatibility'
        ? t('apiAccess.models.autoAll')
        : t('apiAccess.models.upstreamDefault');
  const modelSummaryDetail = configuredModels.length > 0
    ? configuredModels.slice(0, 3).map((model) => model.name).join('、')
    : activeCategory === 'deepseek'
      ? t('apiAccess.models.selectionRequiredHint')
    : hasModelExclusions
      ? t('apiAccess.models.hiddenHint')
      : activeSection === 'openai-compatibility'
        ? t('apiAccess.models.autoHint')
        : t('apiAccess.models.allHint');

  const content = (
    <>
      <div className="config-dialog-backdrop" onMouseDown={(event) => event.currentTarget === event.target && !busy && onClose()}>
      <form ref={providerDialogRef} className="config-dialog management-dialog api-provider-dialog" role="dialog" aria-modal="true" aria-labelledby="api-provider-dialog-title" onSubmit={(event) => void submit(event)}>
        <div className="config-dialog-heading">
          <div>
            <Plus size={19} aria-hidden="true" />
            <h2 id="api-provider-dialog-title">{t(definition.openAi
              ? editingRow ? 'apiAccess.entries.editProvider' : 'apiAccess.entries.addProvider'
              : editingRow ? 'apiAccess.entries.editKey' : 'apiAccess.entries.addKey')}</h2>
          </div>
          <button type="button" className="icon-button quiet" onClick={onClose} disabled={busy} title={t('common.close')} aria-label={t('common.close')}>
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <section className="provider-dialog-section">
        <h3>{t('apiAccess.entries.basic')} · {providerLabel(definition)}</h3>
        <div className="provider-dialog-basic-grid">
        {definition.openAi ? <label><span>{t('apiAccess.entries.name')}</span><input autoFocus value={draft.name} maxLength={80} onChange={(event) => updateTextField('name', event.currentTarget.value)} placeholder={t('apiAccess.entries.namePlaceholder')} /></label> : null}
        <label><span>{t('apiAccess.field.remark')}</span><input value={draft.remark} maxLength={80} onChange={(event) => updateTextField('remark', event.currentTarget.value)} placeholder={t('apiAccess.remarkPlaceholder')} /></label>
        <label><span>{t('apiAccess.field.baseUrl')}</span><input value={draft.baseUrl} onChange={(event) => updateTextField('baseUrl', event.currentTarget.value)} placeholder={activeSection === 'codex-api-key' || activeSection === 'openai-compatibility' ? t('apiAccess.baseRequiredPlaceholder') : t('apiAccess.baseOptionalPlaceholder')} /></label>
        </div>
        {!definition.openAi ? <div className="provider-dialog-secret">
          <label><span>{t('apiAccess.field.key')}</span><input autoFocus type={keyVisible ? 'text' : 'password'} autoComplete="off" spellCheck={false}
            value={draft.apiKey} onChange={(event) => updateTextField('apiKey', event.currentTarget.value)} placeholder="sk-..." /></label>
          <button type="button" className="secondary-button compact-button" aria-pressed={keyVisible} onClick={() => setKeyVisible((value) => !value)}>
            {t(keyVisible ? 'apiAccess.entries.hideKey' : 'apiAccess.entries.showKey')}
          </button>
        </div> : null}
        {draft.groupKeys ? <ProviderGroupKeysEditor keys={draft.groupKeys} disabled={busy} onChange={(groupKeys) => {
            discoveryRequestRef.current += 1; setModelLoading(false); setModelError(''); setFormError('');
            setDraft((current) => ({ ...current, groupKeys }));
          }} /> : null}
        </section>
        <div className="model-config-card" ref={modelCardRef}>
          <div className="model-config-heading">
            <div><span>{t('apiAccess.models.title')}</span><small>{t('apiAccess.models.description')}</small></div>
            <button type="button" className="secondary-button compact-button" onClick={openModelDiscovery} disabled={busy}>
              <RefreshCw size={15} />{t('apiAccess.models.fetch')}
            </button>
          </div>
          {(draft.groupKeys?.length ?? 0) > 1 ? <label><span>{t('apiAccess.entries.discoveryKey')}</span><select value={draft.groupKeys!.some((key) => key.id === discoveryKeyId) ? discoveryKeyId : draft.groupKeys![0].id}
            onChange={(event) => { discoveryRequestRef.current += 1; setModelLoading(false); setDiscoveryKeyId(event.currentTarget.value); }}>
            {draft.groupKeys!.map((key, index) => <option value={key.id} key={key.id}>{t('apiAccess.entries.keyNumber', { number: index + 1 })} · {maskSecret(readString(key.value, 'api-key'))}</option>)}
          </select></label> : null}
          <div className={`model-config-summary ${configuredModels.length || hasModelExclusions ? 'has-models' : ''}`}>
            <strong>{modelSummaryTitle}</strong>
            <span>{modelSummaryDetail}</span>
          </div>
          <div className="model-config-entries">
            {draft.models.map((model, index) => (
              <div className="provider-model-config" key={index}><div className="model-config-entry">
                <input
                  value={model.name}
                  onChange={(event) => updateModel(index, { name: event.currentTarget.value })}
                  placeholder={t('apiAccess.models.namePlaceholder')}
                  aria-label={t('apiAccess.models.namePlaceholder')}
                  disabled={busy}
                />
                <input
                  value={model.alias ?? ''}
                  onChange={(event) => updateModel(index, { alias: event.currentTarget.value })}
                  placeholder={t('apiAccess.models.aliasPlaceholder')}
                  aria-label={t('apiAccess.models.aliasPlaceholder')}
                  disabled={busy}
                />
                <button
                  type="button"
                  className="icon-button quiet danger"
                  onClick={() => removeModel(index)}
                  disabled={busy}
                  title={t('apiAccess.models.remove')}
                  aria-label={t('apiAccess.models.remove')}
                >
                  <Trash2 size={14} />
                </button>
              </div>
                <ProviderModelFields section={activeSection} value={model.config ?? { ...(model.thinking ? { thinking: model.thinking } : {}) }} onChange={(config) => updateModel(index, { config, thinking: isRecord(config.thinking) ? config.thinking : undefined })} />
              </div>
            ))}
            <button type="button" className="secondary-button compact-button model-config-add" onClick={addModel} disabled={busy}>
              <Plus size={14} />{t('apiAccess.models.add')}
            </button>
          </div>
          {modelError && !modelDiscoveryOpen ? <MessageNotice message={modelError} onDismiss={() => setModelError('')} /> : null}
        </div>
        <details className="provider-advanced-settings">
          <summary>{t('apiAccess.advanced')}</summary>
          <div className="provider-advanced-fields">
            <AdvancedSettingsGroup title={providerText('modelRules')}>
            <label><span>{t('apiAccess.field.prefix')}</span><input value={draft.prefix ?? ''} onChange={(event) => updateTextField('prefix', event.currentTarget.value)} placeholder={t('apiAccess.prefixPlaceholder')} /></label>
            {activeSection !== 'openai-compatibility' ? (
              <label className="multiline-field">
                <span>{t('apiAccess.field.excludedModels')}</span>
                <textarea value={draft.excludedModelsText ?? ''} onChange={(event) => updateTextField('excludedModelsText', event.currentTarget.value)} rows={3} placeholder={'model-old-*\nmodel-preview'} />
              </label>
            ) : null}
        {activeCategory === 'openai-compatibility' ? (
          <div className="thinking-level-config">
            <div className="thinking-level-heading">
              <strong>{t('apiAccess.thinking.title')}</strong>
              <span>{t('apiAccess.thinking.description')}</span>
            </div>
            <div className="thinking-level-entry">
              <input
                value={thinkingLevelInput}
                onChange={(event) => setThinkingLevelInput(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    addThinkingLevel();
                  }
                }}
                placeholder={t('apiAccess.thinking.placeholder')}
              />
              <button type="button" className="secondary-button compact-button" onClick={addThinkingLevel} disabled={!thinkingLevelInput.trim()}>
                <Plus size={14} />{t('apiAccess.thinking.add')}
              </button>
            </div>
            {(draft.thinkingLevels?.length ?? 0) > 0 ? (
              <div className="thinking-level-tags">
                {draft.thinkingLevels?.map((level) => (
                  <span key={level}>
                    {level}
                    <button type="button" onClick={() => removeThinkingLevel(level)} title={t('apiAccess.thinking.delete', { level })} aria-label={t('apiAccess.thinking.deleteAria', { level })}>
                      <X size={12} />
                    </button>
                  </span>
                ))}
              </div>
            ) : <small className="thinking-level-empty">{t('apiAccess.thinking.empty')}</small>}
            </div>
        ) : null}
            </AdvancedSettingsGroup>
            <AdvancedSettingsGroup title={providerText('connectionRouting')}>
            <label><span>{t('apiAccess.field.priority')}</span><input type="number" step="1" value={draft.priority} onChange={(event) => updateTextField('priority', event.currentTarget.value)} placeholder={t('common.optional')} /></label>
            {!definition.openAi ? <>
              <label><span>{t('apiAccess.field.proxyUrl')}</span><input value={draft.proxyUrl ?? ''} onChange={(event) => updateTextField('proxyUrl', event.currentTarget.value)} placeholder="socks5://127.0.0.1:1080" /></label>
              <label><span>{t('apiAccess.entries.weight')}</span><input type="number" step="1" max="1000000" value={draft.weight ?? ''} placeholder="1" onChange={(event) => { const weight = event.currentTarget.value; setDraft((current) => ({ ...current, weight })); }} /></label>
            </> : null}
            </AdvancedSettingsGroup>
            <AdvancedSettingsGroup title={providerText('requestBehavior')}>
            {!definition.openAi ? <ProviderKeyTemplateFields section={activeSection} value={draft.templateFields ?? {}} inherited={{}} onChange={(templateFields) => setDraft((current) => ({ ...current, templateFields }))} /> : null}
            <ProviderGroupTemplateFields section={activeSection} includeErrors={definition.openAi} value={draft.templateFields ?? {}} onChange={(templateFields) => setDraft((current) => ({ ...current, templateFields }))} />
            <label className="multiline-field">
              <span>{t('apiAccess.field.headers')}</span>
              <textarea value={draft.headersText ?? ''} onChange={(event) => updateTextField('headersText', event.currentTarget.value)} rows={3} placeholder={'X-Team: production\nAuthorization: Bearer ...'} />
              <small>{providerText('headersHint')}</small>
            </label>
            {!grouped && activeSection === 'claude-api-key' ? (
              <div className="provider-cloak-settings">
                <label>
                  <span>{t('apiAccess.cloak.mode')}</span>
                  <select value={draft.cloakMode ?? ''} onChange={(event) => updateTextField('cloakMode', event.currentTarget.value)}>
                    <option value="">{t('apiAccess.cloak.default')}</option>
                    <option value="auto">{t('apiAccess.cloak.auto')}</option>
                    <option value="always">{t('apiAccess.cloak.always')}</option>
                    <option value="never">{t('apiAccess.cloak.never')}</option>
                  </select>
                </label>
                <label className="multiline-field">
                  <span>{t('apiAccess.cloak.words')}</span>
                  <textarea value={draft.cloakSensitiveWordsText ?? ''} onChange={(event) => updateTextField('cloakSensitiveWordsText', event.currentTarget.value)} rows={3} placeholder={'internal-name\nworkspace-id'} />
                </label>
                <div className="provider-advanced-toggle">
                  <div><strong>{t('apiAccess.cloak.strict')}</strong><span>{t('apiAccess.cloak.strictDescription')}</span></div>
                  <label className="switch-control" title={t('apiAccess.cloak.enableStrict')}><input type="checkbox" checked={Boolean(draft.cloakStrictMode)} onChange={(event) => updateBooleanField('cloakStrictMode', event.currentTarget.checked)} /><span className="switch-track" /></label>
                </div>
                <div className="provider-advanced-toggle">
                  <div><strong>{t('apiAccess.cloak.cacheUser')}</strong><span>{t('apiAccess.cloak.cacheUserDescription')}</span></div>
                  <select aria-label={t('apiAccess.cloak.cacheUser')} value={draft.cloakCacheUserId == null ? '' : String(draft.cloakCacheUserId)} onChange={(event) => updateOptionalBooleanField('cloakCacheUserId', event.currentTarget.value)}>
                    <option value="">{t('apiAccess.option.inherit')}</option>
                    <option value="true">{t('common.enabled')}</option>
                    <option value="false">{t('common.disabled')}</option>
                  </select>
                </div>
              </div>
            ) : null}
            {!grouped && (activeSection === 'codex-api-key' || activeSection === 'xai-api-key') ? (
              <div className="provider-advanced-toggle">
                <div><strong>WebSocket</strong><span>{t('apiAccess.websocket.description')}</span></div>
                <label className="switch-control" title={t('apiAccess.websocket.enable')}><input type="checkbox" checked={Boolean(draft.websockets)} onChange={(event) => updateBooleanField('websockets', event.currentTarget.checked)} /><span className="switch-track" /></label>
              </div>
            ) : null}
            <div className="provider-advanced-toggle">
              <div><strong>{t('apiAccess.cooling.title')}</strong><span>{t('apiAccess.cooling.description')}</span></div>
              <select aria-label={t('apiAccess.cooling.title')} value={draft.disableCooling == null ? '' : String(draft.disableCooling)} onChange={(event) => updateOptionalBooleanField('disableCooling', event.currentTarget.value)}>
                <option value="">{t('apiAccess.option.inherit')}</option>
                <option value="true">{t('apiAccess.cooling.disable')}</option>
                <option value="false">{t('apiAccess.cooling.enable')}</option>
              </select>
            </div>
            </AdvancedSettingsGroup>
          </div>
        </details>
        {formError ? (
          <MessageNotice message={formError} onDismiss={() => setFormError('')} />
        ) : null}
        <div className="config-dialog-actions two-actions">
          <button type="button" className="secondary-button" onClick={onClose} disabled={busy}>{t('common.cancel')}</button>
          <button type="submit" className="primary-button" disabled={busy}>{busy ? t('common.saving') : t('common.save')}</button>
        </div>
      </form>
      </div>

      {modelDiscoveryOpen ? (
        <div className="model-discovery-backdrop" onMouseDown={(event) => event.currentTarget === event.target && closeModelDiscovery()}>
          <section ref={modelDialogRef} className="model-discovery-dialog model-transfer-dialog" role="dialog" aria-modal="true" aria-labelledby="model-discovery-title">
            <div className="model-discovery-header">
              <div><h2 id="model-discovery-title">{t('apiAccess.modelDialog.title')}</h2><span>{providerLabel(definition)}</span></div>
              <button type="button" className="icon-button quiet" onClick={closeModelDiscovery} title={t('common.close')} aria-label={t('common.close')}><X size={18} aria-hidden="true" /></button>
            </div>

            <div className="model-transfer-summary">
              <span role="status">{t('apiAccess.modelDialog.summary', { found: modelOptions.length, selected: selectedModels.length })}</span>
              <button type="button" className="secondary-button compact-button" onClick={() => void discoverModels()} disabled={modelLoading}>
                <RefreshCw size={15} className={modelLoading ? 'spin' : ''} />{t('common.refresh')}
              </button>
            </div>

            {modelError ? (
              <MessageNotice source="apiAccess.modelDialog.fetchFailed" message={modelError} onDismiss={() => setModelError('')} />
            ) : null}

            <div className="model-transfer-panels">
              <ModelSelectionPanel models={unselectedModels} selected={false} loading={modelLoading} onMove={moveModels} />
              <ModelSelectionPanel models={selectedModels} selected loading={modelLoading} onMove={moveModels} />
            </div>

            <div className="model-discovery-actions model-transfer-actions">
              {selectedModels.length === 0 ? <span>{t('apiAccess.modelDialog.chooseOne')}</span> : null}
              <button type="button" className="secondary-button" onClick={closeModelDiscovery}>{t('common.cancel')}</button>
              <button type="button" className="primary-button" onClick={applyModelSelection} disabled={modelLoading || selectedModels.length === 0}>{t('apiAccess.modelDialog.apply', { count: selectedModels.length })}</button>
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
  // Keep both dialogs outside the page's query container, which can contain fixed elements in WebKit.
  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
