import { useMemo, useSyncExternalStore } from 'react';
import { ChevronDown, CircleHelp, Clock3, Info, LoaderCircle, PauseCircle, RotateCcw, ShieldAlert } from 'lucide-react';
import { useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';
import { authFileIdentities } from '../services/authFiles';
import {
  authFileHealth,
  cooldownReasonKey,
  normalizeAuthFileCooldowns,
  summarizeAuthFileCooldowns,
  type AuthFileCooldownSnapshot,
} from '../services/authFileHealth';
import './AuthFileHealthStatus.css';

let clockNow = Date.now();
let timer: ReturnType<typeof setInterval> | undefined;
const listeners = new Set<() => void>();
const getClockSnapshot = () => clockNow;
const subscribeClock = (listener: () => void) => {
  listeners.add(listener);
  if (!timer) {
    clockNow = Date.now();
    timer = setInterval(() => {
      clockNow = Date.now();
      listeners.forEach((notify) => notify());
    }, 1000);
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      if (timer !== undefined) clearInterval(timer);
      timer = undefined;
    }
  };
};

type HealthProps = {
  file: Record<string, unknown>;
  snapshot?: AuthFileCooldownSnapshot;
  resetting?: boolean;
  resetDisabled?: boolean;
  onReset?: () => void;
  compact?: boolean;
};

export function AuthFileHealthStatus({ file, receivedAtMs, observedAt, ...resetProps }: Omit<HealthProps, 'snapshot'> & {
  receivedAtMs: number;
  observedAt?: string;
}) {
  const snapshot = useMemo(
    () => normalizeAuthFileCooldowns(file.cooldowns, receivedAtMs, observedAt),
    [file, receivedAtMs, observedAt],
  );
  return snapshot?.records?.length
    ? <TimedHealthStatus file={file} snapshot={snapshot} {...resetProps} />
    : <HealthStatus file={file} snapshot={snapshot} nowMs={receivedAtMs} {...resetProps} />;
}

function TimedHealthStatus(props: HealthProps) {
  const nowMs = useSyncExternalStore(subscribeClock, getClockSnapshot, getClockSnapshot);
  return <HealthStatus {...props} nowMs={nowMs} />;
}

