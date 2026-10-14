import { MessageNotice } from '../appNotice';
import { useConfirmation } from '../components/ConfirmationDialog';
import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import { usableModelAlias } from '../services/modelService';
import {
  ArrowRight,
  BrainCircuit,
  Check,
  GitFork,
  LoaderCircle,
  Plus,
  Search,
  Pencil,
  Trash2,
  X,
  Zap,
} from 'lucide-react';
import { getCurrentLocale, translate, useI18n } from '../i18n';
import { SettingsHelp } from '../components/SettingsHelp';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';

type PresetThinkingEffort = 'low' | 'medium' | 'high' | 'xhigh' | 'max';

type ModelAliasPosition = {
  section: string;
  providerIndex: number | null;
  modelIndex: number;
};

type ThinkingAliasEntry = {
  position: ModelAliasPosition;
  sourceModel: string;
  alias: string;
  effort: string | null;
  provider: string;
  kind: string;
  oauthChannel?: string | null;
};

type SpeedAliasEntry = {
  position: ModelAliasPosition;
  sourceModel: string;
  alias: string;
  serviceTier: string;
  provider: string;
  kind: string;
  oauthChannel?: string | null;
};

type AliasListEntry = ThinkingAliasEntry & {
  serviceTier: string | null;
};

type ThinkingAliasSource = {
  id: string;
  model: string;
  displayName: string | null;
  provider: string;
  kind: string;
  protocol: string;
  reasoningLevels: string[];
};

type ModelAliasSource = ThinkingAliasSource & {
  supportsReasoning: boolean;
  supportsFast: boolean;
};

type ModelAliasEditContext = {
  source: ThinkingAliasSource;
  effort: string | null;
  fast: boolean;
};

const effortOptions = [
  { value: 'low', label: 'Low', hintKey: 'aliases.effort.low' },
  { value: 'medium', label: 'Medium', hintKey: 'aliases.effort.medium' },
  { value: 'high', label: 'High', hintKey: 'aliases.effort.high' },
  { value: 'xhigh', label: 'XHigh', hintKey: 'aliases.effort.xhigh' },
  { value: 'max', label: 'Max', hintKey: 'aliases.effort.max' },
] as const satisfies ReadonlyArray<{ value: PresetThinkingEffort; label: string; hintKey: string }>;

export const combineModelAliasEntries = (
  thinkingEntries: ThinkingAliasEntry[],
  speedEntries: SpeedAliasEntry[],
): AliasListEntry[] => {
  const entries = new Map<string, AliasListEntry>();
  const entryKey = (entry: ThinkingAliasEntry | SpeedAliasEntry) => JSON.stringify([
    entry.position.section, entry.position.providerIndex, entry.oauthChannel,
    entry.position.modelIndex,
  ]);

  thinkingEntries.forEach((entry) => {
    entries.set(entryKey(entry), { ...entry, serviceTier: null });
  });
  speedEntries.forEach((entry) => {
    const key = entryKey(entry);
    const current = entries.get(key);
    entries.set(key, current
      ? { ...current, serviceTier: entry.serviceTier }
      : { ...entry, effort: null });
  });

  return [...entries.values()].sort((left, right) => (
    left.provider.localeCompare(right.provider)
      || left.alias.localeCompare(right.alias)
  ));
};

export const combineModelAliasSources = (
  baseSources: ThinkingAliasSource[],
  thinkingSources: ThinkingAliasSource[],
  speedSources: ThinkingAliasSource[],
): ModelAliasSource[] => {
  const reasoningSourceIds = new Set(thinkingSources.map((source) => source.id));
  const speedSourceIds = new Set(speedSources.map((source) => source.id));
  const sources = new Map<string, ModelAliasSource>();
  [...baseSources, ...speedSources, ...thinkingSources].forEach((source) => {
    sources.set(source.id, {
      ...source,
      supportsReasoning: reasoningSourceIds.has(source.id),
      supportsFast: speedSourceIds.has(source.id),
    });
  });
  return [...sources.values()];
};

export const defaultModelAlias = (
  model: string | null | undefined,
  effort: string,
  fast: boolean,
) => {
  const normalizedModel = model?.trim() ?? '';
  const normalizedEffort = effort.trim().toLowerCase();
  if (!normalizedModel) return '';
  if (!normalizedEffort && !fast) return `${normalizedModel}-alias`;
  return `${normalizedModel}${normalizedEffort ? `-${normalizedEffort}` : ''}${fast ? '-fast' : ''}`;
};

