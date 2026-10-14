import { commitFetchedQuota } from '../services/quotaEnrichment';
import { ChangeEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useConfirmation } from '../components/ConfirmationDialog';
import { QuotaActionFeedback } from '../components/QuotaActionFeedback';
import { AuthFileQuotaPanel } from '../components/AuthFileQuotaPanel';
import { AuthFileSettingsDialog } from '../components/AuthFileSettingsDialog';
import { useQuotaReset } from '../components/useQuotaReset';
import './AuthFileManagementPage.css';
import { MessageNotice, FloatingNotice, useAppNotice } from '../appNotice';
import { AuthFileModelsDialog } from '../components/AuthFileModelsDialog';
import { AuthFileRequestStatus } from '../components/AuthFileRequestStatus';
import { AuthFileHealthStatus } from '../components/AuthFileHealthStatus';
import { AuthFileUsageSummary } from '../components/AuthFileUsageSummary';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import {
  Check,
  ChevronLeft,
  ChevronRight,
  FileDown,
  FolderOpen,
  Import,
  LoaderCircle,
  RefreshCw,
  Search,
  Settings2,
  X,
} from 'lucide-react';
import antigravityIcon from '../assets/icons/antigravity.svg';
import claudeIcon from '../assets/icons/claude.svg';
import codexIcon from '../assets/icons/codex.svg';
import geminiIcon from '../assets/icons/gemini.svg';
import grokIcon from '../assets/icons/grok.svg';
import devinIcon from '../assets/icons/devin.svg';
import kimiIcon from '../assets/icons/kimi-light.svg';
import metaIcon from '../assets/icons/meta.svg';
import vertexIcon from '../assets/icons/vertex.svg';
import workbuddyIcon from '../assets/icons/workbuddy.png';
import {
  formatDate,
  managementApi,
  readBoolean,
  readNumber,
  readString,
  responseList,
} from '../services/managementApi';
import {
  idleQuota,
  loadQuota,
  providerForFile as quotaProviderForFile,
  quotaKey,
} from '../services/quotaService';
import {
  captureQuotaCacheGeneration,
  getQuotaCacheSnapshot,
  pruneQuotaCache,
  updateQuotaCache,
  useQuotaCache,
} from '../services/quotaCache';
import {
  authFileName,
  authFileIdentities,
  authFileCooldownResetIndex,
  dedupeAuthFiles,
  isOAuthCredentialFile,
  isRuntimeOnlyAuthFile,
  oauthModelProvidersFromAuthFiles,
  normalizeOAuthProvider,
  parseAuthFilePriority,
  resetAuthFileCooldown,
  setOAuthCredentialFileDisabled,
  sortAuthFilesByPriority,
} from '../services/authFiles';
import { authFileHealth, normalizeAuthFileCooldowns } from '../services/authFileHealth';
import { codexMetadataFor } from '../services/quotaMetadata';
import { quotaResetInstant, useQuotaClock } from '../services/quotaTime';
import {
  modelMatchesRule,
  normalizeOAuthExcludedRules,
  openOAuthModelNames,
  setOAuthModelsExcluded,
  type OAuthModelDefinition,
} from '../services/oauthModels';
import {
  loadOAuthModelSettings,
  saveOAuthModelSettings,
  type OAuthModelSettings,
  type OAuthModelTarget,
} from '../services/oauthModelSettings';
import { getCurrentLocale, translate, useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';

type AuthFile = Record<string, unknown>;

const providerIcons: Record<string, string> = {
  antigravity: antigravityIcon,
  claude: claudeIcon,
  codex: codexIcon,
  gemini: geminiIcon,
  kimi: kimiIcon,
  vertex: vertexIcon,
  xai: grokIcon,
  devin: devinIcon,
  meta: metaIcon,
  workbuddy: workbuddyIcon,
};

const providerName = (file: AuthFile) => {
  const value = normalizeOAuthProvider(readString(file, 'provider', 'type', 'account_type'));
  if (value === 'workbuddy') return 'WorkBuddy';
  if (value === 'claude') return 'Claude';
  if (value === 'antigravity') return 'Antigravity';
  if (value === 'xai') return 'xAI';
  if (value === 'devin') return 'Devin';
  if (value === 'meta') return 'Muse (Meta)';
  return value ? value.charAt(0).toUpperCase() + value.slice(1) : translate(getCurrentLocale(), 'authFiles.unknownProvider');
};

const providerKey = (file: AuthFile) => {
  return normalizeOAuthProvider(readString(file, 'provider', 'type', 'account_type'));
};

const fileName = authFileName;

const isRuntimeOnly = isRuntimeOnlyAuthFile;

export function AuthFileManagementPage({ initialProvider }: { initialProvider?: string } = {}) {
  const { t } = useI18n();
  const privacy = usePrivacyMask();
  const now = useQuotaClock();
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const [fileSnapshot, setFileSnapshot] = useState<{
    files: AuthFile[];
    receivedAtMs: number;
    observedAt?: string;
  }>({ files: [], receivedAtMs: 0 });
  const { files, receivedAtMs, observedAt } = fileSnapshot;
  const [filter, setFilter] = useState('');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(10);
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(() => new Set());
  const [providerFilter, setProviderFilter] = useState(() => (
    initialProvider ? providerName({ provider: initialProvider }) : 'all'
  ));
  const [statusFilter, setStatusFilter] = useState<'all' | 'enabled' | 'disabled' | 'runtime'>('all');
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [quotaRefreshing, setQuotaRefreshing] = useState(false);
  const [cooldownResetting, setCooldownResetting] = useState<Set<string>>(() => new Set());
  const [error, setError] = useState('');
  const resetQuota = useQuotaReset(askConfirmation, setError);
  const feedback = useAppNotice();
  const { showNotice } = feedback;
  const [copied, setCopied] = useState('');
  const [settingsName, setSettingsName] = useState<string | null>(null);
  const [oauthModelTarget, setOauthModelTarget] = useState<OAuthModelTarget | null>(null);
  const [oauthModelSettings, setOauthModelSettings] = useState<OAuthModelSettings | null>(null);
  const [oauthExcludedRulesText, setOauthExcludedRulesText] = useState('');
  const [modelViewName, setModelViewName] = useState<string | null>(null);
  // Display-only names; requests, filters and copy keep the raw file name.
  const shownName = privacy.credentialName;
  const shownNameFor = (name: string) =>
    privacy.text(name, authFileIdentities(files.find((file) => fileName(file) === name) ?? {}));
  const [oauthModelSearch, setOauthModelSearch] = useState('');
  const [oauthModelLoading, setOauthModelLoading] = useState(false);
  const [oauthModelSaving, setOauthModelSaving] = useState(false);
  const [oauthModelError, setOauthModelError] = useState('');
  const quotas = useQuotaCache();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const mountedRef = useRef(true);
  const fileRequestRef = useRef(0);
  const cooldownResetPendingRef = useRef(new Set<string>());
  const oauthModelRequestRef = useRef(0);
  const oauthModelSaveRef = useRef(false);
  const oauthModels = oauthModelSettings?.models ?? [];
  const oauthExcludedRules = normalizeOAuthExcludedRules(oauthExcludedRulesText.split(/\r?\n/));
  const excludedOauthModelCount = oauthModels.length - openOAuthModelNames(oauthModels, oauthExcludedRules).size;
  const oauthModelProviders = useMemo(() => oauthModelProvidersFromAuthFiles(files)
    .map((provider) => ({ provider, label: providerName({ provider }) }))
    .sort((a, b) => a.label.localeCompare(b.label)), [files]);

  const loadFiles = useCallback(async (showLoading = true) => {
    const requestId = ++fileRequestRef.current;
    if (showLoading) setLoading(true);
    setError('');
    try {
      const payload = await managementApi.get('/credentials');
      if (!mountedRef.current || requestId !== fileRequestRef.current) return;
      const nextFiles = sortAuthFilesByPriority(dedupeAuthFiles(responseList(payload, 'files')));
      setFileSnapshot({ files: nextFiles, receivedAtMs: Date.now(), observedAt: readString(payload, 'observed_at') });
      const validQuotaKeys = new Set(nextFiles.map(quotaKey));
      pruneQuotaCache(validQuotaKeys);
      updateQuotaCache((current) => {
        const next = { ...current };
        nextFiles.forEach((file) => {
          const key = quotaKey(file);
          if (!next[key]) next[key] = idleQuota();
        });
        return next;
      });
    } catch (requestError) {
      if (mountedRef.current && requestId === fileRequestRef.current) setError(String(requestError));
    } finally {
      if (mountedRef.current && requestId === fileRequestRef.current) setLoading(false);
    }
  }, []);

  const resetCooldown = async (file: AuthFile) => {
    const authIndex = authFileCooldownResetIndex(file);
    if (!authIndex || busy || loading || cooldownResetPendingRef.current.has(authIndex)
      || !normalizeAuthFileCooldowns(file.cooldowns, receivedAtMs, observedAt)?.records?.length) return;
    const name = shownName(file);
    cooldownResetPendingRef.current.add(authIndex);
    try {
      const confirmed = await askConfirmation({
        title: t('authFiles.cooldown.resetTitle'),
        message: t('authFiles.cooldown.resetConfirm', { name }),
        confirmText: t('authFiles.cooldown.resetButton'),
      });
      if (!confirmed || !mountedRef.current) return;
      setCooldownResetting((current) => new Set(current).add(authIndex));
      feedback.clearNotice();
      setError('');
      await resetAuthFileCooldown(authIndex);
      if (!mountedRef.current) return;
      // Discard list requests started before the reset. Keep other cards' timer anchors intact.
      fileRequestRef.current += 1;
      setFileSnapshot((current) => ({
        ...current,
        files: current.files.map((item) => authFileCooldownResetIndex(item) === authIndex
          ? { ...item, cooldowns: [] } : item),
      }));
      showNotice({ key: 'authFiles.cooldown.resetSuccess', variables: { name } });
      await loadFiles(false);
    } catch (requestError) {
      if (mountedRef.current) setError(t('authFiles.cooldown.resetFailed', {
        name, message: requestError instanceof Error ? requestError.message : String(requestError),
      }));
    } finally {
      cooldownResetPendingRef.current.delete(authIndex);
      if (mountedRef.current) setCooldownResetting((current) => {
        const next = new Set(current);
        next.delete(authIndex);
        return next;
      });
    }
  };

  const refreshQuota = async (file: AuthFile) => {
    if (readBoolean(file, 'disabled')) return;
    const key = quotaKey(file);
    if (getQuotaCacheSnapshot()[key]?.status === 'loading') return;
    const cacheGeneration = captureQuotaCacheGeneration();
    updateQuotaCache((current) => ({ ...current, [key]: { status: 'loading', rows: [] } }));
    const result = await loadQuota(file);
    commitFetchedQuota(cacheGeneration, file, result);
  };

  const refreshAllQuotas = async () => {
    if (quotaRefreshing || busy || loading) return;
    const queryable = files.filter((file) => !readBoolean(file, 'disabled') && quotaProviderForFile(file));
    if (!queryable.length) return;
    setQuotaRefreshing(true);
    setError('');
    try {
      for (let index = 0; index < queryable.length; index += 4) {
        await Promise.all(queryable.slice(index, index + 4).map((file) => refreshQuota(file)));
      }
    } finally {
      if (mountedRef.current) setQuotaRefreshing(false);
    }
  };

  const closeOauthModels = () => {
    if (oauthModelSaveRef.current) return;
    oauthModelRequestRef.current += 1;
    setOauthModelTarget(null);
    setOauthModelSettings(null);
  };
  const oauthModelsDialogRef = useDialogFocusTrap<HTMLElement>({
    active: Boolean(oauthModelTarget),
    onEscape: oauthModelSaving ? undefined : closeOauthModels,
    preventEscape: oauthModelSaving,
  });

  const openOauthModelSettings = async (target: OAuthModelTarget) => {
    if (oauthModelSaveRef.current) return;
    const requestId = oauthModelRequestRef.current + 1;
    oauthModelRequestRef.current = requestId;
    setOauthModelTarget(target);
    setOauthModelSettings(null);
    setOauthExcludedRulesText('');
    setOauthModelSearch('');
    setOauthModelError('');
    setOauthModelLoading(true);
    try {
      const settings = await loadOAuthModelSettings(target);
      if (oauthModelRequestRef.current !== requestId) return;
      setOauthModelSettings(settings);
      setOauthExcludedRulesText(settings.excludedRules.join('\n'));
    } catch (requestError) {
      if (oauthModelRequestRef.current === requestId) setOauthModelError(String(requestError));
    } finally {
      if (oauthModelRequestRef.current === requestId) setOauthModelLoading(false);
    }
  };

  const saveOauthModels = async () => {
    if (!oauthModelSettings || oauthModelLoading || oauthModelSaveRef.current) return;
    const settings = oauthModelSettings;
    oauthModelSaveRef.current = true;
    setOauthModelSaving(true);
    setOauthModelError('');
    try {
      await saveOAuthModelSettings(settings, oauthExcludedRules);
      showNotice(settings.target.scope === 'credential'
        ? { key: 'authFiles.models.credentialUpdated', variables: { name: shownNameFor(settings.target.name) } }
        : { key: 'authFiles.models.updated', variables: { provider: settings.target.label } });
      oauthModelSaveRef.current = false;
      closeOauthModels();
      if (settings.target.scope === 'credential') void loadFiles(false);
    } catch (requestError) {
      setOauthModelError(String(requestError));
    } finally {
      oauthModelSaveRef.current = false;
      setOauthModelSaving(false);
    }
  };

  const visibleOauthModels = useMemo(() => {
    const query = oauthModelSearch.trim().toLowerCase();
    if (!query) return oauthModels;
    return oauthModels.filter((model) =>
      `${model.id} ${model.displayName ?? ''}`.toLowerCase().includes(query),
    );
  }, [oauthModelSearch, oauthModels]);

  const setOauthModelsExcluded = (models: OAuthModelDefinition[], excluded: boolean) => {
    if (oauthModelSaveRef.current) return;
    setOauthExcludedRulesText((current) =>
      setOAuthModelsExcluded(current.split(/\r?\n/), models, excluded).join('\n'),
    );
  };

  useEffect(() => {
    mountedRef.current = true;
    void loadFiles();
    return () => {
      mountedRef.current = false;
      fileRequestRef.current += 1;
    };
  }, [loadFiles]);

  const providers = useMemo(
    () => Array.from(new Set(files.map(providerName))).sort((left, right) => left.localeCompare(right)),
    [files],
  );

  const visibleFiles = useMemo(() => {
    const query = filter.trim().toLowerCase();
    return files.filter((file) => {
      const providerMatch = providerFilter === 'all' || providerName(file) === providerFilter;
      const disabled = readBoolean(file, 'disabled');
      const runtimeMatch =
        statusFilter === 'all' ||
        (statusFilter === 'disabled' && disabled) ||
        (statusFilter === 'enabled' && !disabled) ||
        (statusFilter === 'runtime' && isRuntimeOnly(file));
      const searchMatch =
        !query ||
        [fileName(file), providerName(file), readString(file, 'email', 'account', 'label')]
          .join(' ')
          .toLowerCase()
          .includes(query);
      return providerMatch && runtimeMatch && searchMatch;
    });
  }, [files, filter, providerFilter, statusFilter]);

  const pageCount = Math.max(1, Math.ceil(visibleFiles.length / pageSize));
  const currentPage = Math.min(page, pageCount);
  const pageFiles = visibleFiles.slice((currentPage - 1) * pageSize, currentPage * pageSize);
  useEffect(() => setPage(1), [filter, providerFilter, statusFilter, pageSize]);
  useEffect(() => setPage((current) => Math.min(current, pageCount)), [pageCount]);

  const toggleDetails = (key: string) => setExpandedFiles((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });

  const handleUpload = async (event: ChangeEvent<HTMLInputElement>) => {
    const selected = Array.from(event.currentTarget.files ?? []);
    event.currentTarget.value = '';
    if (selected.length === 0) return;
    setBusy(true);
    setError('');
    let uploaded = 0;
    const failures: string[] = [];
    for (const file of selected) {
      try {
        await managementApi.uploadAuthFile(file);
        uploaded += 1;
      } catch (requestError) {
        failures.push(`${file.name}：${String(requestError)}`);
      }
    }
    try {
      await loadFiles();
      if (uploaded > 0) showNotice({ key: 'authFiles.uploaded', variables: { count: uploaded } });
      if (failures.length > 0) setError(t('authFiles.uploadFailed', { count: failures.length, errors: failures.join('; ') }));
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setBusy(false);
    }
  };

  const toggleStatus = async (file: AuthFile) => {
    feedback.clearNotice();
    setBusy(true);
    setError('');
    try {
      await setOAuthCredentialFileDisabled(file, !readBoolean(file, 'disabled'));
      await loadFiles(false);
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setBusy(false);
    }
  };

  const deleteFile = async (file: AuthFile) => {
    const name = fileName(file);
    if (isRuntimeOnly(file)) {
      setError(t('authFiles.runtimeDeleteError'));
      return;
    }
    if (!await askConfirmation({ title: t('common.delete'), message: t('authFiles.deleteConfirm', { name: shownName(file) }), confirmText: t('common.delete'), variant: 'danger' })) return;
    setBusy(true);
    setError('');
    try {
      await managementApi.delete('/credentials', { query: { name } });
      showNotice({ key: 'authFiles.deleted' });
      await loadFiles();
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setBusy(false);
    }
  };

  const openAuthFilesDirectory = async () => {
    setBusy(true);
    setError('');
    try {
      await managementApi.openAuthFilesDirectory();
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setBusy(false);
    }
  };

  const copyName = async (name: string) => {
    try {
      await navigator.clipboard.writeText(name);
      setCopied(name);
      window.setTimeout(() => setCopied((current) => (current === name ? '' : current)), 1500);
    } catch {
      setError(t('common.copyFailed'));
    }
  };

  const disabledCount = files.filter((file) => readBoolean(file, 'disabled')).length;
  const runtimeCount = files.filter(isRuntimeOnly).length;

  return (
    <section className="page management-page auth-files-page">
      {confirmationDialog}
      <header className="auth-files-heading">
        <div><h2>{t('authFiles.title')}</h2><p>{t('authFiles.summary', { files: files.length, disabled: disabledCount })}</p></div>
        <div className="auth-files-heading-actions">
        <button type="button" className="secondary-button compact-button auth-toolbar-quiet" disabled={loading || busy || oauthModelSaving || oauthModelProviders.length === 0} onClick={() => {
          const provider = oauthModelProviders.find((item) => item.label === providerFilter) ?? oauthModelProviders[0];
          if (provider) void openOauthModelSettings({ ...provider, scope: 'provider' });
        }}><Settings2 size={15} />{t('authFiles.models.toolbarButton')}</button>
        <button type="button" className="secondary-button compact-button auth-toolbar-quiet" disabled={busy} onClick={() => void openAuthFilesDirectory()}><FolderOpen size={15} />{t('authFiles.openDirectory')}</button>
        <button type="button" className="secondary-button compact-button" onClick={() => void loadFiles()} disabled={loading || busy}>
          <RefreshCw size={16} className={loading ? 'spin' : ''} />{t('authFiles.toolbar.refreshList')}
        </button>
        <button type="button" className="secondary-button compact-button auth-toolbar-quiet" onClick={() => void refreshAllQuotas()} disabled={loading || busy || quotaRefreshing || !files.some((file) => !readBoolean(file, 'disabled') && quotaProviderForFile(file))}>
          <RefreshCw size={16} className={quotaRefreshing ? 'spin' : ''} />{t('authFiles.toolbar.refreshQuota')}
        </button>
        <button type="button" className="primary-button compact-button" onClick={() => fileInputRef.current?.click()} disabled={busy}>
          <Import size={16} />{t('authFiles.import')}
        </button>
        <input ref={fileInputRef} type="file" accept=".json,application/json" multiple hidden onChange={(event) => void handleUpload(event)} />
        </div>
      </header>

      {error ? <MessageNotice message={error} onDismiss={() => setError('')} /> : null}
      <FloatingNotice key={feedback.revision} notice={feedback.notice} onDismiss={feedback.clearNotice} />

      <section className="panel auth-files-panel real-auth-files-panel">
        <div className="auth-files-toolbar">
          <div className="auth-files-search"><Search size={16} aria-hidden="true" />
          <input value={filter} onChange={(event) => setFilter(event.currentTarget.value)} placeholder={t('authFiles.searchPlaceholder')} aria-label={t('authFiles.searchPlaceholder')} />
          </div>
          <div className="auth-files-filters">
          <select value={providerFilter} onChange={(event) => setProviderFilter(event.currentTarget.value)} aria-label={t('authFiles.filter.allProviders')}>
            <option value="all">{t('authFiles.filter.allProviders')}</option>
            {providers.map((provider) => <option key={provider} value={provider}>{provider}</option>)}
          </select>
          <select value={statusFilter} onChange={(event) => setStatusFilter(event.currentTarget.value as typeof statusFilter)} aria-label={t('authFiles.filter.allStatuses')}>
            <option value="all">{t('authFiles.filter.allStatuses')}</option>
            <option value="enabled">{t('authFiles.filter.enabled')}</option>
            <option value="disabled">{t('authFiles.filter.disabled')}</option>
            <option value="runtime">{t('authFiles.filter.runtime')}</option>
          </select>
          </div>
        </div>

        {loading ? (
          <div className="management-loading"><LoaderCircle size={20} className="spin" />{t('authFiles.loading')}</div>
        ) : visibleFiles.length === 0 ? (
          <div className="management-empty"><FileDown size={24} /><strong>{files.length ? t('authFiles.empty.filtered') : t('authFiles.empty.none')}</strong><span>{files.length ? t('authFiles.empty.tryFilter') : t('authFiles.empty.upload')}</span></div>
        ) : (
          <div className="auth-file-table-scroll">
          <div className="auth-file-list-head" aria-hidden="true">
            <span>{t('authFiles.list.credential')}</span>
            <span>{t('authFiles.list.plan')}</span><span>{t('authFiles.list.status')}</span>
            <span>{t('authFiles.list.recent')}</span><span title={t('authFiles.requests.totalsHint')}>{t('authFiles.usage.title')}</span>
            <span>{t('authFiles.list.quota')}</span>
            <span>{t('authFiles.list.actions')}</span>
          </div>
          <div className="auth-file-card-grid" role="list" aria-label={t('authFiles.title')}>
            {pageFiles.map((file) => {
              const name = fileName(file);
              const key = quotaKey(file);
              const icon = providerIcons[providerKey(file)] ?? geminiIcon;
              const disabled = readBoolean(file, 'disabled');
              const priority = parseAuthFilePriority(file.priority) ?? 0;
              const identities = authFileIdentities(file);
              const identity = privacy.text(readString(file, 'email', 'project_id', 'label'), identities);
              const displayName = privacy.text(name, identities);
              const note = privacy.text(readString(file, 'note'), identities);
              const quota = quotas[quotaKey(file)] ?? idleQuota();
              const quotaProvider = quotaProviderForFile(file);
              const metadata = providerKey(file) === 'codex' ? codexMetadataFor(file) : undefined;
              const plan = quota.plan || metadata?.plan || readString(file, 'plan_type', 'plan');
              const expiry = quota.subscriptionActiveUntil || metadata?.subscriptionActiveUntil;
              const expiryMs = quotaResetInstant(expiry);
              const health = authFileHealth(file);
              const cooldownSnapshot = normalizeAuthFileCooldowns(file.cooldowns, receivedAtMs, observedAt);
              const hasCooldown = Boolean(cooldownSnapshot?.records?.length);
              const showCooldown = hasCooldown || cooldownSnapshot?.records === null;
              const statusLabel = hasCooldown && !health.disabled ? t('authFiles.list.cooling') : t(health.label === 'authFiles.health.active' ? 'authFiles.list.available' : health.label);
              const expanded = expandedFiles.has(key);
              const detailsId = `auth-file-details-${encodeURIComponent(key)}`;
              const cooldownResetIndex = authFileCooldownResetIndex(file);
              const resettingCooldown = Boolean(cooldownResetIndex && cooldownResetting.has(cooldownResetIndex));
              return (
                <article className={`auth-file-card ${disabled ? 'is-disabled' : ''}`} key={key} role="listitem" aria-label={identity || displayName}>
                  <div className="auth-credential-row">
                  <header className={`auth-card-header auth-list-cell ${identity ? '' : 'filename-only'}`}>
                    <img src={icon} alt="" data-provider={providerKey(file)} className={providerKey(file) === 'devin' ? 'provider-logo devin-logo' : 'provider-logo'} />
                    <div className="auth-card-identity">
                      <strong title={identity || displayName}>{identity || displayName}</strong>
                      <span className="auth-card-filename" title={displayName}>{identity ? displayName : providerName(file)}</span>
                      {isRuntimeOnly(file) ? <span className="auth-card-provider">{t('authFiles.runtime')}</span> : null}
                    </div>
                  </header>
                  <div className="auth-list-cell auth-list-plan" data-label={t('authFiles.list.plan')}>
                    {plan ? <span title={plan}>{plan}</span> : null}
                    {expiryMs !== undefined ? <small title={t('authFiles.list.expiry', { time: formatDate(new Date(expiryMs).toISOString()) })}>{expiryMs <= now ? t('authFiles.list.expired') : t('authFiles.list.daysRemaining', { count: Math.ceil((expiryMs - now) / 86_400_000) })}</small> : null}
                  </div>
                  <div className="auth-list-cell auth-list-status" data-label={t('authFiles.list.status')}>
                    <span className={`auth-status-badge ${hasCooldown && !health.disabled ? 'warning' : health.tone}`} title={statusLabel}>{statusLabel}</span>
                    <small>{t('authFiles.priority.button', { priority })}</small>
                  </div>
                  <div className="auth-list-cell auth-list-recent" data-label={t('authFiles.list.recent')}><AuthFileRequestStatus file={file} compact /></div>
                  <div className="auth-list-cell auth-list-usage" data-label={t('authFiles.usage.title')}><AuthFileUsageSummary file={file} /></div>
                  <div className="auth-list-cell auth-list-quota" data-label={t('authFiles.list.quota')}>
                    {quotaProvider ? <AuthFileQuotaPanel compact dense quota={quota} file={file} disabled={busy || disabled} onRefresh={() => void refreshQuota(file)} onReset={quotaProvider === 'codex' || quotaProvider === 'claude' ? () => void resetQuota(file, quota) : undefined} /> : <span className="auth-list-muted">{t('authFiles.list.noQuota')}</span>}
                    <QuotaActionFeedback quota={quota} name={displayName} />
                  </div>
                  <footer className="auth-card-actions auth-list-cell" data-label={t('authFiles.list.actions')}>
                    <div className="auth-list-icon-actions">
                      <button type="button" className="auth-list-action" onClick={() => void refreshQuota(file)} disabled={busy || disabled || !quotaProvider || quota.status === 'loading'} title={t('authFiles.quota.refresh')}><RefreshCw size={14} className={quota.status === 'loading' ? 'spin' : ''} aria-hidden="true" />{t('authFiles.action.refreshQuota')}</button>
                      <button type="button" className="auth-list-action" onClick={() => setSettingsName(name)} disabled={busy || resettingCooldown || !isOAuthCredentialFile(file)} title={t(isOAuthCredentialFile(file) ? 'authFiles.settings.title' : 'authFiles.fileOnly')}>{t('authFiles.action.settings')}</button>
                      <button type="button" className="auth-list-action" onClick={() => setModelViewName(name)} disabled={busy || !providerKey(file)} title={t('authFiles.models.viewTitle')}>{t('authFiles.action.models')}</button>
                      <button type="button" className="auth-list-action muted" onClick={() => void copyName(name)} disabled={busy} title={t('authFiles.copyName')}>{copied === name ? <Check size={14} aria-hidden="true" /> : null}{t('authFiles.action.copy')}</button>
                      <button type="button" className="auth-list-action danger" onClick={() => void deleteFile(file)} disabled={busy || resettingCooldown || isRuntimeOnly(file)} title={t('common.delete')}>{t('common.delete')}</button>
                    </div>
                    <div className="auth-list-main-actions">
                      <button type="button" className={`${disabled ? 'primary-button' : 'secondary-button'} compact-button auth-card-toggle`} role="switch" aria-checked={!disabled} aria-label={`${t(disabled ? 'common.enable' : 'common.disable')} ${identity || displayName}`} onClick={() => void toggleStatus(file)} disabled={busy || resettingCooldown || !isOAuthCredentialFile(file)} title={t(isOAuthCredentialFile(file) ? disabled ? 'common.enable' : 'common.disable' : 'authFiles.fileOnly')}>{t(disabled ? 'common.enable' : 'common.disable')}</button>
                      <button type="button" className="auth-list-details-button" onClick={() => toggleDetails(key)} aria-expanded={expanded} aria-controls={detailsId}>{t(expanded ? 'authFiles.list.collapse' : 'authFiles.list.details')}</button>
                    </div>
                  </footer>
                  </div>
                  {showCooldown ? <AuthFileHealthStatus compact file={file} receivedAtMs={receivedAtMs} observedAt={observedAt}
                    resetting={resettingCooldown} resetDisabled={busy || loading}
                    onReset={cooldownResetIndex ? () => void resetCooldown(file) : undefined} /> : null}
                  <div className="auth-list-details" id={detailsId} hidden={!expanded}>
                    {!showCooldown ? <AuthFileHealthStatus file={file} receivedAtMs={receivedAtMs} observedAt={observedAt} /> : null}
                    <AuthFileRequestStatus file={file} summary />
                    <div className="auth-card-meta">
                      <strong title={displayName}>{displayName}</strong>
                      <span>{t('authFiles.priority.button', { priority })}</span>
                      {expiryMs !== undefined ? <span>{t('authFiles.list.expiry', { time: formatDate(new Date(expiryMs).toISOString()) })}</span> : null}
                      <span>{t('authFiles.list.size')} · {readNumber(file, 'size') === null ? t('authFiles.unknownSize') : `${Math.ceil((readNumber(file, 'size') ?? 0) / 1024)} KB`}</span>
                      <span>{t('authFiles.list.updated')} · {formatDate(file.modtime ?? file.updated_at ?? file.last_refresh)}</span>
                    </div>
                    {note ? <div className="auth-card-note"><span>{t('authFiles.settings.note')}</span><p>{note}</p></div> : null}
                  </div>
                </article>
              );
            })}
          </div>
          </div>
        )}
        {!loading && visibleFiles.length > 0 && files.length > 10 ? <nav className="auth-list-pagination" aria-label={t('authFiles.title')}>
          <span>{t('authFiles.list.pageSummary', { page: currentPage, pages: pageCount, start: (currentPage - 1) * pageSize + 1, end: Math.min(currentPage * pageSize, visibleFiles.length), total: visibleFiles.length })}</span>
          <label>{t('authFiles.list.pageSize')}<select value={pageSize} onChange={(event) => setPageSize(Number(event.currentTarget.value))}>
            {[10, 20, 50].map((size) => <option key={size} value={size}>{t('authFiles.list.perPage', { count: size })}</option>)}
          </select></label>
          <button type="button" className="secondary-button compact-button" disabled={currentPage === 1} onClick={() => setPage(currentPage - 1)}><ChevronLeft size={14} aria-hidden="true" />{t('authFiles.list.previous')}</button>
          <button type="button" className="secondary-button compact-button" disabled={currentPage === pageCount} onClick={() => setPage(currentPage + 1)}>{t('authFiles.list.next')}<ChevronRight size={14} aria-hidden="true" /></button>
        </nav> : null}
      </section>
      {runtimeCount > 0 ? <p className="page-footnote">{t('authFiles.runtimeFootnote', { count: runtimeCount })}</p> : null}

      {settingsName ? <AuthFileSettingsDialog key={settingsName} name={settingsName} displayName={shownNameFor(settingsName)} provider={providerKey(files.find((file) => fileName(file) === settingsName) ?? {})} onClose={() => setSettingsName(null)} onSaved={() => {
        showNotice({ key: 'authFiles.settings.updated', variables: { name: shownNameFor(settingsName) } });
        setSettingsName(null);
        void loadFiles(false);
      }} /> : null}

      {modelViewName ? <AuthFileModelsDialog name={modelViewName} displayName={shownNameFor(modelViewName)} onClose={() => setModelViewName(null)} /> : null}

      {oauthModelTarget ? (
        <div className="model-discovery-backdrop" onMouseDown={(event) => event.currentTarget === event.target && !oauthModelSaving && closeOauthModels()}>
          <section ref={oauthModelsDialogRef} className="model-discovery-dialog auth-model-dialog" role="dialog" aria-modal="true" aria-labelledby="oauth-model-title">
            <div className="model-discovery-header">
              <div>
                <h2 id="oauth-model-title">{t(oauthModelTarget.scope === 'credential' ? 'authFiles.models.title' : 'authFiles.models.globalButton')}</h2>
                {oauthModelTarget.scope === 'credential' ? (
                  <span className="auth-model-target" title={shownNameFor(oauthModelTarget.name)}>{shownNameFor(oauthModelTarget.name)}</span>
                ) : (
                  <label className="auth-model-provider">
                    <span>{t('authFiles.models.provider')}</span>
                    <select value={oauthModelTarget.provider} disabled={oauthModelSaving} onChange={(event) => {
                      const provider = oauthModelProviders.find((item) => item.provider === event.currentTarget.value);
                      if (provider) void openOauthModelSettings({ ...provider, scope: 'provider' });
                    }}>
                      {oauthModelProviders.map((provider) => <option key={provider.provider} value={provider.provider}>{provider.label}</option>)}
                    </select>
                  </label>
                )}
                <span>{t(oauthModelTarget.scope === 'credential' ? 'authFiles.models.description' : 'authFiles.models.globalDescription', { provider: oauthModelTarget.label })}</span>
              </div>
              <button type="button" className="icon-button quiet" onClick={closeOauthModels} disabled={oauthModelSaving} title={t('common.close')} aria-label={t('common.close')}><X size={18} aria-hidden="true" /></button>
            </div>

            <div className="model-discovery-search">
              <Search size={16} aria-hidden="true" />
              <input autoFocus value={oauthModelSearch} onChange={(event) => setOauthModelSearch(event.currentTarget.value)} placeholder={t('authFiles.models.search')} />
            </div>

            <div className="model-discovery-toolbar">
              <span>{t('authFiles.models.summary', { total: oauthModels.length, excluded: excludedOauthModelCount })}</span>
              <div>
                <button type="button" className="secondary-button compact-button" onClick={() => setOauthModelsExcluded(oauthModels, true)} disabled={oauthModelLoading || oauthModelSaving || oauthModels.length === 0} title={t('authFiles.models.excludeAllHint')}>{t('authFiles.models.excludeAll')}</button>
                <button type="button" className="secondary-button compact-button" onClick={() => setOauthModelsExcluded(oauthModels, false)} disabled={oauthModelLoading || oauthModelSaving || oauthModels.length === 0} title={t('authFiles.models.clearHint')}>{t('authFiles.models.clearSelected')}</button>
              </div>
            </div>

            <div className="model-discovery-content">
              {oauthModelLoading ? (
                <div className="model-discovery-message"><LoaderCircle size={20} className="spin" />{t('authFiles.models.loading')}</div>
              ) : oauthModelError && !oauthModelSettings ? (
                <div className="model-discovery-message error"><strong>{t('authFiles.models.loadFailed')}</strong><span>{oauthModelError}</span></div>
              ) : (
                <div className="model-discovery-results">
                  <div>
                    {oauthModelError ? <MessageNotice message={oauthModelError} onDismiss={() => setOauthModelError('')} /> : null}
                    {oauthModelSettings?.catalogError ? <MessageNotice tone="info" message={t('authFiles.models.catalogUnavailable')} /> : null}
                  </div>
                  {visibleOauthModels.length === 0 ? (
                    <div className="model-discovery-message"><strong>{oauthModels.length ? t('authFiles.models.noMatch') : t('authFiles.models.empty')}</strong></div>
                  ) : (
                    <div className="model-discovery-list">
                      {visibleOauthModels.map((model) => {
                        const wildcardRule = oauthExcludedRules.find((rule) => rule.includes('*') && modelMatchesRule(model.id, rule));
                        const checked = oauthExcludedRules.some((rule) => modelMatchesRule(model.id, rule));
                        return (
                          <label className={['model-discovery-row', checked ? 'selected' : '', wildcardRule ? 'rule-blocked' : ''].join(' ')} key={model.id}>
                            <input type="checkbox" checked={checked} disabled={oauthModelSaving || Boolean(wildcardRule)} onChange={(event) => setOauthModelsExcluded([model], event.currentTarget.checked)} />
                            <span><strong title={model.id}>{model.id}</strong>{model.displayName ? <small title={model.displayName}>{model.displayName}</small> : null}{wildcardRule ? <small>{t('authFiles.models.wildcardBlocked', { rule: wildcardRule })}</small> : null}</span>
                            {checked ? <Check size={16} aria-hidden="true" /> : null}
                          </label>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}
            </div>

            <label className="auth-model-rules" htmlFor="oauth-model-rules">
              <span>{t('authFiles.models.rulesLabel')}</span>
              <textarea id="oauth-model-rules" rows={3} spellCheck={false} value={oauthExcludedRulesText} disabled={oauthModelLoading || oauthModelSaving || !oauthModelSettings} onChange={(event) => setOauthExcludedRulesText(event.currentTarget.value)} placeholder={t('authFiles.models.rulesPlaceholder')} aria-describedby="oauth-model-rules-hint" />
              <small id="oauth-model-rules-hint">{t('authFiles.models.rulesHint')}</small>
            </label>

            <div className="model-discovery-actions">
              <button type="button" className="secondary-button" onClick={closeOauthModels} disabled={oauthModelSaving}>{t('common.cancel')}</button>
              <button type="button" className="primary-button" onClick={() => void saveOauthModels()} disabled={oauthModelLoading || oauthModelSaving || !oauthModelSettings}>{oauthModelSaving ? t('common.saving') : t('authFiles.models.save', { count: oauthExcludedRules.length })}</button>
            </div>
          </section>
        </div>
      ) : null}
    </section>
  );
}