function HealthStatus({ file, snapshot, nowMs, resetting = false, resetDisabled = false, onReset, compact = false }: HealthProps & { nowMs: number }) {
  const { t, formatNumber, formatDate } = useI18n();
  const privacy = usePrivacyMask();
  const rawHealth = authFileHealth(file);
  // Core status messages can quote the account that the upstream rejected.
  const health = { ...rawHealth, message: rawHealth.message && privacy.text(rawHealth.message, authFileIdentities(file)) };
  const cooldown = summarizeAuthFileCooldowns(snapshot, nowMs);
  const hasTimers = cooldown.rows.length > 0;
  const healthy = health.label === 'authFiles.health.active';
  if (healthy && !hasTimers && snapshot?.records !== null) return null;
  const hasDetails = hasTimers || Boolean(health.message);
  const reasonSet = new Set(cooldown.active.map(({ record }) => cooldownReasonKey(record.reason)));
  if (reasonSet.has('authFiles.health.reason.credentialQuota')) reasonSet.delete('authFiles.health.reason.quota');
  const reasons = Array.from(reasonSet);
  const tone = health.disabled || (healthy && !hasTimers) ? 'neutral' : hasTimers ? 'warning' : health.tone;
  const separateState = hasTimers && !health.disabled && (
    health.status === 'pending' || health.status === 'refreshing'
    || health.label === 'authFiles.health.reason.invalidGrant'
    || health.label === 'authFiles.health.reason.unauthorized'
    || health.label === 'authFiles.health.reason.tokenExpired'
  );
  const Icon = health.disabled ? PauseCircle : hasTimers ? Clock3
    : tone === 'error' ? ShieldAlert
      : tone === 'warning' || tone === 'info' ? Info : CircleHelp;
  const duration = (seconds: number) => seconds < 60
    ? t('authFiles.health.seconds', { count: formatNumber(seconds) })
    : seconds < 3600 ? t('authFiles.health.minutes', { count: formatNumber(Math.ceil(seconds / 60)) })
      : t('authFiles.health.hours', { count: formatNumber(Math.ceil(seconds / 3600)) });
  const scope = cooldown.credentialWide
    ? cooldown.modelCount ? t('authFiles.health.credentialModels', { count: cooldown.modelCount })
      : t('authFiles.health.credential')
    : cooldown.modelCount ? t('authFiles.health.models', { count: cooldown.modelCount }) : '';
  const title = compact && snapshot?.records === null ? t('authFiles.health.unknownCooldown')
    : health.disabled && !compact ? t(health.label)
    : cooldown.elapsed ? t('authFiles.health.elapsed')
      : scope || (healthy ? t('authFiles.health.unknownCooldown') : t(health.label));
  const subtitle = health.disabled ? scope : reasons.map((key) => t(key)).join(' · ');
  const formatTimestamp = (value: string) => formatDate(value, {
    year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false,
  });
  const summary = <>
    <Icon size={16} className="auth-health-icon" aria-hidden="true" />
    <span className="auth-health-summary-content">
      <span className="auth-health-heading"><strong title={compact && health.message ? health.message : undefined}>{title}</strong>{!compact && subtitle ? <span>{subtitle}</span> : null}</span>
      {!compact && separateState ? <span className="auth-health-hint">{t('authFiles.health.coreStatus', { status: t(health.label) })}</span> : null}
      {!compact && health.message && !hasTimers && !health.label.startsWith('authFiles.health.reason.')
        ? <span className="auth-health-message-preview">{health.message}</span> : null}
      {cooldown.elapsed ? <span className="auth-health-hint">{t('authFiles.health.expiredHint')}</span> : null}
      {!compact && snapshot?.records === null && !healthy ? <span className="auth-health-hint">{t('authFiles.health.unknownCooldown')}</span> : null}
      {snapshot === undefined && health.tone === 'error' ? <span className="auth-health-hint">{t('authFiles.health.missingCooldown')}</span> : null}
    </span>
    {cooldown.active.length > 0 ? <span className="auth-health-countdown">
      {t('authFiles.health.earliest', { time: duration(cooldown.earliestSeconds) })}
    </span> : null}
    {hasDetails ? <span className="auth-health-disclosure">{!compact ? <span>{t('authFiles.health.details')}</span> : null}<ChevronDown size={14} aria-hidden="true" /></span> : null}
  </>;

  return (
    <section className={`auth-file-health ${tone}${hasDetails ? ' detailed' : ''}${compact ? ' auth-file-health-compact' : ''}`} aria-label={t('authFiles.health.title')}>
      {hasDetails ? <details>
        <summary className="auth-health-summary">{summary}</summary>
        <div className="auth-health-body">
          {hasTimers && !healthy && (!compact || separateState || health.disabled) ? <p className="auth-health-core-state">{t('authFiles.health.coreStatus', { status: t(health.label) })}</p> : null}
          {health.message && (!compact || !health.label.startsWith('authFiles.health.reason.')) ? <div className="auth-health-core-message">
            <span>{t('authFiles.health.coreMessage')}</span><p>{health.message}</p>
          </div> : null}
          <ul className="auth-health-records">
            {cooldown.rows.map(({ record, remainingSeconds }, index) => (
              <li key={`${record.scope}:${record.model ?? ''}:${index}`}>
                <div className="auth-health-record-heading">
                  <strong>{record.scope === 'credential' ? t('authFiles.health.credentialScope') : record.model}</strong>
                  <span>{remainingSeconds > 0 ? t('authFiles.health.remaining', { time: duration(remainingSeconds) }) : t('authFiles.health.waiting')}</span>
                </div>
                <div className="auth-health-record-meta">
                  <span>{t(cooldownReasonKey(record.reason))}</span>
                  {record.httpStatus !== undefined ? <span>HTTP {record.httpStatus}</span> : null}
                  {record.backoffLevel !== undefined ? <span title={t('authFiles.health.backoffHint')}>{t('authFiles.health.backoff', { level: record.backoffLevel })}</span> : null}
                </div>
                <div className="auth-health-hint">{t('authFiles.health.deadline')} <time dateTime={record.retryAt}>{formatTimestamp(record.retryAt)}</time></div>
              </li>
            ))}
          </ul>
          {snapshot?.observedAt ? <p className="auth-health-hint">{t('authFiles.health.observed')} <time dateTime={snapshot.observedAt}>{formatTimestamp(snapshot.observedAt)}</time></p> : null}
          {hasTimers ? <p className="auth-health-hint">{t('authFiles.health.note')}</p> : null}
        </div>
      </details> : <div className="auth-health-summary">{summary}</div>}
      {hasTimers && onReset ? <div className="auth-health-actions">
        <span className="auth-health-hint">{t('authFiles.cooldown.resetHint')}</span>
        <button type="button" className="secondary-button compact-button auth-health-reset"
          onClick={onReset} disabled={resetDisabled || resetting} aria-busy={resetting}
          title={t('authFiles.cooldown.resetHint')}>
          {resetting ? <LoaderCircle size={14} className="spin" aria-hidden="true" /> : <RotateCcw size={14} aria-hidden="true" />}
          {t(resetting ? 'authFiles.cooldown.resetting' : 'authFiles.cooldown.resetButton')}
        </button>
      </div> : null}
    </section>
  );
}
