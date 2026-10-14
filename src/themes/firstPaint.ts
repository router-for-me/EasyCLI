import type { ThemeRegistry } from './registry';
import type { SystemThemes, ThemeDefinition, ThemePreference } from './types';

export const FIRST_PAINT_KEY = 'easy-cli-proxy-api.theme-paint';

export type FirstPaint = { system: boolean; appearance: ThemeDefinition['appearance']; light: string; dark: string };

export function firstPaint(registry: ThemeRegistry, preference: ThemePreference, systemThemes: SystemThemes, applied: ThemeDefinition): FirstPaint {
  return {
    system: preference === 'system',
    appearance: applied.appearance,
    light: registry.resolveSystem('light', systemThemes).windowBackground,
    dark: registry.resolveSystem('dark', systemThemes).windowBackground,
    [applied.appearance]: applied.windowBackground,
  };
}

export function cacheFirstPaint(storage: () => Storage, paint: FirstPaint) {
  try { storage().setItem(FIRST_PAINT_KEY, JSON.stringify(paint)); } catch {}
}
