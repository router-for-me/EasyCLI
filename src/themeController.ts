import { themeRegistry, type ThemeRegistry } from './themes/registry';
import type { SystemThemes, ThemeAppearance, ThemeDefinition, ThemePreference } from './themes/types';
export type AppTheme = ThemeAppearance;
export type { ThemePreference } from './themes/types';
type Unlisten = () => void;

export interface NativeThemeSource {
  setTheme(theme: AppTheme | null): Promise<void>;
  readTheme(): Promise<AppTheme | null>;
  listen(listener: (theme: AppTheme) => void): Promise<Unlisten>;
  setBackground(theme: AppTheme, definition: ThemeDefinition): Promise<void>;
  explicitSystemAppearance: boolean;
}

export interface ThemeEnvironment {
  readPreference(): ThemePreference;
  readSystemThemes?(): SystemThemes;
  savePreference(preference: ThemePreference, remembered: SystemThemes): void;
  readMediaTheme(): AppTheme;
  listenMedia(listener: () => void): Unlisten;
  listenResume(listener: () => void): Unlisten;
  applyTheme(theme: AppTheme, definition: ThemeDefinition): void;
  connectNative(): Promise<NativeThemeSource | null>;
}

export function createThemeController(environment: ThemeEnvironment, registry: ThemeRegistry = themeRegistry) {
  let preference = environment.readPreference();
  let systemThemes: SystemThemes = { light: 'light', dark: 'dark', ...environment.readSystemThemes?.() };
  let systemAppearance = environment.readMediaTheme();
  let resolvedThemeId = registry.resolve(preference, systemAppearance, systemThemes).id;
  let native: NativeThemeSource | null = null;
  let disposed = false;
  let generation = 0;
  let observation = 0;
  let configuring = false;
  let nativeAppearance: AppTheme | null | undefined;
  let queue = Promise.resolve();
  const subscribers = new Set<Unlisten>();
  const cleanup: Unlisten[] = [];
  const current = (version: number) => !disposed && version === generation;
  const manualAppearance = () => registry.resolve(preference).appearance;
  const resolve = (system: AppTheme) => registry.resolve(preference, system, systemThemes);
  const apply = (system: AppTheme) => {
    if (preference === 'system') systemAppearance = system;
    const definition = resolve(system);
    environment.applyTheme(definition.appearance, definition);
    if (resolvedThemeId !== definition.id) {
      resolvedThemeId = definition.id;
      subscribers.forEach((listener) => listener());
    }
  };

  function enqueue(work: () => Promise<void>) {
    queue = queue.then(work).catch(() => {
    });
  }

  async function background(theme: AppTheme, version: number) {
    if (current(version) && native) await native.setBackground(theme, resolve(theme));
  }

  async function readSystem(version: number) {
    if (!current(version) || preference !== 'system' || !native) return;
    const reading = ++observation;
    const theme = await native.readTheme().catch(() => null);
    if (!current(version) || reading !== observation) return;
    const resolved = theme ?? environment.readMediaTheme();
    apply(resolved);
    if (theme && native.explicitSystemAppearance && nativeAppearance !== theme) {
      configuring = true;
      try {
        nativeAppearance = theme;
        await native.setTheme(theme);
      } catch {
        nativeAppearance = undefined;
      } finally {
        configuring = false;
      }
      if (nativeAppearance && nativeAppearance !== theme) refreshSystem();
    }
    await background(resolved, version);
  }

  function refreshSystem() {
    if (disposed || preference !== 'system') return;
    if (!native) {
      apply(environment.readMediaTheme());
      return;
    }
    const version = generation;
    enqueue(() => readSystem(version));
  }

  function configureNative() {
    const version = generation;
    enqueue(async () => {
      if (!current(version) || !native) return;
      configuring = true;
      try {
        nativeAppearance = preference === 'system' ? null : manualAppearance();
        await native.setTheme(nativeAppearance);
      } catch {
        nativeAppearance = undefined;
      } finally {
        configuring = false;
      }
      if (!current(version)) return;
      if (preference === 'system') await readSystem(version);
      else {
        if (nativeAppearance && nativeAppearance !== manualAppearance()) configureNative();
        await background(manualAppearance(), version);
      }
    });
  }

  function nativeChanged(theme: AppTheme) {
    if (disposed || !native) return;
    nativeAppearance = theme;
    if (configuring) return;
    if (preference !== 'system') {
      if (theme !== manualAppearance()) configureNative();
      return;
    }
    if (native.explicitSystemAppearance) {
      refreshSystem();
      return;
    }
    ++observation;
    apply(theme);
    const version = generation;
    enqueue(() => background(theme, version));
  }

  apply(preference === 'system' ? environment.readMediaTheme() : manualAppearance());
  cleanup.push(environment.listenMedia(refreshSystem), environment.listenResume(refreshSystem));
  void environment.connectNative().then(async (source) => {
    if (disposed || !source) return;
    native = source;
    try {
      const unlisten = await source.listen(nativeChanged);
      if (disposed) unlisten();
      else cleanup.push(unlisten);
    } catch {
    }
    if (!disposed) configureNative();
  }).catch(() => {
  });

  return {
    getPreference: () => preference,
    getSystemThemes: () => systemThemes,
    getResolvedThemeId: () => resolvedThemeId,
    subscribe(listener: Unlisten) {
      subscribers.add(listener);
      return () => { subscribers.delete(listener); };
    },
    setPreference(next: ThemePreference) {
      if (disposed || (next !== 'system' && !registry.get(next))) return;
      if (preference === next) {
        environment.savePreference(next, systemThemes);
        return;
      }
      preference = next;
      ++generation;
      ++observation;
      environment.savePreference(next, systemThemes);
      if (next !== 'system') apply(manualAppearance());
      else if (!native) apply(environment.readMediaTheme());
      configureNative();
      subscribers.forEach((listener) => listener());
    },
    setSystemTheme(id: string) {
      const definition = registry.get(id);
      if (disposed || !definition || systemThemes[definition.appearance] === id) return;
      systemThemes = { ...systemThemes, [definition.appearance]: id };
      environment.savePreference(preference, systemThemes);
      if (preference === 'system') {
        ++generation;
        ++observation;
        apply(systemAppearance);
        configureNative();
      }
      subscribers.forEach((listener) => listener());
    },
    dispose() {
      disposed = true;
      ++generation;
      cleanup.forEach((unlisten) => unlisten());
      subscribers.clear();
    },
  };
}
