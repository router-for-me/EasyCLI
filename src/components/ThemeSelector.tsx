import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { Check, Monitor, Moon, Sun } from 'lucide-react';
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

export function ThemeSelector({ value, onChange, open, onOpenChange, registry = themeRegistry }: {
  value: string;
  onChange: (id: string) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  registry?: ThemeRegistry;
}) {
  const { t, locale } = useI18n();
  const issue = useThemePreferenceIssue();
  const systemThemes = useSystemThemes();
  const resolvedId = useResolvedThemeId();
  const controls = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const systemToggle = useRef<HTMLButtonElement>(null);
  const retry = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const themeList = useRef<HTMLDivElement>(null);
  const revealedList = useRef<HTMLDivElement | null>(null);
  const id = useId();
  const [active, setActive] = useState(0);
  const [keyboardNavigation, setKeyboardNavigation] = useState(false);
  const [layout, setLayout] = useState<CSSProperties>();
  const [hintLayout, setHintLayout] = useState<CSSProperties>();
  const [announcement, setAnnouncement] = useState('');
  const [appearance, setAppearance] = useState<ThemeAppearance>('light');
  const themes = registry.themes.filter((theme) => theme.appearance === appearance);
  const options = themes.map((theme) => theme.id);
  const selectedId = value === 'system' ? resolvedId : value;
  const definition = registry.get(selectedId);
  const unavailable = value !== 'system' && !definition;
  const name = definition ? t(definition.nameKey) : t('app.theme.unavailable');
  const systemLightHint = t('app.theme.systemLightHint', { name: t(registry.resolveSystem('light', systemThemes).nameKey) });
  const systemDarkHint = t('app.theme.systemDarkHint', { name: t(registry.resolveSystem('dark', systemThemes).nameKey) });
  const issueText = issue === 'read' ? t('app.theme.readFailed') : issue === 'save' ? t('app.theme.saveFailed') : unavailable ? t('app.theme.unavailableHint') : '';

  useLayoutEffect(() => {
    if (!open) { setLayout(undefined); return; }
    const position = () => {
      if (!trigger.current || !menu.current) return;
      const rect = trigger.current.getBoundingClientRect();
      const edge = 12;
      const gap = 8;
      const sidebar = trigger.current.closest('.sidebar');
      const width = Math.min(sidebar && sidebar.clientWidth < 400 ? sidebar.clientWidth - 24 : 280, window.innerWidth - edge * 2);
      menu.current.style.width = `${width}px`;
      // Include the list's hidden content so measuring an already-constrained menu
      // does not change its placement when the window resizes.
      const hiddenListHeight = themeList.current
        ? themeList.current.scrollHeight - themeList.current.clientHeight : 0;
      const desiredHeight = Math.min(menu.current.scrollHeight + hiddenListHeight + 2, 560);
      const above = Math.max(0, rect.top - edge - gap);
      const below = Math.max(0, window.innerHeight - rect.bottom - edge - gap);
      const up = below < desiredHeight && above > below;
      const height = Math.min(desiredHeight, up ? above : below);
      setLayout({
        width, maxHeight: height,
        left: Math.max(edge, Math.min(rect.right - width, window.innerWidth - width - edge)),
        top: up ? rect.top - gap - height : rect.bottom + gap,
      });
    };
    position();
    const scroll = (event: Event) => {
      if (event.target instanceof Node && menu.current?.contains(event.target)) return;
      position();
    };
    window.addEventListener('resize', position);
    window.addEventListener('scroll', scroll, true);
    return () => {
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', scroll, true);
    };
  }, [open, appearance, locale, issueText, systemLightHint, systemDarkHint, registry]);

  useLayoutEffect(() => {
    if (!open) return;
    setActive(Math.max(0, options.indexOf(selectedId)));
    // Options are derived from the stable registry; labels may change independently.
  }, [open, appearance, selectedId, registry]);

  useEffect(() => {
    if (!open) return;
    const close = (event: Event) => {
      const target = event.target as Node;
      if (!controls.current?.contains(target) && !menu.current?.contains(target)) onOpenChange(false);
    };
    document.addEventListener('pointerdown', close);
    document.addEventListener('focusin', close);
    return () => {
      document.removeEventListener('pointerdown', close);
      document.removeEventListener('focusin', close);
    };
  }, [open, onOpenChange]);

  useLayoutEffect(() => {
    const list = themeList.current;
    if (!open || !layout || !list) return;
    const opening = revealedList.current !== list;
    if (!opening && !keyboardNavigation) return;
    revealedList.current = list;
    const index = opening ? Math.max(0, options.indexOf(selectedId)) : active;
    const option = document.getElementById(`${id}-${index}`);
    // Scroll only this viewport, never the page or in response to pointer hover.
    if (!option || !list.contains(option)) return;
    const viewport = list.getBoundingClientRect();
    const row = option.getBoundingClientRect();
    if (row.top < viewport.top) list.scrollTop += row.top - viewport.top;
    else if (row.bottom > viewport.bottom) list.scrollTop += row.bottom - viewport.bottom;
  }, [open, appearance, active, keyboardNavigation, id, layout, selectedId, registry]);

  useEffect(() => {
    const clear = () => setHintLayout(undefined);
    window.addEventListener('resize', clear);
    window.addEventListener('scroll', clear, true);
    return () => { window.removeEventListener('resize', clear); window.removeEventListener('scroll', clear, true); };
  }, []);

  const showHint = () => {
    if (!systemToggle.current) return;
    const rect = systemToggle.current.getBoundingClientRect();
    setHintLayout({ left: Math.max(8, Math.min(rect.right - 220, window.innerWidth - 228)),
      ...(rect.top >= 110 ? { bottom: window.innerHeight - rect.top + 8 } : { top: rect.bottom + 8 }),
    });
  };
  const assignSystem = (theme: ThemeDefinition) => {
    setSystemTheme(theme.id);
    setAnnouncement(t('app.theme.systemAssigned', { name: t(theme.nameKey) }));
  };

  const choose = (next: string) => {
    onChange(next);
    trigger.current?.focus({ preventScroll: true });
  };
  const keyDown = (event: KeyboardEvent<HTMLButtonElement>, targetAppearance: ThemeAppearance) => {
    trigger.current = event.currentTarget;
    const expanded = open && appearance === targetAppearance;
    if (!expanded && ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', ' '].includes(event.key)) {
      event.preventDefault(); setAppearance(targetAppearance); setKeyboardNavigation(true); onOpenChange(true); return;
    }
    if (open && (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10'))) {
      event.preventDefault();
      const theme = registry.get(options[active]);
      if (theme) assignSystem(theme);
      return;
    }
    if (event.key === 'Tab') {
      if (open && !event.shiftKey && retry.current) {
        event.preventDefault();
        retry.current.focus();
      } else onOpenChange(false);
      return;
    }
    if (event.key === 'Escape') {
      if (open) { event.preventDefault(); event.stopPropagation(); onOpenChange(false); }
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', ' '].includes(event.key)) return;
    event.preventDefault();
    setKeyboardNavigation(true);
    if (!open) { onOpenChange(true); return; }
    if (event.key === 'Enter' || event.key === ' ') { choose(options[active]); return; }
    setActive((index) => event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1 :
      (index + (event.key === 'ArrowDown' ? 1 : -1) + options.length) % options.length);
  };

  return (
    <div className="theme-selector" ref={controls}>
      {(['light', 'dark'] as const).map((mode) => {
        const Icon = mode === 'light' ? Sun : Moon;
        const expanded = open && appearance === mode;
        const selected = value !== 'system' && (definition?.appearance ?? 'light') === mode;
        const label = t(mode === 'light' ? 'app.theme.lightGroup' : 'app.theme.darkGroup');
        return <button key={mode} className="theme-selector-trigger" type="button" role="combobox"
          data-appearance={mode} data-selected={selected}
          aria-label={selected ? `${label} — ${t('app.theme.current', { name })}` : label}
          aria-expanded={expanded} aria-haspopup="listbox" aria-controls={`${id}-list`}
          aria-activedescendant={expanded ? `${id}-${active}` : undefined}
          aria-describedby={issueText ? `${id}-issue` : undefined}
          title={selected ? `${label} — ${name}${issueText ? ` — ${issueText}` : ''}` : label}
          onClick={(event) => {
            trigger.current = event.currentTarget;
            setAppearance(mode); setKeyboardNavigation(false); setHintLayout(undefined); onOpenChange(!expanded);
          }} onKeyDown={(event) => keyDown(event, mode)}>
          <Icon size={16} aria-hidden="true" />
          {issueText && mode === (definition?.appearance ?? 'light') ? <i className="theme-selector-warning" aria-hidden="true" /> : null}
        </button>;
      })}
      <button ref={systemToggle} className="theme-selector-system-toggle" type="button" aria-pressed={value === 'system'}
        aria-label={t(value === 'system' ? 'app.theme.disableSystem' : 'app.theme.enableSystem')}
        aria-describedby={`${id}-system-description`} onMouseEnter={showHint} onMouseLeave={() => setHintLayout(undefined)}
        onFocus={showHint} onBlur={() => setHintLayout(undefined)}
        onClick={() => { setHintLayout(undefined); onOpenChange(false); onChange(value === 'system' ? resolvedId : 'system'); }}>
        <Monitor size={15} aria-hidden="true" />
      </button>
      <span id={`${id}-system-description`} className="sr-only">{systemLightHint}，{systemDarkHint}</span>
      <span role="status" className="sr-only">{announcement}</span>
      {issueText ? <span id={`${id}-issue`} className="sr-only">{issueText}</span> : null}
      {open ? createPortal(
        <div ref={menu} className="theme-selector-menu" data-theme-surface="overlay" data-keyboard-navigation={keyboardNavigation}
          style={layout ?? { visibility: 'hidden', top: 0, left: 0 }}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault(); event.stopPropagation();
              onOpenChange(false); trigger.current?.focus();
            }
          }}>
          <div className="theme-selector-heading">{t(appearance === 'light' ? 'app.theme.lightGroup' : 'app.theme.darkGroup')}</div>
          {issueText ? <div className="theme-selector-notice" role="status">
            {issueText}
            {issue === 'save' ? <button ref={retry} type="button"
              onKeyDown={(event) => {
                if (event.key !== 'Tab') return;
                // The portal sits outside the trigger's tab order. Resume navigation there.
                trigger.current?.focus();
                if (event.shiftKey) event.preventDefault();
                else onOpenChange(false);
              }}
              onClick={() => {
                // A successful save removes this button; keep focus on a stable control.
                trigger.current?.focus();
                retryThemeSave();
              }}>{t('app.theme.retrySave')}</button> : null}
          </div> : null}
          <div id={`${id}-list`} className="theme-selector-listbox" role="listbox" aria-label={t('app.theme.choose')}>
            <div key={appearance} ref={themeList} className="theme-selector-themes" onWheel={() => setKeyboardNavigation(false)}>
              {[appearance].map((appearance) => (
                <div role="group" aria-labelledby={`${id}-${appearance}`} key={appearance}>
                  <div id={`${id}-${appearance}`} className="sr-only">{t(appearance === 'light' ? 'app.theme.lightGroup' : 'app.theme.darkGroup')}</div>
                  {themes.filter((theme) => theme.appearance === appearance).map((theme) => {
                    const index = options.indexOf(theme.id);
                    const assigned = registry.resolveSystem(theme.appearance, systemThemes).id === theme.id;
                    const AssignedIcon = theme.appearance === 'light' ? Sun : Moon;
                    return <div key={theme.id} id={`${id}-${index}`} data-theme-id={theme.id} role="option" aria-selected={selectedId === theme.id}
                      aria-label={t(theme.nameKey)} aria-describedby={theme.descriptionKey ? `${id}-${index}-description` : undefined}
                      title={theme.descriptionKey ? `${t(theme.nameKey)}\n${t(theme.descriptionKey)}` : t(theme.nameKey)}
                      className={`theme-selector-option${selectedId === theme.id ? ' selected' : ''}${active === index ? ' active' : ''}`}
                      onPointerDown={(event) => event.preventDefault()} onClick={(event) => { if (event.button === 0 && !event.ctrlKey) choose(theme.id); }}
                      onPointerMove={() => { setKeyboardNavigation(false); setActive(index); }}
                      onContextMenu={(event) => {
                        event.preventDefault(); setActive(index); assignSystem(theme);
                      }}>
                      <ThemeThumbnail theme={theme} />
                      <span className="theme-selector-copy"><span className="theme-selector-name">{t(theme.nameKey)}</span>
                      </span>
                      {theme.descriptionKey ? <span className="sr-only" id={`${id}-${index}-description`}>{t(theme.descriptionKey)}</span> : null}
                      {assigned ? <span className="theme-selector-assigned" title={t(theme.appearance === 'light' ? 'app.theme.systemLightAssigned' : 'app.theme.systemDarkAssigned')}>
                        <AssignedIcon size={12} aria-hidden="true" /><span className="sr-only">{t(theme.appearance === 'light' ? 'app.theme.systemLightAssigned' : 'app.theme.systemDarkAssigned')}</span>
                      </span> : null}
                      {selectedId === theme.id ? <Check size={15} aria-hidden="true" /> : null}
                    </div>;
                  })}
                </div>
              ))}
            </div>
          </div>
          <div className="theme-selector-footer">{t('app.theme.contextHint')}</div>
        </div>, document.body,
      ) : null}
      {hintLayout ? createPortal(<div className="theme-selector-system-hint" role="tooltip" style={hintLayout}>
        <strong>{t(value === 'system' ? 'app.theme.systemEnabled' : 'app.theme.systemDisabled')}</strong>
        <span><Sun size={13} aria-hidden="true" />{systemLightHint}</span>
        <span><Moon size={13} aria-hidden="true" />{systemDarkHint}</span>
      </div>, document.body) : null}
    </div>
  );
}
