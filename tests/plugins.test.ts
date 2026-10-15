import { afterEach, beforeEach, describe, expect, it } from 'bun:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  mergePluginConfigPatch, normalizePluginList, normalizePluginStore, pluginsApi, pluginStoreApi,
} from '../src/services/plugins';
import {
  buildRepositoryURL, collectPluginResourceEntries, getPluginConfirmToken, isOfficialPlugin,
  isOfficialRepository, resolvePluginAssetURL,
} from '../src/services/pluginResources';
import {
  buildGitHubReleasesPageURL, isValidManualReleaseTag, supportsPluginVersionSelection,
  isNewerPluginRelease, refreshPluginReleaseVersions,
} from '../src/services/pluginReleaseVersions';
import { buildPluginConfigDraft, buildPluginConfigPatch } from '../src/services/pluginConfigDraft';
import { getPluginStatus, isPluginInstalled } from '../src/services/pluginStatus';

describe('plugin Management API', () => {
  let originalWindow: PropertyDescriptor | undefined;
  beforeEach(() => {
    originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    Object.defineProperty(globalThis, 'window', { value: {}, writable: true, configurable: true });
  });
  afterEach(() => {
    clearMocks();
    if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
    else Reflect.deleteProperty(globalThis, 'window');
  });

  it('preserves the latest unknown config fields and removes or replaces only edited fields', async () => {
    const requests: unknown[] = [];
    mockIPC((command, args) => {
      expect(command).toBe('management_request');
      const request = args?.request as { method: string; path: string; body?: unknown };
      requests.push(request);
      if (request.method === 'GET') return {
        enabled: true, priority: 7, unknown: { concurrent: 'kept' },
        options: { removed: 1, keep: 2 }, old: 'delete', list: [1, 2],
      };
      return { status: 'ok' };
    });
    await pluginsApi.patchConfig('sample..plugin', { options: { keep: 3 }, old: null, list: [], flag: false, zero: 0 });
    expect(requests).toMatchObject([
      { method: 'GET', path: '/config/plugins/configs/sample%2E%2Eplugin' },
      { method: 'PUT', path: '/config/plugins/configs/sample%2E%2Eplugin', body: {
        enabled: true, priority: 7, unknown: { concurrent: 'kept' },
        options: { keep: 3 }, list: [], flag: false, zero: 0,
      } },
    ]);
    expect((requests[1] as { body: unknown }).body).not.toHaveProperty('old');
  });

  it('refreshes GitHub releases without reusing catalog versions or changing source identity', async () => {
    const calls: string[][] = [];
    mockIPC((command, args) => {
      expect(command).toBe('get_plugin_latest_releases');
      const repositories = args?.repositories as string[];
      calls.push(repositories);
      return repositories.map(repository => repository === 'example/offline'
        ? { repository, version: null, error: 'GitHub API rate limited' }
        : { repository, version: '3.0.0', error: null });
    });
    const store = normalizePluginStore({ plugins: [
      { id: 'sample', source_id: 'official', repository: 'https://github.com/example/sample', install_type: 'github-release', version: '1.0.0', installed: true, installed_version: '2.0.0', install_source_status: 'matched' },
      { id: 'sample', source_id: 'other', repository: 'example/sample', install_type: 'github-release', installed: true, installed_version: '2.0.0', install_source_status: 'different' },
      { id: 'direct', install_type: 'direct', version: '1.0.0', repository: 'example/direct' },
      { id: 'private', install_type: 'github-release', auth_required: true, repository: 'example/private' },
      { id: 'offline', install_type: 'github-release', version: '1.0.0', repository: 'example/offline' },
      { id: 'offline', source_id: 'mirror', install_type: 'github-release', version: '1.0.0', repository: 'https://github.com/example/offline' },
      { id: 'untracked', install_type: 'github-release', repository: 'example/sample', installed: true, installed_version: '2.0.0', install_source_status: 'unrecognized' },
    ], source_errors: [{ source_id: 'existing', message: 'Registry unavailable' }] });
    const result = await refreshPluginReleaseVersions(store);
    expect(calls.map(repositories => [...repositories].sort())).toEqual([['example/offline', 'example/sample']]);
    expect(result.plugins[0]).toMatchObject({ storeId: 'official/sample', version: '3.0.0', updateAvailable: true });
    expect(result.plugins[1]).toMatchObject({ storeId: 'other/sample', version: '3.0.0', updateAvailable: false });
    expect(result.plugins[2]).toEqual(store.plugins[2]);
    expect(result.plugins[3]).toEqual(store.plugins[3]);
    expect(result.plugins[4]).toMatchObject({ version: '1.0.0' });
    expect(result.plugins[5]).toMatchObject({ version: '1.0.0' });
    expect(result.plugins[6]).toMatchObject({ version: '3.0.0', updateAvailable: false });
    expect(result.sourceErrors.map(error => error.message)).toEqual(['Registry unavailable', 'GitHub API rate limited']);
    expect(store.plugins[0].version).toBe('1.0.0');
    await refreshPluginReleaseVersions(store);
    expect(calls).toHaveLength(2);
    expect(await refreshPluginReleaseVersions(normalizePluginStore({ plugins: [{ id: 'direct', install_type: 'direct' }] }))).toMatchObject({ plugins: [{ id: 'direct' }] });
    expect(calls).toHaveLength(2);
  });

  it('defaults only a missing configuration node and propagates transport, auth and endpoint errors', async () => {
    mockIPC(() => { throw 'Management API error (404): not_found'; });
    expect(await pluginsApi.getConfig('sample')).toEqual({});
    for (const message of [
      'Management API error (404): 404 page not found',
      'Management API error (401): unauthorized',
      'Management API request failed: connection refused',
    ]) {
      mockIPC(() => { throw new Error(message); });
      await expect(pluginsApi.patchConfig('sample', { label: 'changed' })).rejects.toThrow(message);
    }
    mockIPC(() => []);
    await expect(pluginsApi.getConfig('sample')).rejects.toThrow('Invalid plugin configuration response');
  });

  it('updates global settings with PATCH so other plugin configs and auth revision survive', async () => {
    let request: unknown;
    mockIPC((_command, args) => { request = args?.request; return {}; });
    await pluginsApi.updateSettings({ enabled: true, storeSources: [], storeAuth: [] });
    expect(request).toMatchObject({ method: 'PATCH', path: '/config/plugins', body: {
      enabled: true, 'store-sources': [], 'store-auth': [],
    } });
    expect((request as { body: unknown }).body).not.toHaveProperty('configs');
    expect((request as { body: unknown }).body).not.toHaveProperty('dir');
  });

  it('reads global source/auth configuration without losing unknown auth fields', async () => {
    mockIPC(() => ({ enabled: false, dir: 'custom/plugins', 'store-sources': ['https://store.test/index.json'],
      'store-auth': [{ host: 'store.test', token: 'fixture-only', custom: true }], configs: {} }));
    expect(await pluginsApi.getSettings()).toEqual({ enabled: false, dir: 'custom/plugins',
      storeSources: ['https://store.test/index.json'],
      storeAuth: [{ host: 'store.test', token: 'fixture-only', custom: true }],
    });
  });

  it('toggles one instance, keeps source/version selection, and returns restart flags', async () => {
    const requests: unknown[] = [];
    mockIPC((_command, args) => {
      requests.push(args?.request);
      return { status: 'ok', id: 'sample', source_id: 'private/source', version: 'v1.2.3+test',
        restart_required: true, file_deleted: true, configured_removed: true };
    });
    await pluginsApi.updateEnabled('sample', false);
    const installed = await pluginStoreApi.install('sample', { sourceId: ' private/source ', version: ' v1.2.3+test ' });
    const deleted = await pluginsApi.deletePlugin('sample');
    expect(requests).toMatchObject([
      { method: 'PUT', path: '/config/plugins/configs/sample/enabled', body: false },
      { method: 'POST', path: '/plugins/store/sample/install', query: { source: 'private/source', version: 'v1.2.3+test' },
        body: { version: 'v1.2.3+test' }, timeoutMs: 120_000 },
      { method: 'DELETE', path: '/plugins/sample' },
    ]);
    expect(installed).toMatchObject({ sourceId: 'private/source', version: 'v1.2.3+test', restartRequired: true });
    expect(deleted).toMatchObject({ fileDeleted: true, configuredRemoved: true, restartRequired: true });
  });

  it('rejects path traversal and separators before requests', async () => {
    const requests: unknown[] = [];
    mockIPC((_command, args) => { requests.push(args); return {}; });
    for (const id of ['', '.', '..', '../config', 'a/b', 'a\\b', '\u0000', 'a b', 'a'.repeat(129)]) {
      expect(() => pluginsApi.updateEnabled(id, true)).toThrow('Invalid plugin ID');
      await expect(pluginStoreApi.install(id)).rejects.toThrow('Invalid plugin ID');
    }
    expect(requests).toEqual([]);
  });
});

