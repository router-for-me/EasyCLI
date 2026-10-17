import { pluginsApi, pluginStoreApi, type PluginListResponse, type PluginStoreInstallResult } from './plugins';

export type PluginActionStatus = 'ready' | 'globalDisabled' | 'restart' | 'pending' | 'cancelled';

/** A successful write is not evidence that the runtime has applied it. */
export async function waitForPluginAction(
  check: () => Promise<PluginActionStatus>,
  cancelled: () => boolean = () => false,
  timeoutMs = 15_000,
  intervalMs = 500,
): Promise<PluginActionStatus> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (cancelled()) return 'cancelled';
    const status = await check();
    if (cancelled()) return 'cancelled';
    if (status !== 'pending' || Date.now() >= deadline) return status;
    await new Promise(resolve => setTimeout(resolve, Math.min(intervalMs, Math.max(0, deadline - Date.now()))));
  }
}

export function pluginToggleStatus(response: PluginListResponse, id: string, enabled: boolean): PluginActionStatus {
  const plugin = response.plugins.find(item => item.id === id);
  if (!plugin || plugin.enabled !== enabled) return 'pending';
  if (!enabled) return plugin.effectiveEnabled ? 'pending' : 'ready';
  if (!response.pluginsEnabled) return 'globalDisabled';
  return plugin.registered && plugin.effectiveEnabled ? 'ready' : 'pending';
}

export const waitForPluginToggle = (id: string, enabled: boolean, cancelled: () => boolean) =>
  waitForPluginAction(async () => pluginToggleStatus(await pluginsApi.list(), id, enabled), cancelled);

export async function waitForPluginInstall(result: PluginStoreInstallResult, cancelled: () => boolean): Promise<PluginActionStatus> {
  // A restart is authoritative: polling cannot make a restart-required install active.
  if (result.restartRequired) return 'restart';
  return waitForPluginAction(async () => {
    const response = await pluginStoreApi.list();
    const plugin = response.plugins.find(item => item.id === result.id && item.sourceId === result.sourceId);
    if (!plugin?.installed || !plugin.configured) return 'pending';
    const normalize = (version: string) => version.replace(/^[vV](?=\d)/, '');
    if (result.version && normalize(plugin.installedVersion) !== normalize(result.version)) return 'pending';
    if (!response.pluginsEnabled) return 'globalDisabled';
    if (!plugin.enabled) return 'ready';
    return plugin.registered && plugin.effectiveEnabled ? 'ready' : 'pending';
  }, cancelled);
}
