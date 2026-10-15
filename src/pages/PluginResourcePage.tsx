import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../i18n';
import { pluginText } from '../i18n/plugins';
import { useOverlayOpen } from '../components/useOverlayOpen';
import type { PluginNavEntry } from '../services/pluginNavigation';
import { EMPTY_BOUNDS, effectiveBounds, newWebviewToken, roundBounds, sameBounds, type WebviewBounds } from '../services/pluginWebview';

const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export function PluginResourcePage({ entry, reloadToken }: { entry: PluginNavEntry; reloadToken: number }) {
  const { t, locale } = useI18n();
  const pt = (key: Parameters<typeof pluginText>[0]) => pluginText(key, locale);
  const host = useRef<HTMLDivElement>(null);
  const overlay = useOverlayOpen();
  const [bounds, setBounds] = useState<WebviewBounds>(EMPTY_BOUNDS);
  const [attempt, setAttempt] = useState(0);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState('');
  const wanted = effectiveBounds({ active: !error, overlay, bounds });
  const wantedRef = useRef(wanted);
  wantedRef.current = wanted;
  const token = useRef(0);
  const handledReload = useRef(reloadToken);

  const measure = useCallback(() => {
    if (!host.current) return;
    const next = roundBounds(host.current.getBoundingClientRect(), window.innerHeight);
    setBounds((previous) => sameBounds(previous, next) ? previous : next);
  }, []);

  useLayoutEffect(() => {
    measure();
    const observer = new ResizeObserver(measure);
    if (host.current) observer.observe(host.current);
    observer.observe(document.documentElement);
    window.addEventListener('resize', measure);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', measure);
    };
  }, [measure]);

  useEffect(() => () => {
    void invoke('hide_plugin_webview', { token: token.current }).catch(() => undefined);
  }, []);

  useEffect(() => {
    const reload = handledReload.current !== reloadToken;
    handledReload.current = reloadToken;
    const mine = newWebviewToken();
    token.current = mine;
    let cancelled = false;
    setReady(false);
    setError('');
    const labels = { back: pt('pageBack'), reload: pt('pageReload'), openExternal: pt('openExternal'), move: pt('pageMove') };
    invoke('show_plugin_webview', { pluginId: entry.pluginID, menuIndex: entry.menuIndex, bounds: wantedRef.current, labels, reload, token: mine })
      .then(() => {
        if (!cancelled) setReady(true);
      })
      .catch((reason) => {
        if (!cancelled) setError(message(reason));
      });
    return () => { cancelled = true; };
  }, [entry.pluginID, entry.menuIndex, reloadToken, attempt, locale]);

  useEffect(() => {
    void invoke('set_plugin_webview_bounds', { bounds: wanted, token: token.current }).catch((reason) => setError(message(reason)));
  }, [ready, wanted.x, wanted.y, wanted.width, wanted.height, wanted.viewportHeight]);

  return <section className="page plugin-resource-page">
    <h1 className="sr-only">{entry.label}</h1>
    <div ref={host} className="plugin-resource-host">
      {error ? <div className="plugin-error plugin-banner" role="alert"><span>{error}</span><button type="button" className="secondary-button" onClick={() => setAttempt((value) => value + 1)}>{pt('retry')}</button></div>
        : !ready && <p className="plugin-resource-status" role="status">{t('common.loading')}</p>}
    </div>
  </section>;
}
