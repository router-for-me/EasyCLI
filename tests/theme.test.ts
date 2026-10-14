import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { createThemeRegistry, themeRegistry } from '../src/themes/registry';
import { FIRST_PAINT_KEY, firstPaint } from '../src/themes/firstPaint';
import type { SystemThemes, ThemeDefinition } from '../src/themes/types';
import { createThemeController, type AppTheme, type NativeThemeSource, type ThemePreference } from '../src/themeController';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function settle() {
  for (let i = 0; i < 40; i++) await Promise.resolve();
}

function setup(options: {
  preference?: ThemePreference;
  systemThemes?: SystemThemes;
  system?: AppTheme;
  media?: AppTheme;
  platform?: 'windows' | 'macos' | 'linux' | 'browser';
} = {}) {
  let saved = options.preference ?? 'system';
  let savedSystemThemes = options.systemThemes;
  let system: AppTheme | null = options.system ?? 'dark';
  let media = options.media ?? 'light';
  let displayed: AppTheme | undefined;
  let displayedId: string | undefined;
  let shell: AppTheme | undefined;
  let backdrop: AppTheme | undefined;
  let backdropColor: string | undefined;
  let mediaListener: (() => void) | undefined;
  let resumeListener: (() => void) | undefined;
  let nativeListener: ((theme: AppTheme) => void) | undefined;
  let removals = 0;
  const writes: ThemePreference[] = [];
  const nativeWrites: (AppTheme | null)[] = [];
  const native: NativeThemeSource = {
    explicitSystemAppearance: options.platform === 'linux',
    async setTheme(theme) {
      nativeWrites.push(theme);
      shell = theme ?? system ?? 'light';
    },
    async readTheme() { return system; },
    async listen(listener) {
      nativeListener = listener;
      return () => { nativeListener = undefined; removals++; };
    },
    async setBackground(theme, definition) { backdrop = theme; backdropColor = definition.windowBackground; },
  };
  const connection = deferred<NativeThemeSource | null>();
  const controller = createThemeController({
    readPreference: () => saved,
    readSystemThemes: () => savedSystemThemes ?? { light: 'light', dark: 'dark' },
    savePreference(value, remembered) { saved = value; savedSystemThemes = remembered; writes.push(value); },
    readMediaTheme: () => media,
    listenMedia(listener) {
      mediaListener = listener;
      return () => { mediaListener = undefined; removals++; };
    },
    listenResume(listener) {
      resumeListener = listener;
      return () => { resumeListener = undefined; removals++; };
    },
    applyTheme(theme, definition) { displayed = theme; displayedId = definition.id; },
    connectNative: () => connection.promise,
  });
  return {
    controller, native, writes, nativeWrites,
    get displayed() { return displayed; },
    get displayedId() { return displayedId; },
    get shell() { return shell; },
    get backdrop() { return backdrop; },
    get backdropColor() { return backdropColor; },
    get saved() { return saved; },
    get savedSystemThemes() { return savedSystemThemes; },
    get removals() { return removals; },
    setSystem(theme: AppTheme | null) { system = theme; },
    emitNative(theme: AppTheme) { shell = theme; nativeListener?.(theme); },
    emitMedia(theme: AppTheme) { media = theme; mediaListener?.(); },
    resume() { resumeListener?.(); },
    async connect() {
      connection.resolve(options.platform === 'browser' ? null : native);
      await settle();
    },
  };
}

