import { describe, expect, test } from 'bun:test';
import { createBrowserMockRuntime } from '../src/mocks/browserMockRuntime';

const request = (runtime: ReturnType<typeof createBrowserMockRuntime>, method: string, path: string, body?: unknown, query?: unknown) =>
  runtime.invoke('management_request', { request: { method, path, body, query } });

describe('browser plugin mock', () => {
  test('exposes installed plugins, update catalog and a non-network resource page', async () => {
    const runtime = createBrowserMockRuntime('running');
    expect(await runtime.invoke('get_plugin_support')).toBe(true);
    const catalog = await request(runtime, 'GET', '/plugins/store') as { plugins: { repository: string; version: string }[] };
    const release = catalog.plugins[0];
    const repository = release.repository.replace(/^https:\/\/github\.com\//, '');
    expect(await runtime.invoke('get_plugin_latest_releases', { repositories: [repository, 'unknown/repository'] })).toEqual([
      { repository, version: release.version, error: null },
      { repository: 'unknown/repository', version: null, error: 'Invalid GitHub repository' },
    ]);
    expect(await request(runtime, 'GET', '/plugins')).toMatchObject({
      plugins_enabled: true,
      plugins: [{ id: 'request-inspector', effective_enabled: true, menus: [{ menu: 'Inspector status' }] }],
    });
    expect(await request(runtime, 'GET', '/plugins/store')).toMatchObject({ plugins: [
      { id: 'request-inspector', installed: true, update_available: true },
      { id: 'response-tags', source_id: 'community-demo', installed: false },
    ] });
    const url = await runtime.invoke('get_plugin_resource_url', { pluginId: 'request-inspector', menuIndex: 0 });
    expect(url).toStartWith('data:text/html;charset=utf-8,');
    await expect(runtime.invoke('get_plugin_resource_url', { pluginId: 'request-inspector', menuIndex: 1 })).rejects.toThrow('Plugin page is unavailable');
  });

  test('keeps configuration, global state and resource availability synchronized', async () => {
    const runtime = createBrowserMockRuntime('running');
    const path = '/config/plugins/configs/request-inspector';
    await request(runtime, 'PUT', path, { enabled: true, priority: 5, options: { first: 1 }, literal: null });
    await request(runtime, 'PATCH', path, { options: { second: false }, nullable: null });
    expect(await request(runtime, 'GET', path)).toEqual({ enabled: true, priority: 5, options: { first: 1, second: false }, literal: null, nullable: null });
    await request(runtime, 'DELETE', `${path}/literal`);
    await request(runtime, 'PUT', `${path}/enabled`, false);
    await expect(runtime.invoke('get_plugin_resource_url', { pluginId: 'request-inspector', menuIndex: 0 })).rejects.toThrow('Plugin page is unavailable');
    await request(runtime, 'PATCH', '/config/plugins', { enabled: false, 'store-sources': [], 'store-auth': [] });
    expect(await runtime.invoke('get_core_config_settings')).toMatchObject({ pluginsEnabled: false });
    expect(await runtime.invoke('get_extended_core_config')).toMatchObject({ plugins: { enabled: false, configs: { 'request-inspector': { priority: 5 } } } });
    expect(await request(runtime, 'GET', '/plugins/store')).toMatchObject({ plugins: [{ id: 'request-inspector' }] });
    const config = await request(runtime, 'GET', path) as Record<string, unknown>;
    expect(config).not.toHaveProperty('literal');
    config.priority = 99;
    expect(await request(runtime, 'GET', path)).toHaveProperty('priority', 5);
  });

  test('persists installs and deletes and honors the selected source and version', async () => {
    const runtime = createBrowserMockRuntime('running');
    await expect(request(runtime, 'POST', '/plugins/store/response-tags/install', {}, { source: 'official' })).rejects.toThrow('not_found');
    expect(await request(runtime, 'POST', '/plugins/store/response-tags/install', {}, { source: 'community-demo', version: 'v0.4.0' })).toMatchObject({
      status: 'installed', id: 'response-tags', source_id: 'community-demo', version: '0.4.0', restart_required: false,
    });
    expect(await request(runtime, 'GET', '/config/plugins/configs/response-tags')).toEqual({ enabled: true });
    expect(await request(runtime, 'DELETE', '/plugins/response-tags')).toMatchObject({ file_deleted: true, configured_removed: true });
    await expect(request(runtime, 'GET', '/config/plugins/configs/response-tags')).rejects.toThrow('Management API error (404): not_found');
    expect(await request(runtime, 'GET', '/plugins')).toMatchObject({ plugins: [{ id: 'request-inspector' }] });
  });

  test('respects core lifecycle and explicit error scenarios', async () => {
    const stopped = createBrowserMockRuntime('stopped');
    await expect(stopped.invoke('get_plugin_support')).rejects.toThrow('core is not ready');
    await stopped.invoke('start_core_process');
    expect(await stopped.invoke('get_plugin_support')).toBe(true);
    await stopped.invoke('stop_core_process');
    await expect(request(stopped, 'GET', '/plugins')).rejects.toThrow('core is not ready');
    const empty = createBrowserMockRuntime('empty');
    await empty.invoke('install_bundled_core');
    await empty.invoke('start_core_process');
    expect(await request(empty, 'GET', '/plugins')).toMatchObject({ plugins_enabled: false, plugins: [] });
    const error = createBrowserMockRuntime('error');
    await expect(error.invoke('get_plugin_support')).rejects.toThrow('Browser Mock error scenario');
    await expect(error.invoke('get_plugin_resource_url')).rejects.toThrow('Browser Mock error scenario');
    await expect(request(error, 'GET', '/plugins')).rejects.toThrow('Browser Mock error scenario');
  });
});
