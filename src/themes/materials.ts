import { themeTokenProperties, type ThemeDefinition, type ThemeTokens } from './types';

const materialSlots = ['surface', 'softSurface', 'workspaceBackground', 'hoverBackground'] as const;

export function themeMaterialTokens(theme: ThemeDefinition): Readonly<ThemeTokens> {
  // A configured thumb applies to both states unless an active override is supplied.
  const tokens = theme.tokens.controlActiveThumb === undefined && theme.tokens.controlThumb !== undefined
    ? { ...theme.tokens, controlActiveThumb: theme.tokens.controlThumb }
    : theme.tokens;
  if (!theme.backdrop) return tokens;
  return {
    ...tokens,
    surface: theme.backdrop.panel,
    softSurface: theme.backdrop.inset ?? theme.backdrop.panel,
    workspaceBackground: theme.backdrop.inset ?? theme.backdrop.panel,
    hoverBackground: theme.backdrop.hover ?? theme.tokens.hoverBackground ?? theme.backdrop.panel,
  };
}

function opaque(color: string, base: string): string {
  if (color.length === 7) return color;
  const alpha = parseInt(color.slice(7), 16) / 255;
  return `#${[1, 3, 5].map((offset) => Math.round(
    parseInt(color.slice(offset, offset + 2), 16) * alpha
    + parseInt(base.slice(offset, offset + 2), 16) * (1 - alpha),
  ).toString(16).padStart(2, '0')).join('')}`;
}

export function themeOverlaySurface(theme: ThemeDefinition): string {
  return opaque(theme.tokens.popoverBackground ?? theme.tokens.surface ?? theme.windowBackground, theme.windowBackground);
}

export function themeOverlayPalette(theme: ThemeDefinition) {
  const surface = themeOverlaySurface(theme);
  return {
    surface,
    workspaceBackground: surface,
    softSurface: opaque(theme.tokens.overlaySoftSurface ?? theme.tokens.softSurface ?? surface, surface),
    hoverBackground: opaque(theme.tokens.overlayHoverBackground ?? theme.tokens.hoverBackground ?? surface, surface),
  };
}

export function themeOverlayRules(theme: ThemeDefinition): string {
  if (!Object.keys(theme.tokens).length) return '';
  const palette = themeOverlayPalette(theme);
  const declarations = materialSlots.flatMap((slot) => {
    return themeTokenProperties[slot].map((property) => `${property}:${palette[slot]}`);
  }).join(';');
  return `:root[data-theme-id] :is(dialog,[role='dialog'],[role='alertdialog'],[role='menu'],[role='tooltip'],[popover],[data-theme-surface='overlay']){${declarations};--theme-sticky-background:${palette.surface};--theme-sticky-hover-background:${palette.hoverBackground}}`;
}