describe('theme preferences and native synchronization', () => {
  for (const platform of ['windows', 'macos'] as const) {
    test(`${platform}: native startup and events win without locking system mode`, async () => {
      const app = setup({ platform, media: 'light', system: 'dark' });
      expect(app.displayed).toBe('light');
      await app.connect();
      expect(app.displayed).toBe('dark');
      expect(app.backdrop).toBe('dark');
      expect(app.nativeWrites).toEqual([null]);
      app.setSystem('light');
      app.emitNative('light');
      await settle();
      expect(app.displayed).toBe('light');
      expect(app.backdrop).toBe('light');
      expect(app.controller.getPreference()).toBe('system');
      expect(app.writes).toEqual([]);
      expect(app.nativeWrites).toEqual([null]);
      app.controller.dispose();
    });
  }

  test('native reading wins over a conflicting WebView change', async () => {
    const app = setup({ system: 'light' });
    await app.connect();
    app.emitMedia('dark');
    await settle();
    expect(app.displayed).toBe('light');
    app.controller.dispose();
  });

  test('preserves an existing manual preference and ignores system/media changes', async () => {
    const app = setup({ preference: 'light', system: 'dark' });
    await app.connect();
    app.emitMedia('dark');
    app.emitNative('dark');
    await settle();
    expect(app.displayed).toBe('light');
    expect(app.shell).toBe('light');
    expect(app.writes).toEqual([]);
    app.controller.dispose();
  });

  test('switches manual → system → manual and persists the preference, not its result', async () => {
    const app = setup({ preference: 'light', system: 'dark' });
    await app.connect();
    app.controller.setPreference('system');
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.saved).toBe('system');
    expect(app.nativeWrites).toEqual(['light', null]);
    app.controller.setPreference('light');
    app.setSystem('dark');
    app.emitNative('dark');
    await settle();
    expect(app.displayed).toBe('light');
    expect(app.shell).toBe('light');
    expect(app.saved).toBe('light');
    app.controller.dispose();
  });

  test('browser preview follows media changes, including after a manual override', async () => {
    const app = setup({ platform: 'browser' });
    await app.connect();
    app.emitMedia('dark');
    expect(app.displayed).toBe('dark');
    app.controller.setPreference('light');
    app.emitMedia('dark');
    expect(app.displayed).toBe('light');
    app.controller.setPreference('system');
    expect(app.displayed).toBe('dark');
    app.emitMedia('light');
    expect(app.displayed).toBe('light');
    app.controller.dispose();
  });

  test('failed native reads and listeners retain the media fallback', async () => {
    const app = setup();
    app.native.readTheme = async () => { throw new Error('unavailable'); };
    app.native.listen = async () => { throw new Error('unsupported'); };
    app.native.setTheme = async () => { throw new Error('unsupported'); };
    await app.connect();
    app.emitMedia('dark');
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.controller.getPreference()).toBe('system');
    app.controller.dispose();
  });

  test('refreshes after resuming from tray/sleep when a notification was missed', async () => {
    const app = setup();
    await app.connect();
    app.setSystem('light');
    app.resume();
    await settle();
    expect(app.displayed).toBe('light');
    expect(app.backdrop).toBe('light');
    app.controller.dispose();
  });

  test('a delayed initial read cannot overwrite a newer native notification', async () => {
    const app = setup();
    const reading = deferred<AppTheme>();
    app.native.readTheme = () => reading.promise;
    await app.connect();
    app.emitNative('dark');
    reading.resolve('light');
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.backdrop).toBe('dark');
    app.controller.dispose();
  });

  test('a delayed system read cannot overwrite a newer manual selection', async () => {
    const app = setup();
    const reading = deferred<AppTheme>();
    app.native.readTheme = () => reading.promise;
    await app.connect();
    app.controller.setPreference('light');
    reading.resolve('dark');
    await settle();
    expect(app.displayed).toBe('light');
    expect(app.shell).toBe('light');
    expect(app.backdrop).toBe('light');
    app.controller.dispose();
  });

  test('serializes slow native writes during rapid preference changes', async () => {
    const app = setup();
    const write = deferred<void>();
    const original = app.native.setTheme;
    app.native.setTheme = async (theme) => {
      if (theme === null) await write.promise;
      await original(theme);
    };
    await app.connect();
    app.controller.setPreference('light');
    app.controller.setPreference('dark');
    app.controller.setPreference('system');
    app.controller.setPreference('light');
    write.resolve();
    await settle();
    expect(app.nativeWrites).toEqual([null, 'light']);
    expect(app.displayed).toBe('light');
    expect(app.shell).toBe('light');
    app.controller.dispose();
  });

  test('Linux reads the portal after clearing a manual override and applies GTK appearance', async () => {
    const app = setup({ platform: 'linux', preference: 'light', system: 'dark' });
    await app.connect();
    app.controller.setPreference('system');
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.shell).toBe('dark');
    expect(app.saved).toBe('system');
    expect(app.nativeWrites).toEqual(['light', null, 'dark']);
    app.emitNative('light');
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.shell).toBe('dark');
    const writeCount = app.nativeWrites.length;
    app.emitNative('dark');
    app.emitMedia('dark');
    await settle();
    expect(app.nativeWrites.length).toBe(writeCount);
    app.setSystem('light');
    app.emitNative('light');
    await settle();
    expect(app.displayed).toBe('light');
    app.controller.dispose();
  });

  test('Linux without a portal preference falls back to WebKit', async () => {
    const app = setup({ platform: 'linux' });
    app.setSystem(null);
    await app.connect();
    app.emitMedia('dark');
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.nativeWrites).toEqual([null]);
    app.controller.dispose();
  });

  test('Linux retains a manual appearance when the portal sends a different theme', async () => {
    const app = setup({ platform: 'linux', preference: 'dark' });
    await app.connect();
    app.setSystem('light');
    app.emitNative('light');
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.shell).toBe('dark');
    app.controller.dispose();
  });

  test('Linux reconciles a system change delivered during an in-flight GTK appearance write', async () => {
    const app = setup({ platform: 'linux', system: 'dark' });
    const write = deferred<void>();
    const original = app.native.setTheme;
    app.native.setTheme = async (theme) => {
      await original(theme);
      if (theme === 'dark') await write.promise;
    };
    await app.connect();
    app.setSystem('light');
    app.emitNative('light');
    write.resolve();
    await settle();
    expect(app.displayed).toBe('light');
    expect(app.shell).toBe('light');
    expect(app.backdrop).toBe('light');
    app.controller.dispose();
  });

  test('Linux reasserts a manual mode when the portal changes GTK during the manual write', async () => {
    const app = setup({ platform: 'linux', preference: 'dark' });
    const write = deferred<void>();
    const original = app.native.setTheme;
    app.native.setTheme = async (theme) => {
      await original(theme);
      await write.promise;
    };
    await app.connect();
    app.setSystem('light');
    app.emitNative('light');
    write.resolve();
    await settle();
    expect(app.displayed).toBe('dark');
    expect(app.shell).toBe('dark');
    expect(app.backdrop).toBe('dark');
    app.controller.dispose();
  });

  test('disposes all observers and ignores late async reads', async () => {
    const app = setup();
    const reading = deferred<AppTheme>();
    app.native.readTheme = () => reading.promise;
    await app.connect();
    app.controller.dispose();
    reading.resolve('dark');
    app.emitMedia('dark');
    await settle();
    expect(app.removals).toBe(3);
    expect(app.displayed).toBe('light');
  });

  test('cleans up a native listener that attaches after disposal', async () => {
    const app = setup();
    const registration = deferred<() => void>();
    let removed = false;
    app.native.listen = () => registration.promise;
    await app.connect();
    app.controller.dispose();
    registration.resolve(() => { removed = true; });
    await settle();
    expect(removed).toBe(true);
    expect(app.nativeWrites).toEqual([]);
  });
});

