import { describe, expect, it } from 'bun:test';
import { normalizePluginList } from '../src/services/plugins';
import { parsePluginPins, reconcilePluginPins, resolvePluginPins } from '../src/services/pluginPins';

const pins = [{ pluginID: 'sample', path: '/first' }, { pluginID: 'sample', path: '/second' }];
const catalog = (menus = ['/first', '/second'], effective = true, global = true) => normalizePluginList({
  plugins_enabled: global, plugins: [{ id: 'sample', enabled: effective, effective_enabled: effective,
    metadata: { name: 'Sample', version: '2.0' }, menus: menus.map(path => ({ path, menu: path })) }],
});
describe('plugin sidebar pins', () => {
  it('round trips multiple pages without retaining URLs, credentials or menu indices', () => {
    expect(parsePluginPins(JSON.stringify([...pins, pins[0], { ...pins[1], url: 'secret', menuIndex: 5 }]))).toEqual(pins);
    for (const raw of ['broken', '{}', '[null,42]', '[{"pluginID":"../bad","path":"/page"}]',
      '[{"pluginID":"sample","path":"https://external.test"}]', '[{"pluginID":"sample","path":"//external.test"}]']) {
      expect(parsePluginPins(raw)).toEqual([]);
    }
  });
  it('resolves by path after an upgrade reorders menus, using the current index', () => {
    const resolved = resolvePluginPins(pins, catalog(['/second', '/first']));
    expect(resolved.map(entry => entry.menuIndex)).toEqual([1, 0]);
    expect(resolved.map(entry => entry.menu.path)).toEqual(['/first', '/second']);
  });
  it('preserves disabled/unloaded pins while hiding their navigation', () => {
    const disabled = catalog([], false);
    expect(reconcilePluginPins(pins, disabled)).toEqual(pins);
    expect(resolvePluginPins(pins, disabled)).toEqual([]);
    const global = catalog([], false, false);
    expect(reconcilePluginPins(pins, global)).toEqual(pins);
    expect(resolvePluginPins(pins, global)).toEqual([]);
    expect(resolvePluginPins(pins, null)).toEqual([]);
  });
  it('prunes removed pages only from a loaded plugin and removes uninstalled plugins', () => {
    expect(reconcilePluginPins(pins, catalog(['/second']))).toEqual([pins[1]]);
    expect(reconcilePluginPins(pins, normalizePluginList({ plugins_enabled: true, plugins: [] }))).toEqual([]);
  });
});
