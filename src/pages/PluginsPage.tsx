import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Download, ExternalLink, Folder, LogIn, Puzzle, RefreshCw, Search, Settings2, Trash2, X } from 'lucide-react';
import { useI18n } from '../i18n';
import { pluginText } from '../i18n/plugins';
import { FloatingNotice, useAppNotice } from '../appNotice';
import { useConfirmation } from '../components/ConfirmationDialog';
import { pluginsApi, pluginStoreApi, type PluginListEntry, type PluginListResponse, type PluginStoreEntry, type PluginStoreResponse } from '../services/plugins';
import { buildPluginResourceRoute, collectPluginResourceEntries, getPluginTitle, isOfficialPlugin, notifyPluginResourcesChanged, type PluginResourceEntry } from '../services/pluginResources';
import { getPluginStatus, isPluginInstalled } from '../services/pluginStatus';
import { refreshPluginReleaseVersions } from '../services/pluginReleaseVersions';
import { PluginConfigDialog } from './PluginConfigDialog';
import { PluginInstallDialog } from './PluginInstallDialog';
import { PluginOAuthDialog } from './PluginOAuthDialog';
import { GitHubTokenHint, isGitHubRateLimit } from './PluginRateLimitHint';
import { PluginSettingsPanel } from './PluginSettingsPanel';
import './PluginsPage.css';

