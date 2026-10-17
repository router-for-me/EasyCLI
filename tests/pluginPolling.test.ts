import { afterEach, expect, test } from 'bun:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { normalizePluginList, type PluginStoreInstallResult } from '../src/services/plugins';
import { pluginToggleStatus, waitForPluginAction, waitForPluginInstall } from '../src/services/pluginPolling';

test('waits for actual runtime state, not merely the enabled setting', () => {
  const response = normalizePluginList({ plugins_enabled: true, plugins: [{ id: 'sample', enabled: true }] });
  expect(pluginToggleStatus(response, 'sample', true)).toBe('pending');
  response.plugins[0].registered = true;
  response.plugins[0].effectiveEnabled = true;
  expect(pluginToggleStatus(response, 'sample', true)).toBe('ready');
  response.plugins[0].enabled = false;
  expect(pluginToggleStatus(response, 'sample', false)).toBe('pending');
  response.plugins[0].effectiveEnabled = false;
  expect(pluginToggleStatus(response, 'sample', false)).toBe('ready');
  response.plugins[0].enabled = true;
  response.pluginsEnabled = false;
  expect(pluginToggleStatus(response, 'sample', true)).toBe('globalDisabled');
  expect(pluginToggleStatus(response, 'missing', true)).toBe('pending');
});

test('polls until ready, returns pending on timeout, and propagates read failures', async () => {
  let count = 0;
  expect(await waitForPluginAction(async () => ++count === 3 ? 'ready' : 'pending', undefined, 1000, 1)).toBe('ready');
  expect(count).toBe(3);
  expect(await waitForPluginAction(async () => 'pending', undefined, 0)).toBe('pending');
  await expect(waitForPluginAction(async () => { throw new Error('offline'); })).rejects.toThrow('offline');
});

test('cancellation discards in-flight results and stops subsequent reads', async () => {
  let cancelled = false;
  let count = 0;
  expect(await waitForPluginAction(async () => { count++; cancelled = true; return 'ready'; }, () => cancelled)).toBe('cancelled');
  expect(await waitForPluginAction(async () => { count++; return 'ready'; }, () => cancelled)).toBe('cancelled');
  expect(count).toBe(1);
});

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
afterEach(() => {
  if (typeof window !== 'undefined') clearMocks();
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else Reflect.deleteProperty(globalThis, 'window');
});

test('install confirmation matches source and returned version before declaring ready', async () => {
  Object.defineProperty(globalThis, 'window', { value: {}, configurable: true, writable: true });
  let reads = 0;
  mockIPC(() => {
    reads++;
    return { plugins_enabled: true, plugins: [
      { id: 'sample', source_id: 'other', installed: true, configured: true, installed_version: '2.0.0', enabled: true, registered: true, effective_enabled: true },
      { id: 'sample', source_id: 'community', installed: true, configured: true, installed_version: reads === 1 ? '1.0.0' : '2.0.0', enabled: true, registered: true, effective_enabled: true },
    ] };
  });
  const result = { id: 'sample', sourceId: 'community', version: 'v2.0.0', restartRequired: false } as PluginStoreInstallResult;
  expect(await waitForPluginInstall(result, () => false)).toBe('ready');
  expect(reads).toBe(2);
  expect(await waitForPluginInstall({ ...result, restartRequired: true }, () => false)).toBe('restart');
  expect(reads).toBe(2);
});
