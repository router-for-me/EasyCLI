import { useCallback, useMemo, useSyncExternalStore } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { PrivacyPreferences } from './services/privacyPreferences';
import { maskIdentifier, maskSensitiveText } from './services/privacyMask';
import { authFileIdentities, authFileName, type AuthFileRecord } from './services/authFiles';

const preferences = new PrivacyPreferences(() => window.localStorage);

export async function initializePrivacyMode(): Promise<void> {
  if (!isTauri()) return;
  try {
    await preferences.initialize({
      load: () => invoke<boolean | null>('get_privacy_mode'),
      save: (enabled) => invoke('save_privacy_mode', { enabled }),
    });
  } catch {
    console.error('Failed to load privacy mode preference; using browser preference');
  }
}

export function usePrivacyMode() {
  const enabled = useSyncExternalStore(preferences.subscribe, preferences.get, preferences.get);
  return [enabled, preferences.set] as const;
}

/**
 * Render-time masking helpers. Use them only where a value is displayed
 * (text, title, aria-label); keep raw values for requests, keys and copying.
 */
export function usePrivacyMask() {
  const [enabled] = usePrivacyMode();
  const text = useCallback((value: string, identities?: readonly string[]) =>
    enabled ? maskSensitiveText(value, identities) : value, [enabled]);
  const identifier = useCallback((value: string) =>
    enabled ? maskIdentifier(value) : value, [enabled]);
  const credentialName = useCallback((file: AuthFileRecord) =>
    text(authFileName(file), authFileIdentities(file)), [text]);
  return useMemo(() => ({ enabled, text, identifier, credentialName }), [enabled, text, identifier, credentialName]);
}
