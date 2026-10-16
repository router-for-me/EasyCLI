type JsonObject = Record<string, unknown>;

export interface PluginMockHost {
  scenario: 'running' | 'stopped' | 'empty' | 'error';
  coreStatus: { ready: boolean };
  coreConfig: { pluginsEnabled: boolean };
  extendedConfig: JsonObject;
}

export type PluginMockResult = { handled: false } | { handled: true; value: unknown };

export interface PluginWebviewMockState {
  visible: boolean;
  bounds: { x: number; y: number; width: number; height: number; viewportHeight: number };
  url: string;
  reloads: number;
  labels: { back: string; reload: string; openExternal: string; move: string } | null;
}

const MOCK_CORE_ORIGIN = 'http://127.0.0.1:8317';
const WEBVIEW_COMMANDS = new Set([
  'show_plugin_webview', 'set_plugin_webview_bounds', 'hide_plugin_webview', 'close_plugin_webview',
]);
const initialWebview = (): PluginWebviewMockState => ({
  visible: false, bounds: { x: 0, y: 0, width: 0, height: 0, viewportHeight: 0 }, url: '', reloads: 0, labels: null,
});

function readToken(value: unknown): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value <= 0) throw new Error('Invalid plugin page token');
  return value;
}

function readLabels(value: unknown): NonNullable<PluginWebviewMockState['labels']> {
  const source = object(value);
  const [back, reload, openExternal, move] = ['back', 'reload', 'openExternal', 'move'].map((key) => {
    const entry = source[key];
    if (typeof entry !== 'string' || !entry) throw new Error('Invalid plugin page labels');
    return entry;
  });
  return { back, reload, openExternal, move };
}

function readBounds(value: unknown): PluginWebviewMockState['bounds'] {
  const source = object(value);
  const [x, y, width, height, viewportHeight] = ['x', 'y', 'width', 'height', 'viewportHeight'].map((key) => {
    const entry = source[key];
    if (typeof entry !== 'number' || !Number.isFinite(entry)) throw new Error('Plugin page bounds must be finite numbers');
    return Math.max(0, Math.round(entry));
  });
  return { x, y, width, height, viewportHeight };
}

const object = (value: unknown): JsonObject =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as JsonObject : {};
const text = (value: unknown): string => typeof value === 'string' ? value : '';
const has = (value: JsonObject, key: string): boolean => Object.prototype.hasOwnProperty.call(value, key);
const clone = <T,>(value: T): T => structuredClone(value);
const COMMUNITY_SOURCE = 'https://plugins.example.invalid/community.json';
const MAX_EXTRA_PAGES = 30;
const NOT_FOUND = 'Management API error (404): not_found';
const CONFIG_SAVED = { status: 'ok', 'config-version': 8 };

const CATALOG = [
  {
    id: 'request-inspector', name: 'Request Inspector', description: 'Inspect request metadata and tune sampling in this browser demo.',
    source_id: 'official', source_name: 'official', source_url: 'https://plugins.example.invalid/official.json',
    repository: 'router-for-me/request-inspector', author: 'router-for-me', version: '1.1.0',
    install_type: 'github-release', auth_required: false, auth_configured: false,
    platforms: [{ goos: 'windows', goarch: 'amd64' }, { goos: 'linux', goarch: 'amd64' }],
    license: 'MIT', tags: ['observability', 'demo'], logo: '', homepage: '',
    config_fields: [
      { name: 'label', type: 'string', description: 'Display label for this inspector.', enum_values: [] },
      { name: 'sample-rate', type: 'number', description: 'Share of requests to sample, from 0 to 1.', enum_values: [] },
      { name: 'redact-secrets', type: 'boolean', description: 'Hide secret values in captured metadata.', enum_values: [] },
      { name: 'mode', type: 'enum', description: 'Detail level for captured metadata.', enum_values: ['summary', 'detailed'] },
    ],
    menus: [
      { path: '/v0/resource/plugins/request-inspector/status', menu: 'Inspector status', description: 'Open the local browser demo status page.' },
      { path: '/v0/resource/plugins/request-inspector/samples', menu: 'Sampled requests', description: 'Open the local browser demo sampling page.' },
    ],
  },
  {
    id: 'response-tags', name: 'Response Tags', description: 'A community demo plugin for custom response tags.',
    source_id: 'community-demo', source_name: 'Community demo', source_url: COMMUNITY_SOURCE,
    repository: 'demo-community/response-tags', author: 'Demo community', version: '0.3.0',
    install_type: 'github-release', auth_required: false, auth_configured: false,
    platforms: [{ goos: 'windows', goarch: 'amd64' }, { goos: 'linux', goarch: 'amd64' }],
    license: 'MIT', tags: ['response', 'demo'], logo: '', homepage: '', config_fields: [], menus: [],
  },
];

