import { themeTokenProperties, type ThemeDefinition, type ThemeTokens } from './types';

const materialSlots = ['surface', 'softSurface', 'workspaceBackground', 'hoverBackground'] as const;

// All existing and future consumers of the semantic palette receive the material.
// No page/card class names belong here. Floating surfaces opt into an opaque scope.
export function themeMaterialTokens(theme: ThemeDefinition): Readonly<ThemeTokens> {
  if (!theme.backdrop) return theme.tokens;
  return {
    ...theme.tokens,
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
  // Default themes keep their CSS palette. Custom solid and gradient themes
  // share the same floating-surface contract.
  if (!Object.keys(theme.tokens).length) return '';
  const palette = themeOverlayPalette(theme);
  const declarations = materialSlots.flatMap((slot) => {
    return themeTokenProperties[slot].map((property) => `${property}:${palette[slot]}`);
  }).join(';');
  // Dialog descendants inherit the same opaque palette, including nested controls.
  // Popovers without an ARIA overlay role use data-theme-surface="overlay".
  return `:root[data-theme-id] :is(dialog,[role='dialog'],[role='alertdialog'],[role='menu'],[role='tooltip'],[popover],[data-theme-surface='overlay']){${declarations};--theme-sticky-background:${palette.surface};--theme-sticky-hover-background:${palette.hoverBackground}}`;
}
