import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  CalendarCheck,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Check,
  CircleAlert,
  CircleCheck,
  Copy,
  ExternalLink,
  FileKey,
  Info,
  LoaderCircle,
  LogIn,
  Plus,
  RefreshCw,
  Sparkles,
  UserRound,
} from 'lucide-react';
import antigravityIcon from '../assets/icons/antigravity.svg';
import claudeIcon from '../assets/icons/claude.svg';
import codexIcon from '../assets/icons/codex.svg';
import grokIcon from '../assets/icons/grok.svg';
import devinIcon from '../assets/icons/devin.svg';
import kimiIcon from '../assets/icons/kimi-light.svg';
import metaIcon from '../assets/icons/meta.svg';
import workbuddyIcon from '../assets/icons/workbuddy.png';
import { useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';
import { FloatingNotice, useAppNotice } from '../appNotice';
import { oauthSubpages, type OAuthSubpage } from '../oauthNavigation';
import {
  changedOAuthAuthFileNames,
  groupOAuthAccountsByProvider,
  snapshotAuthFiles,
  type AuthFileSnapshot,
  type OAuthAccountSummary,
} from '../services/authFiles';
import { managementApi, responseList } from '../services/managementApi';
import { createOAuthLoginSuccessCache } from '../services/oauthLoginState';
import { AuthFileManagementPage } from './AuthFileManagementPage';
import { validateDevinCallback } from '../services/devinOAuth';
import { handleHorizontalTabKey } from '../components/tabKeyboardNavigation';
import { PluginOAuthProviders } from './PluginOAuthProviders';
import { notifyPluginResourcesChanged, PLUGIN_RESOURCES_REFRESH_EVENT } from '../services/pluginResources';
import { VertexLoginCard } from './VertexLoginCard';
import { WorkBuddyTasksPage } from './WorkBuddyTasksPage';

type OAuthProviderId = 'codex' | 'claude' | 'workbuddy' | 'antigravity' | 'kimi' | 'xai' | 'devin' | 'meta';
type OAuthFlowStatus = 'idle' | 'waiting' | 'success' | 'error';

type OAuthProviderState = {
  url?: string;
  userCode?: string;
  state?: string;
  status: OAuthFlowStatus;
  error?: string;
  polling?: boolean;
  callbackUrl?: string;
  callbackSubmitting?: boolean;
  callbackStatus?: 'success' | 'error';
  callbackError?: string;
  refreshing?: boolean;
};

type OAuthStartResult = {
  url: string;
  state?: string | null;
  userCode?: string | null;
  flow?: string | null;
  expiresIn?: number | null;
  opened: boolean;
  openError?: string | null;
};

type WorkBuddyOAuthStartResult = {
  state: string;
  authUrl: string;
  realm: string;
  platform: string;
};

type OAuthBrowserOption = {
  id: string;
  label: string;
};

type OAuthStatusResult = {
  status: string;
  error?: string | null;
};

const oauthProviders = [
  { id: 'codex' as const, name: 'Codex OAuth', icon: codexIcon },
  { id: 'claude' as const, name: 'Claude OAuth', icon: claudeIcon },
  { id: 'workbuddy' as const, name: 'WorkBuddy OAuth', icon: workbuddyIcon },
  { id: 'antigravity' as const, name: 'Antigravity OAuth', icon: antigravityIcon },
  { id: 'kimi' as const, name: 'Kimi OAuth', icon: kimiIcon },
  { id: 'xai' as const, name: 'xAI OAuth', icon: grokIcon },
  { id: 'devin' as const, name: 'Devin OAuth', icon: devinIcon },
  { id: 'meta' as const, name: 'Muse (Meta) OAuth', icon: metaIcon },
];

const OAUTH_CALLBACK_SUPPORTED = new Set<OAuthProviderId>([
  'codex',
  'claude',
  'antigravity',
  'xai',
  'devin',
]);
const BUILTIN_OAUTH_PROVIDER_IDS = oauthProviders.map(provider => provider.id);
const XAI_CALLBACK_URL = 'http://127.0.0.1:56121/callback';
const OAUTH_POLL_INTERVAL_MS = 3000;
const OAUTH_BROWSER_STORAGE_KEY = 'easy-cli-proxy-api.oauth-browser.v3';
const NO_AUTO_OPEN_BROWSER_ID = 'none';
const OAUTH_ACCOUNT_PREVIEW_LIMIT = 3;
const oauthLoginSuccessCache = createOAuthLoginSuccessCache<OAuthProviderId>();

const resolveOAuthBrowserSelection = (
  available: OAuthBrowserOption[],
  preferred: string,
): string => {
  if (preferred === NO_AUTO_OPEN_BROWSER_ID) return preferred;
  if (available.some((browser) => browser.id === preferred)) return preferred;
  return available.find((browser) => browser.id !== 'default')?.id
    ?? available.find((browser) => browser.id === 'default')?.id
    ?? NO_AUTO_OPEN_BROWSER_ID;
};

const loadOAuthBrowserPreference = (): string => {
  try {
    return window.localStorage.getItem(OAUTH_BROWSER_STORAGE_KEY)?.trim() || '';
  } catch {
    return '';
  }
};

const cachedOAuthProviderStates = (): Partial<Record<OAuthProviderId, OAuthProviderState>> => (
  oauthLoginSuccessCache.snapshot().reduce<Partial<Record<OAuthProviderId, OAuthProviderState>>>(
    (states, provider) => ({ ...states, [provider]: { status: 'success' } }),
    {},
  )
);

export function OAuthManagementPage() {
  const { t } = useI18n();
  const [activeSubpage, setActiveSubpage] = useState<OAuthSubpage>('login');
  const [authFilesProvider, setAuthFilesProvider] = useState<string | undefined>();
  const openAuthFiles = useCallback((provider?: string) => {
    setAuthFilesProvider(provider);
    setActiveSubpage('authFiles');
  }, []);
  const subpageIds = oauthSubpages.map((subpage) => subpage.id);
  const handleTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, subpage: OAuthSubpage) => {
    handleHorizontalTabKey(
      event,
      subpageIds,
      subpage,
      setActiveSubpage,
      (next) => document.getElementById(`oauth-subpage-tab-${next}`),
    );
  };

  return (
    <section className="page oauth-management-page">
      <div
        className="agent-subpage-tabs oauth-subpage-tabs"
        role="tablist"
        aria-label={t('oauth.tabs.label')}
      >
        {oauthSubpages.map((subpage) => {
          const active = activeSubpage === subpage.id;
          return (
            <button
              type="button"
              id={`oauth-subpage-tab-${subpage.id}`}
              key={subpage.id}
              role="tab"
              className={active ? 'active' : ''}
              aria-selected={active}
              aria-controls="oauth-subpage-panel"
              tabIndex={active ? 0 : -1}
              onClick={() => {
                setAuthFilesProvider(undefined);
                setActiveSubpage(subpage.id);
              }}
              onKeyDown={(event) => handleTabKeyDown(event, subpage.id)}
            >
              {t(subpage.labelKey)}
            </button>
          );
        })}
      </div>

      <div
        className="oauth-subpage-panel"
        id="oauth-subpage-panel"
        role="tabpanel"
        aria-labelledby={`oauth-subpage-tab-${activeSubpage}`}
      >
        {activeSubpage === 'login' ? (
          <OAuthLoginPage onOpenAuthFiles={openAuthFiles} />
        ) : null}
        {activeSubpage === 'authFiles' ? (
          <AuthFileManagementPage key={authFilesProvider ?? 'all'} initialProvider={authFilesProvider} />
        ) : null}
      </div>
    </section>
  );
}