type Tab = 'installed' | 'store' | 'settings';
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export function PluginsPage() {
  const { t, locale } = useI18n();
  const pt = (key: Parameters<typeof pluginText>[0]) => pluginText(key, locale);
  const [tab, setTab] = useState<Tab>('installed');
  const [supported, setSupported] = useState<boolean | null>(null);
  const [data, setData] = useState<PluginListResponse | null>(null);
  const [store, setStore] = useState<PluginStoreResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const searchRef = useRef<HTMLInputElement>(null);
  const [filter, setFilter] = useState('all');
  const [configPlugin, setConfigPlugin] = useState<PluginListEntry | null>(null);
  const [installEntry, setInstallEntry] = useState<PluginStoreEntry | null>(null);
  const [oauthPlugin, setOAuthPlugin] = useState<PluginListEntry | null>(null);
  const [restartRequired, setRestartRequired] = useState(false);
  const requests = useRef(0);
  const mounted = useRef(false);
  const pending = useRef(false);
  const tabRef = useRef(tab);
  tabRef.current = tab;
  const notice = useAppNotice();
  const { askConfirmation, confirmationDialog } = useConfirmation();

  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++requests.current; }; }, []);
  const load = useCallback(async (target: Tab = tabRef.current, refreshVersions = false) => {
    const revision = ++requests.current;
    setLoading(true); setError('');
    try {
      const support = await invoke<boolean>('get_plugin_support');
      if (!mounted.current || revision !== requests.current) return;
      setSupported(support);
      if (!support) { setData(null); setStore(null); return; }
      const next = await pluginsApi.list();
      if (!mounted.current || revision !== requests.current) return;
      setData(next);
      notifyPluginResourcesChanged();
      if (target === 'store') {
        let nextStore = await pluginStoreApi.list();
        if (mounted.current && revision === requests.current && refreshVersions) nextStore = await refreshPluginReleaseVersions(nextStore);
        if (mounted.current && revision === requests.current) setStore(nextStore);
      }
    } catch (reason) { if (mounted.current && revision === requests.current) setError(message(reason)); }
    finally { if (mounted.current && revision === requests.current) setLoading(false); }
  }, []);
  useEffect(() => { void load(tab); }, [tab, load]);

  // Follow existing files for a bounded period after config writes. Missing
  // binaries cannot load by waiting, and the API does not report loading progress.
  const pendingPlugins = data?.pluginsEnabled
    ? data.plugins.filter(plugin => plugin.path && plugin.enabled && !plugin.effectiveEnabled).map(plugin => plugin.id).sort().join('|')
    : '';
  useEffect(() => {
    if (!pendingPlugins || busy || loading || restartRequired) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let attempts = 0;
    const revision = requests.current;
    const poll = async () => {
      try {
        const next = await pluginsApi.list();
        if (cancelled || requests.current !== revision) return;
        setData(next);
        notifyPluginResourcesChanged();
      } catch { /* The manual refresh exposes errors without replacing action feedback. */ }
      if (!cancelled && ++attempts < 8) timer = setTimeout(() => void poll(), 1500);
    };
    timer = setTimeout(() => void poll(), 1500);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [pendingPlugins, busy, loading, restartRequired]);

  const mutate = async (operation: () => Promise<unknown>, success: string) => {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError('');
    ++requests.current;
    try {
      await operation();
      if (!mounted.current) return;
      notice.showNotice(success, 'success');
      await load();
    } catch (reason) { if (mounted.current) setError(message(reason)); }
    finally { pending.current = false; if (mounted.current) setBusy(false); }
  };
  const remove = async (plugin: PluginListEntry) => {
    const installed = isPluginInstalled(plugin);
    const label = pt(installed ? 'remove' : 'removeConfig');
    const accepted = await askConfirmation({ title: `${label} · ${getPluginTitle(plugin)}`, message: pt(installed ? 'removeHint' : 'removeConfigHint'), confirmText: label, variant: 'danger' });
    if (!accepted || !mounted.current) return;
    await mutate(async () => {
      const result = await pluginsApi.deletePlugin(plugin.id);
      if (mounted.current && result.restartRequired) setRestartRequired(true);
    }, pt(installed ? 'removed' : 'removedConfig'));
  };
  const openResource = (entry: PluginResourceEntry) => window.dispatchEvent(new CustomEvent('app:navigate', { detail: buildPluginResourceRoute(entry.pluginID, entry.menuIndex) }));
  const matches = (values: (string | undefined)[]) => values.join(' ').toLowerCase().includes(search.trim().toLowerCase());
  const plugins = (data?.plugins ?? []).filter(plugin => matches([plugin.id, plugin.metadata?.name, plugin.metadata?.author]));
  const storePlugins = (store?.plugins ?? []).filter(plugin => matches([plugin.id, plugin.name, plugin.description, plugin.author, ...plugin.tags]) && (filter === 'all' || (filter === 'installed' && plugin.installed) || (filter === 'updates' && plugin.updateAvailable)));
  const installedCount = data?.plugins.filter(isPluginInstalled).length ?? 0;
  const configOnlyCount = data?.plugins.filter(plugin => plugin.configured && !isPluginInstalled(plugin)).length ?? 0;
  const locked = busy || loading || Boolean(configPlugin || installEntry || oauthPlugin);

  return <div className="plugins-page">
    <header className="plugins-heading"><div><h1>{pt('title')}</h1><p>{pt('description')}</p></div><button className="secondary-button" disabled={locked} onClick={() => void load(tabRef.current, true)}><RefreshCw size={16} aria-hidden="true" />{t('common.refresh')}</button></header>
    {error && <div className="plugin-error plugin-banner" role="alert"><span>{error}{isGitHubRateLimit(error) ? <GitHubTokenHint /> : null}</span><button className="secondary-button" disabled={busy || loading} onClick={() => void load(tabRef.current, true)}>{pt('retry')}</button></div>}
    {supported === false ? <section className="plugin-empty"><Puzzle size={36} /><p>{pt('unsupported')}</p></section> : supported === null ? <section className="plugin-empty" role="status">{loading ? t('common.loading') : pt('retry')}</section> : <>
      <div className="plugin-summary"><label className="plugin-global-toggle"><input type="checkbox" checked={data?.pluginsEnabled ?? false} disabled={locked || !data} onChange={e => { const enabled = e.target.checked; void mutate(() => pluginsApi.updateSettings({ enabled }), pt('saved')); }} />{pt('global')}</label><span><Folder size={15} aria-hidden="true" />{data?.pluginsDir || pt('directory')}</span><span>{pt('installed')} <strong>{installedCount}</strong>{configOnlyCount > 0 && <> · {pt('configOnly')} <strong>{configOnlyCount}</strong></>}</span></div>
      {data && !data.pluginsEnabled && <p className="plugin-banner">{pt('disabledHint')}</p>}
      {restartRequired && <div className="plugin-banner"><span>{pt('restart')}</span><button className="secondary-button" disabled={locked} onClick={async () => { if (await askConfirmation({ title: pt('restartButton'), message: pt('restartConfirm') })) void mutate(async () => { await invoke('restart_core_process'); if (mounted.current) setRestartRequired(false); }, pt('saved')); }}>{pt('restartButton')}</button></div>}
      <>
        <div className="plugin-navigation">
        <nav className="plugin-tabs" aria-label={pt('title')}>{(['installed', 'store', 'settings'] as const).map(value => <button key={value} className={tab === value ? 'active' : ''} aria-current={tab === value ? 'page' : undefined} disabled={busy} onClick={() => { setTab(value); setSearch(''); setFilter('all'); }}>{pt(value === 'installed' ? 'local' : value)}</button>)}</nav>
          {tab !== 'settings' &&
          <div className="plugin-toolbar">
            <div className="plugin-search">
              <Search size={17} aria-hidden="true" />
              <input ref={searchRef} type="search" value={search} onChange={e => setSearch(e.target.value)} placeholder={pt('search')} aria-label={pt('search')} />
              <button type="button" className="plugin-search-clear" disabled={!search} aria-label={t('common.clear')} title={t('common.clear')}
                onClick={() => { setSearch(''); searchRef.current?.focus(); }}><X size={15} aria-hidden="true" /></button>
            </div>
            {tab === 'store' && <select aria-label={pt('store')} value={filter} onChange={e => setFilter(e.target.value)}>{['all', 'installed', 'updates'].map(value => <option key={value} value={value}>{pt(value as 'all' | 'installed' | 'updates')}</option>)}</select>}
          </div>}
        </div>
        {tab === 'settings' ? <PluginSettingsPanel onSaved={() => { notice.showNotice(pt('saved')); void load(); }} /> : <>
          {loading && <p className="plugin-loading" role="status">{t('common.loading')}…</p>}
          {tab === 'installed' ? <div className="plugin-grid">{plugins.map(plugin => {
            const status = getPluginStatus(plugin, data?.pluginsEnabled ?? false);
            const installed = isPluginInstalled(plugin);
            const meta = [plugin.metadata?.version, plugin.metadata?.author].filter(Boolean).join(' · ');
            const removeLabel = `${pt(installed ? 'remove' : 'removeConfig')} ${getPluginTitle(plugin)}`;
            return <article className="plugin-card plugin-local-card" key={plugin.id}>
            <div className="plugin-card-title"><span className="plugin-icon"><Puzzle size={23} /></span><div><h2>{getPluginTitle(plugin)}</h2>{getPluginTitle(plugin) !== plugin.id && <code>{plugin.id}</code>}</div><span className={`plugin-badge ${status === 'active' ? 'active' : status === 'missingFile' || status === 'notLoaded' ? 'warning' : ''}`}>{pt(status)}</span></div>
            {meta && <p className="plugin-meta">{meta}</p>}{plugin.path && <p className="plugin-path" title={plugin.path}>{plugin.path}</p>}
            {status === 'missingFile' && <p className="plugin-card-description">{pt('missingFileHint')}</p>}
            {status === 'notLoaded' && <p className="plugin-card-description">{pt('notLoadedHint')}</p>}
            {collectPluginResourceEntries([plugin]).length > 0 && <div className="plugin-menus">{collectPluginResourceEntries([plugin]).map(entry => <button key={entry.menuIndex} title={entry.description} className="secondary-button" disabled={locked} onClick={() => openResource(entry)}><ExternalLink size={14} />{entry.label}</button>)}</div>}
            <div className="plugin-card-actions">
            {plugin.effectiveEnabled && plugin.supportsOAuth && plugin.oauthProvider && <button className="secondary-button" disabled={locked} onClick={() => setOAuthPlugin(plugin)}><LogIn size={15} />{pt('oauth')}</button>}
              {installed ? <button className="secondary-button" disabled={locked} onClick={() => void mutate(() => pluginsApi.updateEnabled(plugin.id, !plugin.enabled), pt('saved'))}>{pt(plugin.enabled ? 'disabled' : 'enabled')}</button> : <button className="secondary-button" disabled={locked} onClick={() => { setSearch(plugin.id); setFilter('all'); setTab('store'); }}><Download size={15} />{pt('findInStore')}</button>}
              <button className="secondary-button" disabled={locked} onClick={() => setConfigPlugin(plugin)}><Settings2 size={15} />{pt('configure')}</button>
              {status === 'notLoaded' && <button className="secondary-button" disabled={locked} onClick={() => void invoke('open_core_logs_directory').catch(reason => { if (mounted.current) setError(message(reason)); })}><Folder size={15} />{pt('openLogs')}</button>}
              <button className="icon-button quiet plugin-delete" disabled={locked} aria-label={removeLabel} title={removeLabel} onClick={() => void remove(plugin)}><Trash2 size={16} /></button>
            </div>
          </article>; })}</div> : <>
            {store?.sourceErrors.length ? <details className="plugin-source-errors" open><summary>{pt('sourceErrors')}</summary>{store.sourceErrors.map((entry, index) => <p key={index}>{entry.sourceName || entry.sourceId || entry.sourceUrl}: {entry.message}</p>)}{store.sourceErrors.some(entry => isGitHubRateLimit(entry.message)) ? <GitHubTokenHint /> : null}</details> : null}
            <div className="plugin-grid">{storePlugins.map(entry => <article className="plugin-card" key={entry.storeId}>
              <div className="plugin-card-title"><span className="plugin-icon"><Puzzle size={23} /></span><div><h2>{entry.name || entry.id}</h2><code>{entry.id}</code></div><span className={`plugin-badge ${isOfficialPlugin(entry) ? 'active' : 'warning'}`}>{pt(isOfficialPlugin(entry) ? 'official' : 'thirdParty')}</span></div>
              <p className="plugin-card-description">{entry.description}</p><p className="plugin-meta">{[entry.installedVersion && entry.updateAvailable ? `${entry.installedVersion} → ${entry.version}` : entry.installedVersion || entry.version, entry.author, entry.license].filter(Boolean).join(' · ')}</p><p className="plugin-meta">{pt('source')}: {entry.sourceName || entry.sourceId}</p>
              {entry.platforms.length > 0 && <p className="plugin-meta">{entry.platforms.map(platform => `${platform.goos}/${platform.goarch}`).join(' · ')}</p>}
              <div className="plugin-card-actions">{entry.installed && <span className="plugin-badge">{pt(entry.updateAvailable ? 'updates' : 'installed')}</span>}<button className="primary-button" disabled={locked || (entry.authRequired && !entry.authConfigured)} onClick={() => setInstallEntry(entry)}><Download size={15} />{pt(entry.authRequired && !entry.authConfigured ? 'authMissing' : !entry.installed ? 'install' : entry.updateAvailable ? 'update' : 'reinstall')}</button></div>
            </article>)}</div>
          </>}
          {!loading && !error && (tab === 'installed' ? !plugins.length : !storePlugins.length) && <section className="plugin-empty"><Puzzle size={36} /><h2>{pt(search || filter !== 'all' ? 'noMatches' : 'empty')}</h2>{tab === 'installed' && !search && <><p>{pt('emptyHint')}</p><button className="primary-button" onClick={() => setTab('store')}>{pt('store')}</button></>}</section>}
        </>}
      </>
    </>}
    {configPlugin && <PluginConfigDialog plugin={configPlugin} onClose={() => setConfigPlugin(null)} onSaved={() => { notice.showNotice(pt('saved')); void load(); }} />}
    {installEntry && <PluginInstallDialog entry={installEntry} onClose={() => setInstallEntry(null)} onInstalled={result => { setInstallEntry(null); setRestartRequired(previous => previous || result.restartRequired); notice.showNotice(pt('installedSuccess')); void load(); }} />}
    {oauthPlugin && <PluginOAuthDialog plugin={oauthPlugin} onClose={() => setOAuthPlugin(null)} onCompleted={() => { notice.showNotice(pt('authorized')); void load(); }} />}
    {confirmationDialog}<FloatingNotice notice={notice.notice} onDismiss={notice.clearNotice} />
  </div>;
}
