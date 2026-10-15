import { describe, expect, test } from 'bun:test';
import { createThemeRegistry, themeRegistry, themePreview } from '../src/themes/registry';
import { createThemeApplicator } from '../src/themes/applyTheme';
import { en, zhCN, zhTW, ja } from '../src/i18n/resources';
import type { ThemeBackdrop, ThemeDefinition } from '../src/themes/types';
import { warmSand } from '../src/themes/warmSand';
import { graphite } from '../src/themes/graphite';
import { midnight } from '../src/themes/midnight';
import { themeMaterialTokens, themeOverlayRules, themeOverlaySurface, themeOverlayPalette } from '../src/themes/materials';
import { sakura } from '../src/themes/sakura';
import { backdropImage } from '../src/themes/backdrop';

const additionalThemes = [warmSand, graphite, midnight, sakura];

const custom: ThemeDefinition = { id: 'test-dark', nameKey: 'app.theme.dark', appearance: 'dark', order: 1,
  windowBackground: '#102030', tokens: { surface: '#203040', accent: '#abcdef', sidebarBackground: '#112233' } };
const customControl: ThemeDefinition = { ...custom, id: 'test-control-dark',
  tokens: { ...custom.tokens, controlThumb: '#843E64', controlActiveBackground: '#F6E3ED' } };

const rgb = (hex: string) => [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16));
const luminance = (channels: number[]) => {
  const [r, g, b] = channels.map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return r * 0.2126 + g * 0.7152 + b * 0.0722;
};
const rgbContrast = (foreground: number[], background: number[]) => {
  const values = [luminance(foreground), luminance(background)].sort((a, b) => a - b);
  return (values[1] + 0.05) / (values[0] + 0.05);
};
const contrast = (foreground: string, background: string) => rgbContrast(rgb(foreground), rgb(background));

