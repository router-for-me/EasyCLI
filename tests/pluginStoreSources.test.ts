import { expect, test } from 'bun:test';
import { normalizePluginStore } from '../src/services/plugins';
import { collectPluginStoreSources, pluginSourceKey } from '../src/services/pluginStoreSources';

test('deduplicates sources, keeps duplicate plugin IDs separate, and includes failed sources', () => {
  const store = normalizePluginStore({ plugins: [
    { id: 'same', source_id: 'one', source_name: 'Store' },
    { id: 'second', source_id: 'one', source_name: 'Store' },
    { id: 'same', source_id: 'two', source_name: 'Store' },
  ], source_errors: [{ source_id: 'offline', source_name: 'Offline' }] });
  expect(collectPluginStoreSources(store)).toEqual([
    { key: 'id:offline', label: 'Offline' }, { key: 'id:one', label: 'Store (one)' }, { key: 'id:two', label: 'Store (two)' },
  ]);
  expect(store.plugins.filter(entry => pluginSourceKey(entry) === 'id:two')).toHaveLength(1);
});

test('supports legacy URL-only sources and missing source metadata without colliding with All stores', () => {
  const store = normalizePluginStore({ plugins: [
    { id: 'one', source_url: 'https://registry.test/store.json' }, { id: 'two' },
  ] });
  expect(collectPluginStoreSources(store)).toEqual([
    { key: 'unknown', label: '' },
    { key: 'url:https://registry.test/store.json', label: 'https://registry.test/store.json' },
  ]);
});
