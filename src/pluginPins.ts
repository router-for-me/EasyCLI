import { useEffect, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PLUGIN_RESOURCES_REFRESH_EVENT } from './services/pluginResources';
import { pluginsApi, type PluginListResponse } from './services/plugins';
import { parsePluginPins, PLUGIN_PINS_KEY, pluginPinKey, reconcilePluginPins, resolvePluginPins, type PluginPin } from './services/pluginPins';

const listeners = new Set<() => void>();
let snapshot: PluginPin[] | undefined;
function getPins() {
  if (!snapshot) {
    try { snapshot = parsePluginPins(window.localStorage.getItem(PLUGIN_PINS_KEY)); }
    catch { snapshot = []; }
  }
  return snapshot;
}
function savePins(pins: PluginPin[]) {
  if (JSON.stringify(pins) === JSON.stringify(getPins())) return;
  // Let the UI report a failed write instead of claiming it persisted.
  window.localStorage.setItem(PLUGIN_PINS_KEY, JSON.stringify(pins));
  snapshot = pins;
  listeners.forEach(listener => listener());
}
function storageChanged(event: StorageEvent) {
  if (event.key !== PLUGIN_PINS_KEY && event.key !== null) return;
  snapshot = undefined;
  listeners.forEach(listener => listener());
}
function subscribe(listener: () => void) {
  if (!listeners.size) window.addEventListener('storage', storageChanged);
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (!listeners.size) window.removeEventListener('storage', storageChanged);
  };
}
export function usePluginPins() {
  return useSyncExternalStore(subscribe, getPins, getPins);
}
export function setPluginPinned(pin: PluginPin, pinned: boolean) {
  const remaining = getPins().filter(item => pluginPinKey(item) !== pluginPinKey(pin));
  savePins(pinned ? [...remaining, pin] : remaining);
}
export function removePluginPins(pluginID: string) {
  savePins(getPins().filter(pin => pin.pluginID !== pluginID));
}

export function usePinnedPluginResources(coreReady: boolean) {
  const pins = usePluginPins();
  const [data, setData] = useState<PluginListResponse | null>(null);
  const hasPins = pins.length > 0;
  useEffect(() => {
    if (!coreReady || !hasPins) { setData(null); return; }
    let disposed = false;
    let revision = 0;
    const refresh = async () => {
      const current = ++revision;
      try {
        const supported = await invoke<boolean>('get_plugin_support');
        if (disposed || current !== revision) return;
        const next = supported ? await pluginsApi.list() : null;
        if (disposed || current !== revision) return;
        setData(next);
        if (next) {
          try { savePins(reconcilePluginPins(getPins(), next)); }
          catch { /* Hide stale entries even when preference storage is unavailable. */ }
        }
      } catch {
        if (!disposed && current === revision) setData(null);
      }
    };
    const refreshVisible = () => { if (!document.hidden) void refresh(); };
    void refresh();
    window.addEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refreshVisible);
    window.addEventListener('focus', refreshVisible);
    const timer = window.setInterval(refreshVisible, 15_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
      window.removeEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refreshVisible);
      window.removeEventListener('focus', refreshVisible);
    };
  }, [coreReady, hasPins]);
  return coreReady ? resolvePluginPins(pins, data) : [];
}
