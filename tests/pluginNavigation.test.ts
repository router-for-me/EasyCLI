import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import {
  collapsePluginNavGroups,
  findPluginNavEntry,
  isPluginRoute,
  readCachedPluginNavEntries,
  resolvePluginNavCollapse,
  resolvePluginNavLayout,
  writeCachedPluginNavEntries,
  type PluginNavEntry,
  type PluginNavGroup,
} from '../src/services/pluginNavigation';
import { buildPluginResourceRoute } from '../src/services/pluginResources';

const space = (available: number, extra: Partial<{ rowHeight: number; compact: boolean }> = {}) =>
  ({ available, rowHeight: 34, gap: 2, padding: 4, compact: false, ...extra });
const needed = (rows: number) => rows * 34 + (rows - 1) * 2 + 4;
const group = (id: string, rows: number, extra: Partial<PluginNavGroup> = {}): PluginNavGroup =>
  ({ id, rows, active: false, manual: null, ...extra });

const entry = (pluginID: string, menuIndex: number): PluginNavEntry => ({
  pluginID,
  pluginTitle: pluginID,
  menuIndex,
  path: `/v0/resource/plugins/${pluginID}/${menuIndex}`,
  label: `${pluginID}-${menuIndex}`,
  description: 'desc',
  route: buildPluginResourceRoute(pluginID, menuIndex),
});

describe('collapsePluginNavGroups', () => {
  const plugins = group('plugins', 2);

  test('stays expanded when every row fits', () => {
    expect(collapsePluginNavGroups(space(needed(11)), 9, [plugins])).toEqual([]);
  });

  test('collapses as soon as one pixel is missing', () => {
    expect(collapsePluginNavGroups(space(needed(11) - 1), 9, [plugins])).toEqual(['plugins']);
  });

  test('stays expanded in compact mode, without rows or without a measured row', () => {
    expect(collapsePluginNavGroups(space(0, { compact: true }), 9, [plugins])).toEqual([]);
    expect(collapsePluginNavGroups(space(0), 9, [group('plugins', 0)])).toEqual([]);
    expect(collapsePluginNavGroups(space(0, { rowHeight: 0 }), 9, [plugins])).toEqual([]);
  });

  test('collapses groups without the current page first, from the bottom up, and only as many as needed', () => {
    const groups = [group('folder:a', 2), group('folder:b', 3, { active: true }), group('folder:c', 2), group('plugins', 4)];
    expect(collapsePluginNavGroups(space(needed(9 + 11) - 1), 9, groups)).toEqual(['plugins']);
    expect(collapsePluginNavGroups(space(needed(9 + 5)), 9, groups)).toEqual(['plugins', 'folder:c']);
    expect(collapsePluginNavGroups(space(needed(9 + 3)), 9, groups)).toEqual(['plugins', 'folder:c', 'folder:a']);
    expect(collapsePluginNavGroups(space(needed(9)), 9, groups)).toEqual(['plugins', 'folder:c', 'folder:a', 'folder:b']);
  });

  test('never changes groups the user expanded or collapsed and counts them as they are', () => {
    const groups = [group('folder:a', 3, { manual: 'expanded' }), group('folder:b', 5, { manual: 'collapsed' }), group('plugins', 2)];
    expect(collapsePluginNavGroups(space(needed(9 + 3 + 2)), 9, groups)).toEqual([]);
    expect(collapsePluginNavGroups(space(needed(9)), 9, groups)).toEqual(['plugins']);
  });
});

