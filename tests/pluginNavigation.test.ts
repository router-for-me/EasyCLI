import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import {
  orderPluginNavEntries, mergePluginNavOrder,
  findPluginNavEntry,
  isPluginRoute,
  pluginNavLayout,
  readCachedPluginNavEntries,
  resolvePluginNavLayout,
  writeCachedPluginNavEntries,
  type PluginNavEntry,
} from '../src/services/pluginNavigation';
import { buildPluginResourceRoute } from '../src/services/pluginResources';

const base = { rowHeight: 34, gap: 2, padding: 4, staticCount: 9, compact: false };
const needed = (rows: number) => rows * 34 + (rows - 1) * 2 + 4;

const entry = (pluginID: string, menuIndex: number): PluginNavEntry => ({
  pluginID,
  menuIndex,
  label: `${pluginID}-${menuIndex}`,
  description: 'desc',
  route: buildPluginResourceRoute(pluginID, menuIndex),
});

describe('pluginNavLayout', () => {
  test('stays expanded when every row fits', () => {
    expect(pluginNavLayout({ ...base, pluginCount: 2, available: needed(11) })).toBe('expanded');
  });

  test('collapses as soon as one pixel is missing', () => {
    expect(pluginNavLayout({ ...base, pluginCount: 2, available: needed(11) - 1 })).toBe('collapsed');
  });

  test('stays expanded in compact mode however little room there is', () => {
    expect(pluginNavLayout({ ...base, pluginCount: 5, available: 0, compact: true })).toBe('expanded');
  });

  test('stays expanded without plugin entries or a measured row', () => {
    expect(pluginNavLayout({ ...base, pluginCount: 0, available: 0 })).toBe('expanded');
    expect(pluginNavLayout({ ...base, pluginCount: 3, available: 0, rowHeight: 0 })).toBe('expanded');
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

  test('tolerates invalid JSON and malformed rows', () => {
    store.set('cpa-gui-plugin-nav-entries', '{oops');
    expect(readCachedPluginNavEntries()).toEqual([]);
    store.set('cpa-gui-plugin-nav-entries', JSON.stringify([entry('alpha', 0), { pluginID: 1 }, null, 'x']));
    expect(readCachedPluginNavEntries()).toEqual([entry('alpha', 0)]);
    store.set('cpa-gui-plugin-nav-entries', '{"a":1}');
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

 test('saved order survives backend refresh and appends new pages without mutating input', () => {
  const entries = [entry('a', 0), entry('b', 0), entry('c', 0)];
  const order = [entries[1].route, entries[0].route];
  expect(orderPluginNavEntries(entries, order).map(e => e.pluginID)).toEqual(['b', 'a', 'c']);
  expect(entries.map(e => e.pluginID)).toEqual(['a', 'b', 'c']);
  const hidden = entry('hidden', 0).route;
  expect(mergePluginNavOrder([order[0], hidden, order[1]], [order[1], order[0], entries[2].route])).toEqual([order[1], hidden, order[0], entries[2].route]);
 });
