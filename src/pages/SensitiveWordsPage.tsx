import { useCallback, useEffect, useRef, useState, type ClipboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, Plus, RefreshCw, ShieldAlert, Trash2 } from 'lucide-react';
import { useI18n } from '../i18n';
import { useConfirmation } from '../components/ConfirmationDialog';

type SensitiveWordsSettings = {
  antigravitySensitiveWords: string[];
  devinSensitiveWords: string[];
};

type Provider = 'antigravity' | 'devin';
type WordEntry = { id: number; value: string };
type WordDraft = Record<Provider, WordEntry[]>;

const normalizedWords = (entries: WordEntry[]) => entries.map(({ value }) => value.trim()).filter(Boolean);

type WordEditorProps = {
  provider: Provider;
  entries: WordEntry[];
  composer: string;
  disabled: boolean;
  onAdd: (provider: Provider, value: string) => void;
  onCompose: (provider: Provider, value: string) => void;
  onChange: (provider: Provider, id: number, value: string) => void;
  onRemove: (provider: Provider, id: number) => void;
  onPaste: (provider: Provider, id: number, event: ClipboardEvent<HTMLInputElement>) => void;
};

function WordEditor({ provider, entries, composer, disabled, onAdd, onCompose, onChange, onRemove, onPaste }: WordEditorProps) {
  const { t } = useI18n();
  const title = t(provider === 'antigravity' ? 'config.sensitiveWords.antigravity' : 'config.sensitiveWords.devin');

  return (
    <section className="config-sensitive-words-card">
      <div className="config-sensitive-words-card-heading">
        <h3>{title}</h3>
        <span className="config-sensitive-words-count" aria-label={`${entries.length} ${title}`}>{entries.length}</span>
      </div>
      <div className="config-sensitive-words-composer">
        <input
          className="config-network-input"
          type="text"
          aria-label={`${title} ${t('common.add')}`}
          value={composer}
          disabled={disabled}
          placeholder={t('config.sensitiveWords.addPlaceholder')}
          onChange={(event) => onCompose(provider, event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.nativeEvent.isComposing) return;
            event.preventDefault();
            onAdd(provider, composer);
          }}
          onPaste={(event) => {
            const pasted = event.clipboardData.getData('text');
            if (!/[\r\n]/.test(pasted)) return;
            event.preventDefault();
            const input = event.currentTarget;
            const start = input.selectionStart ?? input.value.length;
            const end = input.selectionEnd ?? start;
            onAdd(provider, input.value.slice(0, start) + pasted + input.value.slice(end));
          }}
        />
        <button type="button" className="icon-button quiet" disabled={disabled || !composer.trim()}
          title={t('common.add')} aria-label={`${t('common.add')} ${title}`}
          onClick={() => onAdd(provider, composer)}>
          <Plus size={16} aria-hidden="true" />
        </button>
      </div>
      {entries.length === 0 ? (
        <p className="config-sensitive-words-empty">{t('config.sensitiveWords.empty')}</p>
      ) : (
        <div className="config-sensitive-words-list">
          {entries.map((entry, index) => (
            <div className="config-sensitive-words-row" key={entry.id}>
              <input
                id={`config-${provider}-word-${entry.id}`}
                className="config-network-input"
                type="text"
                aria-label={`${title} ${index + 1}`}
                value={entry.value}
                disabled={disabled}
                onChange={(event) => onChange(provider, entry.id, event.currentTarget.value)}
                onPaste={(event) => onPaste(provider, entry.id, event)}
              />
              <button type="button" className="icon-button quiet" disabled={disabled}
                title={t('common.delete')} aria-label={`${t('common.delete')} ${title} ${index + 1}`}
                onClick={() => onRemove(provider, entry.id)}>
                <Trash2 size={15} aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

export function SensitiveWordsPage({ onDirtyChange }: { onDirtyChange?: (dirty: boolean) => void } = {}) {
  const { t } = useI18n();
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const nextEntryId = useRef(0);
  const [saved, setSaved] = useState<SensitiveWordsSettings | null>(null);
  const [draft, setDraft] = useState<WordDraft>({ antigravity: [], devin: [] });
  const [composer, setComposer] = useState<Record<Provider, string>>({ antigravity: '', devin: '' });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [savedNotice, setSavedNotice] = useState(false);

  const replaceDraft = useCallback((settings: SensitiveWordsSettings) => {
    const toEntries = (words: string[]) => words.map((value) => ({ id: nextEntryId.current++, value }));
    setDraft({
      antigravity: toEntries(settings.antigravitySensitiveWords),
      devin: toEntries(settings.devinSensitiveWords),
    });
    setComposer({ antigravity: '', devin: '' });
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const result = await invoke<SensitiveWordsSettings>('get_core_sensitive_words_settings');
      setSaved(result);
      replaceDraft({
        ...result,
        antigravitySensitiveWords: result.antigravitySensitiveWords.length
          ? result.antigravitySensitiveWords
          : ['Claude', 'GPT-5', 'GPT-6', 'Codex', 'Anthropic'],
      });
      setSavedNotice(false);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, [replaceDraft]);

  useEffect(() => { void load(); }, [load]);

  const words: SensitiveWordsSettings = {
    antigravitySensitiveWords: [...normalizedWords(draft.antigravity), ...(composer.antigravity.trim() ? [composer.antigravity.trim()] : [])],
    devinSensitiveWords: [...normalizedWords(draft.devin), ...(composer.devin.trim() ? [composer.devin.trim()] : [])],
  };
  const dirty = saved !== null && (
    JSON.stringify(words.antigravitySensitiveWords) !== JSON.stringify(saved.antigravitySensitiveWords)
    || JSON.stringify(words.devinSensitiveWords) !== JSON.stringify(saved.devinSensitiveWords)
  );

  useEffect(() => { onDirtyChange?.(dirty); }, [dirty, onDirtyChange]);

  const changeWord = (provider: Provider, id: number, value: string) => {
    setDraft((current) => ({
      ...current,
      [provider]: current[provider].map((entry) => entry.id === id ? { ...entry, value } : entry),
    }));
    setSavedNotice(false);
  };

  const composeWord = (provider: Provider, value: string) => {
    setComposer((current) => ({ ...current, [provider]: value }));
    setSavedNotice(false);
  };

  const addWords = (provider: Provider, value: string) => {
    const entries = value.split(/\r\n|\n|\r/).map((word) => word.trim()).filter(Boolean)
      .map((word) => ({ id: nextEntryId.current++, value: word }));
    if (entries.length === 0) return;
    setDraft((current) => ({ ...current, [provider]: [...current[provider], ...entries] }));
    setComposer((current) => ({ ...current, [provider]: '' }));
    setSavedNotice(false);
  };

  const removeWord = (provider: Provider, id: number) => {
    setDraft((current) => ({ ...current, [provider]: current[provider].filter((entry) => entry.id !== id) }));
    setSavedNotice(false);
  };

  const pasteWords = (provider: Provider, id: number, event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData('text');
    if (!/[\r\n]/.test(pasted)) return;
    event.preventDefault();
    const input = event.currentTarget;
    const lines = pasted.split(/\r\n|\n|\r/).filter((line) => line.trim());
    if (lines.length === 0) return;
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? start;
    const entries = lines.map((value, index) => ({ id: index === 0 ? id : nextEntryId.current++, value }));
    entries[0].value = input.value.slice(0, start) + entries[0].value;
    entries[entries.length - 1].value += input.value.slice(end);
    setDraft((current) => ({
      ...current,
      [provider]: current[provider].flatMap((entry) => entry.id === id ? entries : [entry]),
    }));
    setSavedNotice(false);
  };

  const save = async () => {
    if (!dirty || saving) return;
    setSaving(true);
    setError('');
    try {
      const result = await invoke<SensitiveWordsSettings>('save_core_sensitive_words_settings', {
        settings: words,
      });
      setSaved(result);
      replaceDraft(result);
      setSavedNotice(true);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  };

  const refresh = async () => {
    if (dirty && !await askConfirmation({
      title: t('config.sensitiveWords.discardTitle'),
      message: t('config.sensitiveWords.discardMessage'),
      confirmText: t('common.refresh'),
    })) return;
    await load();
  };

  return (
    <section className="panel config-sensitive-words-panel">
      <div className="config-panel-heading">
        <div className="config-heading-title">
          <ShieldAlert size={18} aria-hidden="true" />
          <h2>{t('config.sensitiveWords.title')}</h2>
        </div>
        <div className="config-heading-actions">
          {dirty ? <span className="state-pill">{t('config.network.unsaved')}</span>
            : savedNotice ? <span className="state-pill success">{t('config.network.saved')}</span> : null}
          <button type="button" className="icon-button quiet" title={t('common.refresh')} aria-label={t('common.refresh')}
            disabled={loading || saving} onClick={() => void refresh()}>
            <RefreshCw size={16} aria-hidden="true" />
          </button>
          <button type="button" className="primary-button compact-button" disabled={loading || saving || !dirty}
            onClick={() => void save()}>
            <Check size={16} aria-hidden="true" />
            {saving ? t('common.saving') : t('common.save')}
          </button>
        </div>
      </div>
      {error ? <div className="config-form-message error" role="alert">{error}</div> : null}
      {loading ? <p>{t('common.loading')}</p> : saved === null ? (
        <button type="button" className="secondary-button compact-button" onClick={() => void load()}>{t('common.retry')}</button>
      ) : (
        <div className="config-sensitive-words-grid">
          <WordEditor provider="antigravity" entries={draft.antigravity} composer={composer.antigravity}
            disabled={saving} onAdd={addWords} onCompose={composeWord} onChange={changeWord}
            onRemove={removeWord} onPaste={pasteWords} />
          <WordEditor provider="devin" entries={draft.devin} composer={composer.devin}
            disabled={saving} onAdd={addWords} onCompose={composeWord} onChange={changeWord}
            onRemove={removeWord} onPaste={pasteWords} />
        </div>
      )}
      {confirmationDialog}
    </section>
  );
}
