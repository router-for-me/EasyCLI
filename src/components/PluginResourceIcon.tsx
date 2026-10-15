import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Puzzle } from 'lucide-react';
import { resolvePluginAssetURL, safePluginWebURL, type PluginResourceEntry } from '../services/pluginResources';

export function PluginResourceIcon({ entry }: { entry: PluginResourceEntry }) {
  const [url, setURL] = useState('');
  useEffect(() => {
    let disposed = false;
    const absolute = safePluginWebURL(entry.pluginLogo);
    setURL(absolute);
    if (!absolute && entry.pluginLogo.startsWith('/')) {
      void invoke<string>('get_plugin_resource_url', { pluginId: entry.pluginID, menuIndex: entry.menuIndex })
        .then(resourceURL => { if (!disposed) setURL(resolvePluginAssetURL(entry.pluginLogo, resourceURL)); })
        .catch(() => { /* Missing or invalid logos use the generic plugin icon. */ });
    }
    return () => { disposed = true; };
  }, [entry.pluginID, entry.menuIndex, entry.pluginLogo]);
  return url ? <img className="nav-plugin-logo" src={url} alt="" referrerPolicy="no-referrer" onError={() => setURL('')} />
    : <Puzzle size={16} strokeWidth={1.75} aria-hidden="true" />;
}
