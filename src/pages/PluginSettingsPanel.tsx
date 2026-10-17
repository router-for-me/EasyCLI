import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useI18n } from '../i18n';
import { pluginText } from '../i18n/plugins';
import { isRecord } from '../services/managementApi';
import { safePluginWebURL } from '../services/pluginResources';
import { pluginsApi, type PluginSettings } from '../services/plugins';

export function PluginSettingsPanel({ onSaved, globalControl }: { onSaved: () => void; globalControl: ReactNode }) {
  const { t, locale } = useI18n();
  const pt = (key: Parameters<typeof pluginText>[0]) => pluginText(key, locale);
  const [original, setOriginal] = useState<PluginSettings | null>(null);
  const [directory, setDirectory] = useState('');
  const [sources, setSources] = useState('');
  const [auth, setAuth] = useState('[]');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const pending = useRef(false);
  const generation = useRef(0);
  useEffect(() => {
    const current = ++generation.current;
    setError('');
    void pluginsApi.getSettings().then(data => {
      if (generation.current !== current) return;
      setOriginal(data); setDirectory(data.dir); setSources(data.storeSources.join('\n')); setAuth(JSON.stringify(data.storeAuth, null, 2));
    }).catch(e => { if (generation.current === current) setError(String(e)); });
    return () => { ++generation.current; };
  }, [attempt]);
  const save = async () => {
    if (pending.current || !original) return;
    setError('');
    const list = sources.split('\n').map(value => value.trim()).filter(Boolean);
    let rules: unknown;
    try { rules = JSON.parse(auth); } catch { setError(pt('invalidSettings')); return; }
    if (!directory.trim() || list.some(url => !safePluginWebURL(url)) || !Array.isArray(rules) || !rules.every(isRecord)) { setError(pt('invalidSettings')); return; }
    const changes: Partial<PluginSettings> = {};
    if (directory.trim() !== original.dir) changes.dir = directory.trim();
    if (JSON.stringify(list) !== JSON.stringify(original.storeSources)) changes.storeSources = list;
    if (JSON.stringify(rules) !== JSON.stringify(original.storeAuth)) changes.storeAuth = rules;
    pending.current = true; setBusy(true);
    const current = generation.current;
    try {
      if (Object.keys(changes).length) await pluginsApi.updateSettings(changes);
      if (current !== generation.current) return;
      setOriginal({ ...original, ...changes }); onSaved();
    } catch (e) { if (current === generation.current) setError(String(e)); }
    finally { pending.current = false; if (current === generation.current) setBusy(false); }
  };
  return <form className="plugin-settings" onSubmit={e => { e.preventDefault(); void save(); }}>
    {error && <p className="plugin-error" role="alert">{error}</p>}
    {!original ? (error ? <button className="secondary-button" type="button" onClick={() => setAttempt(n => n + 1)}>{pt('retry')}</button> : <p className="plugin-loading" role="status">{t('common.loading')}</p>) : <>
      <fieldset className="plugin-settings-card" disabled={busy}>
        <legend>{pt('storageSettings')}</legend>
        <div className="plugin-settings-columns plugin-settings-basics">{globalControl}
        <label className="plugin-field">{pt('directory')}<input value={directory} onChange={e => setDirectory(e.target.value)} /></label></div>
      </fieldset>
      <fieldset className="plugin-settings-card" disabled={busy}>
        <legend>{pt('registrySettings')}</legend>
        <div className="plugin-settings-columns">
        <label className="plugin-field">{pt('sources')}<textarea rows={5} value={sources} onChange={e => setSources(e.target.value)} spellCheck={false} /><small>{pt('sourcesHint')}</small></label>
        <label className="plugin-field">{pt('auth')}<textarea rows={5} value={auth} onChange={e => setAuth(e.target.value)} spellCheck={false} /><small>{pt('authHint')}</small></label>
        </div>
      </fieldset>
      <div className="plugin-settings-actions"><button className="primary-button" type="submit" disabled={busy}>{busy ? t('common.loading') : t('common.save')}</button></div>
    </>}
  </form>;
}