describe('theme registry and application', () => {
  test('configured thumbs apply to both states and permit an independent active override', () => {
    for (const theme of [customControl, { ...sakura, tokens: { ...sakura.tokens, ...customControl.tokens } }]) {
      expect(themeMaterialTokens(theme).controlThumb).toBe('#843E64');
      expect(themeMaterialTokens(theme).controlActiveThumb).toBe('#843E64');
      expect(themeMaterialTokens({ ...theme, tokens: { ...theme.tokens, controlActiveThumb: '#6A3554' } }).controlActiveThumb).toBe('#6A3554');
    }
    expect(themeMaterialTokens(custom).controlActiveThumb).toBeUndefined();
  });
  test('gradient materials share aliases while floating surfaces stay opaque', () => {
    const material = themeMaterialTokens(sakura);
    expect(material.surface).toBe(sakura.backdrop!.panel);
    expect(material.softSurface).toBe(sakura.backdrop!.inset);
    expect(material.hoverBackground).toBe(sakura.backdrop!.hover);
    expect(themeOverlaySurface(sakura)).toBe('#F7E3ED');
    const rules = themeOverlayRules(sakura);
    for (const property of ['--bg-card', '--theme-surface', '--bg-secondary']) {
      expect(rules).toContain(`${property}:#`);
    }
    expect(rules).toContain('[data-theme-surface=\'overlay\']');
    expect(rules).not.toContain(sakura.backdrop!.panel);
    expect(themeMaterialTokens(graphite)).toBe(graphite.tokens);
    expect(themeOverlayRules(graphite)).toContain('--theme-surface:#24282D');
    expect(themeOverlayRules(sakura)).toContain('--theme-soft-surface:#F3E0EA');
    expect(themeOverlayRules(sakura)).toContain('--theme-hover-background:#F1DDE7');
    expect(themeOverlayRules(themeRegistry.resolve('light'))).toBe('');
    expect(themeOverlaySurface({ ...sakura, tokens: { surface: '#00000080' }, windowBackground: '#ffffff' })).toBe('#7f7f7f');
  });
  test('defaults system selection to light/dark and derives preview colors', () => {
    const registry = createThemeRegistry([...themeRegistry.themes, custom]);
    expect(registry.resolve('system', 'dark').id).toBe('dark');
    expect(registry.resolve('system', 'light').id).toBe('light');
    for (const theme of additionalThemes) {
      expect(registry.resolve(theme.id, 'dark').id).toBe(theme.id);
      expect(registry.resolve(theme.id, 'light').id).toBe(theme.id);
    }
    expect(registry.resolve('test-dark', 'light').id).toBe('test-dark');
    expect(registry.resolve('missing', 'dark').id).toBe('light');
    expect(themePreview(custom)).toEqual({ background: '#102030', surface: '#203040', accent: '#abcdef', sidebar: '#112233' });
  });
  test('partial palettes derive floating surfaces and composite translucent overrides', () => {
    expect(themeOverlayPalette(custom)).toEqual({
      surface: '#203040', workspaceBackground: '#203040', softSurface: '#203040', hoverBackground: '#203040',
    });
    expect(themeOverlayPalette({ ...custom, windowBackground: '#ffffff', tokens: {
      popoverBackground: '#00000080', overlaySoftSurface: '#ffffff80', overlayHoverBackground: '#00000080',
    } })).toEqual({ surface: '#7f7f7f', workspaceBackground: '#7f7f7f', softSurface: '#bfbfbf', hoverBackground: '#3f3f3f' });
  });
  test('system selection uses the remembered theme only when it matches the appearance', () => {
    const remembered = { light: 'sakura-light', dark: 'midnight-dark' };
    expect(themeRegistry.resolve('system', 'light', remembered).id).toBe('sakura-light');
    expect(themeRegistry.resolve('system', 'dark', remembered).id).toBe('midnight-dark');
    expect(themeRegistry.resolveSystem('light', { ...remembered, light: 'sakura-light' }).id).toBe('sakura-light');
    expect(themeRegistry.resolve('graphite-dark', 'light', remembered).id).toBe('graphite-dark');
    for (const id of ['removed-theme', 'midnight-dark', 'system']) {
      const unavailable = { ...remembered, light: id };
      expect(themeRegistry.resolve('system', 'light', unavailable).id).toBe('light');
      expect(themeRegistry.resolve('system', 'dark', unavailable).id).toBe('midnight-dark');
    }
    for (const id of ['removed-theme', 'sakura-light', 'system']) {
      expect(themeRegistry.resolveSystem('dark', { ...remembered, dark: id }).id).toBe('dark');
    }
  });
  test('rejects collisions, reserved IDs, unsupported slots and invalid native colors', () => {
    for (const theme of [
      { ...custom, id: 'dark' }, { ...custom, id: 'system' }, { ...custom, id: '../bad' },
      { ...custom, windowBackground: '#11223344' }, { ...custom, tokens: { surface: 'url(bad)' } },
      { ...custom, tokens: { arbitraryCss: '#ffffff' } },
    ]) expect(() => createThemeRegistry([...themeRegistry.themes, theme as ThemeDefinition])).toThrow();
  });
  test('removed themes leave the picker and safely fall back for saved selections', () => {
    for (const id of ['mist-blue-light', 'rose-paper-light', 'cyberpunk-dark', 'pine-dark']) {
      expect(themeRegistry.get(id)).toBeUndefined();
      expect(themeRegistry.resolve(id, 'dark').id).toBe('light');
      expect(themeRegistry.resolveSystem('light', { light: id, dark: 'midnight-dark' }).id).toBe('light');
      expect(themeRegistry.resolveSystem('dark', { light: 'sakura-light', dark: id }).id).toBe('dark');
    }
  });
  test('rejects CSS injection, malformed gradients and unbounded backdrop layers', () => {
    const valid = sakura.backdrop!;
    const linear = { kind: 'linear', angle: 120, stops: [{ color: '#ffccdd', at: 0 }, { color: '#ffffff', at: 100 }] };
    const invalidLayers = [
      [], Array(5).fill(linear), [null], [{ ...linear, kind: 'image', url: 'https://example.com/a.png' }],
      [{ ...linear, angle: NaN }], [{ ...linear, angle: -10 }],
      [{ ...linear, stops: [{ color: 'url(remote)', at: 0 }, { color: '#ffffff', at: 100 }] }],
      [{ ...linear, stops: [{ color: '#ffffff', at: 50 }, { color: '#ffccdd', at: 10 }] }],
      [{ ...linear, stops: [{ color: '#ffffff', at: 0 }] }],
      [{ ...linear, stops: [{ color: '#ffffff', at: 0 }, { color: '#ffccdd', at: Infinity }] }],
      [{ kind: 'radial', center: [0, 101], size: [100, 100], stops: linear.stops }],
      [{ kind: 'radial', center: [0, 0], size: [0, 100], stops: linear.stops }],
      [{ kind: 'radial', center: [0], size: [100, 100], stops: linear.stops }],
    ];
    for (const layers of invalidLayers) {
      expect(() => createThemeRegistry([...themeRegistry.themes, {
        ...custom, backdrop: { ...valid, layers },
      } as ThemeDefinition])).toThrow('Invalid theme backdrop');
    }
    for (const backdrop of [null, { ...valid, sidebar: 'transparent' }, { ...valid, panel: 'url(remote)' },
      { ...valid, inset: 'url(remote)' }, { ...valid, hover: '#bad' }]) {
      expect(() => createThemeRegistry([...themeRegistry.themes, { ...custom, backdrop } as ThemeDefinition])).toThrow();
    }
  });
  test('backdrops are immutable and thumbnails use the same artwork and materials', () => {
    const input: ThemeBackdrop = { ...structuredClone(sakura.backdrop!), layers: [
      { kind: 'radial', center: [100, 0], size: [85, 100], stops: [{ color: '#FFE4EF', at: 0 }, { color: '#FFE4EF00', at: 100 }] },
      { kind: 'linear', angle: 135, stops: [{ color: '#FFE4EF', at: 0 }, { color: '#FAF5EE', at: 100 }] },
    ] };
    const registry = createThemeRegistry([...themeRegistry.themes, { ...custom, backdrop: input }]);
    const registered = registry.get(custom.id)!;
    const image = backdropImage(registered.backdrop!);
    (input.layers[0].stops[0] as { color: string }).color = '#000000';
    expect(backdropImage(registered.backdrop!)).toBe(image);
    expect(Object.isFrozen(registered.backdrop!.layers[0].stops[0])).toBe(true);
    expect(themePreview(registered)).toEqual({
      background: `${image}, ${custom.windowBackground}`, sidebar: sakura.backdrop!.sidebar,
      surface: sakura.backdrop!.panel, accent: custom.tokens.accent,
    });
    expect(image).toContain('radial-gradient(ellipse 85% 100% at 100% 0%');
    expect(image).toContain('linear-gradient(135deg');
  });
  test.each([custom, customControl, ...additionalThemes])('switching from $id to a default clears only managed properties', (theme) => {
    const properties = new Map<string, string>([['--unrelated', 'keep']]);
    const style = {
      setProperty: (key: string, value: string) => { properties.set(key, value); },
      removeProperty: (key: string) => { properties.delete(key); },
      backgroundColor: '', colorScheme: '',
    };
    const root = { style, dataset: {} as Record<string, string> };
    const body = { style: { backgroundColor: '' } };
    const styles = new Set<unknown>();
    const apply = createThemeApplicator({ documentElement: root, body,
      querySelector: () => null,
      head: { append: (element: unknown) => styles.add(element) },
      createElement: () => { const element = { dataset: {}, textContent: '', remove: () => styles.delete(element) }; return element; },
    } as unknown as Document);
    for (const defaultId of ['light', 'dark']) {
      apply(theme);
      expect(root.dataset).toEqual({ theme: theme.appearance, themeId: theme.id,
        ...(theme.backdrop ? { themeBackdrop: 'gradient' } : {}),
      });
      if (theme.backdrop) expect(properties.get('--theme-backdrop-image')).toBe(backdropImage(theme.backdrop));
      if (theme.backdrop) expect(properties.get('--theme-sticky-background')).toContain('100vw 100vh no-repeat fixed');
      expect(properties.get('--theme-surface')).toBe(themeMaterialTokens(theme).surface);
      expect(properties.get('--sidebar-popover-bg')).toBe(themeOverlaySurface(theme));
      expect(styles.size).toBe(1);
      apply(theme);
      expect(styles.size).toBe(1);
      expect(body.style.backgroundColor).toBe(theme.windowBackground);
      apply(themeRegistry.resolve(defaultId));
      expect(styles.size).toBe(0);
      expect([...properties]).toEqual([['--unrelated', 'keep']]);
      expect(root.dataset).toEqual({ theme: defaultId, themeId: defaultId });
      expect(style.colorScheme).toBe(defaultId);
    }
  });
  test('reinitializing the applicator clears the previous custom palette and artwork', () => {
    const properties = new Map<string, string>([['--unrelated', 'keep']]);
    const styles = new Set<unknown>();
    const document = { documentElement: { dataset: {}, style: {
      setProperty: (key: string, value: string) => properties.set(key, value),
      removeProperty: (key: string) => properties.delete(key),
    } }, body: { style: {} }, head: { append: (element: unknown) => styles.add(element) },
    querySelector: () => [...styles][0] ?? null,
    createElement: () => { const element = { dataset: {}, textContent: '', remove: () => styles.delete(element) }; return element; },
    } as unknown as Document;
    createThemeApplicator(document)(midnight);
    expect(properties.has('--theme-sticky-background')).toBe(true);
    createThemeApplicator(document)(themeRegistry.resolve('light'));
    expect([...properties]).toEqual([['--unrelated', 'keep']]);
    expect(styles.size).toBe(0);
  });
  test.each(additionalThemes)('$id overlay controls stay opaque and readable', (theme) => {
    const surface = themeOverlaySurface(theme);
    const soft = theme.tokens.overlaySoftSurface ?? theme.tokens.softSurface!;
    const hover = theme.tokens.overlayHoverBackground ?? theme.tokens.hoverBackground!;
    for (const background of [surface, soft, hover]) {
      for (const text of ['textPrimary', 'textSecondary', 'textTertiary', 'textMuted'] as const) {
        expect(contrast(theme.tokens[text]!, background)).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
  test.each(additionalThemes)('$id text, action labels and focus indicators retain readable contrast', (theme) => {
    const tokens = theme.tokens;
    for (const text of ['textPrimary', 'textSecondary', 'textTertiary', 'textMuted', 'accent', 'accentStrong'] as const) {
      for (const surface of ['workspaceBackground', 'surface', 'softSurface', 'hoverBackground', 'accentSoft'] as const) {
        expect(contrast(tokens[text]!, tokens[surface]!)).toBeGreaterThanOrEqual(4.5);
      }
    }
    for (const text of ['sidebarText', 'sidebarTextStrong', 'sidebarTextMuted'] as const) {
      for (const surface of ['sidebarBackground', 'sidebarSelected', 'sidebarHover', 'popoverBackground'] as const) {
        expect(contrast(tokens[text]!, tokens[surface]!)).toBeGreaterThanOrEqual(4.5);
      }
    }
    for (const accent of ['accent', 'accentStrong'] as const) {
      expect(contrast(tokens.accentContrast!, tokens[accent]!)).toBeGreaterThanOrEqual(4.5);
    }
    for (const background of [tokens.primaryButtonBackground ?? tokens.accent!, tokens.primaryButtonHoverBackground ?? tokens.accentStrong!]) {
      expect(contrast(tokens.primaryButtonText ?? tokens.accentContrast!, background)).toBeGreaterThanOrEqual(4.5);
    }
    expect(contrast(tokens.controlActiveThumb ?? tokens.controlThumb ?? tokens.accentContrast!, tokens.controlActiveBackground ?? tokens.accent!)).toBeGreaterThanOrEqual(3);
    for (const surface of ['surface', 'workspaceBackground', 'sidebarBackground', 'sidebarHover'] as const) {
      expect(contrast(tokens.focusRing!, tokens[surface]!)).toBeGreaterThanOrEqual(3);
    }
  });
  test.each([sakura, midnight])('$id text and focus remain legible over the gradient and composited translucent surfaces', (theme) => {
    const backdrop = theme.backdrop!;
    const colors = [theme.windowBackground, ...backdrop.layers.flatMap((layer) => layer.stops.map((stop) => stop.color))].map(rgb);
    const darkest = [0, 1, 2].map((channel) => Math.min(...colors.map((value) => value[channel])));
    const lightest = [0, 1, 2].map((channel) => Math.max(...colors.map((value) => value[channel])));
    const composite = (paint: string, background: number[]) => {
      const alpha = paint.length === 9 ? parseInt(paint.slice(7), 16) / 255 : 1;
      return rgb(paint).map((channel, i) => channel * alpha + background[i] * (1 - alpha));
    };
    for (const bound of [darkest, lightest]) {
      const panel = composite(backdrop.panel, bound);
      const inset = composite(backdrop.inset!, panel);
      const surfaces = [bound, panel, inset, composite(backdrop.hover!, inset),
        ...[backdrop.sidebar, backdrop.inset!, backdrop.hover!].map((paint) => composite(paint, bound))];
      for (const surface of surfaces) {
        for (const key of ['textPrimary', 'textSecondary', 'textTertiary', 'textMuted', 'accent', 'accentStrong',
          'sidebarText', 'sidebarTextStrong', 'sidebarTextMuted'] as const) {
          expect(rgbContrast(rgb(theme.tokens[key]!), surface)).toBeGreaterThanOrEqual(4.5);
        }
        expect(rgbContrast(rgb(theme.tokens.focusRing!), surface)).toBeGreaterThanOrEqual(3);
      }
    }
  });
  test('all shipped metadata and picker text have translations in every locale', () => {
    const keys = Object.keys(zhCN).filter((key) => key.startsWith('app.theme.')) as (keyof typeof zhCN)[];
    for (const messages of [zhCN, zhTW, en, ja]) {
      for (const key of keys) expect(messages[key]?.length).toBeGreaterThan(0);
      for (const theme of themeRegistry.themes) {
        expect(messages[theme.nameKey]?.length).toBeGreaterThan(0);
        if (theme.descriptionKey) expect(messages[theme.descriptionKey]?.length).toBeGreaterThan(0);
      }
    }
  });
});