export function OAuthLoginPage({ onOpenAuthFiles }: { onOpenAuthFiles?: (provider?: string) => void } = {}) {
  const [workbuddyTasksExpanded, setWorkbuddyTasksExpanded] = useState(false);
  const { t } = useI18n();
  const privacy = usePrivacyMask();
  const [accounts, setAccounts] = useState<Map<string, OAuthAccountSummary[]> | null>(null);
  const [states, setStates] = useState<Partial<Record<OAuthProviderId, OAuthProviderState>>>(
    cachedOAuthProviderStates,
  );
  const feedback = useAppNotice();
  const { showNotice } = feedback;
  const [browsers, setBrowsers] = useState<OAuthBrowserOption[]>([]);
  const [browsersLoading, setBrowsersLoading] = useState(true);
  const [selectedBrowser, setSelectedBrowser] = useState(loadOAuthBrowserPreference);
  const [workbuddyRealm, setWorkbuddyRealm] = useState<'intl' | 'cn'>('intl');
  const workbuddyRealmRef = useRef(workbuddyRealm);
  workbuddyRealmRef.current = workbuddyRealm;
  const [quickTask, setQuickTask] = useState<string | null>(null);
  const pollingTimers = useRef<Partial<Record<OAuthProviderId, number>>>({});
  const pollingRequests = useRef<Partial<Record<OAuthProviderId, boolean>>>({});
  const pollingSessions = useRef<Partial<Record<OAuthProviderId, string>>>({});
  const credentialSnapshots = useRef<Partial<Record<OAuthProviderId, AuthFileSnapshot>>>({});

  useEffect(() => {
    let active = true;
    void invoke<OAuthBrowserOption[]>('list_oauth_browsers')
      .then((available) => {
        if (!active) return;
        setBrowsers(available);
        setSelectedBrowser((current) => resolveOAuthBrowserSelection(available, current));
      })
      .catch((error) => {
        if (!active) return;
        console.warn('Failed to detect installed browsers', error);
        const fallback = [{ id: 'default', label: 'System Default' }];
        setBrowsers(fallback);
        setSelectedBrowser((current) => resolveOAuthBrowserSelection(fallback, current));
      })
      .finally(() => {
        if (active) setBrowsersLoading(false);
      });
    return () => { active = false; };
  }, []);

  const loadAccounts = useCallback(async () => {
    try {
      const payload = await managementApi.get('/credentials');
      setAccounts(groupOAuthAccountsByProvider(responseList(payload, 'files')));
    } catch (error) {
      // Counts are informational; keep the login cards usable while the core is unavailable.
      console.warn('Failed to load saved OAuth accounts', error);
      setAccounts(null);
    }
  }, []);

  useEffect(() => {
    void loadAccounts();
    const refresh = () => void loadAccounts();
    window.addEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refresh);
    return () => window.removeEventListener(PLUGIN_RESOURCES_REFRESH_EVENT, refresh);
  }, [loadAccounts]);

  useEffect(() => {
    try {
      window.localStorage.setItem(OAUTH_BROWSER_STORAGE_KEY, selectedBrowser);
    } catch {
    }
  }, [selectedBrowser]);

  const updateProviderState = useCallback(
    (provider: OAuthProviderId, next: Partial<OAuthProviderState>) => {
      setStates((current) => ({
        ...current,
        [provider]: {
          status: 'idle',
          ...(current[provider] ?? {}),
          ...next,
        },
      }));
    },
    [],
  );

  const clearPollingTimer = useCallback((provider: OAuthProviderId) => {
    const timer = pollingTimers.current[provider];
    if (timer !== undefined) window.clearInterval(timer);
    delete pollingSessions.current[provider];
    delete pollingTimers.current[provider];
    delete pollingRequests.current[provider];
  }, []);

  const completeProviderAuth = useCallback(
    (provider: OAuthProviderId) => {
      clearPollingTimer(provider);
      oauthLoginSuccessCache.mark(provider);
      updateProviderState(provider, {
        url: undefined,
        state: undefined,
        userCode: undefined,
        status: 'success',
        error: undefined,
        polling: false,
        callbackUrl: '',
        callbackSubmitting: false,
        callbackStatus: undefined,
        callbackError: undefined,
      });
    },
    [clearPollingTimer, updateProviderState],
  );

  const captureCredentialSnapshot = useCallback(async (provider: OAuthProviderId) => {
    const payload = await managementApi.get('/credentials');
    credentialSnapshots.current[provider] = snapshotAuthFiles(responseList(payload, 'files'));
  }, []);

  const applyDefaultCredentialPriority = useCallback(async (provider: OAuthProviderId) => {
    const before = credentialSnapshots.current[provider];
    delete credentialSnapshots.current[provider];
    if (!before) return;

    const payload = await managementApi.get('/credentials');
    const names = changedOAuthAuthFileNames(before, responseList(payload, 'files'), provider);
    await Promise.all(names.map((name) => managementApi.patch('/credentials/fields', {
      name,
      priority: 0,
    })));
  }, []);

  const startPolling = useCallback(
    (provider: OAuthProviderId, state: string, targetRealm?: 'intl' | 'cn') => {
      clearPollingTimer(provider);
      pollingSessions.current[provider] = state;
      const isCurrent = () => pollingSessions.current[provider] === state;
      if (provider === 'workbuddy') {
        const realmToPoll = targetRealm || workbuddyRealmRef.current;
        const checkStatus = async () => {
          if (pollingRequests.current[provider]) return;
          pollingRequests.current[provider] = true;
          try {
            const result = await invoke<{ status: string; message?: string; uid?: string }>(
              'poll_workbuddy_oauth',
              { state, realm: realmToPoll },
            );
            if (!isCurrent()) return;
            if (result.status === 'success') {
              completeProviderAuth(provider);
              void loadAccounts();
              showNotice(t('oauth.loginSuccess', { provider: providerLabel(provider) }), 'success');
            } else if (result.status === 'error' || result.status === 'expired') {
              updateProviderState(provider, {
                status: 'error',
                error: result.message || t('oauth.authFailed'),
                polling: false,
              });
              clearPollingTimer(provider);
              showNotice(
                { key: 'oauth.loginFailed', variables: {
                  provider: providerLabel(provider),
                  detail: result.message ? `: ${result.message}` : '',
                } },
                'error',
              );
            }
          } catch (error) {
            if (!isCurrent()) return;
            updateProviderState(provider, {
              status: 'error',
              error: String(error),
              polling: false,
            });
            clearPollingTimer(provider);
            showNotice(String(error), 'error');
          } finally {
            if (isCurrent()) delete pollingRequests.current[provider];
          }
        };
        pollingTimers.current[provider] = window.setInterval(
          () => void checkStatus(),
          OAUTH_POLL_INTERVAL_MS,
        );
        void checkStatus();
        return;
      }
      const checkStatus = async () => {
        if (pollingRequests.current[provider]) return;
        pollingRequests.current[provider] = true;
        try {
          const result = await invoke<OAuthStatusResult>('get_oauth_status', { state });
          if (!isCurrent()) return;
          const status = (result.status || '').toLowerCase();
          if (status === 'ok') {
            let priorityError = '';
            try {
              await applyDefaultCredentialPriority(provider);
            } catch (error) {
              priorityError = String(error);
            }
            if (!isCurrent()) return;
            completeProviderAuth(provider);
            void loadAccounts();
            showNotice(
              priorityError
                ? t('oauth.priorityApplyFailed', { error: priorityError })
                : t('oauth.loginSuccess', { provider: providerLabel(provider) }),
              priorityError ? 'error' : 'success',
            );
          } else if (status === 'error') {
            updateProviderState(provider, {
              status: 'error',
              error: result.error || t('oauth.authFailed'),
              polling: false,
            });
            clearPollingTimer(provider);
            showNotice(
              { key: 'oauth.loginFailed', variables: {
                provider: providerLabel(provider),
                detail: result.error ? `: ${result.error}` : '',
              } },
              'error',
            );
          }
        } catch (error) {
          if (!isCurrent()) return;
          updateProviderState(provider, {
            status: 'error',
            error: String(error),
            polling: false,
          });
          clearPollingTimer(provider);
          showNotice(String(error), 'error');
        } finally {
          if (isCurrent()) delete pollingRequests.current[provider];
        }
      };
      pollingTimers.current[provider] = window.setInterval(
        () => void checkStatus(),
        OAUTH_POLL_INTERVAL_MS,
      );
    },
    [applyDefaultCredentialPriority, clearPollingTimer, completeProviderAuth, loadAccounts, showNotice, t, updateProviderState],
  );

  useEffect(() => {
    return () => {
      pollingSessions.current = {};
      Object.values(pollingTimers.current).forEach((timer) => {
        if (timer !== undefined) window.clearInterval(timer);
      });
    };
  }, []);

  const startLogin = async (provider: OAuthProviderId) => {
    clearPollingTimer(provider);
    updateProviderState(provider, {
      url: undefined,
      state: undefined,
      userCode: undefined,
      status: 'waiting',
      polling: true,
      error: undefined,
      callbackUrl: '',
      callbackStatus: undefined,
      callbackError: undefined,
    });

    if (provider === 'workbuddy') {
      try {
        const result = await invoke<WorkBuddyOAuthStartResult>('start_workbuddy_oauth', {
          realm: workbuddyRealm,
          platform: 'CLI',
        });
        updateProviderState(provider, {
          url: result.authUrl,
          state: result.state,
          status: 'waiting',
          polling: true,
        });
        const targetRealm = (result.realm as 'intl' | 'cn') || workbuddyRealm;
        startPolling(provider, result.state, targetRealm);
      } catch (error) {
        updateProviderState(provider, {
          status: 'error',
          error: String(error),
          polling: false,
        });
        showNotice(String(error), 'error');
      }
      return;
    }

    try {
      await captureCredentialSnapshot(provider);
      const result = await invoke<OAuthStartResult>('start_oauth_login', {
        provider,
        browser: selectedBrowser,
      });
      if (!result.state) {
        updateProviderState(provider, {
          url: result.url,
          state: undefined,
          status: 'error',
          error: t('oauth.missingState'),
          polling: false,
        });
        showNotice({ key: 'oauth.missingStatePolling' }, 'error');
        return;
      }

      updateProviderState(provider, {
        url: result.url,
        state: result.state,
        userCode: result.userCode ?? undefined,
        status: 'waiting',
        polling: true,
      });
      startPolling(provider, result.state);

      if (!result.opened) {
        showNotice(
          result.openError
            ? t('oauth.openFailedDetail', { error: result.openError })
            : t('oauth.openFailed'),
          'info',
        );
      }
    } catch (error) {
      delete credentialSnapshots.current[provider];
      updateProviderState(provider, {
        status: 'error',
        error: String(error),
        polling: false,
      });
      showNotice(String(error), 'error');
    }
  };

  const runWorkBuddyQuickTask = async (kind: 'checkin' | 'all', label: string) => {
    setQuickTask(kind);
    try {
      const payload = await invoke<Record<string, unknown>>(
        kind === 'checkin' ? 'run_workbuddy_checkin' : 'run_workbuddy_all_tasks',
        { uid: null },
      );
      let detail = '';
      if (typeof payload.msg === 'string' && payload.msg.trim()) {
        detail = `：${payload.msg}`;
      } else if (Array.isArray(payload.results) && payload.results.length > 0) {
        detail = `：${(payload.results as Record<string, unknown>[])
          .map((r) => r.msg || r.error || (r.ok ? '成功' : '失败'))
          .join('；')}`;
      }
      showNotice(`WorkBuddy ${label}${detail || '已执行完成'}`, 'success');
      console.info('WorkBuddy task result', payload);
    } catch (error) {
      showNotice(`WorkBuddy ${label}失败：${error}`, 'error');
    } finally {
      setQuickTask(null);
    }
  };

  const refreshLoginLink = async (provider: OAuthProviderId) => {
    const currentState = states[provider]?.state;
    clearPollingTimer(provider);
    updateProviderState(provider, { refreshing: true });
    try {
      if (currentState) {
        await managementApi.delete('/oauth/session', { query: { state: currentState } });
      }
    } catch (error) {
      if (provider === 'devin') {
        updateProviderState(provider, { refreshing: false });
        if (currentState) startPolling(provider, currentState);
        showNotice(String(error), 'error');
        return;
      }
      console.warn('Failed to cancel the previous OAuth session before refreshing', error);
    }
    try {
      await startLogin(provider);
    } finally {
      updateProviderState(provider, { refreshing: false });
    }
  };

  const openAuthUrl = async (url?: string) => {
    if (!url) return;
    try {
      await invoke('open_oauth_url', {
        url,
        browser: selectedBrowser === NO_AUTO_OPEN_BROWSER_ID ? 'default' : selectedBrowser,
      });
    } catch (error) {
      showNotice(String(error), 'error');
    }
  };

  const copyAuthUrl = async (url?: string) => {
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      showNotice({ key: 'oauth.linkCopied' }, 'success');
    } catch {
      showNotice({ key: 'oauth.linkCopyFailed' }, 'error');
    }
  };

  const copyDeviceCode = async (code?: string) => {
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
      showNotice({ key: 'oauth.deviceCodeCopied' }, 'success');
    } catch {
      showNotice({ key: 'oauth.deviceCodeCopyFailed' }, 'error');
    }
  };

  const submitCallback = async (provider: OAuthProviderId) => {
    const current = states[provider];
    const callbackInput = (current?.callbackUrl || '').trim();
    if (!callbackInput) {
      showNotice(
        provider === 'xai' ? t('oauth.pasteXaiCallback') : t('oauth.pasteCallback'),
        'error',
      );
      return;
    }

    if (provider === 'devin') {
      const error = validateDevinCallback(callbackInput, current?.state);
      if (error) {
        showNotice(t(error === 'state_mismatch' ? 'oauth.devinStateMismatch' : 'oauth.invalidCallback'), 'error');
        return;
      }
    }
    const redirectUrl = resolveCallbackUrl(provider, callbackInput, current?.state);
    if (!redirectUrl) {
      showNotice(
        provider === 'xai'
          ? t('oauth.invalidXaiCallback')
          : t('oauth.invalidCallback'),
        'error',
      );
      return;
    }

    updateProviderState(provider, {
      callbackSubmitting: true,
      callbackStatus: undefined,
      callbackError: undefined,
    });
    try {
      await invoke('submit_oauth_callback', { provider, redirectUrl });
      updateProviderState(provider, {
        callbackSubmitting: false,
        callbackStatus: 'success',
      });
      showNotice({ key: 'oauth.callbackSubmittedNotice' }, 'success');
    } catch (error) {
      updateProviderState(provider, {
        callbackSubmitting: false,
        callbackStatus: 'error',
        callbackError: String(error),
      });
      showNotice(String(error), 'error');
    }
  };

  return (
    <section className="page management-page oauth-login-page">
      <header className="management-header">
        <label className="oauth-browser-picker">
          <span>{t('oauth.browser.label')}</span>
          <select
            value={selectedBrowser}
            onChange={(event) => setSelectedBrowser(event.currentTarget.value)}
            aria-label={t('oauth.browser.label')}
            disabled={browsersLoading}
          >
            {browsersLoading ? <option value="">{t('oauth.browser.detecting')}</option> : null}
            {browsers.map((browser) => (
              <option value={browser.id} key={browser.id}>
                {browser.id === 'default' ? t('oauth.browser.systemDefault') : browser.label}
              </option>
            ))}
            <option value={NO_AUTO_OPEN_BROWSER_ID}>{t('oauth.browser.noAutoOpen')}</option>
          </select>
        </label>
        <button type="button" className="secondary-button" onClick={notifyPluginResourcesChanged}>
          <RefreshCw size={16} aria-hidden="true" />{t('common.refresh')}
        </button>
      </header>

      <FloatingNotice key={feedback.revision} notice={feedback.notice} onDismiss={feedback.clearNotice} />
      <aside className="oauth-intro" aria-label={t('oauth.intro.title')}>
        <span className="oauth-intro-icon" aria-hidden="true"><Info size={18} /></span>
        <div className="oauth-intro-text">
          <strong>{t('oauth.intro.title')}</strong>
          <p>{t('oauth.intro.body')}</p>
        </div>
        {onOpenAuthFiles ? (
          <button type="button" className="secondary-button compact-button oauth-intro-action" onClick={() => onOpenAuthFiles()}>
            <FileKey size={15} aria-hidden="true" />
            {t('oauth.intro.viewFiles')}
          </button>
        ) : null}
      </aside>
      <div className="oauth-grid">
        {oauthProviders.map((provider) => {
          const state = states[provider.id] ?? { status: 'idle' as const };
          const canSubmitCallback = OAUTH_CALLBACK_SUPPORTED.has(provider.id) && Boolean(state.url);
          const providerAccounts = accounts?.get(provider.id) ?? [];
          const hasAccounts = providerAccounts.length > 0;
          const loginLabel = state.polling
            ? t('oauth.waitingAuthorization')
            : hasAccounts || state.status === 'success'
              ? t('oauth.addAccount')
              : t('oauth.signIn');
          const LoginIcon = hasAccounts || state.status === 'success' ? Plus : LogIn;

          return (
            <section className={`panel oauth-card${provider.id === 'workbuddy' && workbuddyTasksExpanded ? ' is-expanded' : ''}${state.polling ? ' is-waiting' : ''}`} key={provider.id}>
              <div className="provider-title-row">
                <img src={provider.icon} alt="" className={provider.id === 'devin' ? 'provider-logo devin-logo' : 'provider-logo'} />
                <div>
                  <h2>{provider.name}</h2>
                  {accounts === null ? null : hasAccounts ? (
                    <button
                      type="button"
                      className="oauth-account-count has-accounts"
                      onClick={() => onOpenAuthFiles?.(provider.id)}
                      disabled={!onOpenAuthFiles}
                      title={t('oauth.accounts.view', { provider: provider.name })}
                    >
                      <UserRound size={13} aria-hidden="true" />
                      {providerAccounts.length === 1
                        ? t('oauth.accounts.one')
                        : t('oauth.accounts.count', { count: providerAccounts.length })}
                      {onOpenAuthFiles ? <ChevronRight size={13} aria-hidden="true" /> : null}
                    </button>
                  ) : (
                    <span className="oauth-account-count">{t('oauth.accounts.none')}</span>
                  )}
                </div>
              </div>

              <div className="oauth-card-body">
                {hasAccounts ? (
                  <ul className="oauth-account-list" aria-label={t('oauth.accounts.listLabel', { provider: provider.name })}>
                    {providerAccounts.slice(0, OAUTH_ACCOUNT_PREVIEW_LIMIT).map((account) => {
                      const label = privacy.text(account.label, account.identities);
                      return (
                        <li key={account.name} className={account.disabled ? 'disabled' : undefined} title={account.disabled ? `${label} · ${t('oauth.accounts.disabled')}` : label}>
                          {label}
                        </li>
                      );
                    })}
                    {providerAccounts.length > OAUTH_ACCOUNT_PREVIEW_LIMIT ? (
                      <li className="more">{t('oauth.accounts.more', { count: providerAccounts.length - OAUTH_ACCOUNT_PREVIEW_LIMIT })}</li>
                    ) : null}
                  </ul>
                ) : null}
                {state.polling ? (
                  <div className="oauth-flow-status waiting" role="status">
                    <LoaderCircle size={15} className="spin" aria-hidden="true" />
                    <span>{t('oauth.flow.waiting')}</span>
                  </div>
                ) : state.status === 'success' ? (
                  <div className="oauth-flow-status success" role="status">
                    <CircleCheck size={15} aria-hidden="true" />
                    <span>{t('oauth.flow.success')}</span>
                  </div>
                ) : state.status === 'error' ? (
                  <div className="oauth-flow-status error" role="alert">
                    <CircleAlert size={15} aria-hidden="true" />
                    <span>{t('oauth.flow.error', { error: privacy.text(state.error || t('oauth.authFailed')) })}</span>
                  </div>
                ) : null}
                {provider.id === 'workbuddy' ? (
                  <div style={{ marginTop: 8, marginBottom: 8 }}>
                    <label style={{ fontSize: '0.8125rem', display: 'flex', flexDirection: 'column', gap: 4 }}>
                      <span style={{ color: 'var(--text-secondary)' }}>服务区域 (Realm)</span>
                      <select
                        value={workbuddyRealm}
                        disabled={state.polling}
                        onChange={(e) => setWorkbuddyRealm(e.target.value as 'intl' | 'cn')}
                        style={{ padding: '4px 8px', borderRadius: 6, border: '1px solid var(--border-color)', background: 'var(--bg-input)', fontSize: '0.8125rem' }}
                      >
                        <option value="intl">国际版 (www.workbuddy.ai)</option>
                        <option value="cn">国内版 (copilot.tencent.com)</option>
                      </select>
                    </label>
                  </div>
                ) : null}
                <p className="oauth-hint">{t(provider.id === 'devin' ? 'oauth.devinHint' : provider.id === 'meta' ? 'oauth.metaHint' : 'oauth.hint')}</p>
                {state.url ? (
                  <div className="oauth-auth-url-box">
                    <div className="oauth-auth-url-label">{t('oauth.authorizationLink')}</div>
                    <div className="oauth-auth-url-value" title={state.url}>{state.url}</div>
                    <div className="oauth-auth-url-actions">
                      <button type="button" className="secondary-button compact-button" onClick={() => void copyAuthUrl(state.url)}>
                        <Copy size={15} aria-hidden="true" />{t('oauth.copyLink')}
                      </button>
                      <button type="button" className="secondary-button compact-button" onClick={() => void openAuthUrl(state.url)}>
                        <ExternalLink size={15} aria-hidden="true" />{t('oauth.openLink')}
                      </button>
                    </div>
                    {state.userCode ? (
                      <div className="oauth-device-code-box">
                        <div className="oauth-auth-url-label">{t('oauth.deviceCodeLabel')}</div>
                        <div className="oauth-device-code-value">{state.userCode}</div>
                        <button type="button" className="secondary-button compact-button" onClick={() => void copyDeviceCode(state.userCode)}>
                          <Copy size={15} aria-hidden="true" />{t('oauth.copyDeviceCode')}
                        </button>
                      </div>
                    ) : null}
                  </div>
                ) : null}

                {canSubmitCallback ? (
                  <div className="oauth-callback-block">
                    <div className="oauth-callback-row">
                      <input
                        value={state.callbackUrl ?? ''}
                        onChange={(event) => {
                          const value = event.currentTarget.value;
                          updateProviderState(provider.id, {
                            callbackUrl: value,
                            callbackStatus: undefined,
                            callbackError: undefined,
                          });
                        }}
                        placeholder={provider.id === 'xai' ? t('oauth.xaiCallbackPlaceholder') : t('oauth.callbackPlaceholder')}
                      />
                      <button type="button" className="secondary-button" disabled={state.callbackSubmitting} onClick={() => void submitCallback(provider.id)}>
                        {state.callbackSubmitting ? <LoaderCircle size={16} className="spin" aria-hidden="true" /> : <Check size={16} aria-hidden="true" />}
                        {t('oauth.submitCallback')}
                      </button>
                    </div>
                  </div>
                ) : null}
              </div>

              <div className="button-row management-card-actions">
                <button type="button" className="primary-button" disabled={Boolean(state.polling) || browsersLoading} onClick={() => void startLogin(provider.id)}>
                  {state.polling ? <LoaderCircle size={16} className="spin" aria-hidden="true" /> : <LoginIcon size={16} aria-hidden="true" />}
                  {loginLabel}
                </button>
                {state.url ? (
                  <button type="button" className="secondary-button" disabled={browsersLoading || state.refreshing} onClick={() => void refreshLoginLink(provider.id)}>
                    <RefreshCw size={16} className={state.refreshing ? 'spin' : undefined} aria-hidden="true" />
                    {state.refreshing ? t('oauth.refreshingLink') : t('oauth.refreshLink')}
                  </button>
                ) : null}
              </div>

              {provider.id === 'workbuddy' ? (
                <>
                  <div className="button-row management-card-actions">
                    <button
                      type="button"
                      className="secondary-button"
                      disabled={providerAccounts.length === 0}
                      title={providerAccounts.length === 0 ? '请先完成 WorkBuddy 登录' : '今日签到'}
                      onClick={() => void runWorkBuddyQuickTask('checkin', '每日签到')}
                    >
                      {quickTask === 'checkin' ? (
                        <LoaderCircle size={16} className="spin" aria-hidden="true" />
                      ) : (
                        <CalendarCheck size={16} aria-hidden="true" />
                      )}
                      每日签到
                    </button>
                    <button
                      type="button"
                      className="secondary-button"
                      disabled={providerAccounts.length === 0}
                      title={providerAccounts.length === 0 ? '请先完成 WorkBuddy 登录' : '一键完成全部任务'}
                      onClick={() => void runWorkBuddyQuickTask('all', '全部任务')}
                    >
                      {quickTask === 'all' ? (
                        <LoaderCircle size={16} className="spin" aria-hidden="true" />
                      ) : (
                        <Sparkles size={16} aria-hidden="true" />
                      )}
                      一键完成
                    </button>
                    <button
                      type="button"
                      className={`secondary-button ${workbuddyTasksExpanded ? 'active' : ''}`}
                      onClick={() => setWorkbuddyTasksExpanded((prev) => !prev)}
                      aria-expanded={workbuddyTasksExpanded}
                    >
                      {workbuddyTasksExpanded ? (
                        <ChevronUp size={16} aria-hidden="true" />
                      ) : (
                        <ChevronDown size={16} aria-hidden="true" />
                      )}
                      {workbuddyTasksExpanded ? '收起任务中心' : '任务中心'}
                    </button>
                  </div>

                  {workbuddyTasksExpanded ? (
                    <WorkBuddyTasksPage embedded />
                  ) : null}
                </>
              ) : null}
            </section>
          );
        })}
        <VertexLoginCard />
        <PluginOAuthProviders
          builtInProviderIds={[...BUILTIN_OAUTH_PROVIDER_IDS, 'vertex']}
          browser={selectedBrowser === NO_AUTO_OPEN_BROWSER_ID ? 'default' : selectedBrowser || 'default'}
        />
      </div>
    </section>
  );
}

