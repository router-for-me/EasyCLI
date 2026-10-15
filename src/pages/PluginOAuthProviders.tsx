import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { LogIn, Puzzle, RefreshCw } from 'lucide-react';
import { useI18n } from '../i18n';
import { pluginOAuthText } from '../i18n/pluginOAuth';
import { pluginsApi, type PluginListEntry } from '../services/plugins';
import { collectPluginOAuthProviders } from '../services/pluginOAuthProviders';
import { getPluginTitle, PLUGIN_RESOURCES_REFRESH_EVENT } from '../services/pluginResources';
import { PluginOAuthDialog } from './PluginOAuthDialog';
import './PluginOAuthProviders.css';

export function PluginOAuthProviders({ builtInProviderIds, browser = 'default' }: {
  builtInProviderIds: readonly string[];
  browser?: string;
}) {
  const { t, locale } = useI18n();
  const [plugins, setPlugins] = useState<PluginListEntry[]>([]);
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [completed, setCompleted] = useState<Set<string>>(() => new Set());
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(false);
  const revision = useRef(0);
  const mounted = useRef(false);
  const retryTimer = useRef<number | undefined>(undefined);

  const load = useCallback(() => {
    window.clearTimeout(retryTimer.current);
    const current = ++revision.current;
    setLoading(true);
    setError(false);
    const active = () => mounted.current && current === revision.current;
    const refresh = async (attempt: number) => {
      try {
        if (attempt === 0) {
          const supported = await invoke<boolean>('get_plugin_support');
          if (!active()) return;
          if (!supported) {
            setPlugins([]);
            setSelectedProvider(null);
            return;
          }
        }
        const response = await pluginsApi.list();
        if (!active()) return;
        const next = response.pluginsEnabled
          ? collectPluginOAuthProviders(response.plugins, builtInProviderIds) : [];
        setPlugins(next);
        setSelectedProvider(previous => next.some(plugin => plugin.oauthProvider === previous) ? previous : null);
        // Installation can finish before the core registers the plugin. Keep following
        // that transition even if the user has already left the plugin manager.
        if (attempt < 8 && response.pluginsEnabled
          && response.plugins.some(plugin => plugin.path && plugin.enabled && !plugin.effectiveEnabled)) {
          retryTimer.current = window.setTimeout(() => {
            if (active()) void refresh(attempt + 1);
          }, 1500);
        }
      } catch {
        if (active()) {
          setPlugins([]);
          setSelectedProvider(null);
          setError(true);
        }
      } finally {
        if (active()) setLoading(false);
      }
    };
    return refresh(0);
  }, [builtInProviderIds]);

  useEffect(() => {
    mounted.current = true;
    const refresh = () => { void load(); };
    refresh();
    window.addEventListener('focus', refresh);
    window.addEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refresh);
    return () => {
      mounted.current = false;
      ++revision.current;
      window.clearTimeout(retryTimer.current);
      window.removeEventListener('focus', refresh);
      window.removeEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refresh);
    };
  }, [load]);

  const selectedPlugin = plugins.find(plugin => plugin.oauthProvider === selectedProvider);
  return <>
    {error && <div className="plugin-oauth-provider-status" role="alert">
      <span>{pluginOAuthText('loadFailed', locale)}</span>
      <button type="button" className="secondary-button" disabled={loading} onClick={() => void load()}>
        <RefreshCw size={16} aria-hidden="true" />{t('common.refresh')}
      </button>
    </div>}
    {plugins.map(plugin => {
      const provider = plugin.oauthProvider!;
      const authorized = completed.has(provider);
      return <section className="panel oauth-card plugin-oauth-card" key={provider}>
        <div className="provider-title-row">
          <Puzzle className="provider-logo" size={40} aria-hidden="true" />
          <div>
            <h2>{pluginOAuthText('providerTitle', locale).replace('{name}', getPluginTitle(plugin))}</h2>
            {authorized && <span className="state-pill success">{t('oauth.status.completed')}</span>}
          </div>
        </div>
        <div className="oauth-card-body plugin-oauth-card-body"><p className="oauth-hint">{pluginOAuthText('providerHint', locale)}</p></div>
        <div className="button-row management-card-actions">
          <button type="button" className="primary-button" onClick={() => setSelectedProvider(provider)}>
            <LogIn size={16} aria-hidden="true" />{t(authorized ? 'oauth.loginAnother' : 'oauth.startLogin')}
          </button>
        </div>
      </section>;
    })}
    {selectedPlugin && <PluginOAuthDialog
      key={selectedPlugin.oauthProvider}
      plugin={selectedPlugin}
      browser={browser}
      onClose={() => setSelectedProvider(null)}
      onCompleted={() => setCompleted(previous => new Set(previous).add(selectedPlugin.oauthProvider!))}
    />}
  </>;
}
