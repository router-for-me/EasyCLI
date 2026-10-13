import { describe, expect, it } from 'bun:test';
import { UsagePreferences } from '../src/services/usagePreferences';
import { getInitialVisibleColumns, getInitialColumnWidths, getInitialCollapseErrors } from '../src/pages/UsageEventsView';

const pageSize = 'cpa-gui.usage-events-page-size.v1';
const columns = 'cpa-gui.usage-events-visible-cols.v6';
function storage(initial: Record<string, string> = {}): Storage {
  const values = new Map(Object.entries(initial));
  return {
    get length() { return values.size; },
    key: (index) => [...values.keys()][index] ?? null,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => { values.set(key, value); },
    removeItem: (key) => { values.delete(key); },
    clear: () => values.clear(),
  };
}

describe('usage preferences across updates', () => {
  it('preserves older column choices and widths, including previous defaults', () => {
    const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
    const browser = storage({
      'cpa-gui.usage-events-visible-cols.v5': JSON.stringify(['time', 'model', 'provider', 'result', 'total', 'cache', 'latency', 'speed', 'cost', 'effort']),
      'cpa-gui.usage-events-col-widths.v3': JSON.stringify({ provider: 135, model: 150 }),
    });
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: browser });
    try {
      expect(getInitialVisibleColumns()).toEqual(['time', 'model', 'source', 'key', 'result', 'total', 'cache', 'latency', 'speed', 'cost']);
      expect(getInitialColumnWidths().provider).toBe(135);
      browser.setItem('cpa-gui.usage-events-visible-cols.v5', JSON.stringify(['time', 'provider', 'key', 'source', 'model', 'effort', 'result', 'request', 'latency', 'speed', 'total', 'cache', 'cost']));
      expect(getInitialVisibleColumns()).toEqual(['time', 'provider', 'key', 'source', 'model', 'result', 'request', 'latency', 'speed', 'total', 'cache', 'cost']);
      browser.removeItem('cpa-gui.usage-events-visible-cols.v5');
      browser.setItem('cpa-gui.usage-events-visible-cols.v4', '["provider","model"]');
      expect(getInitialVisibleColumns()).toEqual(['provider', 'model']);
      browser.setItem('cpa-gui.usage-events-visible-cols.v3', '["source","model"]');
      expect(getInitialVisibleColumns()).toEqual(['provider', 'model']);
      expect(getInitialCollapseErrors()).toBe(true);
      browser.setItem('cpa-gui.usage-events-collapse-errors.v1', '0');
      expect(getInitialCollapseErrors()).toBe(false);
      browser.clear();
      expect(getInitialVisibleColumns()).toContain('key');
      browser.setItem(columns, JSON.stringify(['time', 'model', 'source', 'result', 'total', 'cache', 'latency', 'speed', 'cost']));
      expect(getInitialVisibleColumns()).toContain('key');
      browser.setItem(columns, JSON.stringify(['time', 'source']));
      expect(getInitialVisibleColumns()).toEqual(['time', 'source']);
    } finally {
      if (original) Object.defineProperty(globalThis, 'localStorage', original);
      else Reflect.deleteProperty(globalThis, 'localStorage');
    }
  });
  it('migrates browser preferences and restores them with an empty WebView cache', async () => {
    let disk: Record<string, string> = {};
    const backend = { load: async () => disk, save: async (values: Record<string, string>) => { disk = values; } };
    const oldBrowser = storage({ [pageSize]: '100', [columns]: '["time","provider"]', unrelated: 'private' });
    const beforeUpdate = new UsagePreferences(() => oldBrowser);
    await beforeUpdate.initialize(backend);
    expect(disk).toEqual({ [pageSize]: '100', [columns]: '["time","provider"]' });
    const afterUpdate = new UsagePreferences(() => storage());
    await afterUpdate.initialize(backend);
    expect(afterUpdate.getItem(pageSize)).toBe('100');
    expect(afterUpdate.getItem(columns)).toBe('["time","provider"]');
  });

  it('prefers disk over stale browser defaults and serializes rapid changes', async () => {
    let disk: Record<string, string> = { [pageSize]: '100' };
    const browser = storage({ [pageSize]: '50' });
    const preferences = new UsagePreferences(() => browser);
    await preferences.initialize({ load: async () => disk, save: async (values) => {
      await new Promise((resolve) => setTimeout(resolve, values[pageSize] === '200' ? 10 : 0));
      disk = values;
    } });
    expect(preferences.getItem(pageSize)).toBe('100');
    preferences.setItem(pageSize, '200');
    preferences.setItem(pageSize, '100');
    preferences.setItem(columns, '["provider"]');
    await preferences.flush();
    expect(disk).toEqual({ [pageSize]: '100', [columns]: '["provider"]' });
  });

  it('does not overwrite unreadable disk preferences with defaults', async () => {
    let saves = 0;
    const preferences = new UsagePreferences(() => storage());
    await expect(preferences.initialize({ load: async () => { throw new Error('unreadable'); }, save: async () => { saves++; } })).rejects.toThrow();
    preferences.setItem(pageSize, '50');
    await preferences.flush();
    expect(saves).toBe(0);
  });

  it('retains preferences when localStorage is unavailable', async () => {
    let disk: Record<string, string> = { [pageSize]: '100' };
    const preferences = new UsagePreferences(() => { throw new Error('storage unavailable'); });
    await preferences.initialize({ load: async () => disk, save: async (values) => { disk = values; } });
    expect(preferences.getItem(pageSize)).toBe('100');
    preferences.setItem(pageSize, '200');
    await preferences.flush();
    expect(disk[pageSize]).toBe('200');
  });
});
