import { buildPluginResourceRoute, type PluginResourceEntry } from './pluginResources';

export type PluginNavEntry = Pick<PluginResourceEntry, 'pluginID' | 'pluginTitle' | 'menuIndex' | 'label' | 'description' | 'route'> & { path: string };
export type PluginNavLayout = 'expanded' | 'collapsed';
export type PluginNavOverride = { layout: PluginNavLayout; auto: PluginNavLayout; count: number };

const CACHE_KEY = 'cpa-gui-plugin-nav-entries.v2';
const ROUTE_PREFIX = '/plugin-pages/';

export const isPluginRoute = (value: string): boolean => value.startsWith(ROUTE_PREFIX);

export const findPluginNavEntry = (entries: PluginNavEntry[], route: string): PluginNavEntry | undefined =>
  entries.find((entry) => entry.route === route);

export const toPluginNavEntry = ({ pluginID, pluginTitle, menuIndex, menu, label, description, route }: PluginResourceEntry): PluginNavEntry =>
  ({ pluginID, pluginTitle, menuIndex, path: menu.path, label, description, route });

export const PLUGIN_NAV_GROUP = 'plugins';
export const pluginFolderGroup = (id: string) => `folder:${id}`;

export type PluginNavGroup = { id: string; rows: number; active: boolean; manual: PluginNavLayout | null };
export type PluginNavSpace = { available: number; rowHeight: number; gap: number; padding: number; compact: boolean };

export function collapsePluginNavGroups(space: PluginNavSpace, rows: number, groups: PluginNavGroup[]): string[] {
  if (space.compact || space.rowHeight <= 0) return [];
  const fits = (count: number) => count * space.rowHeight + Math.max(count - 1, 0) * space.gap + space.padding <= space.available;
  let total = rows + groups.reduce((sum, group) => sum + (group.manual === 'collapsed' ? 0 : group.rows), 0);
  const candidates = groups.filter((group) => group.manual === null && group.rows > 0).reverse()
    .sort((a, b) => Number(a.active) - Number(b.active));
  const collapsed: string[] = [];
  for (const group of candidates) {
    if (fits(total)) break;
    collapsed.push(group.id);
    total -= group.rows;
  }
  return collapsed;
}

export function resolvePluginNavCollapse(
  space: PluginNavSpace,
  rows: number,
  groups: PluginNavGroup[],
  override: PluginNavOverride | null,
): { collapsed: string[]; pluginsAuto: PluginNavLayout } {
  const auto = collapsePluginNavGroups(space, rows, groups);
  const pluginsAuto = auto.includes(PLUGIN_NAV_GROUP) ? 'collapsed' : 'expanded';
  const plugins = groups.find((group) => group.id === PLUGIN_NAV_GROUP);
  const layout = resolvePluginNavLayout(pluginsAuto, plugins?.rows ?? 0, override);
  if (!plugins || layout === pluginsAuto) return { collapsed: auto, pluginsAuto };
  const manual = groups.map((group) => group === plugins ? { ...group, manual: layout } : group);
  const collapsed = collapsePluginNavGroups(space, rows, manual);
  return { collapsed: layout === 'collapsed' ? [...collapsed, PLUGIN_NAV_GROUP] : collapsed, pluginsAuto };
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
    && typeof entry.pluginTitle === 'string'
    && typeof entry.path === 'string' && entry.path.length > 0
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
