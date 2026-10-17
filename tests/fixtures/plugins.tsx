import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { I18nProvider } from '../../src/i18n';
import { PluginsPage } from '../../src/pages/PluginsPage';
import '../../src/styles/index.css';

const params = new URLSearchParams(location.search);
localStorage.setItem('easy-cli-proxy-api.locale', params.get('locale') ?? 'en');
document.documentElement.dataset.theme = params.get('theme') ?? 'light';
type Request = { method: string; path: string; query?: Record<string, string>; body?: unknown };
const state = {
  localSelection: '/downloads/local-demo-v1.0.0.dll' as string | null,
  localInstallError: '',
  supported: !params.has('unsupported'),
  commands: [] as { command: string; args: unknown }[],
  requests: [] as Request[],
  installs: [] as Request[],
  navigations: [] as unknown[],
  failNextList: false,
  failNextInstall: false,
  latestReleaseVersion: '3.0.0',
  holdInstall: false,
  releaseInstall: null as null | (() => void),
  settings: {
    enabled: true, dir: 'plugins', 'store-sources': ['https://registry.example.test/plugins.json'],
    'store-auth': [{ host: 'registry.example.test', 'token-env': 'PLUGIN_FIXTURE_TOKEN' }],
    'auth-revision': 9,
    configs: {
      'sample..plugin': { enabled: true, priority: 3, endpoint: 'https://example.test/api', untouched: { retained: true } },
      dormant: { enabled: false },
      kiro: { enabled: true },
      unloaded: { enabled: true },
    } as Record<string, Record<string, unknown>>,
  },
};
Object.assign(window, { pluginsFixture: state });
window.addEventListener('app:navigate', (event) => state.navigations.push((event as CustomEvent).detail));
const pluginEntries: Record<string, unknown>[] = [
  { id: 'sample..plugin', path: '/plugins/sample..plugin.dll', configured: true, registered: true,
    supports_oauth: true, oauth_provider: 'sample-sso',
    metadata: { name: 'Sample provider', version: '1.0.0', author: 'Fixture author' },
    config_fields: [{ name: 'endpoint', type: 'string', description: 'Endpoint URL' }],
    menus: [null, { path: '', menu: 'Empty' }, { path: '/v0/resource/plugins/sample..plugin/usage', menu: 'Usage dashboard' }],
  },
  { id: 'dormant', path: '/plugins/dormant.dll', configured: true, registered: true,
    supports_oauth: true, oauth_provider: 'dormant-sso',
    metadata: { name: 'Dormant provider', version: '1.0.0' }, menus: [], config_fields: [],
  },
  { id: 'kiro', path: '', configured: true, registered: false,
    supports_oauth: false, metadata: null, menus: [], config_fields: [],
  },
  { id: 'unloaded', path: '/plugins/unloaded.dll', configured: true, registered: false,
    supports_oauth: false, metadata: { name: 'Unloaded provider', version: '1.0.0' }, menus: [], config_fields: [],
  },
];
const storeEntries: Record<string, unknown>[] = [
  { id: 'sample..plugin', name: 'Sample provider', source_id: 'official', source_name: 'Official',
    source_url: 'https://official.example.test/plugins.json', repository: 'router-for-me/sample',
    version: '2.0.0', install_type: 'github-release', installed: true, installed_version: '1.0.0', update_available: true, install_source_status: 'matched',
    description: 'Official fixture provider', platforms: [{ goos: 'windows', goarch: 'amd64' }], tags: ['chat'],
  },
  { id: 'analytics', name: 'Community analytics', source_id: 'community', source_name: 'Community',
    source_url: 'https://registry.example.test/plugins.json', repository: 'router-for-me/analytics',
    version: '1.0.0', install_type: 'github-release', description: 'Third-party registry with official-looking metadata',
  },
  { id: 'direct', name: 'Direct artifact', source_id: 'official', source_name: 'Official',
    repository: 'router-for-me/direct', version: '1.0.0', install_type: 'direct',
  },
  { id: 'private', name: 'Private provider', source_id: 'private', source_name: 'Private',
    repository: 'private/provider', auth_required: true, auth_configured: false, install_type: 'github-release',
  },
  { id: 'kiro', name: 'Kiro', source_id: 'official', source_name: 'Official',
    repository: 'router-for-me/kiro', version: '0.1.1', install_type: 'github-release', installed: false,
    description: 'A configured provider whose plugin file has not been installed.',
  },
];

if (params.has('external-repository')) storeEntries[0].repository = 'https://github.com/rensumo/cpa-plugin-zcode';

if (params.has('foreign-source')) storeEntries.unshift({ ...storeEntries[0], source_id: 'foreign', source_name: 'Foreign source',
  repository: 'foreign/sample', install_source_status: 'different',
});