describe('resolvePluginNavCollapse', () => {
  const groups = [group('folder:a', 2), group('plugins', 3)];

  test('reports the automatic plugin layout without an override', () => {
    expect(resolvePluginNavCollapse(space(needed(9 + 2)), 9, groups, null)).toEqual({ collapsed: ['plugins'], pluginsAuto: 'collapsed' });
  });

  test('keeps a manual plugin expansion and collapses other groups to make room', () => {
    const override = { layout: 'expanded' as const, auto: 'collapsed' as const, count: 3 };
    expect(resolvePluginNavCollapse(space(needed(9 + 3)), 9, groups, override)).toEqual({ collapsed: ['folder:a'], pluginsAuto: 'collapsed' });
  });

  test('keeps a manual plugin collapse and ignores a stale override', () => {
    const override = { layout: 'collapsed' as const, auto: 'expanded' as const, count: 3 };
    expect(resolvePluginNavCollapse(space(needed(20)), 9, groups, override)).toEqual({ collapsed: ['plugins'], pluginsAuto: 'expanded' });
    expect(resolvePluginNavCollapse(space(needed(20)), 9, [groups[0], group('plugins', 4)], override)).toEqual({ collapsed: [], pluginsAuto: 'expanded' });
  });
});

describe('resolvePluginNavLayout', () => {
  test('follows the automatic layout without an override', () => {
    expect(resolvePluginNavLayout('expanded', 2, null)).toBe('expanded');
    expect(resolvePluginNavLayout('collapsed', 2, null)).toBe('collapsed');
  });

  test('manual choice wins while the automatic layout and count are unchanged', () => {
    const override = { layout: 'collapsed' as const, auto: 'expanded' as const, count: 2 };
    expect(resolvePluginNavLayout('expanded', 2, override)).toBe('collapsed');
  });

  test('reverts to automatic when the layout or the entry count changes', () => {
    const override = { layout: 'collapsed' as const, auto: 'expanded' as const, count: 2 };
    expect(resolvePluginNavLayout('collapsed', 2, override)).toBe('collapsed');
    expect(resolvePluginNavLayout('expanded', 3, override)).toBe('expanded');
  });
});

describe('plugin routes', () => {
  const entries = [entry('alpha', 0), entry('a/b', 1)];

  test('looks entries up by route', () => {
    expect(findPluginNavEntry(entries, entries[1].route)).toBe(entries[1]);
    expect(findPluginNavEntry(entries, '/plugin-pages/missing/0')).toBeUndefined();
    expect(isPluginRoute(entries[0].route)).toBe(true);
    expect(isPluginRoute('plugins')).toBe(false);
  });
});

describe('plugin entry cache', () => {
  const store = new Map<string, string>();

  beforeEach(() => {
    store.clear();
    (globalThis as { window?: unknown }).window = {
      localStorage: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => { store.set(key, value); },
      },
    };
  });

  afterEach(() => {
    delete (globalThis as { window?: unknown }).window;
  });

  test('round-trips entries', () => {
    const entries = [entry('alpha', 0)];
    writeCachedPluginNavEntries(entries);
    expect(readCachedPluginNavEntries()).toEqual(entries);
  });

  test('ignores the previous cache without menu paths', () => {
    const { path: _path, ...legacy } = entry('alpha', 0);
    store.set('cpa-gui-plugin-nav-entries', JSON.stringify([legacy]));
    expect(readCachedPluginNavEntries()).toEqual([]);
    store.set('cpa-gui-plugin-nav-entries.v2', JSON.stringify([legacy]));
    expect(readCachedPluginNavEntries()).toEqual([]);
  });

  test('tolerates invalid JSON and malformed rows', () => {
    store.set('cpa-gui-plugin-nav-entries.v2', '{oops');
    expect(readCachedPluginNavEntries()).toEqual([]);
    store.set('cpa-gui-plugin-nav-entries.v2', JSON.stringify([entry('alpha', 0), { pluginID: 1 }, null, 'x']));
    expect(readCachedPluginNavEntries()).toEqual([entry('alpha', 0)]);
    store.set('cpa-gui-plugin-nav-entries.v2', '{"a":1}');
    expect(readCachedPluginNavEntries()).toEqual([]);
  });

  test('tolerates unavailable storage', () => {
    (globalThis as { window?: unknown }).window = {
      get localStorage(): never { throw new Error('blocked'); },
    };
    expect(readCachedPluginNavEntries()).toEqual([]);
    expect(() => writeCachedPluginNavEntries([entry('alpha', 0)])).not.toThrow();
  });
});
