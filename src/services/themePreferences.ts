import { isThemeSelection, type SystemThemes, type ThemeSelection } from '../themes/types';

type Backend = {
  load(): Promise<unknown>;
  save(selection: ThemeSelection): Promise<unknown>;
};
export type ThemePreferenceIssue = 'read' | 'save' | null;

export class ThemePreferences {
  private value = 'light';
  private systemThemes: SystemThemes = { light: 'light', dark: 'dark' };
  private backend?: Backend;
  private pending = Promise.resolve();
  private revision = 0;
  private issue: ThemePreferenceIssue = null;
  private subscribers = new Set<() => void>();

  get = () => this.value;
  getSystemThemes = () => this.systemThemes;
  getIssue = () => this.issue;
  subscribe = (listener: () => void) => {
    this.subscribers.add(listener);
    return () => { this.subscribers.delete(listener); };
  };

  private report(issue: ThemePreferenceIssue) {
    this.issue = issue;
    this.subscribers.forEach((listener) => listener());
  }

  async initialize(backend: Backend) {
    this.backend = backend;
    try {
      const saved = await backend.load();
      if (saved !== null && !isThemeSelection(saved)) throw new Error('Invalid theme selection');
      this.value = saved?.selectedId ?? 'light';
      this.systemThemes = { light: saved?.lastLightId ?? 'light', dark: saved?.lastDarkId ?? 'dark' };
      this.report(null);
    } catch {
      this.value = 'light';
      this.systemThemes = { light: 'light', dark: 'dark' };
      this.report('read');
    }
  }

  set(preference: string, remembered: SystemThemes = this.systemThemes) {
    const selection: ThemeSelection = {
      version: 1, selectedId: preference, lastLightId: remembered.light, lastDarkId: remembered.dark,
    };
    if (!isThemeSelection(selection)) throw new Error('Invalid theme selection');
    this.value = preference;
    this.systemThemes = { ...remembered };
    if (!this.backend) return;
    const revision = ++this.revision;
    this.pending = this.pending.then(async () => {
      try {
        await this.backend!.save(selection);
        if (revision === this.revision) this.report(null);
      } catch {
        if (revision === this.revision) this.report('save');
      }
    });
  }

  retry = () => this.set(this.value);
  flush() { return this.pending; }
}
