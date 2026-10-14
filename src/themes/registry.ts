import { isThemeId, themeTokenProperties, type SystemThemes, type ThemeAppearance, type ThemeDefinition, type ThemePreview } from './types';
import { warmSand } from './warmSand';
import { graphite } from './graphite';
import { midnight } from './midnight';
import { light } from './light';
import { dark } from './dark';
import { sakura } from './sakura';
import { backdropImage, normalizeBackdrop } from './backdrop';

export function createThemeRegistry(definitions: readonly ThemeDefinition[]) {
  const byId = new Map<string, ThemeDefinition>();
  for (const definition of definitions) {
    if (!isThemeId(definition.id) || definition.id === 'system' || byId.has(definition.id)) {
      throw new Error(`Invalid or duplicate theme ID: ${definition.id}`);
    }
    if (!['light', 'dark'].includes(definition.appearance) || !definition.nameKey || !Number.isFinite(definition.order)) {
      throw new Error(`Invalid theme metadata: ${definition.id}`);
    }
    // Native window backgrounds must be opaque; tokens may include alpha.
    if (!/^#[\da-f]{6}$/i.test(definition.windowBackground)) throw new Error(`Invalid window background: ${definition.id}`);
    for (const [key, value] of Object.entries(definition.tokens)) {
      if (!Object.prototype.hasOwnProperty.call(themeTokenProperties, key) || !/^#(?:[\da-f]{6}|[\da-f]{8})$/i.test(value)) {
        throw new Error(`Invalid theme color: ${definition.id}.${key}`);
      }
    }
    byId.set(definition.id, Object.freeze({ ...definition,
      tokens: Object.freeze({ ...definition.tokens }),
      ...(definition.backdrop === undefined ? {} : { backdrop: normalizeBackdrop(definition.backdrop, definition.id) }),
    }));
  }
  for (const appearance of ['light', 'dark'] as const) {
    if (byId.get(appearance)?.appearance !== appearance) throw new Error(`Missing default ${appearance} theme`);
  }
  const themes = Object.freeze([...byId.values()].sort((a, b) => a.order - b.order));
  const resolveSystem = (appearance: ThemeAppearance, remembered?: SystemThemes): ThemeDefinition => {
    const theme = byId.get(remembered?.[appearance] ?? appearance);
    return theme?.appearance === appearance ? theme : byId.get(appearance)!;
  };
  return {
    themes,
    get: (id: string) => byId.get(id),
    resolveSystem,
    resolve: (id: string, system: ThemeAppearance = 'light', remembered?: SystemThemes): ThemeDefinition =>
      id === 'system' ? resolveSystem(system, remembered) : byId.get(id) ?? byId.get('light')!,
  };
}

// Add shipped themes here; the controller, persistence and selector need no per-theme branches.
export const themeRegistry = createThemeRegistry([light, dark, warmSand, graphite, midnight, sakura]);
export type ThemeRegistry = ReturnType<typeof createThemeRegistry>;

export function themePreview(theme: ThemeDefinition): ThemePreview {
  const dark = theme.appearance === 'dark';
  return {
    background: `${theme.backdrop ? `${backdropImage(theme.backdrop)}, ` : ''}${theme.tokens.workspaceBackground ?? theme.windowBackground}`,
    sidebar: theme.backdrop?.sidebar ?? theme.tokens.sidebarBackground ?? (dark ? '#0f1218' : '#f7f8fa'),
    surface: theme.backdrop?.panel ?? theme.tokens.surface ?? (dark ? '#141720' : '#ffffff'),
    accent: theme.tokens.accent ?? (dark ? '#5dd3c7' : '#0f766e'),
  };
}
