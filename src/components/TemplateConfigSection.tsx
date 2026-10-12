import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { ExternalLink, Eye, EyeOff, RotateCcw, Save } from 'lucide-react';
import { useI18n } from '../i18n';
import { templateMessages, templateText } from '../i18n/templateConfig';
import { useCoreRuntime, type CoreStatus } from '../coreRuntime';
import { SettingsHelp } from './SettingsHelp';
import {
  applyTemplateChanges, draftTemplateField, readTemplatePath, templateFieldKey, templateFieldValidation, templateFieldSaveValue,
  type TemplateConfigChange, type TemplateConfigDraft, type TemplateConfigField, type TemplateConfigGroup, type TemplateConfigSaveResult, type TemplateText,
} from '../services/templateConfig';
import '../styles/template-config.css';

const restartHint = { zh: '需重启内核', en: 'Core restart required', ja: 'コアの再起動が必要' } satisfies TemplateText;

export function TemplateConfigSection({ groups, visibleGroups, onDirtyGroupsChange }: {
  groups: readonly TemplateConfigGroup[];
  visibleGroups?: readonly string[];
  onDirtyGroupsChange?: (groupIds: readonly string[]) => void;
}) {
  const { locale, t } = useI18n();
  const { status, publishStatus, refreshStatus } = useCoreRuntime();
  const [config, setConfig] = useState<Record<string, unknown> | null>(null);
  const [drafts, setDrafts] = useState<Record<string, TemplateConfigDraft>>({});
  const [loadError, setLoadError] = useState('');
  const [groupErrors, setGroupErrors] = useState<Record<string, string>>({});
  const [saved, setSaved] = useState<Record<string, boolean>>({});
  const [restartNeeded, setRestartNeeded] = useState(false);
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const mounted = useRef(true);
  const revision = useRef(0);
  const reportedDirtyGroups = useRef<string | null>(null);
  const tr = (message: keyof typeof templateMessages) => templateText(templateMessages[message], locale);

  useEffect(() => {
    if (!onDirtyGroupsChange) return;
    const dirtyGroups = groups.filter((group) => group.fields.some((field) => drafts[templateFieldKey(field.path)])).map((group) => group.id);
    const signature = JSON.stringify(dirtyGroups);
    if (reportedDirtyGroups.current === signature) return;
    reportedDirtyGroups.current = signature;
    onDirtyGroupsChange(dirtyGroups);
  }, [drafts, groups, onDirtyGroupsChange]);

  const reload = useCallback(async () => {
    const request = ++revision.current;
    try {
      const value = await invoke<Record<string, unknown>>('get_extended_core_config');
      if (mounted.current && request === revision.current) { setConfig(value); setLoadError(''); }
    } catch (error) {
      if (mounted.current && request === revision.current) setLoadError(String(error));
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let stop: (() => void) | undefined;
    void reload();
    void listen('config-files-changed', () => { if (!disposed) void reload(); }).then((unlisten) => {
      if (!disposed) stop = unlisten;
      else unlisten();
    }).catch((error) => { if (!disposed) setLoadError(String(error)); });
    return () => { disposed = true; mounted.current = false; ++revision.current; stop?.(); };
  }, [reload]);

  const changeField = (group: TemplateConfigGroup, field: TemplateConfigField, value: unknown, remove = false) => {
    if (!config) return;
    const key = templateFieldKey(field.path);
    setDrafts((previous) => {
      const next = { ...previous };
      const draft = draftTemplateField(field, config, previous[key], value, remove);
      if (draft) next[key] = draft;
      else delete next[key];
      return next;
    });
    setGroupErrors((previous) => ({ ...previous, [group.id]: '' }));
    setSaved((previous) => ({ ...previous, [group.id]: false }));
  };

  const discardGroup = (group: TemplateConfigGroup) => {
    setDrafts((previous) => {
      const next = { ...previous };
      for (const field of group.fields) delete next[templateFieldKey(field.path)];
      return next;
    });
    setGroupErrors((previous) => ({ ...previous, [group.id]: '' }));
    setSaved((previous) => ({ ...previous, [group.id]: false }));
  };

  const saveGroup = async (group: TemplateConfigGroup) => {
    if (!config || busy) return;
    const changes: TemplateConfigChange[] = [];
    for (const field of group.fields) {
      const draft = drafts[templateFieldKey(field.path)];
      if (!draft) continue;
      const error = !draft.remove && templateFieldValidation(field, draft.value);
      if (error) {
        setGroupErrors((previous) => ({ ...previous, [group.id]: `${templateText(field.label, locale)}: ${templateText(error, locale)}` }));
        return;
      }
      changes.push({ ...draft, path: [...field.path], value: draft.remove ? null : templateFieldSaveValue(field, draft.value) });
    }
    if (!changes.length) return;
    const groupError = group.validate?.(applyTemplateChanges(config, changes));
    if (groupError) { setGroupErrors((previous) => ({ ...previous, [group.id]: templateText(groupError, locale) })); return; }
    setBusy(group.id);
    setGroupErrors((previous) => ({ ...previous, [group.id]: '' }));
    try {
      const result = await invoke<TemplateConfigSaveResult>('save_extended_core_config', { changes });
      ++revision.current;
      setConfig(result.config);
      setLoadError('');
      setDrafts((previous) => {
        const next = { ...previous };
        for (const change of changes) delete next[templateFieldKey(change.path)];
        return next;
      });
      setSaved((previous) => ({ ...previous, [group.id]: true }));
      if (result.restartRequired) setRestartNeeded(true);
    } catch (error) {
      setGroupErrors((previous) => ({ ...previous, [group.id]: String(error) }));
      // Refresh untouched values while retaining both the draft and its original conflict check.
      await reload();
    } finally { if (mounted.current) setBusy(null); }
  };

  const restart = async () => {
    setBusy('restart');
    try { publishStatus(await invoke<CoreStatus>('restart_core_process')); setRestartNeeded(false); setLoadError(''); }
    catch (error) { setLoadError(String(error)); await refreshStatus(); }
    finally { setBusy(null); }
  };

  const chooseDirectory = async (group: TemplateConfigGroup, field: TemplateConfigField) => {
    setBusy('directory');
    try {
      const selected = await open({ directory: true, multiple: false, title: templateText(field.label, locale) });
      if (mounted.current && typeof selected === 'string') changeField(group, field, selected);
    } catch (error) {
      if (mounted.current) setGroupErrors((previous) => ({ ...previous, [group.id]: String(error) }));
    } finally { if (mounted.current) setBusy(null); }
  };

  const control = (group: TemplateConfigGroup, field: TemplateConfigField, value: unknown, id: string, invalid: boolean) => {
    const disabled = busy !== null || config === null;
    const onChange = (next: unknown) => changeField(group, field, next);
    const common = { id, disabled, 'aria-invalid': invalid || undefined, 'aria-describedby': invalid ? `${id}-error` : undefined };
    if (field.type === 'custom' && field.render) return field.render({ value, onChange, disabled, id });
    if (field.type === 'boolean') return <label className="switch-control template-config-switch"><input {...common} type="checkbox" checked={value === true} onChange={(event) => onChange(event.currentTarget.checked)} /><span className="switch-track" /></label>;
    if (field.type === 'select') {
      const selected = field.options?.findIndex((option) => Object.is(option.value, value)) ?? -1;
      return <select {...common} className="config-network-input" value={String(selected)} onChange={(event) => onChange(field.options?.[Number(event.currentTarget.value)]?.value)}>
        {selected < 0 ? <option value="-1">{String(value ?? '')}</option> : null}
        {field.options?.map((option, index) => <option key={index} value={String(index)}>{templateText(option.label, locale)}</option>)}
      </select>;
    }
    if (field.type === 'string-list' || field.type === 'json') return <textarea {...common} className="config-network-input template-config-textarea" rows={field.rows ?? (field.type === 'json' ? 6 : 3)}
      value={field.type === 'string-list' ? Array.isArray(value) ? value.join('\n') : '' : typeof value === 'string' ? value : JSON.stringify(value ?? {}, null, 2)}
      placeholder={field.placeholder}
      onChange={(event) => onChange(field.type === 'string-list' ? event.currentTarget.value.split('\n') : event.currentTarget.value)}
      onBlur={() => { if (field.type === 'string-list' && Array.isArray(value)) { const clean = value.map(String).map((item) => item.trim()).filter(Boolean); if (JSON.stringify(clean) !== JSON.stringify(value)) onChange(clean); } }} />;
    const input = <input {...common} className="config-network-input" type={field.type === 'number' ? 'number' : field.sensitive && !revealed[id] ? 'password' : 'text'} autoComplete={field.sensitive ? 'new-password' : undefined}
      min={field.min} max={field.max} step={field.step ?? 1} placeholder={field.placeholder} value={typeof value === 'string' || typeof value === 'number' ? value : ''}
      onChange={(event) => onChange(field.type === 'number' && event.currentTarget.value !== '' ? Number(event.currentTarget.value) : event.currentTarget.value)} />;
    const secret = field.sensitive ? <div className="config-secret-input template-config-secret">{input}<button type="button" className="icon-button quiet" disabled={disabled} onClick={() => setRevealed((previous) => ({ ...previous, [id]: !previous[id] }))}
      title={t(revealed[id] ? 'config.keys.hide' : 'config.keys.show')} aria-label={t(revealed[id] ? 'config.keys.hide' : 'config.keys.show')}>{revealed[id] ? <EyeOff size={16} aria-hidden="true" /> : <Eye size={16} aria-hidden="true" />}</button></div> : input;
    if (field.link) {
      const link = field.link;
      return <div className="template-config-directory">{secret}<button type="button" className="secondary-button compact-button" onClick={() => void invoke('open_external_url', { url: link.url }).catch((error) => setGroupErrors((previous) => ({ ...previous, [group.id]: String(error) })))}><ExternalLink size={15} aria-hidden="true" />{templateText(link.label, locale)}</button></div>;
    }
    return field.directory ? <div className="template-config-directory">{secret}<button type="button" className="secondary-button compact-button" disabled={disabled} onClick={() => void chooseDirectory(group, field)} aria-label={`${tr('chooseDirectory')}: ${templateText(field.label, locale)}`}>{tr('chooseDirectory')}</button></div> : secret;
  };

  const visible = groups.filter((group) => !visibleGroups || visibleGroups.includes(group.id));
  return <div className="template-config-section" hidden={visible.length === 0}>
    {loadError ? <div className="template-config-notice template-config-error" role="alert"><span>{loadError}</span><button type="button" className="secondary-button compact-button" disabled={busy !== null} onClick={() => void reload()}>{tr('reload')}</button></div> : null}
    {!config && !loadError ? <p role="status">{tr('loading')}</p> : null}
    {restartNeeded ? <div className="template-config-notice" role="status"><span>{tr('restart')}</span>{status?.running ? <button type="button" className="secondary-button compact-button" disabled={busy !== null} onClick={() => void restart()}>{tr('restartNow')}</button> : null}</div> : null}
    {groups.map((group) => {
      const dirty = group.fields.some((field) => drafts[templateFieldKey(field.path)]);
      const needsRestart = group.fields.some((field) => field.restart);
      return <section key={group.id} id={`config-group-${group.id}`} tabIndex={-1} className={`config-card template-config-card${group.fields.some(field => ['custom', 'json', 'string-list'].includes(field.type)) ? ' config-settings-wide' : ''}${dirty ? ' template-config-card-dirty' : ''}`} hidden={!visible.includes(group)} aria-labelledby={`template-group-${group.id}`}>
        <div className="template-config-heading"><div className="template-config-heading-main"><h3 id={`template-group-${group.id}`}>{templateText(group.title, locale)}</h3>{group.description ? <SettingsHelp label={templateText(group.title, locale)}>{templateText(group.description, locale)}</SettingsHelp> : null}</div>{needsRestart ? <span className="template-config-restart-hint"><RotateCcw size={12} aria-hidden="true" />{templateText(restartHint, locale)}</span> : null}</div>
        <div className="template-config-fields">{group.fields.map((field, index) => {
          const key = templateFieldKey(field.path);
          const original = readTemplatePath(config, field.path);
          const draft = drafts[key];
          const exists = draft ? !draft.remove : original.exists;
          const value = draft && !draft.remove ? draft.value : draft?.remove || !original.exists ? field.defaultValue : original.value;
          const validation = draft && !draft.remove ? templateFieldValidation(field, value) : null;
          const id = `template-${group.id}-${index}`;
          const description = [templateText(field.description, locale), field.type === 'string-list' ? tr('list') : ''].filter(Boolean).join(' ');
          const resetLabel = `${tr('default')}: ${templateText(field.label, locale)}`;
          return <div key={key} id={`template-field-${group.id}-${index}`} data-field-type={field.type} tabIndex={-1} className={`template-config-field${field.type === 'custom' || field.type === 'json' || field.type === 'string-list' ? ' template-config-field-wide' : ''}${field.type === 'boolean' ? ' template-config-toggle' : ''}${draft ? ' template-config-field-dirty' : ''}`}>
            <div className="template-config-label">{field.type === 'custom' ? <span id={`${id}-label`}>{templateText(field.label, locale)}</span> : <label htmlFor={id}>{templateText(field.label, locale)}</label>}<div className="template-config-label-tools">{description ? <SettingsHelp label={templateText(field.label, locale)}>{description}</SettingsHelp> : null}{exists ? <button type="button" className="template-config-reset" disabled={busy !== null || !config} title={resetLabel} aria-label={resetLabel} onClick={() => changeField(group, field, null, true)}><RotateCcw size={14} aria-hidden="true" /></button> : null}</div></div>
            {field.type === 'custom' ? <details className="template-config-custom"><summary aria-labelledby={`${id}-label ${id}-edit`} aria-describedby={validation ? `${id}-error` : undefined}><span id={`${id}-edit`}>{tr('editDetails')}</span></summary><div className="template-config-control" role="group" aria-labelledby={`${id}-label`}>{control(group, field, value, id, Boolean(validation))}</div></details> : <div className="template-config-control">{control(group, field, value, id, Boolean(validation))}</div>}
            {field.link?.hint ? <small className="template-config-link-hint">{templateText(field.link.hint, locale)}</small> : null}
            {validation ? <small id={`${id}-error`} className="template-config-error" role="alert">{templateText(validation, locale)}</small> : null}
          </div>;
        })}</div>
        {groupErrors[group.id] ? <p className="template-config-error" role="alert">{groupErrors[group.id]}</p> : null}
        <div className="template-config-actions"><span className={dirty ? 'template-config-action-status-dirty' : saved[group.id] ? 'template-config-action-status-saved' : undefined} role="status">{dirty ? tr('dirty') : saved[group.id] ? tr('saved') : ''}</span><div>{dirty ? <button type="button" className="secondary-button compact-button" disabled={busy !== null} onClick={() => discardGroup(group)}>{tr('discard')}</button> : null}<button type="button" className="primary-button compact-button" disabled={!dirty || busy !== null || !config} onClick={() => void saveGroup(group)}><Save size={15} aria-hidden="true" />{busy === group.id ? tr('saving') : tr('save')}</button></div></div>
      </section>;
    })}
  </div>;
}
