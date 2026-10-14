import { themeTokenProperties, type ThemeDefinition } from './types';
import { backdropImage } from './backdrop';
import { themeMaterialTokens, themeOverlayRules, themeOverlayPalette } from './materials';

export function createThemeApplicator(document: Document) {
  const ownedProperties = [...Object.values(themeTokenProperties).flat(),
    '--theme-backdrop-image', '--theme-backdrop-color', '--theme-backdrop-sidebar',
    '--theme-backdrop-panel', '--theme-sticky-background', '--theme-sticky-hover-background'];
  let overlayStyle = document.querySelector<HTMLStyleElement>('style[data-theme-materials]') ?? undefined;
  return (theme: ThemeDefinition) => {
    const root = document.documentElement;
    for (const property of ownedProperties) root.style.removeProperty(property);
    delete root.dataset.themeBackdrop;
    root.dataset.theme = theme.appearance;
    root.dataset.themeId = theme.id;
    root.style.colorScheme = theme.appearance;
    for (const [key, value] of Object.entries(themeMaterialTokens(theme))) {
      for (const property of themeTokenProperties[key as keyof typeof themeTokenProperties]) {
        root.style.setProperty(property, value);
      }
    }
    const overlayRules = themeOverlayRules(theme);
    if (overlayRules) {
      const palette = themeOverlayPalette(theme);
      for (const [property, value] of Object.entries({
        '--theme-overlay-surface': palette.surface,
        '--sidebar-popover-bg': palette.surface,
        '--theme-overlay-soft-surface': palette.softSurface,
        '--theme-overlay-hover-background': palette.hoverBackground,
      })) root.style.setProperty(property, value);
    }
    if (theme.backdrop) {
      root.dataset.themeBackdrop = 'gradient';
      const canvas = theme.backdrop.layers.map((layer) =>
        `${backdropImage({ ...theme.backdrop!, layers: [layer] })} center / 100vw 100vh no-repeat fixed`,
      ).join(', ');
      const base = theme.tokens.workspaceBackground ?? theme.windowBackground;
      const hover = theme.backdrop.hover ?? theme.backdrop.panel;
      for (const [property, value] of Object.entries({
        '--theme-backdrop-image': backdropImage(theme.backdrop),
        '--theme-backdrop-color': theme.tokens.workspaceBackground ?? theme.windowBackground,
        '--theme-backdrop-sidebar': theme.backdrop.sidebar,
        '--theme-backdrop-panel': theme.backdrop.panel,
        '--theme-sticky-background': `${canvas} ${base}`,
        '--theme-sticky-hover-background': `linear-gradient(${hover}, ${hover}), ${canvas} ${base}`,
      })) {
        root.style.setProperty(property, value);
      }
    }
    if (overlayRules) {
      if (!overlayStyle) {
        overlayStyle = document.createElement('style');
        overlayStyle.dataset.themeMaterials = '';
        document.head.append(overlayStyle);
      }
      overlayStyle.textContent = overlayRules;
    } else {
      overlayStyle?.remove();
      overlayStyle = undefined;
    }
    root.style.backgroundColor = theme.windowBackground;
    if (document.body) document.body.style.backgroundColor = theme.windowBackground;
  };
}
