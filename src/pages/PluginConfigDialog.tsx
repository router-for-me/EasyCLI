import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { LoaderCircle, Settings2, X } from 'lucide-react';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import { useI18n } from '../i18n';
import { pluginConfigText, type PluginConfigMessage } from '../i18n/pluginConfig';
import { pluginsApi, type PluginConfigField, type PluginConfigObject, type PluginListEntry } from '../services/plugins';
import {
  buildPluginConfigDraft, buildPluginConfigPatch, normalizePluginConfigFieldType,
  type PluginConfigDraft, type PluginDraftValue,
} from '../services/pluginConfigDraft';
import './PluginConfigDialog.css';

function customConfig(config: PluginConfigObject): PluginConfigObject {
  return Object.fromEntries(Object.entries(config).filter(([key]) => key !== 'enabled' && key !== 'priority'));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function PluginConfigDialog({ plugin, onClose, onSaved }: {
  plugin: PluginListEntry;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t, locale } = useI18n();
  const pt = (key: PluginConfigMessage) => pluginConfigText(key, locale);
  const id = useId();
  const [draft, setDraft] = useState<PluginConfigDraft | null>(null);
  const [raw, setRaw] = useState('{}');
  const [original, setOriginal] = useState<PluginConfigObject>({});
  const [rawTouched, setRawTouched] = useState(false);
  const [rawError, setRawError] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [discard, setDiscard] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const request = useRef(0);
  const pending = useRef(false);
  const fields = useMemo(() => plugin.configFields.filter(field => field.name !== 'enabled' && field.name !== 'priority'), [plugin.configFields]);

  useEffect(() => {
    const generation = ++request.current;
    setLoading(true); setDraft(null); setError(''); setRawError(''); setDiscard(false);
    setRawTouched(false);
    void pluginsApi.getConfig(plugin.id).then(config => {
      if (request.current !== generation) return;
      setDraft(buildPluginConfigDraft({ enabled: plugin.enabled, configFields: fields }, config));
      const custom = customConfig(config);
      setOriginal(custom);
      setRaw(JSON.stringify(custom, null, 2));
    }).catch(reason => {
      if (request.current === generation) setError(errorMessage(reason));
    }).finally(() => {
      if (request.current === generation) setLoading(false);
    });
    return () => { request.current += 1; };
  }, [plugin.id, plugin.enabled, fields, attempt]);

  const dirty = Boolean(draft && (draft.enabledTouched || draft.priorityTouched || Object.values(draft.touchedFields).some(Boolean) || rawTouched));
  const close = () => {
    if (pending.current) return;
    if (dirty) setDiscard(true);
    else onClose();
  };
  const dialogRef = useDialogFocusTrap<HTMLElement>({
    onEscape: discard ? () => setDiscard(false) : close,
    preventEscape: saving,
  });
  const changeDraft = (updater: (current: PluginConfigDraft) => PluginConfigDraft) => {
    setDraft(current => current ? updater(current) : current);
    setError(''); setDiscard(false);
  };
  const changeField = (name: string, value: PluginDraftValue) => changeDraft(current => ({
    ...current,
    values: { ...current.values, [name]: value },
    touchedFields: { ...current.touchedFields, [name]: true },
    errors: { ...current.errors, [name]: '' },
  }));

  const save = async () => {
    if (!draft || loading || pending.current || !dirty) return;
    const { patch, errors } = buildPluginConfigPatch(draft, fields, key => pt(key.replace('plugin_management.', '') as PluginConfigMessage));
    const removeKeys: string[] = [];
    let nextRawError = '';
    if (!fields.length && rawTouched) {
      try {
        const parsed: unknown = JSON.parse(raw);
        if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) nextRawError = pt('expected_object');
        else if (Object.prototype.hasOwnProperty.call(parsed, 'enabled') || Object.prototype.hasOwnProperty.call(parsed, 'priority')) nextRawError = pt('reserved');
        else {
          const custom = parsed as PluginConfigObject;
          for (const key of new Set([...Object.keys(original), ...Object.keys(custom)])) {
            const removed = !Object.prototype.hasOwnProperty.call(custom, key);
            if (removed) removeKeys.push(key);
            else if (JSON.stringify(original[key]) !== JSON.stringify(custom[key]))
              Object.defineProperty(patch, key, { value: custom[key], enumerable: true, configurable: true, writable: true });
          }
        }
      } catch { nextRawError = pt('invalid_json'); }
    }
    setDraft(current => current ? { ...current, errors } : current);
    setRawError(nextRawError);
    if (Object.keys(errors).length || nextRawError) {
      setError(pt('invalid'));
      window.requestAnimationFrame(() => dialogRef.current?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus());
      return;
    }
    if (!Object.keys(patch).length && !removeKeys.length) { onClose(); return; }
    pending.current = true;
    setSaving(true); setError('');
    const generation = request.current;
    try {
      await pluginsApi.patchConfig(plugin.id, patch, !fields.length && rawTouched ? { nullMeansDelete: false, removeKeys } : undefined);
      if (request.current === generation) { onSaved(); onClose(); }
    } catch (reason) {
      if (request.current === generation) setError(`${pt('saveFailed')}: ${errorMessage(reason)}`);
    } finally {
      pending.current = false;
      if (request.current === generation) setSaving(false);
    }
  };

  const renderField = (field: PluginConfigField, index: number) => {
    if (!draft) return null;
    const type = normalizePluginConfigFieldType(field);
    const value = draft.values[field.name];
    const fieldId = `${id}-field-${index}`;
    const fieldError = Object.prototype.hasOwnProperty.call(draft.errors, field.name) ? draft.errors[field.name] : '';
    const inputProps = {
      id: fieldId,
      'aria-invalid': Boolean(fieldError),
      'aria-describedby': `${fieldId}-hint${fieldError ? ` ${fieldId}-error` : ''}`,
    };
    return <div className="plugin-config-field" key={field.name}>
      <label htmlFor={fieldId}><span>{field.name}</span><code>{type}</code></label>
      {type === 'boolean' ? <select {...inputProps} value={draft.touchedFields[field.name] !== true && typeof original[field.name] !== 'boolean' ? '' : value === true ? 'true' : 'false'} onChange={event => changeField(field.name, event.currentTarget.value === 'true')}>
        <option value="" disabled>{pt('inherit')}</option>
        <option value="true">{t('common.enable')}</option><option value="false">{t('common.disable')}</option>
      </select> : type === 'enum' ? <select {...inputProps} value={typeof value === 'string' ? value : ''} onChange={event => changeField(field.name, event.currentTarget.value)}>
        <option value="">{pt('inherit')}</option>
        {typeof value === 'string' && value && !field.enumValues.includes(value) ? <option value={value}>{value}</option> : null}
        {field.enumValues.map(option => <option key={option} value={option}>{option}</option>)}
      </select> : type === 'array' || type === 'object' ? <textarea {...inputProps} rows={5} spellCheck={false} autoComplete="off" value={typeof value === 'string' ? value : ''} placeholder={type === 'array' ? '[]' : '{}'} onChange={event => changeField(field.name, event.currentTarget.value)} />
        : <input {...inputProps} type="text" inputMode={type === 'number' ? 'decimal' : type === 'integer' ? 'numeric' : undefined} autoComplete="off" spellCheck={false} value={typeof value === 'string' ? value : ''} onChange={event => changeField(field.name, event.currentTarget.value)} />}
      <small id={`${fieldId}-hint`}>{field.description}</small>
      {fieldError ? <small className="plugin-config-error" id={`${fieldId}-error`}>{fieldError}</small> : null}
    </div>;
  };

  const content = <div className="config-dialog-backdrop plugin-config-backdrop" onMouseDown={event => { if (event.currentTarget === event.target) close(); }}>
    <section ref={dialogRef} className="config-dialog plugin-config-dialog" role="dialog" aria-modal="true" aria-labelledby={`${id}-title`} aria-busy={loading || saving}>
      <header className="config-dialog-heading">
        <div><h2 id={`${id}-title`}><Settings2 size={20} aria-hidden="true" />{pt('title')}</h2><p>{plugin.metadata?.name || plugin.id}</p></div>
        <button type="button" className="icon-button quiet" disabled={saving} onClick={close} aria-label={t('common.close')}><X size={18} /></button>
      </header>
      <form onSubmit={event => { event.preventDefault(); void save(); }} noValidate>
        <div className="plugin-config-body">
          {loading ? <p className="plugin-config-loading" role="status"><LoaderCircle size={20} className="spin" />{t('common.loading')}</p> : draft ? <fieldset disabled={saving}>
            <div className="plugin-config-common">
              <label className="plugin-config-toggle"><input type="checkbox" checked={draft.enabled} onChange={event => { const checked = event.currentTarget.checked; changeDraft(current => ({ ...current, enabled: checked, enabledTouched: true })); }} /><span>{pt('enabled')}</span></label>
              <small>{pt('enabledHint')}</small>
              <div className="plugin-config-field">
                <label htmlFor={`${id}-priority`}>{pt('priority')}</label>
                <input id={`${id}-priority`} type="text" inputMode="numeric" value={draft.priority} aria-invalid={Boolean(draft.errors.priority)} aria-describedby={`${id}-priority-hint${draft.errors.priority ? ` ${id}-priority-error` : ''}`} onChange={event => { const value = event.currentTarget.value; changeDraft(current => ({ ...current, priority: value, priorityTouched: true, errors: { ...current.errors, priority: '' } })); }} />
                <small id={`${id}-priority-hint`}>{pt('priorityHint')}</small>
                {draft.errors.priority ? <small className="plugin-config-error" id={`${id}-priority-error`}>{draft.errors.priority}</small> : null}
              </div>
            </div>
            {fields.length ? <section className="plugin-config-parameters"><h3>{pt('fields')}</h3><p>{pt('fieldsHint')}</p>{fields.map(renderField)}</section> : <div className="plugin-config-field plugin-config-parameters">
              <label htmlFor={`${id}-raw`}>{pt('raw')}</label>
              <small id={`${id}-raw-hint`}>{pt('rawHint')}</small>
              <textarea id={`${id}-raw`} rows={10} value={raw} spellCheck={false} autoComplete="off" aria-invalid={Boolean(rawError)} aria-describedby={`${id}-raw-hint${rawError ? ` ${id}-raw-error` : ''}`} onChange={event => { setRaw(event.currentTarget.value); setRawTouched(true); setRawError(''); setError(''); setDiscard(false); }} />
              {rawError ? <small className="plugin-config-error" id={`${id}-raw-error`}>{rawError}</small> : null}
            </div>}
          </fieldset> : null}
        </div>
        <footer className="plugin-config-footer">
          {error ? <p className="plugin-config-error" role="alert">{!draft && !loading ? `${pt('loadFailed')}: ` : ''}{error}</p> : null}
          {discard ? <div className="plugin-config-discard" role="alert"><span>{pt('unsaved')}</span><div><button type="button" className="secondary-button" onClick={() => setDiscard(false)}>{pt('keepEditing')}</button><button type="button" className="danger-button" onClick={onClose}>{pt('discard')}</button></div></div> : null}
          <div className="plugin-config-actions">
            <button type="button" className="secondary-button" disabled={saving} onClick={close}>{t('common.cancel')}</button>
            {!loading && !draft ? <button type="button" className="primary-button" onClick={() => setAttempt(value => value + 1)}>{pt('retry')}</button> : <button type="submit" className="primary-button" disabled={loading || saving || !dirty}>{saving ? <LoaderCircle size={16} className="spin" /> : null}{t(saving ? 'common.saving' : 'common.save')}</button>}
          </div>
        </footer>
      </form>
    </section>
  </div>;
  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