describe('plugin normalization and resources', () => {
  it('does not treat the core configuration-only response as installed or loading', () => {
    const response = normalizePluginList({ plugins_enabled: true, plugins_dir: 'plugins', plugins:
      ['example', 'kiro', 'model-fallback-router'].map(id => ({
        id, path: '', configured: true, registered: false, enabled: true,
        effective_enabled: false, metadata: null, config_fields: [], menus: [],
      })),
    });
    expect(response.plugins.filter(isPluginInstalled)).toHaveLength(0);
    expect(response.plugins.map(plugin => getPluginStatus(plugin, response.pluginsEnabled)))
      .toEqual(['missingFile', 'missingFile', 'missingFile']);
    expect(collectPluginResourceEntries(response.plugins)).toEqual([]);

    const [configured] = response.plugins;
    const discovered = { ...configured, path: 'plugins/windows/amd64/kiro-v0.1.1.dll' };
    expect(getPluginStatus(discovered, true)).toBe('notLoaded');
    expect(getPluginStatus(discovered, false)).toBe('globalDisabled');
    expect(getPluginStatus({ ...discovered, enabled: false }, true)).toBe('inactive');
    // A loaded library can stay registered after its on-disk file disappears.
    // Keep it available for unloading instead of offering configuration removal.
    const loaded = { ...configured, registered: true, effectiveEnabled: true };
    expect(isPluginInstalled(loaded)).toBe(true);
    expect(getPluginStatus(loaded, true)).toBe('active');
    expect(getPluginStatus(loaded, false)).toBe('globalDisabled');
  });

  it('retains runtime capabilities, schema fallback and backend-declared resource paths', () => {
    const result = normalizePluginList({ plugins_enabled: true, plugins_dir: '/plugins', plugins: [null, {}, {
      id: 'sample', enabled: false, effective_enabled: true, registered: true,
      supports_oauth: true, oauth_provider: 'custom-oauth', supports_quota: true, quota_provider: 'custom',
      metadata: { name: 'Sample', config_fields: [{ name: 'count', type: 'integer' }] },
      menus: [null, { path: '', menu: 'Unavailable' }, { path: '/v0/resource/plugins/sample/ui?view=usage', menu: 'Usage' }],
    }] });
    expect(result.plugins).toHaveLength(1);
    expect(result.plugins[0]).toMatchObject({ enabled: false, supportsOAuth: true, oauthProvider: 'custom-oauth',
      supportsQuota: true, quotaProvider: 'custom', configFields: [{ name: 'count', type: 'integer' }],
    });
    const [resource] = collectPluginResourceEntries(result.plugins);
    expect(resource).toMatchObject({ pluginID: 'sample', label: 'Usage', menuIndex: 2 });
    expect(resolvePluginAssetURL(resource.menu.path, 'http://127.0.0.1:8317')).toBe(
      'http://127.0.0.1:8317/v0/resource/plugins/sample/ui?view=usage',
    );
    expect(collectPluginResourceEntries([{ ...result.plugins[0], effectiveEnabled: false }])).toEqual([]);
    for (const path of ['javascript:alert(1)', 'data:text/html,<script>', '//evil.test/ui', '/\\evil.test/ui']) {
      expect(resolvePluginAssetURL(path, 'http://127.0.0.1:8317')).toBe('');
    }
  });

  it('keeps registry identity and failures when duplicate IDs exist across sources', () => {
    const result = normalizePluginStore({ plugins: [
      { id: 'sample', source_id: 'official', installed: true, installed_source_id: 'official', install_source_status: 'match' },
      { id: 'sample', source_id: 'third-party', installed: false },
    ], source_errors: [{ source_id: 'offline', source_url: 'https://offline.test', message: 'Unavailable' }] });
    expect(result.plugins.map((entry) => entry.storeId)).toEqual(['official/sample', 'third-party/sample']);
    expect(result.plugins[0]).toMatchObject({ installedSourceId: 'official', installSourceStatus: 'match' });
    expect(result.sourceErrors[0]).toMatchObject({ sourceId: 'offline', message: 'Unavailable' });
    expect(() => normalizePluginStore({ error: 'unavailable' })).toThrow();
    expect(() => normalizePluginList('not JSON')).toThrow();
  });
});

