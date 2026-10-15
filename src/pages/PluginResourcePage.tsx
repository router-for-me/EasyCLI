import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, ExternalLink } from 'lucide-react';
import { useI18n } from '../i18n';
import { pluginText } from '../i18n/plugins';
import type { PluginResourceEntry } from '../services/pluginResources';
import './PluginsPage.css';

export function PluginResourceView({ entry, url, onBack, onError }: {
  entry: PluginResourceEntry; url: string; onBack: () => void; onError: (error: string) => void;
}) {
  const { locale } = useI18n();
  return <section className="plugin-resource">
    <div className="plugin-toolbar">
      <button className="secondary-button" onClick={onBack}><ArrowLeft size={16} />{pluginText('back', locale)}</button>
      <strong>{entry.label}</strong>
      <button className="secondary-button" onClick={() => void invoke('open_external_url', { url }).catch(error => onError(String(error)))}>
        <ExternalLink size={16} />{pluginText('openExternal', locale)}
      </button>
    </div>
    <iframe key={url} src={url} title={entry.label} referrerPolicy="no-referrer"
      sandbox="allow-scripts allow-same-origin allow-forms allow-popups" allow="clipboard-read; clipboard-write" />
  </section>;
}

export function PluginResourcePage({ entry, onBack }: { entry: PluginResourceEntry; onBack: () => void }) {
  const { t, locale } = useI18n();
  const [url, setURL] = useState('');
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let disposed = false;
    setURL(''); setError('');
    void invoke<string>('get_plugin_resource_url', { pluginId: entry.pluginID, menuIndex: entry.menuIndex })
      .then(value => { if (!disposed) {
        // The native command rechecks the live catalog. Never open another page
        // if its menu moved between the sidebar refresh and the click.
        const resolved = new URL(value);
        // Browser demo IPC returns data URLs; native IPC returns HTTP(S) only.
        if (resolved.protocol === 'http:' || resolved.protocol === 'https:') {
          const expected = new URL(entry.menu.path, resolved.origin);
          if (resolved.href !== expected.href) throw new Error(pluginText('pageChanged', locale));
        }
        setURL(value);
      } })
      .catch(reason => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; };
  }, [entry.pluginID, entry.menuIndex, entry.menu.path, retry, locale]);
  return <div className="plugins-page">
    {error && <div className="plugin-error plugin-banner" role="alert">{error}<button className="secondary-button" onClick={() => setRetry(value => value + 1)}>{pluginText('retry', locale)}</button></div>}
    {url ? <PluginResourceView entry={entry} url={url} onBack={onBack} onError={setError} />
      : <><button className="secondary-button" onClick={onBack}>{pluginText('back', locale)}</button>{!error && <p role="status">{t('common.loading')}</p>}</>}
  </div>;
}
