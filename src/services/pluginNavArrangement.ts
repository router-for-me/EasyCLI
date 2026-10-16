import type { PluginNavEntry } from './pluginNavigation';

export type PluginNavNode =
  | { kind: 'page'; path: string }
  | { kind: 'folder'; id: string; name: string; children: string[] };

export type PluginNavArrangement = {
  version: 1;
  aliases: Record<string, string>;
  custom: PluginNavNode[];
};

export type ResolvedPluginPage = { entry: PluginNavEntry; label: string; renamed: boolean };
export type ResolvedPluginNode =
  | { kind: 'page'; page: ResolvedPluginPage }
  | { kind: 'folder'; id: string; name: string; pages: ResolvedPluginPage[] };
export type ResolvedPluginNav = { custom: ResolvedPluginNode[]; plugins: ResolvedPluginPage[] };

export const MAX_PLUGIN_NAV_NAME = 64;

export const emptyPluginNavArrangement = (): PluginNavArrangement => ({ version: 1, aliases: {}, custom: [] });

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const isText = (value: unknown): value is string => typeof value === 'string' && value.length > 0;

export function parsePluginNavArrangement(value: unknown): PluginNavArrangement | null {
  if (!isRecord(value) || value.version !== 1 || !isRecord(value.aliases) || !Array.isArray(value.custom)) return null;
  if (!Object.values(value.aliases).every(isText)) return null;
  const valid = value.custom.every((node) => isRecord(node) && (
    node.kind === 'page' ? isText(node.path)
      : node.kind === 'folder' && isText(node.id) && isText(node.name)
        && Array.isArray(node.children) && node.children.every(isText)
  ));
  return valid ? structuredClone(value as PluginNavArrangement) : null;
}

export function resolvePluginNav(arrangement: PluginNavArrangement, entries: PluginNavEntry[]): ResolvedPluginNav {
  const available = new Map<string, PluginNavEntry>();
  entries.forEach((entry) => { if (!available.has(entry.path)) available.set(entry.path, entry); });
  const placed = new Set<string>();
  const folders = new Set<string>();
  const take = (path: string): ResolvedPluginPage | null => {
    const entry = available.get(path);
    if (!entry || placed.has(path)) return null;
    placed.add(path);
    const alias = Object.prototype.hasOwnProperty.call(arrangement.aliases, path) ? arrangement.aliases[path] : '';
    return { entry, label: alias || entry.label, renamed: Boolean(alias) };
  };
  const custom = arrangement.custom.flatMap((node): ResolvedPluginNode[] => {
    if (node.kind === 'page') {
      const page = take(node.path);
      return page ? [{ kind: 'page', page }] : [];
    }
    if (folders.has(node.id)) return [];
    folders.add(node.id);
    const pages = node.children.flatMap((path) => take(path) ?? []);
    return [{ kind: 'folder', id: node.id, name: node.name, pages }];
  });
  const plugins = [...available.keys()].flatMap((path) => take(path) ?? []);
  return { custom, plugins };
}

const withoutPath = (arrangement: PluginNavArrangement, path: string): PluginNavNode[] =>
  arrangement.custom.flatMap((node): PluginNavNode[] => {
    if (node.kind === 'page') return node.path === path ? [] : [node];
    return [{ ...node, children: node.children.filter((child) => child !== path) }];
  });

export function renamePluginPage(arrangement: PluginNavArrangement, path: string, name: string, original: string): PluginNavArrangement {
  const aliases = { ...arrangement.aliases };
  const value = name.trim().slice(0, MAX_PLUGIN_NAV_NAME);
  if (!value || value === original) delete aliases[path];
  else aliases[path] = value;
  return { ...arrangement, aliases };
}

export function movePluginPageToTop(arrangement: PluginNavArrangement, path: string): PluginNavArrangement {
  return { ...arrangement, custom: [...withoutPath(arrangement, path), { kind: 'page', path }] };
}

export function movePluginPageToFolder(arrangement: PluginNavArrangement, path: string, id: string): PluginNavArrangement {
  return {
    ...arrangement,
    custom: withoutPath(arrangement, path).map((node) =>
      node.kind === 'folder' && node.id === id ? { ...node, children: [...node.children, path] } : node),
  };
}

export function returnPluginPage(arrangement: PluginNavArrangement, path: string): PluginNavArrangement {
  return { ...arrangement, custom: withoutPath(arrangement, path) };
}

export function createPluginNavFolder(arrangement: PluginNavArrangement, id: string, name: string, path?: string): PluginNavArrangement {
  const custom: PluginNavNode[] = [...(path ? withoutPath(arrangement, path) : arrangement.custom), { kind: 'folder', id, name, children: path ? [path] : [] }];
  return { ...arrangement, custom };
}

export function renamePluginNavFolder(arrangement: PluginNavArrangement, id: string, name: string): PluginNavArrangement {
  const value = name.trim().slice(0, MAX_PLUGIN_NAV_NAME);
  if (!value) return arrangement;
  return {
    ...arrangement,
    custom: arrangement.custom.map((node) => node.kind === 'folder' && node.id === id ? { ...node, name: value } : node),
  };
}

export function deletePluginNavFolder(arrangement: PluginNavArrangement, id: string): PluginNavArrangement {
  return { ...arrangement, custom: arrangement.custom.filter((node) => node.kind !== 'folder' || node.id !== id) };
}

export function findResolvedPluginPage(nav: ResolvedPluginNav, route: string): ResolvedPluginPage | undefined {
  const pages = [...nav.custom.flatMap((node) => node.kind === 'page' ? [node.page] : node.pages), ...nav.plugins];
  return pages.find((page) => page.entry.route === route);
}
