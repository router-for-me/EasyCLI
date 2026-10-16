import { describe, expect, test } from 'bun:test';
import {
  createPluginNavFolder,
  deletePluginNavFolder,
  emptyPluginNavArrangement,
  findResolvedPluginPage,
  movePluginPageToFolder,
  movePluginPageToTop,
  parsePluginNavArrangement,
  renamePluginNavFolder,
  renamePluginPage,
  resolvePluginNav,
  returnPluginPage,
  type PluginNavArrangement,
} from '../src/services/pluginNavArrangement';
import { PluginNavArrangementStore } from '../src/services/pluginNavArrangementStore';
import type { PluginNavEntry } from '../src/services/pluginNavigation';

const path = (name: string) => `/v0/resource/plugins/demo/${name}`;
const entry = (name: string, menuIndex: number): PluginNavEntry => ({
  pluginID: 'demo',
  pluginTitle: 'Demo Plugin',
  menuIndex,
  path: path(name),
  label: name.toUpperCase(),
  description: `${name} page`,
  route: `/plugin-pages/demo/${menuIndex}`,
});
const entries = [entry('a', 0), entry('b', 1), entry('c', 2), entry('d', 3)];
const labels = (pages: { label: string }[]) => pages.map((page) => page.label);
const arrangement = (custom: PluginNavArrangement['custom'], aliases: Record<string, string> = {}): PluginNavArrangement =>
  ({ version: 1, aliases, custom });

describe('resolvePluginNav', () => {
  test('keeps every page in the plugin group by default', () => {
    const nav = resolvePluginNav(emptyPluginNavArrangement(), entries);
    expect(nav.custom).toEqual([]);
    expect(labels(nav.plugins)).toEqual(['A', 'B', 'C', 'D']);
  });

  test('places custom pages and folders in order and applies aliases', () => {
    const nav = resolvePluginNav(arrangement([
      { kind: 'folder', id: 'f1', name: 'Tools', children: [path('c'), path('a')] },
      { kind: 'page', path: path('d') },
    ], { [path('a')]: 'Alpha' }), entries);
    expect(nav.custom.map((node) => node.kind === 'page' ? node.page.label : `${node.name}:${labels(node.pages)}`)).toEqual(['Tools:C,Alpha', 'D']);
    expect(labels(nav.plugins)).toEqual(['B']);
    expect(findResolvedPluginPage(nav, entries[0].route)).toMatchObject({ label: 'Alpha', renamed: true });
  });

  test('keeps unavailable pages in the record and restores them in place', () => {
    const layout = arrangement([{ kind: 'folder', id: 'f1', name: 'Tools', children: [path('a'), path('b')] }]);
    const without = resolvePluginNav(layout, entries.slice(1));
    expect(without.custom).toEqual([{ kind: 'folder', id: 'f1', name: 'Tools', pages: [expect.objectContaining({ label: 'B' })] }]);
    expect(labels(resolvePluginNav(layout, entries).custom.flatMap((node) => node.kind === 'folder' ? node.pages : []))).toEqual(['A', 'B']);
  });

  test('uses the first occurrence of a duplicated path or folder id', () => {
    const nav = resolvePluginNav(arrangement([
      { kind: 'page', path: path('a') },
      { kind: 'folder', id: 'f1', name: 'First', children: [path('a'), path('b')] },
      { kind: 'folder', id: 'f1', name: 'Second', children: [path('c')] },
    ]), entries);
    expect(nav.custom.map((node) => node.kind === 'page' ? node.page.label : `${node.name}:${labels(node.pages)}`)).toEqual(['A', 'First:B']);
    expect(labels(nav.plugins)).toEqual(['C', 'D']);
  });

  test('shows empty folders', () => {
    const nav = resolvePluginNav(arrangement([
      { kind: 'folder', id: 'empty', name: 'Empty', children: [] },
      { kind: 'folder', id: 'full', name: 'Full', children: [path('a'), path('b')] },
      { kind: 'page', path: path('c') },
    ]), entries);
    expect(nav.custom.map((node) => node.kind === 'folder' ? `${node.name}:${node.pages.length}` : node.page.label)).toEqual(['Empty:0', 'Full:2', 'C']);
  });
});

describe('arrangement changes', () => {
  test('moves pages between the main menu, folders and the plugin group', () => {
    let layout = createPluginNavFolder(emptyPluginNavArrangement(), 'f1', 'Tools', path('a'));
    layout = movePluginPageToTop(layout, path('b'));
    layout = movePluginPageToFolder(layout, path('b'), 'f1');
    expect(layout.custom).toEqual([{ kind: 'folder', id: 'f1', name: 'Tools', children: [path('a'), path('b')] }]);
    layout = movePluginPageToTop(layout, path('a'));
    expect(layout.custom).toEqual([
      { kind: 'folder', id: 'f1', name: 'Tools', children: [path('b')] },
      { kind: 'page', path: path('a') },
    ]);
    layout = returnPluginPage(layout, path('a'));
    layout = deletePluginNavFolder(layout, 'f1');
    expect(layout.custom).toEqual([]);
  });

  test('renames pages and folders and restores original names', () => {
    let layout = renamePluginPage(emptyPluginNavArrangement(), path('a'), '  Alpha  ', 'A');
    expect(layout.aliases).toEqual({ [path('a')]: 'Alpha' });
    expect(renamePluginPage(layout, path('a'), '', 'A').aliases).toEqual({});
    expect(renamePluginPage(layout, path('a'), 'A', 'A').aliases).toEqual({});
    expect(renamePluginPage(layout, path('a'), 'x'.repeat(100), 'A').aliases[path('a')]).toHaveLength(64);
    layout = createPluginNavFolder(layout, 'f1', 'New folder');
    expect(renamePluginNavFolder(layout, 'f1', '  ')).toBe(layout);
    expect(renamePluginNavFolder(layout, 'f1', ' Work ').custom).toEqual([{ kind: 'folder', id: 'f1', name: 'Work', children: [] }]);
  });
});

