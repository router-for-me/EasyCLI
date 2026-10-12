import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, LoaderCircle, Settings2, X } from 'lucide-react';
import { useI18n } from '../i18n';
import { managementApi } from '../services/managementApi';
import { loadAuthFileSettings, saveAuthFileSettings, type AuthFileSettingsDraft, type BooleanOverride } from '../services/authFileSettings';
import { modelMatchesRule, normalizeOAuthExcludedRules, oauthModelCandidates, oauthModelsFromPayload, setOAuthModelsExcluded, type OAuthModelDefinition } from '../services/oauthModels';
import './AuthFileSettingsDialog.css';
import { CredentialAdvancedFields, CredentialHeadersEditor, credentialProviderKey } from './CredentialAdvancedFields';
import { modelSearchText, type ModelOption } from '../services/modelService';

export function AuthFileSettingsDialog({ name, displayName = name, provider = '', onClose, onSaved }: {
  name: string;
  /** Privacy-masked name for display; `name` stays the request key. */
  displayName?: string;
  provider?: string;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useI18n();
  const id = useId();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const savingRef = useRef(false);
  const [original, setOriginal] = useState<AuthFileSettingsDraft | null>(null);
  const [draft, setDraft] = useState<AuthFileSettingsDraft | null>(null);
  const [models, setModels] = useState<OAuthModelDefinition[]>([]);
  const [pickerModels, setPickerModels] = useState<ModelOption[]>([]);
  const [catalogError, setCatalogError] = useState('');
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const dirty = draft !== null && (Boolean(draft.normalizeCloakMetadata?.length) || JSON.stringify(draft) !== JSON.stringify(original));

  useEffect(() => {
    const previousFocus = document.activeElement;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    const dialog = dialogRef.current;
    dialog?.showModal();
    return () => {
      dialog?.close();
      document.body.style.overflow = previousOverflow;
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, []);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError('');
    setCatalogLoading(true);
    setCatalogError('');
    void loadAuthFileSettings(name).then((value) => {
      if (active) { setOriginal(value); setDraft(value); }
    }).catch((reason: unknown) => {
      if (active) setError(reason instanceof Error ? reason.message : String(reason));
    }).finally(() => { if (active) setLoading(false); });
    void managementApi.get('/credentials/models', { name }).then((payload) => {
      const catalog = oauthModelsFromPayload(payload);
      if (!active) return;
      setModels(catalog);
      setPickerModels(catalog.map((model) => ({ name: model.id, alias: model.displayName })));
    }).catch((reason: unknown) => {
      if (active) setCatalogError(reason instanceof Error ? reason.message : String(reason));
    }).finally(() => { if (active) setCatalogLoading(false); });
    return () => { active = false; };
  }, [name, attempt]);

  const close = () => {
    if (savingRef.current) return;
    onClose();
  };
  const update = <K extends keyof AuthFileSettingsDraft>(key: K, value: AuthFileSettingsDraft[K]) => {
    setDraft((current) => current ? { ...current, [key]: value } : current);
    setError('');
  };
  const save = async () => {
    if (!draft || !original || savingRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setError('');
    try {
      const changed = await saveAuthFileSettings(name, original, draft);
      if (changed) onSaved();
      else onClose();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };
  const rules = normalizeOAuthExcludedRules((draft?.excluded_models ?? '').split(/\r?\n/));
  const query = search.trim().toLowerCase();
  const candidates = oauthModelCandidates(models, rules).filter((model) => modelSearchText({ name: model.id, displayName: model.displayName }).includes(query));
  const resolvedProvider = credentialProviderKey(name, provider);
  const showWebsockets = resolvedProvider === 'codex' || resolvedProvider === 'xai' || draft?.websockets === 'true' || draft?.websockets === 'false';
  const textField = (key: 'prefix' | 'proxy_url' | 'priority' | 'weight') => (
    <label className="credential-settings-field">
      <span>{t(`authFiles.settings.${key}`)} <code>{key}</code></span>
      <input value={draft?.[key] ?? ''} onChange={(event) => update(key, event.currentTarget.value)}
        inputMode={key === 'priority' || key === 'weight' ? 'numeric' : undefined}
        placeholder={key === 'priority' ? '0' : key === 'weight' ? '1' : key === 'proxy_url' ? 'socks5://127.0.0.1:1080' : ''}
        autoComplete="off" spellCheck={false} />
      <small>{t(`authFiles.settings.${key}Hint`)}</small>
    </label>
  );
  const booleanField = (key: 'disable_cooling' | 'websockets') => (
    <label className="credential-settings-field">
      <span>{t(`authFiles.settings.${key}`)} <code>{key}</code></span>
      <select value={draft?.[key] ?? ''} onChange={(event) => update(key, event.currentTarget.value as BooleanOverride)}>
        <option value="">{t('authFiles.settings.inherit')}</option>
        <option value="true">{t('common.enable')}</option>
        <option value="false">{t('common.disable')}</option>
      </select>
      <small>{t(`authFiles.settings.${key}Hint`)}</small>
    </label>
  );

  const content = (
    <dialog ref={dialogRef} className="credential-settings-dialog" aria-labelledby={`${id}-title`}
      onCancel={(event) => { event.preventDefault(); close(); }} onKeyDown={(event) => {
        if (event.key !== 'Tab') return;
        const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('button, input, select, textarea, summary'))
          .filter((element) => !element.matches(':disabled') && element.getClientRects().length > 0);
        const first = controls[0];
        const last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }}>
      <form onSubmit={(event) => { event.preventDefault(); void save(); }}>
        <header className="credential-settings-heading">
          <div><h2 id={`${id}-title`}><Settings2 size={20} />{t('authFiles.settings.title')}</h2><p>{displayName}</p></div>
          <button type="button" className="icon-button quiet" disabled={saving} onClick={close} aria-label={t('common.close')}><X size={18} /></button>
        </header>
        <div className="credential-settings-body" aria-busy={loading}>
          {loading ? <div className="credential-settings-loading"><LoaderCircle size={20} className="spin" />{t('common.loading')}</div> : draft ? (
            <fieldset disabled={saving}>
              <section className="credential-settings-section">
                <h3>{t('authFiles.settings.routing')}</h3>
                <div className="credential-settings-grid">{textField('prefix')}{textField('proxy_url')}{textField('priority')}{textField('weight')}{booleanField('disable_cooling')}{showWebsockets ? booleanField('websockets') : null}</div>
              </section>
              <section className="credential-settings-section">
                <h3>{t('authFiles.settings.excluded_models')} <code>excluded_models</code></h3>
                <label className="credential-settings-field">
                  <span>{t('authFiles.settings.rules')}</span>
                  <textarea rows={3} value={draft.excluded_models} onChange={(event) => update('excluded_models', event.currentTarget.value)} spellCheck={false} placeholder={'gpt-example\nclaude-*'} />
                  <small>{t('authFiles.settings.excludedHint')}</small>
                </label>
                <details className="credential-settings-catalog">
                  <summary>{t('authFiles.settings.catalog', { count: models.length })}</summary>
                  {catalogLoading ? <p role="status">{t('common.loading')}</p> : null}
                  {catalogError ? <p className="credential-settings-warning">{t('authFiles.settings.catalogError')}<small>{catalogError}</small></p> : null}
                  <input aria-label={t('authFiles.settings.searchModels')} placeholder={t('authFiles.settings.searchModels')} value={search} onChange={(event) => setSearch(event.currentTarget.value)} />
                  <div className="credential-settings-models">
                    {candidates.map((model) => {
                      const wildcard = rules.some((rule) => rule.includes('*') && modelMatchesRule(model.id, rule));
                      return <label key={model.id} title={wildcard ? t('authFiles.settings.wildcard') : model.displayName}>
                        <input type="checkbox" checked={rules.some((rule) => modelMatchesRule(model.id, rule))} disabled={wildcard}
                          onChange={(event) => update('excluded_models', setOAuthModelsExcluded(rules, [model], event.currentTarget.checked).join('\n'))} />
                        <span>{model.id}{wildcard ? <small>{t('authFiles.settings.wildcard')}</small> : null}</span>
                      </label>;
                    })}
                    {!catalogLoading && !candidates.length ? <p>{t('authFiles.settings.noModels')}</p> : null}
                  </div>
                </details>
              </section>
              <CredentialAdvancedFields name={name} provider={provider} advanced={draft.advanced} models={pickerModels} modelsLoading={catalogLoading} modelsError={catalogError} disabled={saving} onChange={(advanced) => update('advanced', advanced)} />
              <section className="credential-settings-section">
                <h3>{t('authFiles.settings.additional')}</h3>
                <CredentialHeadersEditor value={draft.headers} disabled={saving} onChange={(headers) => update('headers', headers)} />
                <label className="credential-settings-field">
                  <span>{t('authFiles.settings.note')} <code>note</code></span>
                  <textarea rows={2} value={draft.note} onChange={(event) => update('note', event.currentTarget.value)} />
                </label>
              </section>
            </fieldset>
          ) : null}
        </div>
        <footer className="credential-settings-footer">
          {error ? <p className="credential-settings-error" role="alert">{error}</p> : null}
          <div className="credential-settings-actions">
            <button type="button" className="secondary-button" disabled={saving} onClick={close}>{t('common.cancel')}</button>
            {!loading && !draft ? <button type="button" className="primary-button" onClick={() => setAttempt((value) => value + 1)}>{t('common.refresh')}</button> : <button type="submit" className="primary-button" disabled={!draft || loading || saving || !dirty}>{saving ? <LoaderCircle size={16} className="spin" /> : <Check size={16} />}{t(saving ? 'common.saving' : 'common.save')}</button>}
          </div>
        </footer>
      </form>
    </dialog>
  );
  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
