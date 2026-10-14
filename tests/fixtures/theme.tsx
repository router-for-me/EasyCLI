import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import App from '../../src/App';
import { I18nProvider } from '../../src/i18n';
import { initializeTheme, initializeThemePreferences } from '../../src/theme';
import type { AppTheme } from '../../src/themeController';
import { extensionRegistry, ThemeExtensionFixture } from './themeExtensions';
import '../../src/styles/index.css';

const params = new URLSearchParams(location.search);
const platform = params.get('platform') || 'windows';
let saveError = params.has('save-error');
// Stale browser data must not affect the desktop selection.
localStorage.setItem('easy-cli-proxy-api.theme', 'dark');
localStorage.setItem('easy-cli-proxy-api.locale', params.get('locale') || 'zh-CN');
let system: AppTheme = params.get('system') === 'light' ? 'light' : 'dark';
let override: AppTheme | null = null;
let appearance: AppTheme = system;
const calls: { cmd: string; args: unknown }[] = [];
const notify = (theme: AppTheme) => emit('tauri://theme-changed', theme);
mockWindows('main');
mockIPC(async (cmd, args: any) => {
  calls.push({ cmd, args });
  if (cmd === 'get_theme_selection') {
    if (params.has('read-error')) throw new Error('Unreadable configuration');
    const response = await fetch('/__theme_fixture_selection');
    return response.ok ? response.json() : null;
  }
  if (cmd === 'save_theme_selection') {
    if (saveError) throw new Error('Disk full');
    await fetch('/__theme_fixture_selection', { method: 'POST', body: JSON.stringify(args.selection) });
    return;
  }
  if (cmd === 'detect_core_platform') return { os: platform, arch: 'x86_64' };
  if (cmd === 'get_linux_system_theme') return params.has('no-portal') ? null : system;
  if (cmd === 'plugin:window|theme') return override ?? system;
  if (cmd === 'plugin:window|set_theme') {
    override = args.value ?? null;
    appearance = override ?? (platform === 'linux' ? 'light' : system);
    const changed = appearance;
    setTimeout(() => void notify(changed), 0);
    return;
  }
  if (cmd.includes('background_color') || cmd === 'set_app_locale') return;
  if (cmd === 'plugin:app|version') return '0.0.0';
  if (cmd === 'get_core_status') return { installed: false, running: false, starting: false, managed: false };
  if (cmd === 'get_gui_settings') return { host: '127.0.0.1', port: 8317, closeBehavior: 'ask' };
  if (cmd === 'get_core_config_settings') return { apiKeys: [] };
  if (cmd === 'get_core_tls_settings') return { enabled: false };
  if (cmd === 'check_app_update') return { updateAvailable: false, currentVersion: '0.0.0' };
  if (cmd === 'get_app_update_task') return { running: false, phase: 'idle' };
  if (cmd === 'check_latest_core') return { version: '0.0.0' };
  throw new Error(`Theme fixture does not implement ${cmd}`);
}, { shouldMockEvents: true });
Object.assign(window, {
  isTauri: platform !== 'browser',
  themeFixture: {
    calls,
    get appearance() { return appearance; },
    setSaveError(value: boolean) { saveError = value; },
    async setSystem(theme: AppTheme) {
      system = theme;
      if (platform === 'linux' || override === null) {
        appearance = theme;
        await notify(theme);
      }
    },
  },
});
await initializeThemePreferences();
initializeTheme(params.has('extensions') ? extensionRegistry : undefined);
createRoot(document.getElementById('root')!).render(
  <StrictMode><I18nProvider>{params.has('extensions') ? <ThemeExtensionFixture /> : <App />}</I18nProvider></StrictMode>,
);
