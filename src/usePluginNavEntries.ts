import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { pluginsApi } from './services/plugins';
import { collectPluginResourceEntries, PLUGIN_RESOURCES_REFRESH_EVENT } from './services/pluginResources';
import {
  readCachedPluginNavEntries,
  toPluginNavEntry,
  writeCachedPluginNavEntries,
  type PluginNavEntry,
} from './services/pluginNavigation';

export function usePluginNavEntries(coreReady: boolean) {
  const [entries, setEntries] = useState<PluginNavEntry[]>(readCachedPluginNavEntries);
  const [fetched, setFetched] = useState(false);
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    const refresh = () => setRevision((value) => value + 1);
    window.addEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refresh);
    return () => window.removeEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refresh);
  }, []);

  useEffect(() => {
    if (!coreReady) {
      setFetched(false);
      return undefined;
    }
    let cancelled = false;
    void (async () => {
      try {
        const supported = await invoke<boolean>('get_plugin_support');
        const data = supported ? await pluginsApi.list() : null;
        if (cancelled) return;
        const next = data?.pluginsEnabled ? collectPluginResourceEntries(data.plugins).map(toPluginNavEntry) : [];
        setEntries(next);
        writeCachedPluginNavEntries(next);
        setFetched(true);
      } catch {
        return;
      }
    })();
    return () => { cancelled = true; };
  }, [coreReady, revision]);

  return { entries, fetched };
}