export const uniqueModelAlias = (
  alias: string,
  existingModelNames: string[],
) => {
  const normalizedAlias = alias.trim();
  if (!normalizedAlias) return '';
  const existingNames = new Set(
    existingModelNames
      .map((name) => name.trim().toLowerCase())
      .filter(Boolean),
  );
  if (!existingNames.has(normalizedAlias.toLowerCase())) return normalizedAlias;

  let suffix = 2;
  let candidate = `${normalizedAlias}-${suffix}`;
  while (existingNames.has(candidate.toLowerCase())) {
    suffix += 1;
    candidate = `${normalizedAlias}-${suffix}`;
  }
  return candidate;
};

export const thinkingAliasSourceKindLabel = (kind: string) => {
  if (kind === 'codex-oauth') return 'Codex OAuth';
  if (kind === 'antigravity-oauth') return 'Antigravity OAuth';
  if (kind === 'claude-oauth') return 'Claude OAuth';
  if (kind === 'aistudio-oauth') return 'AI Studio OAuth';
  if (kind === 'vertex-oauth') return 'Vertex OAuth';
  if (kind === 'kimi-oauth') return 'Kimi OAuth';
  if (kind === 'xai-oauth') return 'xAI OAuth';
  if (kind === 'devin-oauth') return 'Devin OAuth';
  if (kind === 'codex-api') return 'Codex API';
  if (kind === 'claude-api') return 'Claude API';
  if (kind === 'gemini-api') return 'Gemini API';
  if (kind === 'openai-compatible') return translate(getCurrentLocale(), 'aliases.source.openAiCompatible');
  return translate(getCurrentLocale(), 'aliases.source.other');
};

const thinkingAliasProviderDetail = (kind: string, provider: string) => (
  provider === thinkingAliasSourceKindLabel(kind)
    ? translate(getCurrentLocale(), 'aliases.source.available')
    : provider
);

const thinkingAliasSourceDetail = (source: ThinkingAliasSource) => (
  thinkingAliasProviderDetail(source.kind, source.provider)
);

