import { describe, expect, test } from 'bun:test';
import { createBrowserMockRuntime } from '../src/mocks/browserMockRuntime';
import type { PluginWebviewMockState } from '../src/mocks/pluginMock';

const BOUNDS = { x: 10, y: 20, width: 300, height: 200, viewportHeight: 768 };
const LABELS = { back: 'Back', reload: 'Reload', openExternal: 'Open in browser', move: 'Move controls' };
const show = (runtime: ReturnType<typeof createBrowserMockRuntime>, payload: Record<string, unknown> = {}) =>
  runtime.invoke('show_plugin_webview', { pluginId: 'request-inspector', menuIndex: 0, bounds: BOUNDS, labels: LABELS, reload: false, token: 1, ...payload });

const request = (runtime: ReturnType<typeof createBrowserMockRuntime>, method: string, path: string, body?: unknown, query?: unknown) =>
  runtime.invoke('management_request', { request: { method, path, body, query } });

describe('browser plugin mock', () => {
  test('exposes installed plugins and the update catalog', async () => {
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
      plugins: [{ id: 'request-inspector', effective_enabled: true, menus: [{ menu: 'Inspector status' }, { menu: 'Sampled requests' }] }],
    });
    expect(await request(runtime, 'GET', '/plugins/store')).toMatchObject({ plugins: [
      { id: 'request-inspector', installed: true, update_available: true },
      { id: 'response-tags', source_id: 'community-demo', installed: false },
    ] });
  });

  test('adds clamped extra plugin pages on request', async () => {
    const menus = async (count: number) => {
      const runtime = createBrowserMockRuntime('running', () => {}, 0, count);
      const list = await request(runtime, 'GET', '/plugins') as { plugins: { menus: { menu: string }[] }[] };
      return list.plugins[0].menus.map(menu => menu.menu);
    };
    expect(await menus(3)).toEqual(['Inspector status', 'Sampled requests', 'Extra page 1', 'Extra page 2', 'Extra page 3']);
    expect((await menus(500)).length).toBe(32);
    expect((await menus(-4)).length).toBe(2);
    const runtime = createBrowserMockRuntime('running', () => {}, 0, 2);
    expect(await show(runtime, { menuIndex: 3 })).toEndWith('/extra-2');
  });

  test('simulates the native plugin webview without loading a page', async () => {
    const states: PluginWebviewMockState[] = [];
    const runtime = createBrowserMockRuntime('running', () => {}, 0, 0, (state) => states.push(state));
    const last = () => states[states.length - 1];
    const place = (bounds: unknown, token = 2) => runtime.invoke('set_plugin_webview_bounds', { bounds, token });
    expect(await show(runtime)).toBe('http://127.0.0.1:8317/v0/resource/plugins/request-inspector/status');
    expect(last()).toMatchObject({ visible: true, bounds: BOUNDS, reloads: 0, labels: LABELS });
    await show(runtime, { reload: true, token: 2 });
    expect(last().reloads).toBe(1);
    await place({ x: -4.4, y: 7.6, width: 99.5, height: 50, viewportHeight: 600.4 });
    expect(last()).toMatchObject({ visible: true, bounds: { x: 0, y: 8, width: 100, height: 50, viewportHeight: 600 } });
    await place({ ...BOUNDS, width: 0, height: 50 });
    expect(last().visible).toBe(false);
    await place(BOUNDS);
    await runtime.invoke('hide_plugin_webview', { token: 2 });
    expect(last()).toMatchObject({ visible: false, bounds: BOUNDS });
    await place({ ...BOUNDS, x: 99 });
    expect(last()).toMatchObject({ visible: false, bounds: BOUNDS });
    expect(await show(runtime, { menuIndex: 1, token: 3 })).toEndWith('/samples');
    expect(last()).toMatchObject({ visible: true, reloads: 1 });
    await runtime.invoke('close_plugin_webview');
    expect(last()).toMatchObject({ visible: false, url: '', reloads: 0, labels: null });
  });

  test('only the page that last showed the webview may place or hide it', async () => {
    const states: PluginWebviewMockState[] = [];
    const runtime = createBrowserMockRuntime('running', () => {}, 0, 0, (state) => states.push(state));
    const last = () => states[states.length - 1];
    await show(runtime, { token: 1 });
    await show(runtime, { menuIndex: 1, token: 2 });
    await runtime.invoke('hide_plugin_webview', { token: 1 });
    expect(last().visible).toBe(true);
    await runtime.invoke('set_plugin_webview_bounds', { bounds: { ...BOUNDS, x: 50 }, token: 1 });
    expect(last().bounds.x).toBe(10);
    await runtime.invoke('set_plugin_webview_bounds', { bounds: { ...BOUNDS, x: 50 }, token: 2 });
    expect(last().bounds.x).toBe(50);
    await runtime.invoke('hide_plugin_webview', { token: 2 });
    expect(last().visible).toBe(false);
    await runtime.invoke('set_plugin_webview_bounds', { bounds: BOUNDS, token: 2 });
    expect(last().visible).toBe(false);
    await show(runtime, { token: 4 });
    await expect(show(runtime, { menuIndex: 1, token: 3 })).rejects.toThrow('superseded');
    expect(last()).toMatchObject({ visible: true });
    expect(last().url).toEndWith('/status');
    await expect(show(runtime, { menuIndex: 9, token: 5 })).rejects.toThrow('unavailable');
    expect(last()).toMatchObject({ visible: false });
    await runtime.invoke('set_plugin_webview_bounds', { bounds: BOUNDS, token: 5 });
    expect(last()).toMatchObject({ visible: false });
    await show(runtime, { token: 6 });
    await runtime.invoke('hide_plugin_webview', { token: 8 });
    expect(last()).toMatchObject({ visible: false });
    await expect(show(runtime, { token: 8 })).rejects.toThrow('superseded');
    expect(last()).toMatchObject({ visible: false });
    for (const command of ['hide_plugin_webview', 'set_plugin_webview_bounds']) {
      for (const token of [0, -1, 1.5, '1', undefined]) {
        await expect(runtime.invoke(command, { bounds: BOUNDS, token })).rejects.toThrow('Invalid plugin page token');
      }
    }
  });

  test('does not require a ready core to place, hide or close the webview', async () => {
    const stopped = createBrowserMockRuntime('stopped');
    await expect(stopped.invoke('set_plugin_webview_bounds', { bounds: BOUNDS, token: 1 })).resolves.toBeNull();
    await expect(stopped.invoke('hide_plugin_webview', { token: 1 })).resolves.toBeNull();
    await expect(stopped.invoke('close_plugin_webview')).resolves.toBeNull();
    await expect(show(stopped)).rejects.toThrow('core is not ready');
  });

  test('rejects invalid plugin webview payloads and unavailable pages', async () => {
    const runtime = createBrowserMockRuntime('running');
    await expect(show(runtime, { pluginId: '' })).rejects.toThrow('Invalid plugin id');
    await expect(show(runtime, { pluginId: 7 })).rejects.toThrow('Invalid plugin id');
    for (const menuIndex of [-1, 1.5, '0', undefined, null]) {
      await expect(show(runtime, { menuIndex })).rejects.toThrow('Invalid menu index');
    }
    await expect(show(runtime, { reload: 'yes' })).rejects.toThrow('Invalid reload flag');
    await expect(show(runtime, { reload: undefined })).rejects.toThrow('Invalid reload flag');
    for (const bounds of [undefined, {}, { ...BOUNDS, x: Number.NaN }, { ...BOUNDS, width: '300' }, { ...BOUNDS, height: Infinity }, { ...BOUNDS, viewportHeight: undefined }]) {
      await expect(show(runtime, { bounds })).rejects.toThrow('finite numbers');
      await expect(runtime.invoke('set_plugin_webview_bounds', { bounds, token: 1 })).rejects.toThrow('finite numbers');
    }
    for (const labels of [undefined, {}, { ...LABELS, back: '' }, { ...LABELS, reload: 1 }, { ...LABELS, move: '' }, { back: 'Back', reload: 'Reload', openExternal: 'Open in browser' }]) {
      await expect(show(runtime, { labels })).rejects.toThrow('Invalid plugin page labels');
    }
    for (const token of [0, -1, 1.5, '1', undefined]) {
      await expect(show(runtime, { token })).rejects.toThrow('Invalid plugin page token');
    }
    await expect(show(runtime, { menuIndex: 2 })).rejects.toThrow('Plugin page is unavailable');
    await expect(show(runtime, { pluginId: 'unknown', token: 2 })).rejects.toThrow('Plugin page is unavailable');
    for (const command of ['plugin_webview_back', 'plugin_webview_reload', 'open_plugin_webview_external']) {
      await expect(runtime.invoke(command)).rejects.toThrow();
    }
  });

  test('keeps configuration, global state and resource availability synchronized', async () => {
    const runtime = createBrowserMockRuntime('running');
    const path = '/config/plugins/configs/request-inspector';
    await request(runtime, 'PUT', path, { enabled: true, priority: 5, options: { first: 1 }, literal: null });
    await request(runtime, 'PATCH', path, { options: { second: false }, nullable: null });
    expect(await request(runtime, 'GET', path)).toEqual({ enabled: true, priority: 5, options: { first: 1, second: false }, literal: null, nullable: null });
    await request(runtime, 'DELETE', `${path}/literal`);
    await request(runtime, 'PUT', `${path}/enabled`, false);
    await expect(show(runtime)).rejects.toThrow('Plugin page is unavailable');
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
    await expect(error.invoke('show_plugin_webview')).rejects.toThrow('Browser Mock error scenario');
    await expect(error.invoke('close_plugin_webview')).rejects.toThrow('Browser Mock error scenario');
    await expect(stopped.invoke('show_plugin_webview')).rejects.toThrow('core is not ready');
    await expect(stopped.invoke('close_plugin_webview')).resolves.toBeNull();
    await expect(createBrowserMockRuntime('running').invoke('get_plugin_resource_url')).rejects.toThrow();
    await expect(request(error, 'GET', '/plugins')).rejects.toThrow('Browser Mock error scenario');
  });
});