describe('parsePluginNavArrangement', () => {
  test('accepts a valid layout and rejects unknown versions or malformed nodes', () => {
    const valid = arrangement([{ kind: 'page', path: path('a') }], { [path('a')]: 'Alpha' });
    expect(parsePluginNavArrangement(valid)).toEqual(valid);
    expect(parsePluginNavArrangement({ ...valid, version: 2 })).toBeNull();
    expect(parsePluginNavArrangement({ ...valid, custom: [{ kind: 'folder', id: 'f1', name: 'x', children: [1] }] })).toBeNull();
    expect(parsePluginNavArrangement({ ...valid, aliases: { x: '' } })).toBeNull();
    expect(parsePluginNavArrangement([])).toBeNull();
  });
});

describe('PluginNavArrangementStore', () => {
  const memoryStorage = () => {
    const values = new Map<string, string>();
    return {
      values,
      storage: () => ({
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => { values.set(key, value); },
      }) as unknown as Storage,
    };
  };
  const saved = arrangement([{ kind: 'page', path: path('a') }]);

  test('treats a missing file as the default layout', async () => {
    const { storage } = memoryStorage();
    const store = new PluginNavArrangementStore({ load: async () => null, save: async () => null }, storage);
    expect(store.get().status).toBe('loading');
    await store.load();
    expect(store.get()).toEqual({ status: 'ready', arrangement: emptyPluginNavArrangement() });
  });

  test('never saves over an unreadable or unknown layout', async () => {
    for (const load of [async () => { throw new Error('broken'); }, async () => ({ version: 9 })]) {
      const saves: unknown[] = [];
      const store = new PluginNavArrangementStore({ load, save: async (layout) => saves.push(layout) }, memoryStorage().storage);
      await store.load();
      expect(store.get()).toEqual({ status: 'unreadable', arrangement: emptyPluginNavArrangement() });
      store.update((current) => movePluginPageToTop(current, path('b')));
      await store.flush();
      expect(saves).toEqual([]);
      expect(store.get().arrangement).toEqual(emptyPluginNavArrangement());
    }
  });

  test('shows the cached layout while loading and the saved one afterwards', async () => {
    const { values, storage } = memoryStorage();
    values.set('cpa-gui.plugin-nav-layout.v1', JSON.stringify(arrangement([{ kind: 'page', path: path('c') }])));
    const store = new PluginNavArrangementStore({ load: async () => saved, save: async () => null }, storage);
    expect(store.get()).toEqual({ status: 'loading', arrangement: arrangement([{ kind: 'page', path: path('c') }]) });
    await store.load();
    expect(store.get()).toEqual({ status: 'ready', arrangement: saved });
    expect(JSON.parse(values.get('cpa-gui.plugin-nav-layout.v1')!)).toEqual(saved);
  });

  test('saves changes in order', async () => {
    const saves: PluginNavArrangement[] = [];
    let release = () => {};
    const store = new PluginNavArrangementStore({
      load: async () => saved,
      save: (layout) => new Promise((resolve) => {
        saves.push(layout);
        if (saves.length === 1) release = () => resolve(null);
        else resolve(null);
      }),
    }, memoryStorage().storage);
    await store.load();
    store.update((current) => movePluginPageToTop(current, path('b')));
    store.update((current) => movePluginPageToTop(current, path('c')));
    await Promise.resolve();
    expect(saves).toHaveLength(1);
    release();
    await store.flush();
    expect(saves.map((layout) => layout.custom.length)).toEqual([2, 3]);
  });

  test('rolls back to the last saved layout and drops queued saves after a failure', async () => {
    const saves: PluginNavArrangement[] = [];
    let failures = 0;
    const store = new PluginNavArrangementStore({
      load: async () => saved,
      save: async (layout) => {
        saves.push(layout);
        throw new Error('disk full');
      },
    }, memoryStorage().storage);
    store.onSaveFailed(() => { failures += 1; });
    await store.load();
    store.update((current) => movePluginPageToTop(current, path('b')));
    store.update((current) => movePluginPageToTop(current, path('c')));
    expect(store.get().arrangement.custom).toHaveLength(3);
    await store.flush();
    expect(saves).toHaveLength(1);
    expect(failures).toBe(1);
    expect(store.get()).toEqual({ status: 'ready', arrangement: saved });
  });
});
