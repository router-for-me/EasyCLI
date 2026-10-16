import { invoke } from '@tauri-apps/api/core';
import { emptyPluginNavArrangement, parsePluginNavArrangement, type PluginNavArrangement } from './pluginNavArrangement';

const CACHE_KEY = 'cpa-gui.plugin-nav-layout.v1';

export type PluginNavArrangementStatus = 'loading' | 'ready' | 'unreadable';
export type PluginNavArrangementSnapshot = { status: PluginNavArrangementStatus; arrangement: PluginNavArrangement };
type Backend = {
  load(): Promise<unknown>;
  save(layout: PluginNavArrangement): Promise<unknown>;
};

export class PluginNavArrangementStore {
  private snapshot: PluginNavArrangementSnapshot;
  private saved: PluginNavArrangement;
  private pending = Promise.resolve();
  private generation = 0;
  private listeners = new Set<() => void>();
  private failureListeners = new Set<() => void>();

  constructor(private backend: Backend, private storage: () => Storage) {
    this.saved = this.readCache() ?? emptyPluginNavArrangement();
    this.snapshot = { status: 'loading', arrangement: this.saved };
  }

  private readCache(): PluginNavArrangement | null {
    try {
      return parsePluginNavArrangement(JSON.parse(this.storage().getItem(CACHE_KEY) ?? 'null'));
    } catch {
      return null;
    }
  }

  private cache(arrangement: PluginNavArrangement) {
    try { this.storage().setItem(CACHE_KEY, JSON.stringify(arrangement)); } catch {}
  }

  private publish(snapshot: PluginNavArrangementSnapshot) {
    this.snapshot = snapshot;
    this.listeners.forEach((listener) => listener());
  }

  get = (): PluginNavArrangementSnapshot => this.snapshot;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  onSaveFailed = (listener: () => void) => {
    this.failureListeners.add(listener);
    return () => { this.failureListeners.delete(listener); };
  };

  async load() {
    try {
      const value = await this.backend.load();
      const arrangement = value === null ? emptyPluginNavArrangement() : parsePluginNavArrangement(value);
      if (!arrangement) {
        this.publish({ status: 'unreadable', arrangement: emptyPluginNavArrangement() });
        return;
      }
      this.saved = arrangement;
      this.cache(arrangement);
      this.publish({ status: 'ready', arrangement });
    } catch {
      this.publish({ status: 'unreadable', arrangement: emptyPluginNavArrangement() });
    }
  }

  update = (change: (arrangement: PluginNavArrangement) => PluginNavArrangement) => {
    if (this.snapshot.status !== 'ready') return;
    const next = change(this.snapshot.arrangement);
    if (next === this.snapshot.arrangement) return;
    const generation = this.generation;
    this.publish({ status: 'ready', arrangement: next });
    this.pending = this.pending.then(async () => {
      if (generation !== this.generation) return;
      try {
        await this.backend.save(next);
        this.saved = next;
        this.cache(next);
      } catch {
        this.generation += 1;
        this.publish({ status: 'ready', arrangement: this.saved });
        this.failureListeners.forEach((listener) => listener());
      }
    });
  };

  flush() { return this.pending; }
}

export const pluginNavArrangementStore = new PluginNavArrangementStore({
  load: () => invoke('get_plugin_nav_layout'),
  save: (layout) => invoke('save_plugin_nav_layout', { layout }),
}, () => window.localStorage);