describe('plugin installation trust and versions', () => {
  it('matches core update comparisons for dotted releases and other tags', () => {
    expect(isNewerPluginRelease('v1.9.0', '1.10.0')).toBe(true);
    expect(isNewerPluginRelease('2.0.0', '1.9.0')).toBe(false);
    expect(isNewerPluginRelease('V1.0', '1.0.0')).toBe(false);
    expect(isNewerPluginRelease('', '1.0.0')).toBe(false);
    expect(isNewerPluginRelease('1.0.0', '1.0.0-rc.1')).toBe(true);
  });
  it('requires the official source and a canonical official GitHub repository', () => {
    for (const repository of ['router-for-me/sample', 'https://github.com/router-for-me/sample.git']) {
      expect(isOfficialPlugin({ sourceId: 'official', repository })).toBe(true);
      expect(isOfficialPlugin({ sourceId: 'untrusted', repository })).toBe(false);
    }
    for (const repository of [
      'other/sample', 'https://github.com.evil.test/router-for-me/sample',
      'https://github.com@evil.test/router-for-me/sample', 'https://evil.test/router-for-me/sample',
      'http://github.com/router-for-me/sample', 'https://github.com/router-for-me/../other/sample',
      'https://github.com/router-for-me/sample?redirect=evil', 'javascript:alert(1)',
    ]) expect(isOfficialRepository(repository)).toBe(false);
    expect(buildRepositoryURL('javascript:alert(1)')).toBe('');
    expect(getPluginConfirmToken({ repository: 'other/sample', id: 'plugin' })).toBe('other/sample');
    expect(getPluginConfirmToken({ repository: '', id: 'plugin' })).toBe('plugin');
  });

  it('limits manual selection to GitHub releases and rejects unsafe tags', () => {
    expect(supportsPluginVersionSelection(' GitHub-Release ')).toBe(true);
    expect(supportsPluginVersionSelection('direct')).toBe(false);
    for (const version of ['v1.2.3', '1.0.0-rc.1+build']) expect(isValidManualReleaseTag(version)).toBe(true);
    for (const version of ['', '../v1', 'v1/next', 'x'.repeat(129), 'v1?foo=bar']) expect(isValidManualReleaseTag(version)).toBe(false);
    expect(buildGitHubReleasesPageURL('router-for-me/sample.git')).toBe('https://github.com/router-for-me/sample/releases');
    expect(buildGitHubReleasesPageURL('https://evil.test/router-for-me/sample')).toBe('');
  });
});

