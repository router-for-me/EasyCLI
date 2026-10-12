import { expect, test } from 'bun:test';
import { PrivacyPreferences } from '../src/services/privacyPreferences';

function storage(initial?: string) {
  let value = initial ?? null;
  return { getItem: () => value, setItem: (_key: string, next: string) => { value = next; } } as Storage;
}

test('defaults to off and notifies subscribers only on change', () => {
  const preferences = new PrivacyPreferences(() => storage());
  let notifications = 0;
  preferences.subscribe(() => { notifications += 1; });
  expect(preferences.get()).toBe(false);
  preferences.set(false);
  preferences.set(true);
  preferences.set(true);
  expect(preferences.get()).toBe(true);
  expect(notifications).toBe(1);
});

test('restores privacy mode after browser storage is lost', async () => {
  let disk: boolean | null = null;
  const backend = { load: async () => disk, save: async (next: boolean) => { disk = next; } };
  const first = new PrivacyPreferences(() => storage('on'));
  await first.initialize(backend);
  expect(disk).toBe(true);
  const restarted = new PrivacyPreferences(() => storage());
  await restarted.initialize(backend);
  expect(restarted.get()).toBe(true);
});

test('does not overwrite the saved preference when loading fails', async () => {
  let saves = 0;
  const preferences = new PrivacyPreferences(() => storage('on'));
  await expect(preferences.initialize({
    load: async () => { throw new Error('read failed'); },
    save: async () => { saves += 1; },
  })).rejects.toThrow('read failed');
  expect(saves).toBe(0);
  expect(preferences.get()).toBe(true);
});