function providerLabel(provider: OAuthProviderId) {
  return oauthProviders.find((item) => item.id === provider)?.name ?? provider;
}

function isAbsoluteUrl(value: string) {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
}

function readQueryLikeCallbackInput(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const queryStart = trimmed.indexOf('?');
  const hashStart = trimmed.indexOf('#');
  const rawParams = queryStart >= 0
    ? trimmed.slice(queryStart + 1)
    : hashStart >= 0
      ? trimmed.slice(hashStart + 1)
      : trimmed;
  if (!/(^|[&#?])(code|state|error)=/i.test(rawParams)) return null;
  return new URLSearchParams(rawParams.replace(/^[?#]/, ''));
}

function buildXaiCallbackUrl(input: string, state?: string) {
  const trimmed = input.trim();
  if (!trimmed) return null;
  if (isAbsoluteUrl(trimmed)) return trimmed;
  const params = readQueryLikeCallbackInput(trimmed);
  if (params) {
    const callbackState = params.get('state')?.trim() || state?.trim();
    if (!callbackState) return null;
    const callbackUrl = new URL(XAI_CALLBACK_URL);
    callbackUrl.searchParams.set('state', callbackState);
    for (const key of ['code', 'error', 'error_description']) {
      const value = params.get(key)?.trim();
      if (value) callbackUrl.searchParams.set(key, value);
    }
    return callbackUrl.toString();
  }
  const code = (trimmed.match(/\bcode\s*[:=]\s*([^\s&]+)/i)?.[1] ?? trimmed).trim();
  const callbackState = state?.trim();
  if (!code || !callbackState) return null;
  const callbackUrl = new URL(XAI_CALLBACK_URL);
  callbackUrl.searchParams.set('code', code);
  callbackUrl.searchParams.set('state', callbackState);
  return callbackUrl.toString();
}

function resolveCallbackUrl(provider: OAuthProviderId, input: string, state?: string) {
  return provider === 'xai' ? buildXaiCallbackUrl(input, state) : input.trim();
}
