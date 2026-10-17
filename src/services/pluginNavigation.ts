import { buildPluginResourceRoute, type PluginResourceEntry } from './pluginResources';

export type PluginNavEntry = Pick<PluginResourceEntry, 'pluginID' | 'menuIndex' | 'label' | 'description' | 'route'>;
export type PluginNavLayout = 'expanded' | 'collapsed';
export type PluginNavOverride = { layout: PluginNavLayout; auto: PluginNavLayout; count: number };

const CACHE_KEY = 'cpa-gui-plugin-nav-entries';
const ORDER_KEY = 'cpa-gui-plugin-nav-order';

export function readPluginNavOrder(): string[] {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(ORDER_KEY) ?? '[]');
    return Array.isArray(value) ? [...new Set(value.filter((route): route is string => typeof route === 'string' && route.startsWith('/plugin-pages/')))] : [];
  } catch { return []; }
}

export function writePluginNavOrder(order: string[]) {
  try { window.localStorage.setItem(ORDER_KEY, JSON.stringify(order)); } catch { /* Keep the session order when storage is unavailable. */ }
}

export function orderPluginNavEntries(entries: PluginNavEntry[], order: string[]): PluginNavEntry[] {
  const ranks = new Map(order.map((route, index) => [route, index]));
  return [...entries].sort((a, b) => (ranks.get(a.route) ?? order.length) - (ranks.get(b.route) ?? order.length));
}

// Retain hidden/disabled pages so enabling a plugin restores its position.
export function mergePluginNavOrder(order: string[], visible: string[]): string[] {
  const visibleSet = new Set(visible);
  let index = 0;
  const next = order.map(route => visibleSet.has(route) ? visible[index++] : route);
  return [...next, ...visible.slice(index)];
}
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
