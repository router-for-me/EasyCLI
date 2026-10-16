import { commitFetchedQuota } from '../services/quotaEnrichment';
import { ResetCreditExpiries } from '../components/ResetCreditExpiries';
import { MessageNotice } from '../appNotice';
import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from 'react';
import { AlertCircle, Gauge, ListRestart, LoaderCircle, RefreshCw } from 'lucide-react';
import { useConfirmation } from '../components/ConfirmationDialog';
import { OAuthPageToolbar } from '../components/OAuthPageToolbar';
import { QuotaActionFeedback } from '../components/QuotaActionFeedback';
import { canResetQuota, hasPendingClaudeReset } from '../services/quotaActions';
import { useQuotaReset } from '../components/useQuotaReset';
import antigravityIcon from '../assets/icons/antigravity.svg';
import claudeIcon from '../assets/icons/claude.svg';
import codexIcon from '../assets/icons/codex.svg';
import grokIcon from '../assets/icons/grok.svg';
import devinIcon from '../assets/icons/devin.svg';
import kimiIcon from '../assets/icons/kimi-light.svg';
import { managementApi, readBoolean, responseList } from '../services/managementApi';
import { formatQuotaReset, useQuotaClock } from '../services/quotaTime';
import {
  formatQuotaTimestamp,
  idleQuota,
  loadQuota,
  providerForFile,
  quotaKey,
  type AuthFile,
  type QuotaProvider,
  type QuotaState,
} from '../services/quotaService';
import {
  captureQuotaCacheGeneration,
  getQuotaCacheSnapshot,
  pruneQuotaCache,
  updateQuotaCache,
  useQuotaCache,
} from '../services/quotaCache';
import { dedupeAuthFiles, parseAuthFilePriority, sortAuthFilesByPriority } from '../services/authFiles';
import { useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';
import './QuotaPage.css';

const providerMeta: Record<QuotaProvider, { label: string; icon: string }> = {
  claude: { label: 'Claude', icon: claudeIcon },
  codex: { label: 'Codex', icon: codexIcon },
  kimi: { label: 'Kimi', icon: kimiIcon },
  xai: { label: 'xAI', icon: grokIcon },
  devin: { label: 'Devin', icon: devinIcon },
  antigravity: { label: 'Antigravity', icon: antigravityIcon },
};

const providerOrder: QuotaProvider[] = ['claude', 'antigravity', 'codex', 'xai', 'kimi', 'devin'];
const REFRESH_CONCURRENCY = 4;

export function QuotaPage() {
  const { t } = useI18n();
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const [files, setFiles] = useState<AuthFile[]>([]);
  const quotas = useQuotaCache();
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState('');
  const [category, setCategory] = useState<'all' | QuotaProvider>('all');
  const querying = Object.values(quotas).some((quota) => quota.status === 'loading');

  const loadFiles = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const payload = await managementApi.get('/credentials');
      const allFiles = sortAuthFilesByPriority(dedupeAuthFiles(responseList(payload, 'files')));
      const nextFiles = allFiles.filter((file) => !readBoolean(file, 'disabled') && providerForFile(file));
      setFiles(nextFiles);
      const validQuotaKeys = new Set(allFiles.map(quotaKey));
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
      setError(String(requestError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadFiles();
  }, [loadFiles]);

  const refreshOne = useCallback(async (file: AuthFile) => {
    const key = quotaKey(file);
    if (getQuotaCacheSnapshot()[key]?.status === 'loading') return;
    const cacheGeneration = captureQuotaCacheGeneration();
    updateQuotaCache((current) => ({ ...current, [key]: { status: 'loading', rows: [] } }));
    const result = await loadQuota(file);
    commitFetchedQuota(cacheGeneration, file, result);
  }, [t]);

  const resetQuota = useQuotaReset(askConfirmation, setError);

  const refreshAll = useCallback(async () => {
    if (Object.values(getQuotaCacheSnapshot()).some((quota) => quota.status === 'loading')) return;
    setRefreshing(true);
    setError('');
    const cacheGeneration = captureQuotaCacheGeneration();
    updateQuotaCache((current) => ({
      ...current,
      ...Object.fromEntries(files.map((file) => [quotaKey(file), {
        ...current[quotaKey(file)], status: 'loading', rows: [],
      }])),
    }));
    try {
      for (let index = 0; index < files.length; index += REFRESH_CONCURRENCY) {
        const batch = files.slice(index, index + REFRESH_CONCURRENCY);
        await Promise.all(batch.map(async (file) => {
          const result = await loadQuota(file);
          commitFetchedQuota(cacheGeneration, file, result);
        }));
      }
    } finally {
      setRefreshing(false);
    }
  }, [files, t]);

  const grouped = useMemo(() => {
    const groups = new Map<QuotaProvider, { file: AuthFile; quota: QuotaState }[]>();
    files.forEach((file) => {
      const provider = providerForFile(file);
      if (!provider) return;
      const items = groups.get(provider) ?? [];
      items.push({ file, quota: quotas[quotaKey(file)] ?? idleQuota() });
      groups.set(provider, items);
    });
    return providerOrder.flatMap((provider) => {
      const items = groups.get(provider);
      return items ? [[provider, items] as const] : [];
    });
  }, [files, quotas]);
  const activeCategory = category !== 'all' && grouped.some(([provider]) => provider === category) ? category : 'all';
  const visibleItems = (activeCategory === 'all' ? grouped : grouped.filter(([provider]) => provider === activeCategory))
    .flatMap(([provider, items]) => items.map((item) => ({ provider, ...item })));
  const moveCategory = (event: KeyboardEvent<HTMLDivElement>) => {
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
    if (!buttons.length) return;
    const current = Math.max(0, buttons.findIndex((button) => button === document.activeElement));
    let next = current;
    if (event.key === 'ArrowRight' || event.key === 'ArrowDown') next = (current + 1) % buttons.length;
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') next = (current - 1 + buttons.length) % buttons.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = buttons.length - 1;
    else return;
    event.preventDefault();
    buttons[next]?.focus();
    buttons[next]?.click();
  };

  return (
    <section className="page management-page quota-page" aria-label={t('quota.title')}>
      {confirmationDialog}
      {error ? <MessageNotice message={error} /> : null}
      <OAuthPageToolbar
        icon={<Gauge size={18} />}
        summary={t(files.length === 1 ? 'quota.queryableCredentials.one' : 'quota.queryableCredentials.other', { count: files.length })}
      >
        <button type="button" className="secondary-button compact-button" onClick={() => void loadFiles()} disabled={loading || refreshing || querying}>
          {loading ? <RefreshCw size={16} className="spin" /> : <ListRestart size={16} />}{t('quota.readList')}
        </button>
        <button type="button" className="primary-button compact-button" onClick={() => void refreshAll()} disabled={refreshing || loading || querying || files.length === 0}>
          <RefreshCw size={16} className={refreshing ? 'spin' : ''} />{t('quota.refreshAll')}
        </button>
      </OAuthPageToolbar>
      {loading ? (
        <div className="management-loading"><LoaderCircle size={20} className="spin" />{t('quota.loadingFiles')}</div>
      ) : grouped.length === 0 ? (
        <div className="management-empty"><AlertCircle size={24} /><strong>{t('quota.empty.title')}</strong><span>{t('quota.empty.description')}</span></div>
      ) : (
        <>
          <div className="quota-category-filter" role="radiogroup" aria-label={t('quota.filter.label')} onKeyDown={moveCategory}>
            {([['all', files.length] as const, ...grouped.map(([provider, items]) => [provider, items.length] as const)]).map(([provider, count]) => {
              const selected = activeCategory === provider;
              const label = provider === 'all' ? t('quota.filter.all') : providerMeta[provider].label;
              return (
                <button key={provider} type="button" role="radio" aria-checked={selected} tabIndex={selected ? 0 : -1}
                  aria-label={`${label}, ${t(count === 1 ? 'quota.credentials.one' : 'quota.credentials.other', { count })}`}
                  onClick={() => setCategory(provider)}>
                  {provider === 'all' ? null : <img src={providerMeta[provider].icon} alt="" className={provider === 'devin' ? 'provider-logo devin-logo' : 'provider-logo'} />}
                  <span>{label}</span>
                  <span className="quota-filter-count">{count}</span>
                </button>
              );
            })}
          </div>
          <div className="real-quota-grid">{visibleItems.map(({ provider, file, quota }) => <QuotaCard key={quotaKey(file)} file={file} quota={quota} onRefresh={() => void refreshOne(file)} onReset={provider === 'codex' || provider === 'claude' ? () => void resetQuota(file, quota) : undefined} />)}</div>
        </>
      )}
    </section>
  );
}

export function QuotaCard({ file, quota, onRefresh, onReset }: { file: AuthFile; quota: QuotaState; onRefresh: () => void; onReset?: () => void }) {
  const { locale, t } = useI18n();
  const now = useQuotaClock() + (quota.serverTimeOffsetMs ?? 0);
  const provider = providerForFile(file);
  const privacy = usePrivacyMask();
  const name = privacy.credentialName(file);
  const disabled = readBoolean(file, 'disabled');
  const priority = parseAuthFilePriority(file.priority) ?? 0;
  return (
    <article className="panel real-quota-card">
      <div className="real-quota-card-header">
        <div className="quota-identity">
          <img src={provider ? providerMeta[provider].icon : ''} alt="" className="quota-account-icon" />
          <div><strong title={name}>{name}</strong><span>{provider ? providerMeta[provider].label : t('quota.unknownProvider')}</span></div>
        </div>
        <div className="quota-plan"><strong>{quota.plan || '—'}</strong><span>{quota.subscriptionActiveUntil ? t('quota.expiresAt', { time: formatQuotaTimestamp(quota.subscriptionActiveUntil, locale) }) : ''}</span></div>
        <div className="quota-status"><span className={quota.status === 'success' ? 'quota-status-badge ready' : quota.status === 'error' ? 'quota-status-badge error' : 'quota-status-badge'}>{quota.status === 'success' ? t('common.enabled') : quota.status === 'loading' ? t('quota.querying') : quota.status === 'error' ? t('common.unavailable') : t('quota.notFetched')}</span><small>{t('authFiles.settings.priority')} {priority}</small></div>
        <div className="quota-card-actions">
          <button type="button" className="icon-button quiet" onClick={onRefresh} disabled={disabled || quota.status === 'loading'} title={disabled ? t('quota.fileDisabled') : t('quota.refresh')} aria-label={disabled ? t('quota.fileDisabled') : t('quota.refresh')}><RefreshCw size={16} className={quota.status === 'loading' ? 'spin' : ''} aria-hidden="true" /></button>
        </div>
      </div>
      <QuotaActionFeedback quota={quota} name={name} />
      {quota.status === 'idle' ? <div className="quota-card-message"><span>{disabled ? t('quota.fileDisabled') : t('quota.notFetched')}</span><button type="button" className="secondary-button compact-button" onClick={onRefresh} disabled={disabled}>{disabled ? t('quota.disabled') : t('quota.fetch')}</button></div> : null}
      {quota.status === 'loading' ? <div className="quota-card-message"><LoaderCircle size={18} className="spin" />{t(quota.pendingAction === 'reset' ? 'quota.resetting' : 'quota.querying')}</div> : null}
      {quota.status === 'error' ? <>
        <div className="quota-card-error"><AlertCircle size={18} /><span>{t('authFiles.quota.failed')}{quota.error ? <small>{privacy.text(quota.error)}</small> : null}</span></div>
      </> : null}
      {quota.status === 'success' && (provider === 'codex' || provider === 'claude') ? <div className="quota-reset-credit-summary">
        {(quota.creditsUnlimited || quota.creditBalance !== undefined) ? <span>{t('quota.creditBalance')} <strong>{quota.creditsUnlimited ? t('quota.creditUnlimited') : quota.creditBalance}</strong></span> : null}
        <ResetCreditExpiries quota={quota} />
        <MessageNotice message={quota.resetCreditsError ? name + ': ' + t('quota.resetCreditsWarning', { error: quota.resetCreditsError }) : null} />
      </div> : null}
      {quota.status === 'success' ? <div className="quota-row-list">{quota.rows.map((row, index) => {
        const reset = formatQuotaReset(row.resetAtMs, row.reset, locale, now);
        return <div className="real-quota-row" key={`${row.label}-${index}`}>
          <div><span>{row.label}</span><strong>{row.quotaAvailable ? t('quota.service.antigravity.available') : row.remainingPercent === null ? '—' : t('quota.remaining', { percent: Math.round(row.remainingPercent) })}</strong></div>
          {row.remainingPercent !== null ? <div className="real-quota-track"><span style={{ width: `${Math.max(0, Math.min(100, row.remainingPercent))}%` }} /></div> : null}
          <small>{[row.detail, reset].filter(Boolean).join(' · ')}</small>
        </div>;
      })}</div> : null}
      {onReset && ((quota.resetCredits ?? 0) > 0 || hasPendingClaudeReset(file)) ? <button type="button" className="secondary-button compact-button quota-reset-footer" onClick={onReset} disabled={!canResetQuota(file, quota)} title={t('quota.reset')}>{t(hasPendingClaudeReset(file) ? 'quota.claude.retry' : 'quota.reset')}</button> : null}
    </article>
  );
}
