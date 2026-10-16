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

export type PluginNavDragItem = { kind: 'page'; path: string; inPlugins: boolean } | { kind: 'folder'; id: string };
export type PluginNavDropTarget =
  | { kind: 'page'; path: string; folder: string | null; inPlugins: boolean }
  | { kind: 'folder'; id: string }
  | { kind: 'empty'; folder: string }
  | { kind: 'plugins' };
type PluginNavRef = { kind: 'page'; path: string } | { kind: 'folder'; id: string };
export type PluginNavPlacement =
  | { zone: 'top'; ref: PluginNavRef | null; after: boolean }
  | { zone: 'folder'; id: string; ref: string | null; after: boolean }
  | { zone: 'plugins' };
export type PluginNavDropPlan = { target: PluginNavDropTarget; position: 'before' | 'after' | 'into'; placement: PluginNavPlacement };

const PLUGINS_TARGET: PluginNavDropTarget = { kind: 'plugins' };
const TOP_END: PluginNavPlacement = { zone: 'top', ref: null, after: false };

export function pluginNavDropPlan(item: PluginNavDragItem, target: PluginNavDropTarget, ratio: number): PluginNavDropPlan | null {
  const upper = ratio < 0.5;
  const side = (ref: PluginNavRef): PluginNavDropPlan => ({ target, position: upper ? 'before' : 'after', placement: { zone: 'top', ref, after: !upper } });
  if (item.kind === 'folder') {
    if (target.kind === 'folder') return target.id === item.id ? null : side({ kind: 'folder', id: target.id });
    if (target.kind === 'page' && !target.folder && !target.inPlugins) return side({ kind: 'page', path: target.path });
    return target.kind === 'plugins' ? { target, position: 'before', placement: TOP_END } : null;
  }
  const intoPlugins: PluginNavDropPlan | null = item.inPlugins ? null : { target: PLUGINS_TARGET, position: 'into', placement: { zone: 'plugins' } };
  if (target.kind === 'plugins') return upper ? { target, position: 'before', placement: TOP_END } : intoPlugins;
  if (target.kind === 'empty') return { target, position: 'into', placement: { zone: 'folder', id: target.folder, ref: null, after: false } };
  if (target.kind === 'folder') {
    return ratio < 0.25
      ? { target, position: 'before', placement: { zone: 'top', ref: { kind: 'folder', id: target.id }, after: false } }
      : { target, position: 'into', placement: { zone: 'folder', id: target.id, ref: null, after: false } };
  }
  if (target.path === item.path) return null;
  if (target.inPlugins) return intoPlugins;
  if (!target.folder) return side({ kind: 'page', path: target.path });
  return { target, position: upper ? 'before' : 'after', placement: { zone: 'folder', id: target.folder, ref: target.path, after: !upper } };
}

const sameRef = (node: PluginNavNode, ref: PluginNavRef) =>
  ref.kind === 'page' ? node.kind === 'page' && node.path === ref.path : node.kind === 'folder' && node.id === ref.id;
const insertAt = <T>(list: T[], value: T, index: number, after: boolean): T[] => {
  const at = index < 0 ? list.length : index + (after ? 1 : 0);
  return [...list.slice(0, at), value, ...list.slice(at)];
};

export function placePluginNavItem(arrangement: PluginNavArrangement, item: PluginNavDragItem, placement: PluginNavPlacement): PluginNavArrangement {
  let custom: PluginNavNode[];
  if (item.kind === 'folder') {
    const node = arrangement.custom.find((current) => current.kind === 'folder' && current.id === item.id);
    if (!node || placement.zone !== 'top') return arrangement;
    const rest = arrangement.custom.filter((current) => current !== node);
    const { ref } = placement;
    custom = insertAt(rest, node, ref ? rest.findIndex((current) => sameRef(current, ref)) : -1, placement.after);
  } else {
    const rest = withoutPath(arrangement, item.path);
    if (placement.zone === 'plugins') custom = rest;
    else if (placement.zone === 'top') {
      const { ref } = placement;
      custom = insertAt(rest, { kind: 'page', path: item.path }, ref ? rest.findIndex((current) => sameRef(current, ref)) : -1, placement.after);
    } else {
      if (!rest.some((node) => node.kind === 'folder' && node.id === placement.id)) return arrangement;
      custom = rest.map((node) => node.kind === 'folder' && node.id === placement.id
        ? { ...node, children: insertAt(node.children, item.path, placement.ref ? node.children.indexOf(placement.ref) : -1, placement.after) }
        : node);
    }
  }
  return JSON.stringify(custom) === JSON.stringify(arrangement.custom) ? arrangement : { ...arrangement, custom };
}

export function findResolvedPluginPage(nav: ResolvedPluginNav, route: string): ResolvedPluginPage | undefined {
  const pages = [...nav.custom.flatMap((node) => node.kind === 'page' ? [node.page] : node.pages), ...nav.plugins];
  return pages.find((page) => page.entry.route === route);
}