describe('schema-driven plugin configuration', () => {
  const fields = [
    { name: 'prompt', type: 'string', enumValues: [], description: '' },
    { name: 'items', type: 'array', enumValues: [], description: '' },
    { name: 'count', type: 'integer', enumValues: [], description: '' },
    { name: 'optional', type: 'boolean', enumValues: [], description: '' },
  ];
  const t = (key: string) => key;

  it('does not persist untouched defaults and preserves meaningful string whitespace', () => {
    const draft = buildPluginConfigDraft({ enabled: false, configFields: fields }, {});
    expect(buildPluginConfigPatch(draft, fields, t).patch).toEqual({});
    draft.values.prompt = '  secret or prompt\n';
    draft.touchedFields.prompt = true;
    draft.touchedFields.optional = true;
    expect(buildPluginConfigPatch(draft, fields, t).patch).toEqual({ prompt: '  secret or prompt\n', optional: false });
    draft.values.prompt = '';
    expect(buildPluginConfigPatch(draft, fields, t).patch.prompt).toBeNull();
  });

  it('validates array structure and safe integers, preserving mixed JSON data', () => {
    const draft = buildPluginConfigDraft({ enabled: true, configFields: fields }, {});
    draft.touchedFields.items = true;
    draft.values.items = '{"not":"array"}';
    draft.touchedFields.count = true;
    draft.values.count = '9007199254740993';
    draft.priorityTouched = true;
    draft.priority = '1.5';
    expect(buildPluginConfigPatch(draft, fields, t).errors).toEqual({
      priority: 'plugin_management.invalid_priority', items: 'plugin_management.expected_array',
      count: 'plugin_management.invalid_integer',
    });
    draft.values.items = '[0,false,{"nested":[1]}]';
    draft.values.count = '0';
    draft.priority = '-2';
    expect(buildPluginConfigPatch(draft, fields, t)).toEqual({
      patch: { priority: -2, items: [0, false, { nested: [1] }], count: 0 }, errors: {},
    });
  });

  it('treats prototype-looking config keys as data', () => {
    const current = JSON.parse('{"__proto__":{"keep":true},"constructor":"original","safe":true}');
    const changes = JSON.parse('{"__proto__":{"next":true},"constructor":null}');
    const merged = mergePluginConfigPatch(current, changes);
    expect(Object.hasOwn(merged, '__proto__')).toBe(true);
    expect(merged.__proto__).toEqual({ next: true });
    expect(Object.hasOwn(merged, 'constructor')).toBe(false);
    expect(Object.getPrototypeOf(merged)).toBe(Object.prototype);
    const prototypeField = [{ name: '__proto__', type: 'string', enumValues: [], description: '' }];
    const draft = buildPluginConfigDraft({ enabled: false, configFields: prototypeField }, current);
    draft.values.__proto__ = 'literal';
    draft.touchedFields.__proto__ = true;
    expect(Object.hasOwn(buildPluginConfigPatch(draft, prototypeField, t).patch, '__proto__')).toBe(true);
    const inheritedField = [{ name: 'constructor', type: 'string', enumValues: [], description: '' }];
    const untouched = buildPluginConfigDraft({ enabled: false, configFields: inheritedField }, {});
    untouched.touchedFields = { ...untouched.touchedFields };
    expect(untouched.values.constructor).toBe('');
    expect(Object.keys(buildPluginConfigPatch(untouched, inheritedField, t).patch)).toEqual([]);
  });

  it('keeps literal nulls in raw JSON while removing only explicitly deleted keys', () => {
    expect(mergePluginConfigPatch({ latest: { preserved: true }, remove: 'old', nullable: 1 },
      { nullable: null, newNull: null }, { nullMeansDelete: false, removeKeys: ['remove'] })).toEqual({
      latest: { preserved: true }, nullable: null, newNull: null,
    });
    expect(mergePluginConfigPatch({ nullable: 1 }, { nullable: null })).toEqual({});
  });
});
