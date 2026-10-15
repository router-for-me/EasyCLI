import { MessageNotice } from './appNotice';
import { ThemeSelector } from './components/ThemeSelector';
import { WindowFrame } from './components/WindowFrame';
import { useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  Bot,
  Check,
  ChevronsUpDown,
  ExternalLink,
  Eye,
  EyeOff,
  Gauge,
  History,
  House,
  Languages,
  Lock,
  LogIn,
  MessageCircle,
  Network,
  PackageOpen,
  Puzzle,
  ServerCog,
  Settings,
  Sprout,
  X,
} from 'lucide-react';
import appLogo from './assets/logo.jpg';
import { CoreRuntimeProvider, useCoreRuntime } from './coreRuntime';
import { CoreUpdateProvider, useCoreUpdate } from './coreUpdate';
import { ConfigPanelPage } from './pages/ConfigPanel';
import { ApiAccessPage } from './pages/ApiAccessPage';
import { KernelPage } from './pages/Kernel';
import { VersionManagementPage } from './pages/VersionManagementPage';
import { OAuthManagementPage } from './pages/ManagementPages';
import { QuotaPage } from './pages/QuotaPage';
import { AgentsPage } from './pages/AgentsPage';
import { EasyModePage } from './pages/EasyModePage';
import { UsageRecordsPage } from './pages/UsageRecordsPage';
import { PluginResourcePage } from './pages/PluginResourcePage';
import { PluginResourceIcon } from './components/PluginResourceIcon';
import { usePinnedPluginResources } from './pluginPins';
import { pluginPinKey, resourcePin } from './services/pluginPins';
import { pluginText } from './i18n/plugins';
import { PluginsPage } from './pages/PluginsPage';
import { languageOptions, useI18n } from './i18n';
import { AppUpdateDialog, AppUpdateProvider, useAppUpdate } from './appUpdate';
import { appUpdateIndicatorState } from './appUpdateModel';
import { canOpenAppPage, isAlwaysAvailablePage, requestAppNavigation } from './navigation';
import { useThemePreference } from './theme';
import { usePrivacyMode } from './privacy';
import { useDialogFocusTrap } from './components/useDialogFocusTrap';

const QQ_CONTACT_URL = 'https://qm.qq.com/q/3queDaIG';
const DISCORD_SERVER_URL = 'https://discord.gg/PxvX4D9kgs';

const pages = [
  {
    id: 'easy',
    labelKey: 'app.nav.easy',
    icon: House,
    component: HomePage,
  },
  {
    id: 'home',
    labelKey: 'app.nav.home',
    icon: House,
    component: HomePage,
  },
  {
    id: 'api',
    labelKey: 'app.nav.api',
    icon: Network,
    component: ApiAccessPage,
  },
  {
    id: 'oauth',
    labelKey: 'app.nav.oauth',
    icon: LogIn,
    component: OAuthManagementPage,
  },
  {
    id: 'quota',
    labelKey: 'app.nav.quota',
    icon: Gauge,
    component: QuotaPage,
  },
  {
    id: 'agents',
    labelKey: 'app.nav.agents',
    icon: Bot,
    component: AgentsPage,
  },
  {
    id: 'usage-records',
    labelKey: 'app.nav.usageRecords',
    icon: History,
    component: UsageRecordsPage,
  },
  {
    id: 'plugins',
    labelKey: 'app.nav.plugins',
    icon: Puzzle,
    component: PluginsPage,
  },
  {
    id: 'config',
    labelKey: 'app.nav.config',
    icon: Settings,
    component: ConfigPanelPage,
  },
  {
    id: 'versions',
    labelKey: 'app.nav.versions',
    icon: PackageOpen,
    component: VersionManagementPageWrapper,
  },
] as const;

type PageId = (typeof pages)[number]['id'] | `plugin:${string}`;
type WindowsCloseAction = 'exit' | 'minimize-to-tray';
type WindowsCloseBehavior = 'ask' | WindowsCloseAction;

type WindowsClosePrompt = {
  resolvingAction: WindowsCloseAction | null;
  rememberChoice: boolean;
  error: string | null;
};