export function ThinkingAliasesPage({ embedded = false }: { embedded?: boolean } = {}) {
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const { t } = useI18n();
  const [thinkingEntries, setThinkingEntries] = useState<ThinkingAliasEntry[]>([]);
  const [speedEntries, setSpeedEntries] = useState<SpeedAliasEntry[]>([]);
  const [baseSources, setBaseSources] = useState<ThinkingAliasSource[]>([]);
  const [thinkingSources, setThinkingSources] = useState<ThinkingAliasSource[]>([]);
  const [speedSources, setSpeedSources] = useState<ThinkingAliasSource[]>([]);
  const [selectedSourceId, setSelectedSourceId] = useState('');
  const [effort, setEffort] = useState('');
  const [fastEnabled, setFastEnabled] = useState(false);
  const [alias, setAlias] = useState('');
  const [editingEntry, setEditingEntry] = useState<AliasListEntry | null>(null);
  const [editingSource, setEditingSource] = useState<ThinkingAliasSource | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const modelSearchRef = useRef<HTMLInputElement>(null);
  const [activeSourceIndex, setActiveSourceIndex] = useState(0);
  const modelPickerRef = useRef<HTMLDivElement>(null);
  const generatedAliasRef = useRef('');
  const [loading, setLoading] = useState(true);
  const [busyAlias, setBusyAlias] = useState('');
  const [busyAction, setBusyAction] = useState<'create' | 'edit' | 'delete' | ''>('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [
        nextThinkingEntries,
        nextBaseSources,
        nextThinkingSources,
        nextSpeedEntries,
        nextSpeedSources,
      ] = await Promise.all([
        invoke<ThinkingAliasEntry[]>('get_thinking_aliases'),
        invoke<ThinkingAliasSource[]>('get_model_alias_sources'),
        invoke<ThinkingAliasSource[]>('get_thinking_alias_sources'),
        invoke<SpeedAliasEntry[]>('get_speed_aliases'),
        invoke<ThinkingAliasSource[]>('get_speed_alias_sources'),
      ]);
      setThinkingEntries(nextThinkingEntries);
      setSpeedEntries(nextSpeedEntries);
      setBaseSources(nextBaseSources);
      setThinkingSources(nextThinkingSources);
      setSpeedSources(nextSpeedSources);
      setSelectedSourceId((current) => (
        nextBaseSources.some((source) => source.id === current)
          ? current
          : ''
      ));
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!modelPickerOpen) return undefined;
    const closeOnOutsideClick = (event: PointerEvent) => {
      if (!modelPickerRef.current?.contains(event.target as Node)) {
        setModelPickerOpen(false);
        setSearch('');
      }
    };
    document.addEventListener('pointerdown', closeOnOutsideClick);
    return () => document.removeEventListener('pointerdown', closeOnOutsideClick);
  }, [modelPickerOpen]);

  const sources = useMemo(
    () => {
      const choices = combineModelAliasSources(baseSources, thinkingSources, speedSources);
      if (!editingSource) return choices;
      return [{
        ...editingSource,
        supportsReasoning: editingSource.reasoningLevels.length > 0,
        supportsFast: ['codex-oauth', 'codex-api', 'openai-compatible'].includes(editingSource.kind),
      }, ...choices];
    },
    [baseSources, speedSources, thinkingSources, editingSource],
  );
  const entries = useMemo(
    () => combineModelAliasEntries(thinkingEntries, speedEntries),
    [speedEntries, thinkingEntries],
  );

  useEffect(() => {
    setSelectedSourceId((current) => (
      sources.some((source) => source.id === current) ? current : ''
    ));
  }, [sources]);

  const selectedSource = useMemo(
    () => sources.find((source) => source.id === selectedSourceId) ?? null,
    [selectedSourceId, sources],
  );
  const fastAvailable = Boolean(selectedSource?.supportsFast);

  useEffect(() => {
    if (!fastAvailable) {
      setFastEnabled(false);
    }
  }, [fastAvailable]);

  const filteredSources = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return sources;
    return sources
      .map((source, index) => {
        const model = source.model.toLowerCase();
        const displayName = (source.displayName ?? '').toLowerCase();
        const haystack = `${model} ${displayName} ${source.provider} ${thinkingAliasSourceKindLabel(source.kind)}`
          .toLowerCase();
        let score = 5;
        if (model === query) score = 0;
        else if (displayName === query) score = 1;
        else if (model.startsWith(query)) score = 2;
        else if (displayName.startsWith(query)) score = 3;
        else if (haystack.includes(query)) score = 4;
        return { source, index, score };
      })
      .filter((item) => item.score < 5)
      .sort((left, right) => left.score - right.score || left.index - right.index)
      .map((item) => item.source);
  }, [sources, search]);

  useEffect(() => {
    setActiveSourceIndex(0);
  }, [search, sources]);

  const chooseSource = (source: ModelAliasSource) => {
    setSelectedSourceId(source.id);
    setEffort('');
    setFastEnabled((current) => current && source.supportsFast);
    if (!editingEntry) setAlias('');
    generatedAliasRef.current = '';
    setModelPickerOpen(false);
    setSearch('');
  };

  const chooseEffort = (nextEffort: string) => {
    setEffort(nextEffort);
  };

  const handleModelSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape' && modelPickerOpen) {
      event.preventDefault();
      event.stopPropagation();
      setModelPickerOpen(false);
      setSearch('');
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!modelPickerOpen) {
        setModelPickerOpen(true);
        return;
      }
      if (!filteredSources.length) return;
      const direction = event.key === 'ArrowDown' ? 1 : -1;
      setActiveSourceIndex((current) => (
        (current + direction + filteredSources.length) % filteredSources.length
      ));
      return;
    }
    if (event.key === 'Enter' && modelPickerOpen && filteredSources[activeSourceIndex]) {
      event.preventDefault();
      chooseSource(filteredSources[activeSourceIndex]);
    }
  };

  const normalizedEffort = effort.trim().toLowerCase();
  const defaultAlias = defaultModelAlias(selectedSource?.model, normalizedEffort, fastEnabled);
  const uniqueDefaultAlias = uniqueModelAlias(defaultAlias, sources.map((source) => source.model));
  const availableEfforts = selectedSource?.reasoningLevels ?? [];
  const visibleEffortOptions = availableEfforts.map((value) => {
    const preset = effortOptions.find((option) => option.value === value);
    return preset
      ? { value: preset.value, label: preset.label, title: t(preset.hintKey) }
      : { value, label: value, title: value };
  });

  useEffect(() => {
    if (editingEntry) return;
    setAlias((current) => {
      const currentValue = current.trim();
      const canReplace = !currentValue || currentValue === generatedAliasRef.current;
      if (!canReplace) return current;
      generatedAliasRef.current = uniqueDefaultAlias;
      return uniqueDefaultAlias;
    });
  }, [uniqueDefaultAlias, editingEntry]);

  const createAlias = async () => {
    if (!selectedSource) {
      setError(t('aliases.error.selectModel'));
      return;
    }
    const normalizedAlias = alias.trim();
    if (!normalizedAlias) {
      setError(t('aliases.error.emptyAlias'));
      return;
    }
    const keepingExistingAlias = Boolean(
      editingEntry && editingEntry.alias.trim().toLowerCase() === normalizedAlias.toLowerCase(),
    );
    if (!keepingExistingAlias && !usableModelAlias(normalizedAlias)) {
      setError(t('aliases.error.invalidAlias'));
      return;
    }
    if (fastEnabled && !selectedSource.supportsFast) {
      setError(t('aliases.error.unsupportedFast'));
      return;
    }
    if (normalizedEffort && !selectedSource.supportsReasoning) {
      setError(t('aliases.error.unsupportedEffort'));
      return;
    }
    setBusyAlias(normalizedAlias);
    setBusyAction('create');
    setError('');
    setNotice('');
    try {
      if (editingEntry) {
        await invoke('create_thinking_alias', {
          sourceId: selectedSource.id, alias: normalizedAlias,
          effort: normalizedEffort, fast: fastEnabled, originalAlias: editingEntry.alias,
          originalEntry: editingEntry,
        });
        setNotice(t('aliases.updated', { alias: normalizedAlias }));
      } else if (normalizedEffort) {
        await invoke<ThinkingAliasEntry[]>('create_thinking_alias', {
          sourceId: selectedSource.id,
          alias: normalizedAlias,
          effort: normalizedEffort,
          fast: fastEnabled,
        });
        setNotice(t(fastEnabled ? 'aliases.createdCombined' : 'aliases.created', {
          alias: normalizedAlias,
          effort: normalizedEffort,
        }));
      } else if (fastEnabled) {
        await invoke<SpeedAliasEntry[]>('create_speed_alias', {
          sourceId: selectedSource.id,
          alias: normalizedAlias,
        });
        setNotice(t('speedAliases.created', { alias: normalizedAlias }));
      } else {
        await invoke<ThinkingAliasEntry[]>('create_thinking_alias', {
          sourceId: selectedSource.id,
          alias: normalizedAlias,
          effort: '',
          fast: false,
        });
        setNotice(t('aliases.createdPlain', { alias: normalizedAlias }));
      }
      setAlias('');
      await load();
      setEditorOpen(false);
      resetEditor();
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setBusyAlias('');
      setBusyAction('');
    }
  };

  const resetEditor = () => {
    setEditingEntry(null);
    setEditingSource(null);
    setSelectedSourceId('');
    setEffort('');
    setFastEnabled(false);
    setAlias('');
    setSearch('');
    setModelPickerOpen(false);
    generatedAliasRef.current = '';
  };

  const closeEditor = () => {
    if (busyAlias) return;
    setEditorOpen(false);
    resetEditor();
  };

  const editorRef = useDialogFocusTrap<HTMLElement>({
    active: editorOpen,
    initialFocusRef: modelSearchRef,
    onEscape: busyAlias ? undefined : () => {
      if (modelPickerOpen) setModelPickerOpen(false);
      else closeEditor();
    },
    preventEscape: Boolean(busyAlias),
  });

  const addAlias = () => {
    resetEditor();
    setError('');
    setNotice('');
    setEditorOpen(true);
  };

  const editAlias = async (entry: AliasListEntry) => {
    setBusyAlias(entry.alias);
    setBusyAction('edit');
    setError('');
    setNotice('');
    try {
      const context = await invoke<ModelAliasEditContext>('get_model_alias_edit_source', {
        alias: entry.alias, entry,
      });
      const source = context.source;
      setEditingEntry(entry);
      setEditingSource(source);
      setSelectedSourceId(source.id);
      setEffort(context.effort ?? '');
      setFastEnabled(context.fast);
      setAlias(entry.alias);
      generatedAliasRef.current = '';
      setModelPickerOpen(false);
      setSearch('');
      setEditorOpen(true);
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setBusyAlias('');
      setBusyAction('');
    }
  };

  const deleteAlias = async (entry: AliasListEntry) => {
    if (!await askConfirmation({ title: t('common.delete'), message: t('aliases.deleteConfirm', { alias: entry.alias }), confirmText: t('common.delete'), variant: 'danger' })) return;
    setBusyAlias(entry.alias);
    setBusyAction('delete');
    setError('');
    setNotice('');
    try {
      if (entry.effort || !entry.serviceTier) {
        await invoke<ThinkingAliasEntry[]>('delete_thinking_alias', {
          alias: entry.alias,
          oauthChannel: entry.oauthChannel,
          entry,
        });
      } else {
        await invoke<SpeedAliasEntry[]>('delete_speed_alias', {
          alias: entry.alias,
          oauthChannel: entry.oauthChannel,
          entry,
        });
      }
      setNotice(t('aliases.deleted', { alias: entry.alias }));
      await load();
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setBusyAlias('');
      setBusyAction('');
    }
  };

  return (
    <section className="page management-page thinking-alias-page">
      {confirmationDialog}
      <MessageNotice message={error} onDismiss={() => setError('')} />
      <MessageNotice tone="success" message={!error ? notice : null} onDismiss={() => setNotice('')} />

      <header className="management-header">
        <div>
          {embedded
            ? <h2>{t('app.nav.thinkingAliases')}</h2>
            : <h1>{t('app.nav.thinkingAliases')}</h1>}
        </div>
        <div className="management-heading-actions">
          <span className="muted-summary">{entries.length}</span>
          <button
            type="button"
            className="primary-button compact-button"
            onClick={addAlias}
            disabled={loading || Boolean(busyAlias)}
            title={t('aliases.create')}
            aria-label={t('aliases.create')}
          >
            <Plus size={18} aria-hidden="true" />
          </button>
        </div>
      </header>

      {editorOpen ? (
        <div className="config-dialog-backdrop" onMouseDown={(event) => event.currentTarget === event.target && closeEditor()}>
        <section
          ref={editorRef}
          className="panel management-dialog thinking-alias-editor-panel thinking-alias-dialog"
          role="dialog"
          aria-modal="true"
          aria-labelledby="thinking-alias-editor-title"
        >
            <div className="thinking-alias-panel-heading">
              <span><GitFork size={18} /></span>
              <div>
                <h2 id="thinking-alias-editor-title">{t(editingEntry ? 'common.edit' : 'aliases.create.title')}</h2>
              </div>
              <button type="button" className="icon-button quiet" onClick={closeEditor} disabled={Boolean(busyAlias)} title={t('common.close')} aria-label={t('common.close')}>
                <X size={18} />
              </button>
            </div>

            <div className="thinking-alias-dialog-body">

            <div className="thinking-alias-field thinking-model-field">
            <div className="config-field-label"><label htmlFor="thinking-model-search">{t('aliases.originalModel')}</label><SettingsHelp label={t('aliases.originalModel')}>{t('aliases.sourceHint')}</SettingsHelp></div>
            <div className="thinking-model-picker" ref={modelPickerRef}>
              <div className="thinking-model-search">
                {loading ? <LoaderCircle size={15} className="spin" /> : <Search size={15} />}
                <input
                  id="thinking-model-search"
                  ref={modelSearchRef}
                  role="combobox"
                  autoFocus
                  aria-autocomplete="list"
                  aria-expanded={modelPickerOpen}
                  aria-controls="thinking-model-options"
                  aria-activedescendant={modelPickerOpen && filteredSources[activeSourceIndex]
                    ? `thinking-model-option-${activeSourceIndex}`
                    : undefined}
                  value={modelPickerOpen ? search : selectedSource?.model ?? ''}
                  onFocus={(event) => {
                    setSearch(selectedSource?.model ?? '');
                    setModelPickerOpen(true);
                    event.currentTarget.select();
                  }}
                  onChange={(event) => {
                    const nextSearch = event.currentTarget.value;
                    setSearch(nextSearch);
                    setModelPickerOpen(true);
                    if (
                      selectedSource
                      && nextSearch.trim().toLowerCase() !== selectedSource.model.trim().toLowerCase()
                    ) {
                      setSelectedSourceId('');
                      setEffort('');
                      setFastEnabled(false);
                      if (!editingEntry) setAlias('');
                      generatedAliasRef.current = '';
                    }
                  }}
                  onKeyDown={handleModelSearchKeyDown}
                  placeholder={loading ? t('aliases.loadingModels') : t('aliases.searchModel')}
                  autoComplete="off"
                  spellCheck={false}
                  disabled={loading || Boolean(busyAlias)}
                />
                {!modelPickerOpen && selectedSource ? (
                  <span className="thinking-source-kind">
                    {thinkingAliasSourceKindLabel(selectedSource.kind)}
                  </span>
                ) : null}
              </div>
              {modelPickerOpen ? (
                <div
                  className="thinking-model-list"
                  data-theme-surface="overlay"
                  id="thinking-model-options"
                  role="listbox"
                  aria-label={t('aliases.availableModels')}
                >
                  {filteredSources.length === 0 ? (
                    <div className="thinking-model-empty">
                      {sources.length ? t('aliases.noMatch') : t('aliases.noModels')}
                    </div>
                  ) : filteredSources.map((source, index) => {
                    const selected = source.id === selectedSourceId;
                    return (
                      <button
                        type="button"
                        role="option"
                        aria-selected={selected}
                        id={`thinking-model-option-${index}`}
                        className={`${selected ? 'selected ' : ''}${index === activeSourceIndex ? 'active' : ''}`.trim()}
                        key={source.id}
                        onMouseEnter={() => setActiveSourceIndex(index)}
                        onClick={() => chooseSource(source)}
                        disabled={Boolean(busyAlias)}
                      >
                        <span className="thinking-model-option-copy">
                          <span>
                            <strong title={source.model}>{source.model}</strong>
                            {source.displayName && source.displayName !== source.model
                              ? <small>{source.displayName}</small>
                              : null}
                          </span>
                          <span className="thinking-model-source">
                            <em>{thinkingAliasSourceKindLabel(source.kind)}</em>
                            <small title={thinkingAliasSourceDetail(source)}>
                              {thinkingAliasSourceDetail(source)}
                            </small>
                          </span>
                        </span>
                        {selected ? <Check size={15} /> : null}
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
            {selectedSource ? (
              <div className="thinking-model-selection">
                <span>{thinkingAliasSourceKindLabel(selectedSource.kind)}</span>
                <strong title={thinkingAliasSourceDetail(selectedSource)}>
                  {t('aliases.sourceLabel', { source: thinkingAliasSourceDetail(selectedSource) })}
                </strong>
              </div>
            ) : null}
            </div>

            <div className="thinking-alias-field">
            <div className="thinking-field-heading">
              <strong>{t('aliases.effort.title')}</strong>
            </div>
            <div className="thinking-effort-options">
              <button
                type="button"
                className={!normalizedEffort ? 'active none' : 'none'}
                onClick={() => {
                  chooseEffort('');
                }}
                disabled={Boolean(busyAlias)}
                title={t('aliases.effort.noneHint')}
              >
                {t('aliases.effort.none')}
              </button>
              {visibleEffortOptions.map((option) => (
                <button
                  type="button"
                  className={effort.trim().toLowerCase() === option.value ? 'active' : ''}
                  key={option.value}
                  onClick={() => {
                    chooseEffort(option.value);
                  }}
                  disabled={Boolean(busyAlias) || Boolean(selectedSource && !selectedSource.supportsReasoning)}
                  title={option.title}
                >
                  {option.label}
                </button>
              ))}
            </div>
            {selectedSource && !selectedSource.supportsReasoning ? (
              <small className="thinking-model-hint">{t('aliases.effort.unsupported')}</small>
            ) : null}
            </div>

            <div className="thinking-alias-field">
              <div className="thinking-field-heading">
                <strong>{t('aliases.fast.title')}</strong>
              </div>
              <label className={`thinking-fast-option${fastEnabled ? ' active' : ''}`}>
                <span className="thinking-fast-option-copy">
                  <span><Zap size={15} /> Fast</span>
                </span>
                <span className="switch-control thinking-fast-switch">
                  <input
                    type="checkbox"
                    checked={fastEnabled}
                    onChange={(event) => setFastEnabled(event.currentTarget.checked)}
                    disabled={Boolean(busyAlias) || !fastAvailable}
                    aria-label={t('aliases.fast.title')}
                  />
                  <span className="switch-track" />
                </span>
              </label>
              {selectedSource && !selectedSource.supportsFast ? (
                <small className="thinking-model-hint">{t('aliases.fast.unsupported')}</small>
              ) : null}
            </div>

            <div className="thinking-alias-section-divider" aria-hidden="true" />

            <div className="thinking-alias-field">
              <div className="thinking-field-heading">
                <strong>{t('aliases.aliasName.title')}</strong>
                <span>{t('aliases.aliasName.autoDescription')}</span>
              </div>
              <input
                id="thinking-alias-name"
                className="thinking-alias-input"
                value={alias}
                onChange={(event) => setAlias(event.currentTarget.value)}
                placeholder={selectedSource
                  ? normalizedEffort
                    ? t('aliases.aliasName.example', { model: selectedSource.model, effort: normalizedEffort })
                    : fastEnabled
                      ? t('speedAliases.aliasName.example', { model: selectedSource.model })
                      : t('aliases.aliasName.example', { model: selectedSource.model, effort: 'alias' })
                  : t('aliases.aliasName.selectFirst')}
                disabled={Boolean(busyAlias)}
              />
            </div>

            <div className="thinking-alias-preview">
              {fastEnabled && !normalizedEffort ? <Zap size={18} /> : <BrainCircuit size={18} />}
              <div>
                <span>{selectedSource?.model || t('aliases.notSelected')} <ArrowRight size={13} /> {alias || t('aliases.enterAlias')}</span>
              </div>
            </div>
            </div>

            <div className="thinking-alias-dialog-actions">
            <button type="button" className="secondary-button" disabled={Boolean(busyAlias)} onClick={closeEditor}>
              {t('common.cancel')}
            </button>
            <button
              type="button"
              className="primary-button thinking-alias-create"
              onClick={() => void createAlias()}
              disabled={loading || Boolean(busyAlias)}
            >
              {busyAction === 'create'
                ? <LoaderCircle size={16} className="spin" />
                : fastEnabled && !normalizedEffort ? <Zap size={16} /> : <GitFork size={16} />}
              {busyAction === 'create' ? t(editingEntry ? 'common.saving' : 'aliases.creating') : t(editingEntry ? 'common.save' : 'aliases.create')}
            </button>
            </div>
        </section>
        </div>
      ) : null}

        <section className="panel thinking-alias-list-panel">
          <div className="thinking-alias-list">
            {loading ? (
              <div className="management-loading"><LoaderCircle size={20} className="spin" />{t('aliases.loadingConfig')}</div>
            ) : entries.length === 0 ? (
              <div className="management-empty">
                <GitFork size={25} />
                <strong>{t('aliases.empty.title')}</strong>
                <span>{t('aliases.empty.description')}</span>
              </div>
            ) : entries.map((entry) => (
              <article className="thinking-alias-row" key={JSON.stringify([entry.position, entry.oauthChannel])}>
                <div className="thinking-alias-route">
                  <div className="thinking-alias-route-source">
                    <span title={entry.sourceModel}>{entry.sourceModel}</span>
                    <small>
                      <em>{thinkingAliasSourceKindLabel(entry.kind)}</em>
                      <span title={thinkingAliasProviderDetail(entry.kind, entry.provider)}>
                        {thinkingAliasProviderDetail(entry.kind, entry.provider)}
                      </span>
                    </small>
                  </div>
                  <ArrowRight size={14} />
                  <strong title={entry.alias}>{entry.alias}</strong>
                </div>
                <div className="thinking-alias-badges">
                  {entry.effort ? (
                    <span className="thinking-effort-badge">{entry.effort}</span>
                  ) : null}
                  {entry.serviceTier ? (
                    <span className="thinking-effort-badge fast">{t('speedAliases.fast.title')}</span>
                  ) : null}
                </div>
                <div className="thinking-alias-actions">
                  <button type="button" className="icon-button quiet" disabled={loading || Boolean(busyAlias)}
                    onClick={() => void editAlias(entry)} title={t('common.edit')} aria-label={t('common.edit')}>
                    <Pencil size={15} />
                  </button>
                  <button
                    type="button"
                    className="icon-button quiet danger"
                    onClick={() => void deleteAlias(entry)}
                    disabled={Boolean(busyAlias)}
                    title={t('aliases.delete', { alias: entry.alias })}
                    aria-label={t('aliases.delete', { alias: entry.alias })}
                  >
                    {busyAction === 'delete' && busyAlias === entry.alias
                      ? <LoaderCircle size={15} className="spin" />
                      : <Trash2 size={15} />}
                  </button>
                </div>
              </article>
            ))}
          </div>
        </section>
    </section>
  );
}
