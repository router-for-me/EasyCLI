import { collectPluginResourceEntries, type PluginResourceEntry } from './pluginResources';
import type { PluginListResponse } from './plugins';

export const PLUGIN_PINS_KEY = 'cpa-gui-plugin-pins-v1';
export type PluginPin = { pluginID: string; path: string };
export const pluginPinKey = (pin: PluginPin) => JSON.stringify([pin.pluginID, pin.path]);
export const resourcePin = (entry: PluginResourceEntry): PluginPin => ({ pluginID: entry.pluginID, path: entry.menu.path });

export function parsePluginPins(raw: string | null): PluginPin[] {
  try {
    const value: unknown = JSON.parse(raw ?? '[]');
    if (!Array.isArray(value)) return [];
    const pins = value.filter((pin): pin is PluginPin => pin && typeof pin.pluginID === 'string'
      && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(pin.pluginID)
      && typeof pin.path === 'string' && pin.path.startsWith('/') && !pin.path.startsWith('//')
      && !/[\\\u0000-\u001f]/.test(pin.path));
    return [...new Map(pins.map(pin => [pluginPinKey(pin), { pluginID: pin.pluginID, path: pin.path }])).values()];
  } catch { return []; }
}

/** Only a successful catalog can prune pins. Disabled/unloaded plugins may omit menus. */
export function reconcilePluginPins(pins: PluginPin[], data: PluginListResponse): PluginPin[] {
  if (!data.pluginsEnabled) return pins;
  return pins.filter(pin => {
    const plugin = data.plugins.find(plugin => plugin.id === pin.pluginID);
    return plugin && (!plugin.effectiveEnabled || plugin.menus.some(menu => menu.path === pin.path));
  });
}

export function resolvePluginPins(pins: PluginPin[], data: PluginListResponse | null): PluginResourceEntry[] {
  if (!data?.pluginsEnabled) return [];
  const entries = collectPluginResourceEntries(data.plugins);
  return pins.flatMap(pin => {
    const entry = entries.find(entry => pluginPinKey(resourcePin(entry)) === pluginPinKey(pin));
    return entry ? [entry] : [];
  });
}
