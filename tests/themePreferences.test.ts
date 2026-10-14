import { describe, expect, test } from 'bun:test';
import { ThemePreferences } from '../src/services/themePreferences';
import type { SystemThemes, ThemeSelection } from '../src/themes/types';

const record = (selectedId: string, remembered: SystemThemes = { light: 'light', dark: 'dark' }): ThemeSelection =>
  ({ version: 1, selectedId, lastLightId: remembered.light, lastDarkId: remembered.dark });

describe('desktop theme selections', () => {
  test('defaults to light without creating a configuration', async () => {
    const writes: ThemeSelection[] = [];
    const preferences = new ThemePreferences();
    await preferences.initialize({ load: async () => null, save: async (value) => { writes.push(value); } });
    expect(preferences.get()).toBe('light');
    expect(preferences.getSystemThemes()).toEqual({ light: 'light', dark: 'dark' });
    expect(preferences.getIssue()).toBeNull();
    expect(writes).toEqual([]);
  });
  test('restores arbitrary stable IDs, including temporarily unavailable themes', async () => {
    for (const id of ['light', 'dark', 'system', 'test-dark', 'removed-theme']) {
      const preferences = new ThemePreferences();
      await preferences.initialize({ load: async () => record(id), save: async () => {} });
      expect(preferences.get()).toBe(id);
    }
  });
  test('rejects old values, unsupported versions and invalid IDs without overwriting', async () => {
    for (const value of [
      'dark', { version: 2, selectedId: 'dark' }, record('../bad'),
      { ...record('system'), lastLightId: 'system' }, { ...record('system'), lastDarkId: '../bad' },
      { ...record('system'), lastLightId: 123 },
    ]) {
      const writes: unknown[] = [];
      const preferences = new ThemePreferences();
      await preferences.initialize({ load: async () => value, save: async (next) => { writes.push(next); } });
      expect(preferences.get()).toBe('light');
      expect(preferences.getIssue()).toBe('read');
      expect(writes).toEqual([]);
    }
  });
  test('reads existing selections without history and retains unavailable remembered IDs', async () => {
    for (const [selection, expected] of [
      [{ version: 1, selectedId: 'sakura-light' }, { light: 'light', dark: 'dark' }],
      [{ version: 1, selectedId: 'system', lastDarkId: 'midnight-dark' }, { light: 'light', dark: 'midnight-dark' }],
      [record('system', { light: 'removed-light', dark: 'midnight-dark' }), { light: 'removed-light', dark: 'midnight-dark' }],
    ] as const) {
      const writes: ThemeSelection[] = [];
      const preferences = new ThemePreferences();
      await preferences.initialize({ load: async () => selection, save: async (next) => { writes.push(next); } });
      expect(preferences.get()).toBe(selection.selectedId);
      expect(preferences.getSystemThemes()).toEqual(expected);
      expect(preferences.getIssue()).toBeNull();
      expect(writes).toEqual([]);
    }
  });
  test('serializes complete history snapshots and restores the last pair after restart', async () => {
    const writes: ThemeSelection[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const preferences = new ThemePreferences();
    await preferences.initialize({ load: async () => null, save: async (next) => {
      await gate;
      writes.push(next);
    } });
    preferences.set('sakura-light', { light: 'sakura-light', dark: 'dark' });
    preferences.set('midnight-dark', { light: 'sakura-light', dark: 'midnight-dark' });
    preferences.set('system');
    release(); await preferences.flush();
    expect(writes).toEqual([
      record('sakura-light', { light: 'sakura-light', dark: 'dark' }),
      record('midnight-dark', { light: 'sakura-light', dark: 'midnight-dark' }),
      record('system', { light: 'sakura-light', dark: 'midnight-dark' }),
    ]);
    const restored = new ThemePreferences();
    await restored.initialize({ load: async () => writes.at(-1), save: async () => {} });
    expect(restored.get()).toBe('system');
    expect(restored.getSystemThemes()).toEqual({ light: 'sakura-light', dark: 'midnight-dark' });
  });
  test('a failed read does not write defaults; an explicit selection can recover', async () => {
    const writes: ThemeSelection[] = [];
    const preferences = new ThemePreferences();
    await preferences.initialize({ load: async () => { throw new Error('unreadable'); }, save: async (next) => { writes.push(next); } });
    expect(preferences.get()).toBe('light');
    expect(writes).toEqual([]);
    preferences.set('dark');
    await preferences.flush();
    expect(writes).toEqual([record('dark')]);
    expect(preferences.getIssue()).toBeNull();
  });
  test('serializes rapid changes and persists the final selection', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const writes: string[] = [];
    const preferences = new ThemePreferences();
    await preferences.initialize({ load: async () => null, save: async (next) => {
      if (next.selectedId === 'dark') await gate;
      writes.push(next.selectedId);
    } });
    preferences.set('dark'); preferences.set('test-dark'); preferences.set('light');
    await Promise.resolve();
    expect(writes).toEqual([]);
    release(); await preferences.flush();
    expect(writes).toEqual(['dark', 'test-dark', 'light']);
  });
  test('reports a failed save, keeps the applied selection and supports retry', async () => {
    let fails = true;
    let saved: ThemeSelection | undefined;
    let notifications = 0;
    const preferences = new ThemePreferences();
    const stop = preferences.subscribe(() => { notifications++; });
    await preferences.initialize({ load: async () => null, save: async (next) => {
      if (fails) throw new Error('disk full');
      saved = next;
    } });
    const remembered = { light: 'sakura-light', dark: 'midnight-dark' };
    preferences.set('system', remembered); await preferences.flush();
    expect(preferences.get()).toBe('system');
    expect(preferences.getSystemThemes()).toEqual(remembered);
    expect(preferences.getIssue()).toBe('save');
    fails = false; preferences.retry(); await preferences.flush();
    expect(saved).toEqual(record('system', remembered));
    expect(preferences.getIssue()).toBeNull();
    expect(notifications).toBe(3);
    stop();
  });
});