describe('desktop first paint', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  const script = html.match(/<script>([\s\S]*?)<\/script>/)![1];
  const paint = (stored: Record<string, string>, media: AppTheme = 'light') => {
    const root = { dataset: {} as Record<string, string>, style: {} as Record<string, string> };
    new Function('localStorage', 'window', 'document', script)(
      { getItem: (key: string) => stored[key] ?? null },
      { matchMedia: () => ({ matches: media === 'dark' }) },
      { documentElement: root },
    );
    return { theme: root.dataset.theme, background: root.style.backgroundColor };
  };
  const cache = (preference: ThemePreference, systemThemes: SystemThemes, applied: string) => ({
    [FIRST_PAINT_KEY]: JSON.stringify(firstPaint(themeRegistry, preference, systemThemes, themeRegistry.get(applied)!)),
  });

  test('paints the cached theme before the app loads', () => {
    expect(paint(cache('midnight-dark', { light: 'light', dark: 'dark' }, 'midnight-dark')))
      .toEqual({ theme: 'dark', background: themeRegistry.get('midnight-dark')!.windowBackground });
    const system = cache('system', { light: 'sakura-light', dark: 'graphite-dark' }, 'sakura-light');
    expect(paint(system, 'dark')).toEqual({ theme: 'dark', background: themeRegistry.get('graphite-dark')!.windowBackground });
    expect(paint(system, 'light')).toEqual({ theme: 'light', background: themeRegistry.get('sakura-light')!.windowBackground });
  });

  test('falls back to the previous cache and then to light', () => {
    expect(paint({ 'easy-cli-proxy-api.theme': 'dark' })).toEqual({ theme: 'dark', background: '#0b0d11' });
    expect(paint({ 'easy-cli-proxy-api.theme': 'system' }, 'dark')).toEqual({ theme: 'dark', background: '#0b0d11' });
    expect(paint({}, 'dark')).toEqual({ theme: 'light', background: '#ffffff' });
    expect(paint({ [FIRST_PAINT_KEY]: '{' }, 'dark')).toEqual({ theme: 'light', background: '#ffffff' });
    expect(paint({ [FIRST_PAINT_KEY]: '{', 'easy-cli-proxy-api.theme': 'dark' })).toEqual({ theme: 'dark', background: '#0b0d11' });
  });
});

