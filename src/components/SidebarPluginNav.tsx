import { AppWindow, ChevronRight, Lock } from 'lucide-react';
import { useI18n } from '../i18n';
import type { PluginNavEntry } from '../services/pluginNavigation';

export const PLUGIN_CHILDREN_ID = 'nav-plugin-children';

export function PluginNavToggle({ expanded, onToggle }: { expanded: boolean; onToggle: () => void }) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      className="nav-plugin-toggle"
      aria-expanded={expanded}
      aria-controls={PLUGIN_CHILDREN_ID}
      aria-label={t('app.nav.pluginPages')}
      onClick={onToggle}
    >
      <ChevronRight size={14} strokeWidth={1.75} aria-hidden="true" />
    </button>
  );
}

export function PluginNavChildren({ entries, active, locked, onSelect }: {
  entries: PluginNavEntry[];
  active: string;
  locked: boolean;
  onSelect: (route: string) => void;
}) {
  const { t } = useI18n();
  return (
    <div id={PLUGIN_CHILDREN_ID} className="nav-plugin-children" role="group" aria-label={t('app.nav.pluginPages')}>
      {entries.map((entry) => {
        const isActive = entry.route === active;
        return (
          <button
            key={entry.route}
            type="button"
            className={['nav-plugin-entry', isActive ? 'active' : '', locked ? 'locked' : ''].filter(Boolean).join(' ')}
            aria-current={isActive ? 'page' : undefined}
            data-plugin-route={entry.route}
            disabled={locked}
            title={locked ? t('app.nav.lockedHint') : entry.description}
            onClick={() => onSelect(entry.route)}
          >
            <AppWindow size={16} strokeWidth={1.75} aria-hidden="true" />
            <span>{entry.label}</span>
            {locked ? <Lock size={12} strokeWidth={1.75} className="nav-lock-icon" aria-hidden="true" /> : null}
          </button>
        );
      })}
    </div>
  );
}