type GuiSettings = {
  closeBehavior: WindowsCloseBehavior;
};

function HomePage() {
  return <KernelPage view="home" />;
}

function VersionManagementPageWrapper() {
  return <VersionManagementPage />;
}

function App() {
  return (
    <AppUpdateProvider>
      <CoreRuntimeProvider>
        <CoreUpdateProvider>
          <WindowFrame><AppContent /></WindowFrame>
        </CoreUpdateProvider>
      </CoreRuntimeProvider>
    </AppUpdateProvider>
  );
}

function AppContent() {
  const { locale, setLocale, t } = useI18n();
  const { info: appUpdateInfo, hasUpdate, processing: appUpdateProcessing } = useAppUpdate();
  const { latest: coreLatest, hasUpdate: coreHasUpdate } = useCoreUpdate();
  const [active, setActive] = useState<PageId>('home');
  const [languageMenuOpen, setLanguageMenuOpen] = useState(false);
  const [theme, setTheme] = useThemePreference();
  const [themeMenuOpen, setThemeMenuOpen] = useState(false);
  const [privacyMode, setPrivacyMode] = usePrivacyMode();
  const [windowsClosePrompt, setWindowsClosePrompt] = useState<WindowsClosePrompt | null>(null);
  const closeDialogRef = useDialogFocusTrap<HTMLElement>({
    active: Boolean(windowsClosePrompt),
    onEscape: windowsClosePrompt?.resolvingAction
      ? undefined
      : () => setWindowsClosePrompt(null),
    preventEscape: Boolean(windowsClosePrompt?.resolvingAction),
  });
  const languageMenuRef = useRef<HTMLDivElement>(null);
  const languageButtonRef = useRef<HTMLButtonElement>(null);
  const { status } = useCoreRuntime();
  const coreReady = Boolean(status?.ready);
  const pinnedPages = usePinnedPluginResources(coreReady);
  const pinnedPage = pinnedPages.find(entry => `plugin:${pluginPinKey(resourcePin(entry))}` === active);
  const activePage = pages.find((page) => page.id === active) ?? pages[0];
  const ActivePage = activePage.component;
  const selectedLanguage = languageOptions.find((option) => option.value === locale)
    ?? languageOptions[0];
  const availableUpdateLabel = [
    hasUpdate
      ? t('appUpdate.badgeAvailable', { version: appUpdateInfo?.latestVersion ?? '' })
      : '',
    coreHasUpdate
      ? `${t('kernel.versions.coreCardTitle')}: ${t('kernel.update.available')} ${coreLatest?.version ?? ''}`.trim()
      : '',
  ].filter(Boolean).join(' · ');
  useEffect(() => {
    if (!canOpenAppPage(active, coreReady) || (active.startsWith('plugin:') && !pinnedPage)) {
      setActive('home');
    }
  }, [active, coreReady, pinnedPage]);

  useEffect(() => {
    if (!languageMenuOpen) return undefined;
    const closeFromOutside = (event: PointerEvent) => {
      if (!languageMenuRef.current?.contains(event.target as Node)) {
        setLanguageMenuOpen(false);
      }
    };
    const closeFromKeyboard = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      setLanguageMenuOpen(false);
      languageButtonRef.current?.focus();
    };
    document.addEventListener('pointerdown', closeFromOutside);
    document.addEventListener('keydown', closeFromKeyboard);
    return () => {
      document.removeEventListener('pointerdown', closeFromOutside);
      document.removeEventListener('keydown', closeFromKeyboard);
    };
  }, [languageMenuOpen]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;

    const handleWindowsCloseRequest = async () => {
      try {
        const settings = await invoke<GuiSettings>('get_gui_settings');
        if (settings.closeBehavior !== 'ask') {
          await resolveWindowsCloseRequest(settings.closeBehavior, false);
          return;
        }
      } catch (error) {
        console.error('Failed to read close behavior settings', error);
      }

      setWindowsClosePrompt((current) =>
        current ?? {
          resolvingAction: null,
          rememberChoice: false,
          error: null,
        },
      );
    };

    void listen('windows-close-requested', () => {
      void handleWindowsCloseRequest();
    })
      .then((stop) => {
        if (disposed) {
          stop();
        } else {
          stopListening = stop;
        }
      })
      .catch((error) => {
        console.error('Failed to listen for Windows close confirmation events', error);
      });

    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  const handleLanguageListKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    const options = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="option"]'));
    if (!options.length) return;
    const current = options.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? options.length - 1
        : event.key === 'ArrowDown'
          ? (Math.max(current, -1) + 1) % options.length
          : (current <= 0 ? options.length : current) - 1;
    event.preventDefault();
    options[next]?.focus();
  };

  const select = useCallback((pageId: PageId) => {
    if (!canOpenAppPage(pageId, coreReady)) {
      return;
    }
    if (pageId !== active) requestAppNavigation(() => setActive(pageId));
  }, [active, coreReady]);

  useEffect(() => {
    const handleNavigate = (event: Event) => {
      const customEvent = event as CustomEvent<PageId>;
      if (customEvent.detail) {
        select(customEvent.detail);
      }
    };
    window.addEventListener('app:navigate', handleNavigate);
    return () => window.removeEventListener('app:navigate', handleNavigate);
  }, [select]);

  const openContact = async () => {
    try {
      await invoke('open_external_url', {
        url: locale === 'zh-CN' ? QQ_CONTACT_URL : DISCORD_SERVER_URL,
      });
    } catch (error) {
      console.error('Failed to open the contact link', error);
    }
  };

  const resolveWindowsCloseRequest = async (
    action: WindowsCloseAction,
    remember = windowsClosePrompt?.rememberChoice ?? false,
  ) => {
    setWindowsClosePrompt((current) =>
      current
        ? {
            ...current,
            resolvingAction: action,
            error: null,
          }
        : current,
    );

    try {
      await invoke('resolve_windows_close_request', { action, remember });
      setWindowsClosePrompt(null);
    } catch (error) {
      setWindowsClosePrompt((current) =>
        current
          ? {
              ...current,
              resolvingAction: null,
              error: error instanceof Error ? error.message : String(error),
            }
          : {
              resolvingAction: null,
              rememberChoice: false,
              error: error instanceof Error ? error.message : String(error),
            },
      );
    }
  };

  return (
    <>
      <div className={`app-shell${active === "easy" ? " app-shell-easy-mode" : ""}`}>
        {active !== "easy" ? (
          <aside className="sidebar">
            <div className="sidebar-brand" title={t('app.desktopConsole')}>
              <img src={appLogo} alt="" className="brand-mark brand-logo" />
              <div className="sidebar-brand-text">
                <strong>EasyCLIProxyAPI</strong>
                <span>{t('app.desktopConsole')}</span>
              </div>
            </div>

            <nav className="nav-section" aria-label={t('app.navigation')}>
              {pages.filter((page) => page.id !== 'easy').map((page) => {
                const Icon = page.icon;
                const isActive = page.id === active;
                const locked = !canOpenAppPage(page.id, coreReady);
                const updateIndicator = page.id === 'versions'
                  ? appUpdateIndicatorState(hasUpdate, coreHasUpdate, appUpdateProcessing)
                  : null;
                return (
                  <button
                    key={page.id}
                    type="button"
                    className={[
                      isActive ? 'active' : '',
                      locked ? 'locked' : '',
                    ]
                      .filter(Boolean)
                      .join(' ')}
                    aria-current={isActive ? 'page' : undefined}
                    disabled={locked}
                    title={locked ? t('app.nav.lockedHint') : undefined}
                    onClick={() => select(page.id)}
                  >
                    <Icon size={16} strokeWidth={1.75} aria-hidden="true" />
                    <span>{t(page.labelKey)}</span>
                    {locked ? (
                      <Lock size={12} strokeWidth={1.75} className="nav-lock-icon" aria-hidden="true" />
                    ) : updateIndicator ? (
                      <i
                        className={`nav-update-indicator ${updateIndicator}`}
                        title={updateIndicator === 'processing'
                          ? t('appUpdate.progressTitle')
                          : availableUpdateLabel}
                        aria-label={updateIndicator === 'processing'
                          ? t('appUpdate.progressTitle')
                          : availableUpdateLabel}
                      />
                    ) : null}
                  </button>
                );
              })}
              {pinnedPages.length > 0 && <>
                <div className="nav-plugin-heading">{pluginText('pinnedPlugins', locale)}</div>
                {pinnedPages.map(entry => {
                  const id: PageId = `plugin:${pluginPinKey(resourcePin(entry))}`;
                  return <button key={id} type="button" className={active === id ? 'active' : ''}
                    aria-current={active === id ? 'page' : undefined} title={`${entry.pluginTitle}: ${entry.description}`}
                    onClick={() => select(id)}>
                    <PluginResourceIcon entry={entry} /><span>{entry.label}</span>
                  </button>;
                })}
              </>}
            </nav>

            <div className="sidebar-bottom">
              <div className="sidebar-links">
                <button
                  type="button"
                  className="sidebar-link sidebar-easy-entry"
                  onClick={() => select('easy')}
                >
                  <Sprout size={16} strokeWidth={1.75} aria-hidden="true" />
                  <span>{t('app.nav.easy')}</span>
                </button>
                <button
                  type="button"
                  className="sidebar-link sidebar-contact"
                  title={t('app.contact.title')}
                  onClick={() => void openContact()}
                >
                  <MessageCircle size={16} strokeWidth={1.75} aria-hidden="true" />
                  <span>{t('app.contact.label')}</span>
                  <ExternalLink size={12} strokeWidth={1.75} aria-hidden="true" />
                </button>
              </div>

              <div className="sidebar-utility">
                <div ref={languageMenuRef} className="sidebar-language">
                  <button
                    ref={languageButtonRef}
                    type="button"
                    className="sidebar-language-trigger"
                    aria-label={t('app.language')}
                    aria-haspopup="listbox"
                    aria-expanded={languageMenuOpen}
                    aria-controls="sidebar-language-list"
                    onClick={() => { setThemeMenuOpen(false); setLanguageMenuOpen((open) => !open); }}
                    onKeyDown={(event) => {
                      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
                      event.preventDefault();
                      setThemeMenuOpen(false);
                      setLanguageMenuOpen(true);
                      window.requestAnimationFrame(() => {
                        const options = languageMenuRef.current?.querySelectorAll<HTMLButtonElement>('[role="option"]');
                        options?.[event.key === 'ArrowUp' ? options.length - 1 : 0]?.focus();
                      });
                    }}
                  >
                    <Languages size={16} strokeWidth={1.75} aria-hidden="true" />
                    <span lang={selectedLanguage.value}>{selectedLanguage.nativeLabel}</span>
                    <ChevronsUpDown size={12} strokeWidth={1.75} aria-hidden="true" />
                  </button>
                  {languageMenuOpen ? (
                    <div
                      id="sidebar-language-list"
                      className="sidebar-language-list"
                      data-theme-surface="overlay"
                      role="listbox"
                      aria-label={t('app.language')}
                      onKeyDown={handleLanguageListKeyDown}
                    >
                      {languageOptions.map((option) => {
                        const selected = option.value === locale;
                        return (
                          <button
                            key={option.value}
                            type="button"
                            className={selected ? 'selected' : ''}
                            role="option"
                            aria-selected={selected}
                            onClick={() => {
                              setLocale(option.value);
                              setLanguageMenuOpen(false);
                              window.requestAnimationFrame(() => languageButtonRef.current?.focus());
                            }}
                          >
                            <span lang={option.value}>{option.nativeLabel}</span>
                            {selected ? <Check size={14} strokeWidth={2} aria-hidden="true" /> : null}
                          </button>
                        );
                      })}
                    </div>
                  ) : null}
                </div>

                <div className="sidebar-utility-actions">
                <button
                  type="button"
                  className={`sidebar-privacy-toggle${privacyMode ? ' active' : ''}`}
                  aria-pressed={privacyMode}
                  aria-label={t('app.privacy.label')}
                  title={t(privacyMode ? 'app.privacy.disable' : 'app.privacy.enable')}
                  onClick={() => setPrivacyMode(!privacyMode)}
                >
                  {privacyMode
                    ? <EyeOff size={14} strokeWidth={1.75} aria-hidden="true" />
                    : <Eye size={14} strokeWidth={1.75} aria-hidden="true" />}
                </button>
                <ThemeSelector value={theme} onChange={setTheme} open={themeMenuOpen}
                  onOpenChange={(open) => { setThemeMenuOpen(open); if (open) setLanguageMenuOpen(false); }} />
                </div>
              </div>
            </div>
          </aside>
        ) : null}

        <div className="workspace">
          <main className="content">
            {pinnedPage ? (
              <PluginResourcePage key={pluginPinKey(resourcePin(pinnedPage))} entry={pinnedPage} onBack={() => select('plugins')} />
            ) : isAlwaysAvailablePage(activePage.id) || coreReady ? (
              activePage.id === 'easy' ? (
                <EasyModePage
                  onExit={() => select('home')}
                  theme={theme}
                  setTheme={setTheme}
                  locale={locale}
                  setLocale={setLocale}
                />
              ) : (
                <ActivePage />
              )
            ) : (
              <CoreLockedPage />
            )}
          </main>
        </div>
      </div>

      {windowsClosePrompt ? (
        <div className="close-dialog-backdrop">
          <section
            ref={closeDialogRef}
            className="close-dialog"
            role="alertdialog"
            tabIndex={-1}
            aria-modal="true"
            aria-labelledby="close-dialog-title"
            aria-describedby="close-dialog-description"
          >
            <button
              type="button"
              className="close-dialog-dismiss"
              aria-label={t('common.cancel')}
              title={t('common.cancel')}
              disabled={windowsClosePrompt.resolvingAction !== null}
              onClick={() => setWindowsClosePrompt(null)}
            >
              <X size={17} aria-hidden="true" />
            </button>
            <div className="close-dialog-heading">
              <h2 id="close-dialog-title">{t('app.close.title')}</h2>
            </div>
            <p id="close-dialog-description">
              {t('app.close.description')}
            </p>
            {windowsClosePrompt.error ? (
              <MessageNotice message={windowsClosePrompt.error} onDismiss={() => setWindowsClosePrompt(current => current ? { ...current, error: null } : current)} />
            ) : null}
            <label className="close-dialog-remember">
              <input
                type="checkbox"
                checked={windowsClosePrompt.rememberChoice}
                disabled={windowsClosePrompt.resolvingAction !== null}
                onChange={(event) => {
                  const rememberChoice = event.currentTarget.checked;
                  setWindowsClosePrompt((current) =>
                    current ? { ...current, rememberChoice } : current,
                  );
                }}
              />
              <span>{t('app.close.remember')}</span>
            </label>
            <div className="close-dialog-actions">
              <button
                type="button"
                className="close-choice-button primary-button"
                disabled={windowsClosePrompt.resolvingAction !== null}
                onClick={() => void resolveWindowsCloseRequest('minimize-to-tray')}
              >
                <span>
                  {windowsClosePrompt.resolvingAction === 'minimize-to-tray'
                    ? t('app.close.minimizing')
                    : t('app.close.minimize')}
                </span>
              </button>
              <button
                type="button"
                className="close-choice-button danger-button"
                disabled={windowsClosePrompt.resolvingAction !== null}
                onClick={() => void resolveWindowsCloseRequest('exit')}
              >
                <span>
                  {windowsClosePrompt.resolvingAction === 'exit'
                    ? t('app.close.exiting')
                    : t('app.close.exit')}
                </span>
              </button>
            </div>
          </section>
        </div>
      ) : null}

      <AppUpdateDialog />
    </>
  );
}

function CoreLockedPage() {
  const { t } = useI18n();
  return (
    <section className="page core-locked-page">
      <div className="empty-state core-locked-panel">
        <ServerCog size={26} aria-hidden="true" />
        <strong>{t('app.coreRequired.title')}</strong>
        <span>{t('app.coreRequired.description')}</span>
      </div>
    </section>
  );
}

export default App;