describe('registered themes', () => {
  test('manual selection and restart preserve explicit system assignments', async () => {
    const systemThemes = { light: 'warm-sand-light', dark: 'midnight-dark' };
    const app = setup({ preference: 'sakura-light', systemThemes });
    await app.connect();
    expect(app.controller.getSystemThemes()).toEqual(systemThemes);
    app.controller.setPreference('graphite-dark');
    expect(app.savedSystemThemes).toEqual(systemThemes);
    const restarted = setup({ preference: app.saved, systemThemes: app.savedSystemThemes });
    await restarted.connect();
    expect(restarted.controller.getSystemThemes()).toEqual(systemThemes);
    expect(restarted.displayedId).toBe('graphite-dark');
    app.controller.dispose(); restarted.controller.dispose();
  });
  test('resolved selection notifies on system changes and can be frozen when following stops', async () => {
    const app = setup({ system: 'light', systemThemes: { light: 'sakura-light', dark: 'midnight-dark' } });
    await app.connect();
    const seen: string[] = [];
    const unsubscribe = app.controller.subscribe(() => seen.push(app.controller.getResolvedThemeId()));
    app.setSystem('dark'); app.emitNative('dark'); await settle();
    expect(seen).toContain('midnight-dark');
    app.controller.setPreference(app.controller.getResolvedThemeId());
    app.setSystem('light'); app.emitNative('light'); await settle();
    expect(app.displayedId).toBe('midnight-dark');
    unsubscribe(); app.controller.dispose();
  });
  test('assignment supersedes stale native reads and ignores invalid or disposed writes', async () => {
    const app = setup({ system: 'light' });
    const reading = deferred<AppTheme>();
    let calls = 0;
    app.native.readTheme = () => ++calls === 1 ? reading.promise : Promise.resolve('light');
    await app.connect();
    app.controller.setSystemTheme('sakura-light');
    reading.resolve('dark'); await settle();
    expect(app.displayedId).toBe('sakura-light');
    const writes = app.writes.length;
    for (const id of ['system', 'removed-theme', 'sakura-light']) app.controller.setSystemTheme(id);
    expect(app.writes.length).toBe(writes);
    app.controller.dispose(); app.controller.setSystemTheme('midnight-dark');
    expect(app.writes.length).toBe(writes);
  });
  test('system assignments apply immediately only for the active system appearance', async () => {
    const app = setup({ system: 'light' });
    await app.connect();
    app.controller.setSystemTheme('midnight-dark');
    expect(app.displayedId).toBe('light');
    app.controller.setSystemTheme('sakura-light');
    expect(app.displayedId).toBe('sakura-light');
    app.resume(); await settle();
    app.emitNative('light'); await settle();
    expect(app.displayedId).toBe('sakura-light');
    app.setSystem('dark'); app.emitNative('dark'); await settle();
    expect(app.displayedId).toBe('midnight-dark');
    app.setSystem('light'); app.emitNative('light'); await settle();
    expect(app.displayedId).toBe('sakura-light');
    app.controller.setSystemTheme('warm-sand-light');
    expect(app.displayedId).toBe('warm-sand-light');
    app.controller.dispose();
  });
  for (const platform of ['windows', 'macos', 'linux'] as const) {
    test(`${platform}: follows the assigned light/dark themes without rewriting preferences on system changes`, async () => {
      const app = setup({ platform, preference: 'light', system: 'dark' });
      await app.connect();
      app.controller.setSystemTheme('sakura-light');
      app.controller.setSystemTheme('midnight-dark');
      app.controller.setPreference('system');
      await settle();
      expect(app.displayedId).toBe('midnight-dark');
      expect(app.backdropColor).toBe('#101827');
      expect(app.saved).toBe('system');
      expect(app.savedSystemThemes).toEqual({ light: 'sakura-light', dark: 'midnight-dark' });
      const writeCount = app.writes.length;
      app.setSystem('light'); app.emitNative('light');
      await settle();
      expect(app.displayedId).toBe('sakura-light');
      expect(app.backdropColor).toBe('#FDE8EF');
      app.setSystem('dark'); app.resume();
      await settle();
      expect(app.displayedId).toBe('midnight-dark');
      expect(app.writes.length).toBe(writeCount);
      const restarted = setup({ platform, preference: app.saved, systemThemes: app.savedSystemThemes, system: 'light' });
      await restarted.connect();
      expect(restarted.displayedId).toBe('sakura-light');
      app.controller.dispose(); restarted.controller.dispose();
    });
  }
  test('assigns each appearance independently without changing manual selection', async () => {
    const app = setup({ preference: 'midnight-dark', system: 'dark' });
    await app.connect();
    expect(app.controller.getSystemThemes()).toEqual({ light: 'light', dark: 'dark' });
    app.controller.setSystemTheme('midnight-dark');
    app.controller.setSystemTheme('sakura-light');
    expect(app.displayedId).toBe('midnight-dark');
    expect(app.controller.getPreference()).toBe('midnight-dark');
    app.controller.setPreference('sakura-light');
    app.controller.setPreference('warm-sand-light');
    expect(app.controller.getSystemThemes()).toEqual({ light: 'sakura-light', dark: 'midnight-dark' });
    app.controller.setPreference('system');
    await settle();
    expect(app.displayedId).toBe('midnight-dark');
    app.setSystem('light'); app.emitNative('light');
    await settle();
    expect(app.displayedId).toBe('sakura-light');
    app.controller.setSystemTheme('light');
    await settle();
    expect(app.displayedId).toBe('light');
    expect(app.savedSystemThemes).toEqual({ light: 'light', dark: 'midnight-dark' });
    app.controller.dispose();
  });
  test('falls back per appearance without overwriting unavailable history', async () => {
    const remembered = { light: 'removed-light', dark: 'midnight-dark' };
    const app = setup({ system: 'light', systemThemes: remembered });
    await app.connect();
    expect(app.displayedId).toBe('light');
    app.setSystem('dark'); app.emitNative('dark');
    await settle();
    expect(app.displayedId).toBe('midnight-dark');
    expect(app.controller.getSystemThemes()).toEqual(remembered);
    expect(app.writes).toEqual([]);
    app.controller.dispose();
  });
  test('ignores a stale system read after selecting a different registered theme', async () => {
    const app = setup({ systemThemes: { light: 'sakura-light', dark: 'midnight-dark' } });
    const reading = deferred<AppTheme>();
    app.native.readTheme = () => reading.promise;
    await app.connect();
    app.controller.setPreference('warm-sand-light');
    reading.resolve('dark');
    await settle();
    expect(app.displayedId).toBe('warm-sand-light');
    expect(app.backdropColor).toBe('#F7F3EC');
    expect(app.savedSystemThemes).toEqual({ light: 'sakura-light', dark: 'midnight-dark' });
    app.controller.dispose();
  });
  test('uses remembered themes for the media fallback when the desktop theme API fails', async () => {
    const app = setup({ systemThemes: { light: 'sakura-light', dark: 'midnight-dark' } });
    app.native.readTheme = async () => { throw new Error('unavailable'); };
    await app.connect();
    expect(app.displayedId).toBe('sakura-light');
    app.emitMedia('dark');
    await settle();
    expect(app.displayedId).toBe('midnight-dark');
    expect(app.writes).toEqual([]);
    app.controller.dispose();
  });
  const otherDark: ThemeDefinition = {
    id: 'test-dark', nameKey: 'app.theme.dark', appearance: 'dark', order: 1,
    windowBackground: '#102030', tokens: { accent: '#aabbcc' },
  };
  const registry = createThemeRegistry([...themeRegistry.themes, otherDark]);
  test('applies different IDs with the same appearance and keeps manual selection on system changes', async () => {
    const applied: ThemeDefinition[] = [];
    const backgrounds: string[] = [];
    const nativeWrites: (AppTheme | null)[] = [];
    let listener: ((theme: AppTheme) => void) | undefined;
    const controller = createThemeController({
      readPreference: () => 'dark', savePreference() {}, readMediaTheme: () => 'light',
      listenMedia: () => () => {}, listenResume: () => () => {},
      applyTheme: (_, theme) => { applied.push(theme); },
      connectNative: async () => ({
        explicitSystemAppearance: false,
        async setTheme(theme) { nativeWrites.push(theme); },
        async readTheme() { return 'light'; },
        async listen(callback) { listener = callback; return () => {}; },
        async setBackground(_, theme) { backgrounds.push(theme.windowBackground); },
      }),
    }, registry);
    await settle();
    controller.setPreference('test-dark');
    await settle();
    expect(applied.at(-1)?.id).toBe('test-dark');
    expect(backgrounds.at(-1)).toBe('#102030');
    listener?.('light');
    await settle();
    expect(controller.getPreference()).toBe('test-dark');
    expect(nativeWrites.at(-1)).toBe('dark');
    controller.setPreference('system');
    await settle();
    expect(applied.at(-1)?.id).toBe('light');
    controller.setPreference('unregistered');
    expect(controller.getPreference()).toBe('system');
    controller.dispose();
  });
  test('unknown persisted IDs use fixed light without losing the stored selection', async () => {
    const applied: string[] = [];
    let media: (() => void) | undefined;
    const controller = createThemeController({
      readPreference: () => 'removed-dark', savePreference() {}, readMediaTheme: () => 'dark',
      listenMedia(callback) { media = callback; return () => {}; }, listenResume: () => () => {},
      applyTheme: (_, theme) => { applied.push(theme.id); }, connectNative: async () => null,
    }, registry);
    await settle();
    media?.();
    expect(controller.getPreference()).toBe('removed-dark');
    expect(applied).toEqual(['light']);
    controller.dispose();
  });
});
