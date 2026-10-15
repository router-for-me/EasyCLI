import type { PluginStoreResponse } from './plugins';

type Source = { sourceId: string; sourceUrl: string; sourceName: string };

export const pluginSourceKey = (source: Source) => source.sourceId
  ? `id:${source.sourceId}` : source.sourceUrl ? `url:${source.sourceUrl}` : 'unknown';

export function collectPluginStoreSources(store: PluginStoreResponse) {
  const sources = new Map<string, { key: string; label: string }>();
  for (const source of [...store.plugins, ...store.sourceErrors]) {
    const key = pluginSourceKey(source);
    if (!sources.has(key)) sources.set(key, { key, label: source.sourceName || source.sourceId || source.sourceUrl });
  }
  const options = [...sources.values()];
  return options.map(source => ({ ...source,
    label: source.label && options.some(other => other.key !== source.key && other.label === source.label)
      ? `${source.label} (${source.key.replace(/^(id|url):/, '')})` : source.label,
  })).sort((a, b) => a.label.localeCompare(b.label));
}
