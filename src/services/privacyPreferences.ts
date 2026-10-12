const STORAGE_KEY = 'easy-cli-proxy-api.privacy-mode';
type Backend = {
  load(): Promise<boolean | null>;
  save(enabled: boolean): Promise<unknown>;
};

export class PrivacyPreferences {
  private value?: boolean;
  private backend?: Backend;
  private pending = Promise.resolve();
  private listeners = new Set<() => void>();

  constructor(private storage: () => Storage) {}

  private readCache(): boolean | null {
    try {
      const saved = this.storage().getItem(STORAGE_KEY);
      if (saved === 'on') return true;
      if (saved === 'off') return false;
    } catch {}
    return null;
  }

  get = (): boolean => this.value ?? this.readCache() ?? false;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  async initialize(backend: Backend) {
    // Do not overwrite a file with fallback defaults if reading it fails.
    const saved = await backend.load();
    const cached = this.readCache();
    this.backend = backend;
    if (saved !== null) {
      this.value = saved;
      this.cache(saved);
    } else if (cached !== null) {
      this.set(cached);
      await this.flush();
    }
  }

  private cache(enabled: boolean) {
    try { this.storage().setItem(STORAGE_KEY, enabled ? 'on' : 'off'); } catch {}
  }

  set = (enabled: boolean) => {
    const changed = this.get() !== enabled;
    this.value = enabled;
    this.cache(enabled);
    if (this.backend) {
      this.pending = this.pending.then(() => this.backend!.save(enabled)).then(() => {}, () => {
        console.error('Failed to save privacy mode preference');
      });
    }
    if (changed) this.listeners.forEach((listener) => listener());
  };

  flush() { return this.pending; }
}
