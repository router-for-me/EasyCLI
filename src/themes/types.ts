import type { MessageKey } from '../i18n/resources';

export type ThemeAppearance = 'light' | 'dark';
export type ThemePreference = string;

export const themeTokenProperties = {
  workspaceBackground: ['--theme-workspace-background', '--bg-secondary'],
  surface: ['--theme-surface', '--bg-primary', '--bg-card', '--theme-fff', '--theme-ffffff', '--theme-fffdf8'],
  softSurface: [
    '--theme-soft-surface', '--bg-tertiary', '--bg-quinary', '--count-badge-bg',
    '--theme-f0ede6', '--theme-f0eee8', '--theme-f2efe8', '--theme-f2f0ea',
    '--theme-f4f1ea', '--theme-f4f2ec', '--theme-f6f2ec', '--theme-f8f5ee',
    '--theme-f8f5ef', '--theme-f8f6f1', '--theme-faf8f2', '--theme-faf8f4', '--theme-faf9f5',
  ],
  hoverBackground: ['--theme-hover-background', '--bg-hover'],
  textPrimary: ['--theme-text-primary', '--text-primary', '--theme-2d2a26'],
  textSecondary: ['--theme-text-secondary', '--text-secondary', '--theme-342f2a', '--theme-39352f', '--theme-433f39', '--theme-4f4a43'],
  textTertiary: ['--theme-text-tertiary', '--text-tertiary'],
  textMuted: ['--theme-text-muted', '--text-quaternary'],
  border: ['--theme-border', '--border-color', '--theme-e3e1db', '--theme-eeeae2'],
  borderSubtle: ['--theme-border-subtle', '--border-secondary', '--theme-eeece6', '--theme-e8e5dc', '--theme-e8e4dc'],
  borderStrong: ['--border-primary', '--theme-d5d2cb'],
  borderHover: ['--border-hover', '--theme-c7c2b8'],
  panelShadow: ['--theme-panel-shadow-color'],
  accent: ['--ui-accent', '--primary-color'],
  accentStrong: ['--ui-accent-strong', '--primary-hover'],
  accentSoft: ['--ui-accent-soft'],
  accentContrast: ['--ui-accent-contrast', '--primary-contrast'],
  accentShadow: ['--ui-accent-shadow'],
  primaryButtonBackground: ['--theme-primary-button-background'],
  primaryButtonHoverBackground: ['--theme-primary-button-hover-background'],
  primaryButtonText: ['--theme-primary-button-text'],
  primaryButtonBorder: ['--theme-primary-button-border'],
  primaryButtonHoverBorder: ['--theme-primary-button-hover-border'],
  controlActiveBackground: ['--theme-control-active-background'],
  controlActiveThumb: ['--theme-control-active-thumb'],
  focusRing: ['--ui-focus-ring', '--sidebar-focus'],
  sidebarBackground: ['--sidebar-bg'],
  sidebarText: ['--sidebar-text'],
  sidebarTextStrong: ['--sidebar-text-strong'],
  sidebarTextMuted: ['--sidebar-text-subtle', '--sidebar-icon'],
  sidebarBorder: ['--sidebar-edge', '--sidebar-popover-edge'],
  sidebarHover: ['--sidebar-hover-bg', '--sidebar-press-bg'],
  sidebarSelected: ['--sidebar-selected-bg', '--sidebar-thumb-bg'],
  sidebarSelectedBorder: ['--sidebar-selected-border-color'],
  sidebarSelectedShadow: ['--sidebar-selected-shadow-color'],
  popoverBackground: ['--sidebar-popover-bg', '--theme-overlay-surface'],
  overlaySoftSurface: ['--theme-overlay-soft-surface'],
  overlayHoverBackground: ['--theme-overlay-hover-background'],
  scrim: ['--theme-scrim'],
  controlThumb: ['--theme-control-thumb'],
} as const;

export type ThemeTokens = Partial<Record<keyof typeof themeTokenProperties, string>>;
export type ThemeGradientStop = Readonly<{ color: string; at: number }>;
export type ThemeGradient = Readonly<(
  | { kind: 'linear'; angle: number }
  | { kind: 'radial'; center: readonly [number, number]; size: readonly [number, number] }
) & { stops: readonly ThemeGradientStop[] }>;
export type ThemeBackdrop = Readonly<{
  layers: readonly ThemeGradient[];
  sidebar: string;
  panel: string;
  inset?: string;
  hover?: string;
}>;
export type ThemePreview = { background: string; sidebar: string; surface: string; accent: string };
export type ThemeDefinition = {
  id: string;
  nameKey: MessageKey;
  descriptionKey?: MessageKey;
  appearance: ThemeAppearance;
  order: number;
  windowBackground: string;
  tokens: Readonly<ThemeTokens>;
  backdrop?: ThemeBackdrop;
};

export type SystemThemes = Readonly<Record<ThemeAppearance, string>>;
export type ThemeSelection = {
  version: 1;
  selectedId: string;
  lastLightId?: string;
  lastDarkId?: string;
};
export const isThemeId = (value: unknown): value is string =>
  typeof value === 'string' && /^[a-z][a-z0-9-]{0,63}$/.test(value);

export function isThemeSelection(value: unknown): value is ThemeSelection {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return record.version === 1 && isThemeId(record.selectedId)
    && ['lastLightId', 'lastDarkId'].every((key) =>
      record[key] === undefined || (isThemeId(record[key]) && record[key] !== 'system'));
}
