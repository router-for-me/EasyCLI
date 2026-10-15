import { buildPluginResourceRoute, type PluginResourceEntry } from './pluginResources';

export type PluginNavEntry = Pick<PluginResourceEntry, 'pluginID' | 'menuIndex' | 'label' | 'description' | 'route'>;
export type PluginNavLayout = 'expanded' | 'collapsed';
export type PluginNavOverride = { layout: PluginNavLayout; auto: PluginNavLayout; count: number };

const CACHE_KEY = 'cpa-gui-plugin-nav-entries';
const ROUTE_PREFIX = '/plugin-pages/';

export const isPluginRoute = (value: string): boolean => value.startsWith(ROUTE_PREFIX);

export const findPluginNavEntry = (entries: PluginNavEntry[], route: string): PluginNavEntry | undefined =>
  entries.find((entry) => entry.route === route);

export const toPluginNavEntry = ({ pluginID, menuIndex, label, description, route }: PluginResourceEntry): PluginNavEntry =>
  ({ pluginID, menuIndex, label, description, route });

export function pluginNavLayout({ available, rowHeight, gap, padding, staticCount, pluginCount, compact }: {
  available: number;
  rowHeight: number;
  gap: number;
  padding: number;
  staticCount: number;
  pluginCount: number;
  compact: boolean;
}): PluginNavLayout {
  if (compact || pluginCount <= 0 || rowHeight <= 0) return 'expanded';
  const rows = staticCount + pluginCount;
  const needed = rows * rowHeight + (rows - 1) * gap + padding;
  return needed <= available ? 'expanded' : 'collapsed';
}

export function resolvePluginNavLayout(
  auto: PluginNavLayout,
  count: number,
  override: PluginNavOverride | null,
): PluginNavLayout {
  return override && override.auto === auto && override.count === count ? override.layout : auto;
}

const isNavEntry = (value: unknown): value is PluginNavEntry => {
  if (typeof value !== 'object' || value === null) return false;
  const entry = value as Record<string, unknown>;
  return typeof entry.pluginID === 'string'
    && Number.isInteger(entry.menuIndex)
    && typeof entry.label === 'string'
    && typeof entry.description === 'string'
    && entry.route === buildPluginResourceRoute(entry.pluginID, entry.menuIndex as number);
};

export function readCachedPluginNavEntries(): PluginNavEntry[] {
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(CACHE_KEY) ?? '[]');
    return Array.isArray(parsed) ? parsed.filter(isNavEntry) : [];
  } catch {
    return [];
  }
}

export function writeCachedPluginNavEntries(entries: PluginNavEntry[]) {
  try {
    window.localStorage.setItem(CACHE_KEY, JSON.stringify(entries));
  } catch {
    return;
  }
}
