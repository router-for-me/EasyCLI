import { ResetCreditExpiries } from './ResetCreditExpiries';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import { useI18n } from '../i18n';
import { MessageNotice } from '../appNotice';
import { usePrivacyMask } from '../privacy';
import { formatQuotaTimestamp, type AuthFile, type QuotaState } from '../services/quotaService';
import { canResetQuota, hasPendingClaudeReset } from '../services/quotaActions';
import { formatQuotaReset, useQuotaClock } from '../services/quotaTime';
import './AuthFileQuotaPanel.css';

export function AuthFileQuotaPanel({ quota, file, disabled, onRefresh, onReset, compact = false, dense = false }: {
  quota: QuotaState;
  file: AuthFile;
  disabled: boolean;
  onRefresh: () => void;
  onReset?: () => void;
  compact?: boolean;
  dense?: boolean;
}) {
  const { locale, t } = useI18n();
  const now = useQuotaClock() + (quota.serverTimeOffsetMs ?? 0);
  const loading = quota.status === 'loading';
  const privacy = usePrivacyMask();
  const name = privacy.credentialName(file);
  const error = privacy.text(quota.error ?? '');
  const compactLayout = compact || dense;
  const showReset = Boolean(onReset && ((quota.resetCredits ?? 0) > 0 || hasPendingClaudeReset(file)));
  const accountCreditsLabel = quota.creditsUnlimited
    ? t('quota.creditUnlimited')
    : quota.creditBalance !== undefined ? quota.creditBalance : '';
  const denseMetadata = dense ? [
    quota.subscriptionActiveUntil ? t('quota.subscriptionExpiry', { time: formatQuotaTimestamp(quota.subscriptionActiveUntil, locale) }) : '',
    accountCreditsLabel ? t('quota.creditBalance') + ': ' + accountCreditsLabel : '',
  ].filter(Boolean).join(' · ') : '';
  const renderRow = (row: QuotaState['rows'][number], index: number) => {
    const percent = row.remainingPercent !== null && Number.isFinite(row.remainingPercent) ? Math.max(0, Math.min(100, row.remainingPercent)) : null;
    const reset = formatQuotaReset(row.resetAtMs, row.reset, locale, now);
    const resetText = dense && row.resetAtMs !== undefined && Number.isFinite(row.resetAtMs)
      ? new Intl.DateTimeFormat(locale, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }).format(row.resetAtMs)
      : reset;
    const tone = percent === null ? 'unknown' : percent <= 10 ? 'low' : percent <= 30 ? 'medium' : 'high';
    return <div className={`credential-quota-row ${tone}`} key={`${row.label}-${index}`}>
      <div className="credential-quota-label"><span title={row.label}>{row.label}</span><strong>{percent === null ? '—' : `${Math.round(percent)}%`}</strong></div>
      <div className="credential-quota-track" role={percent === null ? undefined : 'progressbar'} aria-label={`${row.label} · ${t('authFiles.settings.quotaRemaining')}`}
        aria-valuemin={percent === null ? undefined : 0} aria-valuemax={percent === null ? undefined : 100} aria-valuenow={percent ?? undefined}>
        {percent === null ? <span className="sr-only">{t('authFiles.settings.quotaUnknown')}</span> : <i style={{ width: `${percent}%` }} />}
      </div>
      {reset ? <small className={dense ? 'credential-quota-reset-time' : undefined} title={dense ? reset : undefined}>{resetText}</small> : null}
      {row.detail ? <small className={dense ? 'credential-quota-row-detail' : undefined} title={dense ? row.detail : undefined}>{row.detail}</small> : null}
    </div>;
  };
  return (
    <section className={`credential-quota${compactLayout ? ' credential-quota-compact' : ''}${dense ? ' credential-quota-dense' : ''}`} aria-label={t('authFiles.quota.aria')} aria-busy={loading} title={denseMetadata || undefined}>
      {!compactLayout || showReset ? <div className="credential-quota-heading">
        {!compactLayout ? <strong>{t('authFiles.settings.quotaRemaining')}</strong> : null}
        {!compactLayout && quota.plan ? <span className="credential-quota-plan">{quota.plan}</span> : null}
        {showReset ? <button type="button" className="secondary-button compact-button credential-quota-reset" onClick={onReset} disabled={disabled || !canResetQuota(file, quota)}
          title={t('quota.reset')}>{t(hasPendingClaudeReset(file) ? 'quota.claude.retry' : 'quota.reset')}</button> : null}
        {!compactLayout ? <button type="button" className="credential-quota-refresh" disabled={disabled || loading} onClick={onRefresh}
          title={disabled ? t('quota.fileDisabled') : t('authFiles.quota.refresh')}>
          {loading ? <LoaderCircle size={13} className="spin" /> : <RefreshCw size={13} />}
          {t(loading ? 'authFiles.quota.querying' : quota.status === 'idle' ? 'authFiles.quota.fetch' : 'common.refresh')}
        </button> : null}
      </div> : null}
      {loading ? <div className="credential-quota-loading" role="status"><span>{t(quota.pendingAction === 'reset' ? 'quota.resetting' : 'authFiles.quota.loading')}</span><div className="credential-quota-track indeterminate"><i /></div></div> : null}
      {quota.status === 'idle' ? <p className="credential-quota-empty">{t(file.disabled === true ? 'authFiles.status.disabled' : dense ? 'quota.notFetched' : 'authFiles.settings.quotaIdle')}</p> : null}
      {quota.status === 'error' ? <div className="credential-quota-error" role="status">
        {dense && error ? <details className="credential-quota-error-details"><summary title={error}>{t('authFiles.quota.failed')}</summary><small>{error}</small></details>
          : <><span>{t('authFiles.quota.failed')}</span>{error ? <small>{error}</small> : null}</>}
      </div> : null}
      {quota.status === 'success' ? <>
        {quota.rows.length ? <>
          <div className="credential-quota-rows">{(compactLayout ? quota.rows.slice(0, 2) : quota.rows).map(renderRow)}</div>
          {compactLayout && quota.rows.length > 2 ? <details className="credential-quota-more">
            <summary>{t('authFiles.health.details')} (+{quota.rows.length - 2})</summary>
            <div className="credential-quota-rows">{quota.rows.slice(2).map((row, index) => renderRow(row, index + 2))}</div>
          </details> : null}
        </> : <p className="credential-quota-empty">{t('authFiles.quota.empty')}</p>}
        {!dense ? <div className="credential-quota-footnotes">
          {quota.subscriptionActiveUntil ? <small>{t('quota.subscriptionExpiry', { time: formatQuotaTimestamp(quota.subscriptionActiveUntil, locale) })}</small> : null}
          {accountCreditsLabel ? <small>{t('quota.creditBalance')}: {accountCreditsLabel}</small> : null}

        </div> : null}
        <ResetCreditExpiries quota={quota} />
        <MessageNotice message={quota.resetCreditsError ? name + ': ' + t('quota.resetCreditsWarning', { error: quota.resetCreditsError }) : null} />
      </> : null}
    </section>
  );
}