mockIPC(async (command, args) => {
  state.commands.push({ command, args });
  if (command === 'start_oauth_login') return { url: 'https://login.example.test/authorize?state=sample-state', state: 'sample-state' };
  if (command === 'get_oauth_status') return { status: 'wait' };
  if (command === 'plugin:dialog|open') return state.localSelection;
  if (command === 'install_local_plugin') {
    if (state.localInstallError) throw new Error(state.localInstallError);
    pluginEntries.push({ id: 'local-demo', path: '/plugins/windows/amd64/local-demo-v1.0.0.dll', configured: false, registered: false, metadata: null, menus: [], config_fields: [] });
    return '/plugins/windows/amd64/local-demo-v1.0.0.dll';
  }
  if (command === 'get_plugin_support') return state.supported;
  if (command === 'get_plugin_latest_releases') return (args?.repositories as string[]).map(repository => ({ repository, version: state.latestReleaseVersion, error: null }));
  if (command === 'open_external_url' || command === 'restart_core_process') return null;
  if (command !== 'management_request') return null;
  const request = args?.request as Request;
  state.requests.push(structuredClone(request));
  if (request.path === '/oauth/session' && request.method === 'DELETE') return { status: 'ok', cancelled: true };
  if (request.path === '/plugins' && request.method === 'GET') {
    if (state.failNextList) { state.failNextList = false; throw new Error('Fixture list temporarily unavailable'); }
    return {
      plugins_enabled: state.settings.enabled, plugins_dir: state.settings.dir,
      plugins: pluginEntries.map(entry => ({ ...entry,
        enabled: state.settings.configs[String(entry.id)]?.enabled === true,
        effective_enabled: entry.registered === true && state.settings.enabled && state.settings.configs[String(entry.id)]?.enabled === true,
      })),
    };
  }
  if (request.path === '/plugins/store' && request.method === 'GET') return {
    plugins_enabled: state.settings.enabled, plugins_dir: state.settings.dir, plugins: storeEntries,
    source_errors: [{ source_id: 'offline', source_name: 'Offline source', message: 'Fixture registry unavailable' }],
  };
  if (request.path === '/config/plugins') {
    if (request.method === 'GET') return structuredClone(state.settings);
    if (request.method === 'PATCH') { Object.assign(state.settings, request.body); return { status: 'ok' }; }
  }
  const configMatch = /^\/config\/plugins\/configs\/([^/]+)(\/enabled)?$/.exec(request.path);
  if (configMatch) {
    const id = decodeURIComponent(configMatch[1]);
    if (request.method === 'GET') return structuredClone(state.settings.configs[id] ?? {});
    if (request.method === 'PUT') {
      state.settings.configs[id] = configMatch[2]
        ? { ...state.settings.configs[id], enabled: request.body }
        : request.body as Record<string, unknown>;
      return { status: 'ok' };
    }
  }
  const installMatch = /^\/plugins\/store\/([^/]+)\/install$/.exec(request.path);
  if (installMatch && request.method === 'POST') {
    state.installs.push(structuredClone(request));
    if (state.holdInstall) await new Promise<void>(resolve => { state.releaseInstall = resolve; });
    if (state.failNextInstall) { state.failNextInstall = false; throw new Error('Fixture installation failed'); }
    const id = decodeURIComponent(installMatch[1]);
    const entry = storeEntries.find(candidate => candidate.id === id && candidate.source_id === request.query?.source);
    if (!entry) throw new Error('Incorrect plugin store source');
    entry.installed = true;
    entry.installed_version = request.query?.version || entry.version;
    entry.update_available = false;
    state.settings.configs[id] ??= { enabled: false };
    return { status: 'ok', id, source_id: entry.source_id, version: entry.installed_version,
      install_type: entry.install_type, restart_required: true, plugins_enabled: state.settings.enabled };
  }
  const deleteMatch = /^\/plugins\/([^/]+)$/.exec(request.path);
  if (deleteMatch && request.method === 'DELETE') {
    const id = decodeURIComponent(deleteMatch[1]);
    const index = pluginEntries.findIndex(entry => entry.id === id);
    const removed = pluginEntries[index];
    if (index >= 0) pluginEntries.splice(index, 1);
    delete state.settings.configs[id];
    return { status: 'ok', id, file_deleted: Boolean(removed?.path), configured_removed: true,
      restart_required: removed?.registered === true };
  }
  throw new Error(`Unhandled fixture request: ${request.method} ${request.path}`);
});

createRoot(document.getElementById('root')!).render(params.has('resource')
  ? <h1>Sample resource preview</h1>
  : <I18nProvider><main style={{ padding: '24px', maxWidth: '1280px', margin: '0 auto' }}><PluginsPage /></main></I18nProvider>);
