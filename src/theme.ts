import { useSyncExternalStore } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { createThemeController, type AppTheme, type ThemePreference } from './themeController';
import { ThemePreferences } from './services/themePreferences';
import { createThemeApplicator } from './themes/applyTheme';
import type { ThemeSelection } from './themes/types';
import { themeRegistry, type ThemeRegistry } from './themes/registry';

export type { AppTheme, ThemePreference } from './themeController';

const preferences = new ThemePreferences();
const applyTheme = createThemeApplicator(document);

export function detectThemePreference(): ThemePreference {
  return preferences.get();
}

export async function initializeThemePreferences(): Promise<void> {
  if (!isTauri()) return;
  await preferences.initialize({
    load: () => invoke<ThemeSelection | null>('get_theme_selection'),
    save: (selection) => invoke('save_theme_selection', { selection }),
  });
}

export function useThemePreferenceIssue() {
  return useSyncExternalStore(preferences.subscribe, preferences.getIssue);
}

export const retryThemeSave = preferences.retry;

let controller: ReturnType<typeof createThemeController> | undefined;

export function initializeTheme(registry: ThemeRegistry = themeRegistry) {
  if (controller) return controller;
  const media = window.matchMedia?.('(prefers-color-scheme: dark)');
  controller = createThemeController({
    readPreference: detectThemePreference,
    readSystemThemes: preferences.getSystemThemes,
    savePreference(preference, remembered) {
      preferences.set(preference, remembered);
    },
    readMediaTheme: () => media?.matches ? 'dark' : 'light',
    listenMedia(listener) {
      if (!media) return () => {};
      if (typeof media.addEventListener === 'function') {
        media.addEventListener('change', listener);
        return () => media.removeEventListener('change', listener);
      }
      media.addListener(listener);
      return () => media.removeListener(listener);
    },
    listenResume(listener) {
      const visible = () => { if (document.visibilityState === 'visible') listener(); };
      window.addEventListener('focus', listener);
      document.addEventListener('visibilitychange', visible);
      return () => {
        window.removeEventListener('focus', listener);
        document.removeEventListener('visibilitychange', visible);
      };
    },
    applyTheme: (_appearance, definition) => applyTheme(definition),
    async connectNative() {
      if (!isTauri()) return null;
      const currentWindow = getCurrentWindow();
      const currentWebview = getCurrentWebview();
      const { os } = await invoke<{ os: string }>('detect_core_platform');
      return {
        explicitSystemAppearance: os === 'linux',
        setTheme: (theme) => currentWindow.setTheme(theme),
        readTheme: () => os === 'linux'
          ? invoke<AppTheme | null>('get_linux_system_theme')
          : currentWindow.theme(),
        listen: (listener) => currentWindow.onThemeChanged(({ payload }) => listener(payload)),
        async setBackground(_appearance, definition) {
          const background = definition.windowBackground;
          await Promise.allSettled([
            currentWindow.setBackgroundColor(background),
            currentWebview.setBackgroundColor(background),
          ]);
        },
      };
    },
  }, registry);
  return controller;
}

export function useThemePreference() {
  const themeController = initializeTheme();
  const preference = useSyncExternalStore(themeController.subscribe, themeController.getPreference);
  return [preference, themeController.setPreference] as const;
}

export function useSystemThemes() {
  const themeController = initializeTheme();
  return useSyncExternalStore(themeController.subscribe, themeController.getSystemThemes);
}

export function useResolvedThemeId() {
  const themeController = initializeTheme();
  return useSyncExternalStore(themeController.subscribe, themeController.getResolvedThemeId);
}

export function setSystemTheme(id: string) {
  initializeTheme().setSystemTheme(id);
}

if (import.meta.hot) {
  import.meta.hot.dispose(() => controller?.dispose());
}