function mergePatch(current: unknown, patch: unknown): unknown {
  if (!patch || typeof patch !== 'object' || Array.isArray(patch)) return clone(patch);
  const entries = new Map(Object.entries(object(current)));
  for (const [key, value] of Object.entries(patch)) entries.set(key, mergePatch(entries.get(key), value));
  return Object.fromEntries(entries);
}

export const clampPluginPages = (value: unknown): number => {
  const count = Math.round(Number(value));
  return Number.isFinite(count) ? Math.max(0, Math.min(MAX_EXTRA_PAGES, count)) : 0;
};

/** Pure in-memory plugin API. The plugin page webview is only simulated and never contacts a core. */
export function createPluginMock(host: PluginMockHost, extraPages = 0, onWebview: (state: PluginWebviewMockState) => void = () => {}) {
  const extraMenus = Array.from({ length: clampPluginPages(extraPages) }, (_, index) => ({
    path: `/v0/resource/plugins/request-inspector/extra-${index + 1}`,
    menu: `Extra page ${index + 1}`, description: 'Extra browser demo page.',
  }));
  const installed = new Map<string, { version: string; sourceId: string }>();
  let initialized = false;
  // Seed only when the plugin feature is used so existing offline configuration
  // scenarios keep their original absent fields and optimistic-edit expectations.
  function initialize() {
    if (initialized) return;
    initialized = true;
    if (host.scenario !== 'empty') installed.set('request-inspector', { version: '1.0.0', sourceId: 'official' });
    const existing = object(host.extendedConfig.plugins);
    host.extendedConfig.plugins = {
      enabled: host.scenario !== 'empty', dir: 'plugins',
      'store-sources': [COMMUNITY_SOURCE], 'store-auth': [],
      ...existing,
      configs: { ...(host.scenario === 'empty' ? {} : {
        'request-inspector': { enabled: true, priority: 0, label: 'Browser demo', 'sample-rate': 0.25, 'redact-secrets': true, mode: 'summary' },
      }), ...object(existing.configs) },
    };
    host.coreConfig.pluginsEnabled = object(host.extendedConfig.plugins).enabled === true;
  }

  let webview = initialWebview();
  let owner: number | null = null;
  let latestToken = 0;
  const updateWebview = (patch: Partial<PluginWebviewMockState>) => {
    webview = { ...webview, ...patch };
    onWebview(clone(webview));
  };

  const settings = () => object(host.extendedConfig.plugins);
  const configs = () => object(settings().configs);
  const enabled = () => settings().enabled === true;
  const directory = () => text(settings().dir) || 'plugins';
  const filePath = (id: string) => `${directory()}/windows/amd64/${id}.dll`;
  const plugin = (id: string) => {
    const entry = CATALOG.find(item => item.id === id);
    const local = installed.get(id);
    const config = object(configs()[id]);
    const active = Boolean(local) && enabled() && config.enabled === true;
    return {
      id, path: local ? filePath(id) : '', configured: has(configs(), id), registered: active,
      enabled: config.enabled === true, effective_enabled: active,
      supports_oauth: false, oauth_provider: '', supports_quota: false, quota_provider: '', logo: '',
      config_fields: entry?.config_fields ?? [], menus: active ? [...entry?.menus ?? [], ...id === 'request-inspector' ? extraMenus : []] : [],
      metadata: local && entry ? {
        name: entry.name, version: local.version, author: entry.author,
        github_repository: entry.repository, logo: '', config_fields: entry.config_fields,
      } : null,
    };
  };

  function configRequest(method: string, path: string, body: unknown) {
    const parts = path.slice('/config/plugins'.length).split('/').filter(Boolean).map(decodeURIComponent);
    let current = settings();
    for (const key of parts.slice(0, -1)) {
      if (!has(current, key)) {
        if (method === 'GET' || method === 'DELETE') throw new Error(NOT_FOUND);
        Object.defineProperty(current, key, { value: {}, enumerable: true, configurable: true, writable: true });
      }
      current = object(current[key]);
    }
    const key = parts[parts.length - 1];
    if (method === 'GET') {
      if (key && !has(current, key)) throw new Error(NOT_FOUND);
      return clone(key ? current[key] : current);
    }
    if (method === 'DELETE') {
      if (!key || !has(current, key)) throw new Error(NOT_FOUND);
      delete current[key];
    } else if (method === 'PUT' || method === 'PATCH') {
      const value = method === 'PATCH' ? mergePatch(key ? current[key] : current, body) : clone(body);
      if (key) Object.defineProperty(current, key, { value, enumerable: true, configurable: true, writable: true });
      else host.extendedConfig.plugins = value;
    } else throw new Error('Management API error (405): method_not_allowed');
    host.coreConfig.pluginsEnabled = enabled();
    return CONFIG_SAVED;
  }

  function webviewCommand(command: string, payload: JsonObject): unknown {
    if (command === 'show_plugin_webview') {
      const pluginId = text(payload.pluginId);
      const menuIndex = payload.menuIndex;
      if (!pluginId) throw new Error('Invalid plugin id');
      if (typeof menuIndex !== 'number' || !Number.isInteger(menuIndex) || menuIndex < 0) throw new Error('Invalid menu index');
      if (typeof payload.reload !== 'boolean') throw new Error('Invalid reload flag');
      const bounds = readBounds(payload.bounds);
      const labels = readLabels(payload.labels);
      const token = readToken(payload.token);
      if (token <= latestToken) throw new Error('Plugin page request was superseded');
      latestToken = token;
      owner = token;
      const entry = plugin(pluginId);
      const menu = entry.menus[menuIndex];
      if (!entry.effective_enabled || !menu) {
        owner = null;
        updateWebview({ visible: false });
        throw new Error('Plugin page is unavailable');
      }
      const url = `${MOCK_CORE_ORIGIN}${menu.path}`;
      const reloads = webview.url === url && payload.reload ? webview.reloads + 1 : webview.reloads;
      updateWebview({ url, reloads, labels, bounds, visible: bounds.width > 0 && bounds.height > 0 });
      return url;
    }
    if (command === 'close_plugin_webview') {
      owner = null;
      webview = initialWebview();
      onWebview(clone(webview));
    } else if (command === 'set_plugin_webview_bounds') {
      const bounds = readBounds(payload.bounds);
      if (readToken(payload.token) === owner) updateWebview({ bounds, visible: bounds.width > 0 && bounds.height > 0 });
    } else {
      const token = readToken(payload.token);
      if (token >= latestToken) {
        latestToken = token;
        owner = null;
        updateWebview({ visible: false });
      }
    }
    return null;
  }

  function handle(command: string, payload: JsonObject): PluginMockResult {
    const request = object(payload.request);
    const path = text(request.path);
    const pluginRequest = command === 'management_request' && (
      path === '/plugins' || path.startsWith('/plugins/') || path === '/config/plugins' || path.startsWith('/config/plugins/')
    );
    if (!pluginRequest && command !== 'get_plugin_support' && command !== 'get_plugin_latest_releases' && !WEBVIEW_COMMANDS.has(command)) return { handled: false };
    if (host.scenario === 'error') throw new Error(`Browser Mock error scenario: ${command}`);
    if (!host.coreStatus.ready && (!WEBVIEW_COMMANDS.has(command) || command === 'show_plugin_webview')) {
      throw new Error('Browser Mock: the core is not ready');
    }
    initialize();
    const result = (value: unknown): PluginMockResult => ({ handled: true, value: clone(value) });
    if (command === 'get_plugin_support') return result(true);
    if (WEBVIEW_COMMANDS.has(command)) return result(webviewCommand(command, payload));
    if (command === 'get_plugin_latest_releases') {
      const repositories = Array.isArray(payload.repositories) ? payload.repositories.map(text) : [];
      return result(repositories.map(repository => {
        const entry = CATALOG.find(entry => entry.repository.replace(/^https:\/\/github\.com\//, '') === repository);
        return entry ? { repository, version: entry.version, error: null } : { repository, version: null, error: 'Invalid GitHub repository' };
      }));
    }
    const method = text(request.method).toUpperCase();
    if (path === '/config/plugins' || path.startsWith('/config/plugins/')) return result(configRequest(method, path, request.body));
    if (method === 'GET' && path === '/plugins') return result({
      plugins_enabled: enabled(), plugins_dir: directory(),
      plugins: [...new Set([...installed.keys(), ...Object.keys(configs())])].map(plugin),
    });
    if (method === 'GET' && path === '/plugins/store') {
      const sources = settings()['store-sources'];
      const catalog = CATALOG.filter(entry => entry.source_id === 'official' || (Array.isArray(sources) && sources.includes(entry.source_url)));
      return result({ plugins_enabled: enabled(), plugins_dir: directory(), source_errors: [], plugins: catalog.map(entry => {
        const local = installed.get(entry.id);
        return { ...entry, ...plugin(entry.id), name: entry.name, version: entry.version,
          store_id: `${entry.source_id}/${entry.id}`, installed: Boolean(local), installed_version: local?.version ?? '',
          installed_source_id: local?.sourceId ?? '', install_source_status: local ? 'matched' : '',
          update_available: Boolean(local && local.version !== entry.version),
        };
      }) });
    }
    const install = /^\/plugins\/store\/([^/]+)\/install$/.exec(path);
    if (method === 'POST' && install) {
      const id = decodeURIComponent(install[1]);
      const query = object(request.query);
      const entry = CATALOG.find(item => item.id === id && (!query.source || item.source_id === query.source));
      if (!entry) throw new Error(NOT_FOUND);
      const version = (text(query.version) || text(object(request.body).version) || entry.version).replace(/^v/, '');
      installed.set(id, { version, sourceId: entry.source_id });
      const next = { ...configs(), [id]: { ...object(configs()[id]), enabled: true } };
      host.extendedConfig.plugins = { ...settings(), configs: next };
      return result({ status: 'installed', id, version, source_id: entry.source_id, source_name: entry.source_name,
        source_url: entry.source_url, install_type: entry.install_type, path: filePath(id),
        plugins_enabled: enabled(), restart_required: false });
    }
    const deletion = /^\/plugins\/([^/]+)$/.exec(path);
    if (method === 'DELETE' && deletion) {
      const id = decodeURIComponent(deletion[1]);
      const existed = installed.delete(id);
      const config = configs();
      const configured = has(config, id);
      if (!existed && !configured) throw new Error(NOT_FOUND);
      delete config[id];
      return result({ status: 'deleted', id, path: existed ? filePath(id) : '', file_deleted: existed,
        configured_removed: configured, restart_required: false });
    }
    throw new Error(NOT_FOUND);
  }

  return { handle };
}
