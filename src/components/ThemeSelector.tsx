import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown, Monitor, Moon, Sun, SunMoon } from 'lucide-react';
import { useI18n } from '../i18n';
import { retryThemeSave, setSystemTheme, useResolvedThemeId, useSystemThemes, useThemePreferenceIssue } from '../theme';
import { themePreview, themeRegistry, type ThemeRegistry } from '../themes/registry';
import type { ThemeAppearance, ThemeDefinition } from '../themes/types';
import './ThemeSelector.css';

function ThemeThumbnail({ theme }: { theme: ThemeDefinition }) {
  const preview = themePreview(theme);
  return (
    <span className="theme-thumbnail" aria-hidden="true" style={{
      '--preview-bg': preview.background, '--preview-sidebar': preview.sidebar,
      '--preview-surface': preview.surface, '--preview-accent': preview.accent,
    } as CSSProperties}>
      <span className="theme-thumbnail-sidebar"><i /><i /><i /></span>
      <span className="theme-thumbnail-content"><i /><i /><i /></span>
    </span>
  );
}

type Mode = ThemeAppearance | 'system';

export function ThemeSelector({ value, onChange, open, onOpenChange, registry = themeRegistry }: {
  value: string;
  onChange: (id: string) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  registry?: ThemeRegistry;
}) {
  const { t } = useI18n();
  const issue = useThemePreferenceIssue();
  const systemThemes = useSystemThemes();
  const resolvedId = useResolvedThemeId();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState<CSSProperties>();
  const fixed = value === 'system' ? undefined : registry.get(value);
  const unavailable = value !== 'system' && !fixed;
  const mode: Mode = value === 'system' ? 'system' : fixed?.appearance ?? 'light';
  const slot = (appearance: ThemeAppearance) =>
    fixed?.appearance === appearance ? fixed.id : registry.resolveSystem(appearance, systemThemes).id;
  const current = registry.get(resolvedId) ?? registry.get('light')!;
  const name = unavailable ? t('app.theme.unavailable') : t(current.nameKey);
  const issueText = issue === 'read' ? t('app.theme.readFailed') : issue === 'save' ? t('app.theme.saveFailed')
    : unavailable ? t('app.theme.unavailableHint') : '';
  const label = `${t('app.theme.triggerHint')} — ${t('app.theme.current', { name })}`;

  useLayoutEffect(() => {
    if (!open) { setLayout(undefined); return; }
    const position = () => {
      if (!trigger.current) return;
      const rect = trigger.current.getBoundingClientRect();
      const edge = 12;
      const width = Math.min(300, window.innerWidth - edge * 2);
      const above = rect.top - edge - 8;
      const below = window.innerHeight - rect.bottom - edge - 8;
      const up = above >= below;
      setLayout({
        width, maxHeight: Math.max(0, up ? above : below),
        left: Math.max(edge, Math.min(rect.left, window.innerWidth - width - edge)),
        ...(up ? { bottom: window.innerHeight - rect.top + 8 } : { top: rect.bottom + 8 }),
      });
    };
    position();
    window.addEventListener('resize', position);
    return () => window.removeEventListener('resize', position);
  }, [open]);

  useEffect(() => {
    if (open && layout) menu.current?.querySelector<HTMLButtonElement>('.theme-selector-mode[aria-pressed="true"]')?.focus({ preventScroll: true });
  }, [open, layout === undefined]);

  useEffect(() => {
    if (!open) return;
    const close = (event: Event) => {
      const target = event.target as Node;
      if (!trigger.current?.contains(target) && !menu.current?.contains(target)) onOpenChange(false);
    };
    document.addEventListener('pointerdown', close);
    document.addEventListener('focusin', close);
    return () => {
      document.removeEventListener('pointerdown', close);
      document.removeEventListener('focusin', close);
    };
  }, [open, onOpenChange]);

  const closeToTrigger = () => { onOpenChange(false); trigger.current?.focus(); };
  const chooseMode = (next: Mode) => {
    if (fixed) setSystemTheme(fixed.id);
    onChange(next === 'system' ? 'system' : slot(next));
  };
  const chooseTheme = (theme: ThemeDefinition) => {
    setSystemTheme(theme.id);
    if (mode !== 'system') onChange(theme.id);
  };
  const modes: { id: Mode; label: string; Icon: typeof Sun }[] = [
    { id: 'light', label: t('app.theme.light'), Icon: Sun },
    { id: 'dark', label: t('app.theme.dark'), Icon: Moon },
    { id: 'system', label: t('app.theme.system'), Icon: Monitor },
  ];

  return (
    <div className="theme-selector">
      <button ref={trigger} className="theme-selector-trigger" type="button"
        aria-haspopup="dialog" aria-expanded={open} aria-label={label}
        title={issueText ? `${label} — ${issueText}` : label} onClick={() => onOpenChange(!open)}
        onKeyDown={(event) => { if (event.key === 'Escape' && open) { event.preventDefault(); onOpenChange(false); } }}>
        <SunMoon size={16} strokeWidth={1.75} aria-hidden="true" />
        <span>{t('app.theme.triggerLabel')}</span>
        <ChevronDown className="theme-selector-chevron" size={12} aria-hidden="true" />
        {issueText ? <i className="theme-selector-warning" aria-hidden="true" /> : null}
      </button>
      {open ? createPortal(
        <div ref={menu} className="theme-selector-menu" role="dialog" aria-label={t('app.theme.label')}
          data-theme-surface="overlay" style={layout ?? { visibility: 'hidden', top: 0, left: 0 }}
          onKeyDown={(event) => {
            if (event.key !== 'Escape') return;
            event.preventDefault(); event.stopPropagation(); closeToTrigger();
          }}>
          {issueText ? <div className="theme-selector-notice" role="status">
            {issueText}
            {issue === 'save' ? <button type="button" onClick={() => { trigger.current?.focus(); retryThemeSave(); }}>
              {t('app.theme.retrySave')}
            </button> : null}
          </div> : null}
          <div className="theme-selector-modes" role="group" aria-label={t('app.theme.label')}>
            {modes.map(({ id, label: modeLabel, Icon }) => (
              <button key={id} type="button" data-mode={id} aria-pressed={mode === id}
                className={`theme-selector-mode${mode === id ? ' selected' : ''}`} onClick={() => chooseMode(id)}>
                <Icon size={14} aria-hidden="true" />{modeLabel}
              </button>
            ))}
          </div>
          {(['light', 'dark'] as const).map((appearance) => {
            const chosen = slot(appearance);
            const group = t(appearance === 'light' ? 'app.theme.lightGroup' : 'app.theme.darkGroup');
            return (
              <div key={appearance} className={`theme-selector-group${mode === 'system' || mode === appearance ? '' : ' inactive'}`}
                data-appearance={appearance} role="group" aria-label={group}>
                <div className="theme-selector-heading" aria-hidden="true">{group}</div>
                <div className="theme-selector-grid">
                  {registry.themes.filter((theme) => theme.appearance === appearance).map((theme) => (
                    <button key={theme.id} type="button" data-theme-id={theme.id} aria-pressed={chosen === theme.id}
                      title={theme.descriptionKey ? `${t(theme.nameKey)}\n${t(theme.descriptionKey)}` : t(theme.nameKey)}
                      className={`theme-selector-card${chosen === theme.id ? ' selected' : ''}`}
                      onClick={() => chooseTheme(theme)}>
                      <ThemeThumbnail theme={theme} />
                      <span className="theme-selector-name">{t(theme.nameKey)}</span>
                      {chosen === theme.id ? <Check className="theme-selector-check" size={12} aria-hidden="true" /> : null}
                    </button>
                  ))}
                </div>
              </div>
            );
          })}
        </div>, document.body,
      ) : null}
    </div>
  );
}
