import { captureQuotaCacheGeneration } from './quotaCache';
import { readClaudeResetGrants, type AnthropicResetGrantStatus } from './claudeResetGrants';
import { selectResetGrant } from './selectClaudeResetGrant';
import { resetGrantOperations, RETRY_WINDOW_MS } from './claudeResetOperations';
import { buildXaiBillingSummary, mergeXaiBillingSummaries, type XaiBillingConfig } from './xaiBilling';
import {
  apiCallErrorMessage,
  isRecord,
  managementApi,
  normalizeAuthIndex,
  readString,
} from './managementApi';
import { authFileName } from './authFiles';
import { DEVIN_QUOTA_DATA, DEVIN_QUOTA_HEADERS, DEVIN_QUOTA_URL, readDevinQuota } from './devinQuota';
import { antigravityProjectFor, codexMetadataFor, isPaidXaiFile } from './quotaMetadata';
import { quotaResetFor, quotaResetInstant } from './quotaTime';
import { getCurrentLocale, translate, type AppLocale } from '../i18n';

const quotaText = (
  key: Parameters<typeof translate>[1],
  variables?: Parameters<typeof translate>[2],
) => translate(getCurrentLocale(), key, variables);

export type AuthFile = Record<string, unknown>;
export type QuotaProvider = 'claude' | 'codex' | 'kimi' | 'xai' | 'antigravity' | 'devin';
export type QuotaStatus = 'idle' | 'loading' | 'success' | 'error';
export type QuotaRow = {
  label: string;
  remainingPercent: number | null;
  reset?: string;
  resetAtMs?: number;
  detail?: string;
};
export type QuotaState = {
  status: QuotaStatus;
  rows: QuotaRow[];
  error?: string;
  plan?: string;
  creditBalance?: string;
  creditsUnlimited?: boolean;
  claudeResetGrants?: AnthropicResetGrantStatus;
  resetCredits?: number;
  resetCreditsApplicable?: number;
  resetCreditsError?: string;
  resetCreditsEarliestExpiry?: string;
  resetCreditExpiries?: string[];
  subscriptionActiveUntil?: string;
  serverTimeOffsetMs?: number;
  fetchedAt?: number;
  pendingAction?: 'reset';
  actionResult?: { action: 'reset'; status: 'success' | 'refresh-error' | 'error'; error?: string };
};

export const idleQuota = (): QuotaState => ({ status: 'idle', rows: [] });

const endpointByProvider: Record<QuotaProvider, string> = {
  devin: DEVIN_QUOTA_URL,
  claude: 'https://api.anthropic.com/api/oauth/usage',
  codex: 'https://chatgpt.com/backend-api/wham/usage',
  kimi: 'https://api.kimi.com/coding/v1/usages',
  xai: 'https://cli-chat-proxy.grok.com/v1/billing',
  antigravity: 'https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary',
};

const CLAUDE_PROFILE_URL = 'https://api.anthropic.com/api/oauth/profile';
const XAI_WEEKLY_URL = 'https://cli-chat-proxy.grok.com/v1/billing?format=credits';
const CODEX_RESET_CREDITS_URL =
  'https://chatgpt.com/backend-api/wham/rate-limit-reset-credits';
const CODEX_RESET_CREDITS_CONSUME_URL =
  'https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume';
const ANTIGRAVITY_CODE_ASSIST_URL =
  'https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist';

const headersByProvider: Record<QuotaProvider, Record<string, string>> = {
  devin: DEVIN_QUOTA_HEADERS,
  claude: {
    Authorization: 'Bearer $TOKEN$',
    'Content-Type': 'application/json',
    'anthropic-beta': 'oauth-2025-04-20',
  },
  codex: {
    Authorization: 'Bearer $TOKEN$',
    'Content-Type': 'application/json',
    'User-Agent': 'codex-tui/0.149.1 (Mac OS 26.5.2; arm64) iTerm.app/3.6.11 (codex-tui; 0.149.1)',
  },
  kimi: { Authorization: 'Bearer $TOKEN$' },
  xai: {
    Authorization: 'Bearer $TOKEN$',
    'x-xai-token-auth': 'xai-grok-cli',
    'x-grok-client-version': '0.2.91',
    accept: '*/*',
    'user-agent': 'grok-pager/0.2.91 grok-shell/0.2.91 (macos; aarch64)',
  },
  antigravity: {
    Authorization: 'Bearer $TOKEN$',
    'Content-Type': 'application/json',
    'User-Agent': 'antigravity/cli/1.0.13 (aidev_client; os_type=darwin; arch=arm64)',
  },
};

export const providerForFile = (file: AuthFile): QuotaProvider | null => {
  const value = readString(file, 'provider', 'type', 'account_type').toLowerCase().replace(/_/g, '-');
  if (value === 'x-ai' || value === 'grok') return 'xai';
  if (value === 'cognition') return 'devin';
  if (value === 'anthropic') return 'claude';
  if (value === 'anti-gravity') return 'antigravity';
  return ['claude', 'codex', 'kimi', 'xai', 'antigravity', 'devin'].includes(value)
    ? (value as QuotaProvider)
    : null;
};

export const fileName = authFileName;

export const quotaKey = (file: AuthFile) =>
  `${fileName(file)}::${normalizeAuthIndex(file.auth_index ?? file.authIndex)}`;

const parseBody = (value: unknown): unknown => {
  if (typeof value !== 'string') return value;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
};

const numberValue = (value: unknown): number | null => {
  if (isRecord(value) && 'val' in value) return numberValue(value.val);
  if (typeof value !== 'number' && typeof value !== 'string') return null;
  if (typeof value === 'string' && !value.trim()) return null;
  const parsed = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};

const clampPercent = (value: unknown): number | null => {
  const parsed = numberValue(value);
  if (parsed === null) return null;
  return Math.max(0, Math.min(100, parsed));
};

const remainingFromUsedPercent = (value: unknown): number | null => {
  const used = clampPercent(value);
  return used === null ? null : Math.max(0, Math.min(100, 100 - used));
};

const quotaFraction = (value: unknown): number | null => {
  if (typeof value === 'string' && value.trim().endsWith('%')) {
    const parsed = Number(value.trim().slice(0, -1));
    return Number.isFinite(parsed) ? Math.max(0, Math.min(1, parsed / 100)) : null;
  }
  const parsed = numberValue(value);
  return parsed === null ? null : Math.max(0, Math.min(1, parsed));
};

const formatDateTime = (date: Date): string | undefined => {
  if (Number.isNaN(date.getTime())) return undefined;
  return new Intl.DateTimeFormat(getCurrentLocale(), {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date);
};

export const formatQuotaTimestamp = (
  value: string | undefined,
  locale: AppLocale = getCurrentLocale(),
): string => {
  if (!value) return '—';
  const ms = quotaResetInstant(value);
  if (ms === undefined) return '—';
  const date = new Date(ms);
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date);
};

const absoluteResetLabel = (value: unknown): string | undefined => {
  const ms = quotaResetInstant(value);
  return ms === undefined ? undefined : formatDateTime(new Date(ms));
};

const relativeResetLabel = (value: unknown): string | undefined => {
  const seconds = numberValue(value);
  if (seconds === null || seconds <= 0) return undefined;
  const minutes = Math.max(1, Math.ceil(seconds / 60));
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor(minutes / 60);
  const remainingHours = Math.floor((minutes % 1440) / 60);
  const remainingMinutes = minutes % 60;
  if (days > 0) {
    return remainingHours > 0
      ? quotaText('quota.service.relative.daysHours', { days, hours: remainingHours })
      : quotaText('quota.service.relative.days', { days });
  }
  if (hours > 0) {
    return remainingMinutes > 0
      ? quotaText('quota.service.relative.hoursMinutes', { hours, minutes: remainingMinutes })
      : quotaText('quota.service.relative.hours', { hours });
  }
  return quotaText('quota.service.relative.minutes', { minutes });
};

const formatUsdFromCents = (value: number | null) =>
  value === null
    ? undefined
    : new Intl.NumberFormat(getCurrentLocale(), { style: 'currency', currency: 'USD' }).format(value / 100);

const codexResetLabel = (window: Record<string, unknown>): string | undefined =>
  absoluteResetLabel(window.reset_at ?? window.resetAt)
  ?? relativeResetLabel(window.reset_after_seconds ?? window.resetAfterSeconds);

const FIVE_HOUR_SECONDS = 18_000;
const WEEK_SECONDS = 604_800;
const MIN_MONTH_SECONDS = 28 * 86_400;
const MAX_MONTH_SECONDS = 31 * 86_400;

const formatWindowDuration = (seconds: number | null): string => {
  if (seconds === null || seconds <= 0) return quotaText('quota.service.duration.unknown');
  const day = 86_400;
  const hour = 3_600;
  const minute = 60;
  if (seconds % day === 0) return quotaText('quota.service.duration.days', { count: seconds / day });
  if (seconds % hour === 0) return quotaText('quota.service.duration.hours', { count: seconds / hour });
  if (seconds % minute === 0) return quotaText('quota.service.duration.minutes', { count: seconds / minute });
  return quotaText('quota.service.duration.seconds', { count: seconds });
};

const codexWindowLabel = (
  duration: number | null,
  prefix: string,
  kind: 'primary' | 'secondary',
) => {
  if (duration === FIVE_HOUR_SECONDS) return `${prefix}${quotaText('quota.service.limit.fiveHours')}`;
  if (duration === WEEK_SECONDS) return `${prefix}${quotaText('quota.service.limit.week')}`;
  if (duration !== null && duration >= MIN_MONTH_SECONDS && duration <= MAX_MONTH_SECONDS) {
    return `${prefix}${quotaText('quota.service.limit.month')}`;
  }
  if (duration !== null) {
    return `${prefix}${quotaText('quota.service.limit.duration', { duration: formatWindowDuration(duration) })}`;
  }
  if (kind === 'primary') return `${prefix}${quotaText('quota.service.limit.fiveHours')}`;
  return `${prefix}${quotaText('quota.service.limit.week')}`;
};

const codexWindowRows = (value: Record<string, unknown>): QuotaRow[] => {
  const windows: Array<{
    raw: unknown;
    kind: 'primary' | 'secondary';
    prefix: string;
    source: Record<string, unknown>;
  }> = [];
  const addRateLimit = (rawRateLimit: unknown, prefix: string) => {
    if (!isRecord(rawRateLimit)) return;
    const entries: typeof windows = [
      { raw: rawRateLimit.primary_window ?? rawRateLimit.primaryWindow, kind: 'primary', prefix, source: rawRateLimit },
      { raw: rawRateLimit.secondary_window ?? rawRateLimit.secondaryWindow, kind: 'secondary', prefix, source: rawRateLimit },
    ];
    const order = ({ raw, kind }: typeof entries[number]) => {
      const duration = isRecord(raw) ? numberValue(raw.limit_window_seconds ?? raw.limitWindowSeconds) : null;
      if (duration === FIVE_HOUR_SECONDS) return 0;
      if (duration === WEEK_SECONDS || (duration !== null && duration >= MIN_MONTH_SECONDS && duration <= MAX_MONTH_SECONDS)) return 1;
      return kind === 'primary' ? 0 : 1;
    };
    windows.push(...entries.sort((a, b) => order(a) - order(b)));
  };

  addRateLimit(value.rate_limit ?? value.rateLimit, '');
  addRateLimit(
    value.code_review_rate_limit ?? value.codeReviewRateLimit,
    `${quotaText('quota.service.codeReview')} `,
  );
  const additional = value.additional_rate_limits ?? value.additionalRateLimits;
  if (Array.isArray(additional)) {
    additional.forEach((item, index) => {
      if (!isRecord(item)) return;
      const name = readString(item, 'limit_name', 'limitName', 'metered_feature', 'meteredFeature')
        || quotaText('quota.service.additional', { index: index + 1 });
      addRateLimit(item.rate_limit ?? item.rateLimit, `${name} `);
    });
  }

  return windows.map(({ raw, kind, prefix, source }): QuotaRow | null => {
    if (!isRecord(raw)) return null;
    const duration = numberValue(raw.limit_window_seconds ?? raw.limitWindowSeconds);
    const reached = source.limit_reached === true || source.limitReached === true || source.allowed === false;
    const resetAtMs = quotaResetFor(raw, ['reset_at', 'resetAt'], ['reset_after_seconds', 'resetAfterSeconds']);
    return {
      label: codexWindowLabel(duration, prefix, kind),
      remainingPercent: remainingFromUsedPercent(raw.used_percent ?? raw.usedPercent)
        ?? (reached && resetAtMs !== undefined ? 0 : null),
      reset: codexResetLabel(raw),
      resetAtMs,
    };
  }).filter((row): row is QuotaRow => row !== null);
};

export const codexResetCreditsFor = (payload: unknown): number | undefined => {
  const value = parseBody(payload);
  if (!isRecord(value)) return undefined;
  const credits = isRecord(value.rate_limit_reset_credits)
    ? value.rate_limit_reset_credits
    : isRecord(value.rateLimitResetCredits)
      ? value.rateLimitResetCredits
      : null;
  const count = numberValue(credits?.available_count ?? credits?.availableCount);
  return count === null ? undefined : Math.max(0, Math.floor(count));
};

/** Read the account-level credits returned by the Codex usage endpoint. */
export const codexAccountCreditsFor = (
  payload: unknown,
): { balance?: string; unlimited: boolean } => {
  const value = parseBody(payload);
  if (!isRecord(value) || !isRecord(value.credits)) return { unlimited: false };
  const rawBalance = value.credits.balance;
  const balance = typeof rawBalance === 'number'
    ? String(rawBalance)
    : typeof rawBalance === 'string' && rawBalance.trim() !== ''
      ? rawBalance.trim()
      : undefined;
  return {
    ...(balance && /^\d+(?:\.\d+)?$/.test(balance) && Number.isFinite(Number(balance)) ? { balance } : {}),
    unlimited: value.credits.unlimited === true,
  };
};

export const codexResetCreditDetailsFor = (
  payload: unknown,
  nowMs = Date.now(),
): { availableCount?: number; applicableAvailableCount?: number; earliestExpiry?: string; expiries?: string[] } => {
  const value = parseBody(payload);
  if (!isRecord(value)) return {};
  const credits = Array.isArray(value.credits)
    ? value.credits
      .filter(isRecord)
      .filter((credit) =>
        readString(credit, 'reset_type', 'resetType') === 'codex_rate_limits'
        && readString(credit, 'status') === 'available',
      )
    : [];
  const availableCount = numberValue(value.available_count ?? value.availableCount);
  const applicableCount = numberValue(value.applicable_available_count ?? value.applicableAvailableCount);
  const validCredits = credits.filter((credit) => {
    const expiry = quotaResetInstant(credit.expires_at ?? credit.expiresAt);
    return expiry !== undefined && expiry > nowMs;
  });
  const expiries = validCredits
    .map((credit) => readString(credit, 'expires_at', 'expiresAt'))
    .map((expiresAt) => ({ expiresAt, expiresAtMs: quotaResetInstant(expiresAt) ?? NaN }))
    .filter((credit) =>
      credit.expiresAt
      && Number.isFinite(credit.expiresAtMs)
      && credit.expiresAtMs > nowMs,
    )
    .sort((left, right) => left.expiresAtMs - right.expiresAtMs)
    .map(credit => credit.expiresAt);

  return {
    availableCount: availableCount === null
      ? validCredits.length || undefined
      : Math.max(0, Math.floor(availableCount)),
    ...(applicableCount === null ? {} : { applicableAvailableCount: Math.max(0, Math.floor(applicableCount)) }),
    earliestExpiry: expiries[0],
    expiries,
  };
};

export const quotaRowsFor = (provider: QuotaProvider, payload: unknown): QuotaRow[] => {
  if (provider === 'devin') return readDevinQuota(payload).windows.map((window) => ({
    label: quotaText(window.id === 'daily' ? 'quota.service.daily' : 'quota.service.weekly'),
    remainingPercent: window.remainingPercent,
    resetAtMs: window.resetAtMs,
  }));
  const value = parseBody(payload);
  if (!isRecord(value)) return [];

  if (provider === 'codex') {
    const rows = codexWindowRows(value);
    const control = value.spend_control;
    const individual = isRecord(control) ? control.individual_limit : undefined;
    if (isRecord(individual)) {
      const limit = numberValue(individual.limit);
      const used = numberValue(individual.used);
      const remaining = numberValue(individual.remaining);
      const format = (amount: number) => new Intl.NumberFormat(getCurrentLocale(), {
        maximumFractionDigits: 2,
      }).format(amount);
      rows.push({
        label: quotaText('quota.service.individualSpendingLimit'),
        remainingPercent: clampPercent(individual.remaining_percent)
          ?? remainingFromUsedPercent(individual.used_percent)
          ?? (remaining !== null && limit !== null && limit > 0
            ? clampPercent(remaining / limit * 100) : null),
        reset: codexResetLabel(individual),
        resetAtMs: quotaResetFor(individual, ['reset_at'], ['reset_after_seconds']),
        detail: used !== null && limit !== null
          ? `${quotaText('quota.service.usedOf', { used: format(used), limit: format(limit) })} ${readString(individual, 'unit')}`.trim()
          : undefined,
      });
    }
    return rows;
  }

  if (provider === 'claude') {
    const labels: Record<string, string> = {
      five_hour: quotaText('quota.service.window.fiveHour'),
      seven_day: quotaText('quota.service.window.sevenDay'),
      seven_day_oauth_apps: quotaText('quota.service.window.sevenDayOAuth'),
      seven_day_opus: quotaText('quota.service.window.sevenDayOpus'),
      seven_day_sonnet: quotaText('quota.service.window.sevenDaySonnet'),
      seven_day_cowork: quotaText('quota.service.window.sevenDayCowork'),
      iguana_necktie: quotaText('quota.service.window.sevenDayFable'),
    };
    const fableCandidates = (Array.isArray(value.limits) ? value.limits : []).filter(isRecord)
      .filter((limit) => {
        const scope = isRecord(limit.scope) ? limit.scope : null;
        const name = readString(scope?.model, 'display_name', 'displayName').toLowerCase();
        return readString(limit, 'kind').toLowerCase() === 'weekly_scoped'
          && ['fable', 'fable 5'].includes(name) && numberValue(limit.percent) !== null;
      });
    const fable = fableCandidates.find((limit) => limit.is_active === true) ?? fableCandidates[0];
    const rows = Object.entries(labels)
      .filter(([key]) => key !== 'iguana_necktie' || !fable)
      .map(([key]): QuotaRow | null => {
        const raw = value[key];
        if (!isRecord(raw) || !('utilization' in raw)) return null;
        return {
          label: labels[key],
          remainingPercent: remainingFromUsedPercent(raw.utilization),
          reset: absoluteResetLabel(raw.resets_at ?? raw.resetsAt),
          resetAtMs: quotaResetFor(raw, ['resets_at', 'resetsAt']),
        };
      })
      .filter((row): row is QuotaRow => row !== null);
    if (fable) rows.push({
      label: quotaText('quota.service.window.sevenDayFable'),
      remainingPercent: remainingFromUsedPercent(fable.percent),
      reset: absoluteResetLabel(fable.resets_at ?? fable.resetsAt),
      resetAtMs: quotaResetFor(fable, ['resets_at', 'resetsAt']),
    });
    const extraUsage = isRecord(value.extra_usage)
      ? value.extra_usage
      : isRecord(value.extraUsage)
        ? value.extraUsage
        : null;
    if (extraUsage && booleanValue(extraUsage.is_enabled ?? extraUsage.isEnabled) === true) {
      const monthlyLimit = numberValue(extraUsage.monthly_limit ?? extraUsage.monthlyLimit);
      const usedCredits = numberValue(extraUsage.used_credits ?? extraUsage.usedCredits);
      const computedRemaining = monthlyLimit !== null && monthlyLimit > 0 && usedCredits !== null
        ? ((monthlyLimit - usedCredits) / monthlyLimit) * 100
        : null;
      const usedLabel = formatUsdFromCents(usedCredits);
      const limitLabel = formatUsdFromCents(monthlyLimit);
      rows.push({
        label: quotaText('quota.service.extraUsage'),
        remainingPercent:
          remainingFromUsedPercent(extraUsage.utilization)
          ?? clampPercent(computedRemaining),
        detail: usedLabel && limitLabel
          ? quotaText('quota.service.usedOf', { used: usedLabel, limit: limitLabel })
          : undefined,
      });
    }
    return rows;
  }

  if (provider === 'kimi') {
    const items: unknown[] = Array.isArray(value.limits) ? [...value.limits] : [];
    if (isRecord(value.usage)) {
      items.push({ ...value.usage, label: readString(value.usage, 'name', 'title') || quotaText('quota.service.weekly') });
    }
    return items
      .map((raw, index): QuotaRow | null => {
        if (!isRecord(raw)) return null;
        const detail = isRecord(raw.detail) ? raw.detail : raw;
        const limit = numberValue(detail.limit);
        const used = numberValue(detail.used);
        const remaining = numberValue(detail.remaining);
        const usedValue = used ?? (limit !== null && remaining !== null ? limit - remaining : null);
        if (usedValue === null && limit === null) return null;
        const window = isRecord(raw.window) ? raw.window : null;
        const duration = numberValue(window?.duration ?? raw.duration ?? detail.duration);
        const unit = (readString(window, 'timeUnit', 'time_unit')
          || readString(raw, 'timeUnit', 'time_unit')
          || readString(detail, 'timeUnit', 'time_unit')).toLowerCase().replace(/^time_unit_/, '');
        const durationText = duration !== null && duration > 0
          ? unit.startsWith('week')
            ? quotaText('quota.service.duration.days', { count: duration * 7 })
            : unit.startsWith('day')
              ? quotaText('quota.service.duration.days', { count: duration })
              : unit.startsWith('hour')
                ? quotaText('quota.service.duration.hours', { count: duration })
                : unit.startsWith('second')
                  ? quotaText('quota.service.duration.seconds', { count: duration })
                  : duration % 60 === 0
                    ? quotaText('quota.service.duration.hours', { count: duration / 60 })
                    : quotaText('quota.service.duration.minutes', { count: duration })
          : '';
        const durationLabel = durationText
          ? quotaText('quota.service.window.duration', { duration: durationText })
          : '';
        return {
          label:
            readString(raw, 'label', 'name', 'title', 'scope')
            || readString(detail, 'name', 'title', 'scope')
            || durationLabel
            || quotaText('quota.service.limit.numbered', { index: index + 1 }),
          remainingPercent: clampPercent(
            limit !== null && limit > 0
              ? (Math.max(0, limit - (usedValue ?? 0)) / limit) * 100
              : (usedValue ?? 0) > 0
                ? 0
                : null,
          ),
          reset:
            absoluteResetLabel(
              detail.reset_at ?? detail.resetAt ?? detail.reset_time ?? detail.resetTime,
            )
            ?? relativeResetLabel(detail.reset_in ?? detail.resetIn ?? detail.ttl),
          resetAtMs: quotaResetFor(detail, ['reset_at', 'resetAt', 'reset_time', 'resetTime'], ['reset_in', 'resetIn', 'ttl']),
          detail: limit === null ? undefined : `${usedValue ?? 0} / ${limit}`,
        };
      })
      .filter((row): row is QuotaRow => row !== null);
  }

  if (provider === 'xai') {
    if (value.mode === 'paid-health' || value.mode === 'paid-info') return [{
      label: quotaText('quota.service.xaiPaidAccount'),
      remainingPercent: null,
      detail: quotaText(value.mode === 'paid-health'
        ? 'quota.service.xaiPaidHealth' : 'quota.service.xaiPaidQuotaUnavailable'),
    }];
    const build = (payload: unknown) => {
      if (!isRecord(payload)) return null;
      return buildXaiBillingSummary(
        (isRecord(payload.config) ? payload.config : payload) as XaiBillingConfig,
      );
    };
    const billing = isRecord(value.weekly) || isRecord(value.monthly)
      ? mergeXaiBillingSummaries(build(value.weekly), build(value.monthly))
      : build(value);
    if (!billing) return [];
    const rows: QuotaRow[] = [];
    const weeklyReset = {
      reset: absoluteResetLabel(billing.periodEnd),
      resetAtMs: billing.resetAtMs ?? undefined,
    };
    const hasWeeklyData = billing.periodType === 'weekly';
    if (hasWeeklyData) {
      rows.push({
        label: quotaText('quota.service.weekly'),
        remainingPercent: remainingFromUsedPercent(billing.usagePercent),
        ...weeklyReset,
      });
    }
    billing.productUsage.forEach((item) => rows.push({
      label: item.product,
      remainingPercent: remainingFromUsedPercent(item.usagePercent),
    }));
    const monthlyReset = {
      reset: absoluteResetLabel(billing.billingPeriodEnd),
      resetAtMs: quotaResetInstant(billing.billingPeriodEnd),
    };
    const amount = (cap: number | null, used: number | null) => {
      const remaining = cap !== null && used !== null ? Math.max(0, cap - used) : null;
      return cap === null ? formatUsdFromCents(remaining)
        : `${formatUsdFromCents(remaining)} / ${formatUsdFromCents(cap)}`;
    };
    if (billing.onDemandCapCents !== null && billing.onDemandCapCents > 0) {
      rows.push({
        label: quotaText('quota.service.onDemand'),
        remainingPercent: remainingFromUsedPercent(billing.onDemandUsedPercent),
        detail: amount(billing.onDemandCapCents, billing.onDemandUsedCents),
      });
    }
    if ((billing.monthlyLimitCents !== null || billing.usedCents !== null || billing.billingPeriodEnd)
      && !(hasWeeklyData && billing.monthlyLimitCents === 0 && billing.usedCents === 0)) {
      rows.push({
        label: quotaText('quota.service.monthlyIncluded'),
        remainingPercent: remainingFromUsedPercent(billing.usedPercent),
        detail: amount(billing.monthlyLimitCents, billing.includedUsedCents),
        ...monthlyReset,
      });
    }
    if (billing.prepaidBalanceCents !== null && billing.prepaidBalanceCents > 0) {
      rows.push({ label: quotaText('quota.service.prepaidBalance'), remainingPercent: null,
        detail: formatUsdFromCents(billing.prepaidBalanceCents) });
    }
    return rows.length > 0 ? rows : [{
      label: quotaText(billing.periodType === 'weekly'
        ? 'quota.service.weekly' : 'quota.service.monthlyIncluded'),
      remainingPercent: null,
    }];
  }
  const nested = parseBody(value.body);
  const summary = !Array.isArray(value.groups) && isRecord(nested) ? nested : value;
  const groups = Array.isArray(summary.groups) ? summary.groups : [];
  return groups.flatMap((group) => {
    if (!isRecord(group) || !Array.isArray(group.buckets)) return [];
    const order = (bucket: unknown) => {
      const window = readString(bucket, 'window').toLowerCase();
      return ['5h', 'five-hour', 'five_hour'].includes(window) ? 0 : ['weekly', 'week'].includes(window) ? 1 : 2;
    };
    const buckets = [...group.buckets].sort((a, b) => order(a) - order(b));
    const groupLabel = readString(group, 'display_name', 'displayName')
      || quotaText('quota.service.quota');
    const groupDescription = readString(group, 'description');
    return buckets
      .map((bucket, index): QuotaRow | null => {
        if (!isRecord(bucket)) return null;
        const remaining = quotaFraction(bucket.remaining_fraction ?? bucket.remainingFraction);
        if (remaining === null) return null;
        const bucketLabel = readString(bucket, 'display_name', 'displayName', 'window');
        const label = bucketLabel && (buckets.length > 1 || bucketLabel !== groupLabel)
          ? `${groupLabel} · ${bucketLabel}`
          : groupLabel;
        return {
          label: label || quotaText('quota.service.quota.numbered', { index: index + 1 }),
          remainingPercent: remaining * 100,
          reset: absoluteResetLabel(bucket.reset_time ?? bucket.resetTime),
          resetAtMs: quotaResetFor(bucket, ['reset_time', 'resetTime']),
          detail: readString(bucket, 'description') || groupDescription || undefined,
        };
      })
      .filter((row): row is QuotaRow => row !== null);
  });
};

const resolveProjectId = async (file: AuthFile): Promise<string> => {
  const direct = antigravityProjectFor(file);
  if (direct) return direct;
  try {
    const payload = parseBody(await managementApi.get('/credentials/download', { name: fileName(file) }));
    return isRecord(payload) ? antigravityProjectFor(payload) : '';
  } catch {
    return '';
  }
};

const resolveCodexAccountId = (file: AuthFile): string => codexMetadataFor(file).accountId;

const xaiUserIdFromRecord = (record: Record<string, unknown>): string => {
  const nestedRecords = [record, record.metadata, record.attributes]
    .filter(isRecord);
  for (const source of nestedRecords) {
    const direct = readString(source, 'sub', 'subject', 'user_id', 'userId');
    if (direct) return direct;
    for (const container of [source.oauth, source.user]) {
      if (!isRecord(container)) continue;
      const nested = readString(container, 'sub', 'subject', 'user_id', 'userId', 'id');
      if (nested) return nested;
    }
  }
  return '';
};

const resolveXaiUserId = (file: AuthFile): string =>
  xaiUserIdFromRecord(file);

const requestQuotaPayload = async (
  authIndex: string,
  url: string,
  header: Record<string, string>,
  method: 'GET' | 'POST' = 'GET',
  data?: string,
  timeoutMs?: number,
  responseClock?: { serverTimeOffsetMs?: number },
) => {
  const response = await managementApi.post<Record<string, unknown>>('/requests/api-call', {
    authIndex,
    method,
    url,
    header,
    data,
  }, { timeoutMs });
  const status = Number(response.status_code ?? response.statusCode ?? 0);
  if (status < 200 || status >= 300) {
    throw new Error(apiCallErrorMessage(response));
  }
  if (responseClock) {
    const header = isRecord(response.header) ? response.header : {};
    const raw = Object.entries(header).find(([key]) => key.toLowerCase() === 'date')?.[1];
    const serverTime = quotaResetInstant(Array.isArray(raw) ? raw[0] : raw);
    responseClock.serverTimeOffsetMs = serverTime === undefined ? undefined : serverTime - Date.now();
  }
  return parseBody(response.body ?? response.bodyText);
};

const callXaiPaidHealth = async (authIndex: string): Promise<unknown> => {
  const header = { Authorization: 'Bearer $TOKEN$', accept: 'application/json' };
  const [profile, chat] = await Promise.allSettled([
    requestQuotaPayload(authIndex, 'https://api.x.ai/v1/me', header, 'GET', undefined, 15_000),
    requestQuotaPayload(authIndex, 'https://api.x.ai/v1/chat/completions', {
      ...header, 'Content-Type': 'application/json',
    }, 'POST', JSON.stringify({
      model: 'grok-4.5',
      messages: [{ role: 'user', content: 'ping' }],
      max_tokens: 1,
      stream: false,
    }), 15_000),
  ]);
  if (chat.status === 'rejected') throw chat.reason;
  const record = profile.status === 'fulfilled' && isRecord(profile.value) ? profile.value : {};
  return {
    mode: 'paid-health', plan_type: 'Paid',
    userId: readString(record, 'user_id', 'userId') || undefined,
    teamId: readString(record, 'team_id', 'teamId') || undefined,
  };
};
const callXaiQuota = async (file: AuthFile): Promise<unknown> => {
  const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
  if (!authIndex) throw new Error(quotaText('quota.service.error.missingAuthIndex'));
  if (isPaidXaiFile(file)) return callXaiPaidHealth(authIndex);
  const header = { ...headersByProvider.xai };
  const userId = await resolveXaiUserId(file);
  if (userId) header['x-userid'] = userId;
  const [weekly, monthly] = await Promise.allSettled([
    requestQuotaPayload(authIndex, XAI_WEEKLY_URL, header),
    requestQuotaPayload(authIndex, endpointByProvider.xai, header),
  ]);
  const payload = {
    weekly: weekly.status === 'fulfilled' ? weekly.value : null,
    monthly: monthly.status === 'fulfilled' ? monthly.value : null,
  };
  if (quotaRowsFor('xai', payload).length > 0) return payload;
  const billingError = weekly.status === 'rejected' && monthly.status === 'rejected'
    ? weekly.reason : new Error(quotaText('quota.service.error.unrecognized'));
  try {
    return await callXaiPaidHealth(authIndex);
  } catch {
    throw billingError;
  }
};

// Optional subscription requests run after the quota has been committed to the shared cache.
export const enrichXaiQuotaPlan = async (file: AuthFile, quota: QuotaState): Promise<QuotaState> => {
  if (providerForFile(file) !== 'xai' || quota.status !== 'success') return quota;
  const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
  if (!authIndex) return quota;
  const header = { ...headersByProvider.xai };
  const userId = resolveXaiUserId(file);
  if (userId) header['x-userid'] = userId;
  const [user, settings] = await Promise.allSettled([
    requestQuotaPayload(authIndex, 'https://cli-chat-proxy.grok.com/v1/user?include=subscription', header, 'GET', undefined, 8_000),
    requestQuotaPayload(authIndex, 'https://cli-chat-proxy.grok.com/v1/settings', header, 'GET', undefined, 8_000),
  ]);
  const readPlan = (result: PromiseSettledResult<unknown>, keys: string[]) => {
    if (result.status !== 'fulfilled' || !isRecord(result.value)) return '';
    for (const key of keys) {
      const value = result.value[key];
      if (typeof value === 'string' && value.trim()) return value.trim();
    }
    return '';
  };
  const plan = readPlan(settings, ['subscription_tier_display', 'subscriptionTierDisplay'])
    || readPlan(user, ['subscriptionTier', 'subscription_tier']);
  return plan ? { ...quota, plan } : quota;
};

const xaiBillingPlan = (payload: unknown): string | undefined => {
  if (!isRecord(payload)) return undefined;
  const configs = [payload.monthly, payload.weekly, payload].filter(isRecord)
    .map((record) => isRecord(record.config) ? record.config : record);
  for (const config of configs) {
    const limit = buildXaiBillingSummary(config as XaiBillingConfig)?.monthlyLimitCents;
    if (limit === 150_000) return 'SuperGrok Heavy';
    if (limit === 15_000) return 'SuperGrok';
  }
  return undefined;
};

const booleanValue = (value: unknown): boolean | null => {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return Number.isFinite(value) ? value !== 0 : null;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (['true', '1', 'yes', 'y', 'on'].includes(normalized)) return true;
    if (['false', '0', 'no', 'n', 'off'].includes(normalized)) return false;
  }
  return null;
};

const resolveClaudePlan = (payload: unknown): string | undefined => {
  if (!isRecord(payload)) return undefined;
  const account = isRecord(payload.account) ? payload.account : null;
  const organization = isRecord(payload.organization) ? payload.organization : null;
  if (booleanValue(account?.has_claude_max) === true) return 'Max';
  if (booleanValue(account?.has_claude_pro) === true) return 'Pro';
  if (
    readString(organization, 'organization_type').toLowerCase() === 'claude_team'
    && readString(organization, 'subscription_status').toLowerCase() === 'active'
  ) return 'Team';
  if (
    booleanValue(account?.has_claude_max) === false
    && booleanValue(account?.has_claude_pro) === false
  ) return 'Free';
  return undefined;
};

const loadClaudePlan = async (file: AuthFile): Promise<string | undefined> => {
  const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
  if (!authIndex) return undefined;
  try {
    return resolveClaudePlan(
      await requestQuotaPayload(authIndex, CLAUDE_PROFILE_URL, headersByProvider.claude),
    );
  } catch {
    return undefined;
  }
};

const loadAntigravityPlan = async (file: AuthFile): Promise<string | undefined> => {
  const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
  if (!authIndex) return undefined;
  try {
    const payload = await requestQuotaPayload(
      authIndex,
      ANTIGRAVITY_CODE_ASSIST_URL,
      headersByProvider.antigravity,
      'POST',
      JSON.stringify({ metadata: { ideType: 'ANTIGRAVITY' } }),
    );
    if (!isRecord(payload)) return undefined;
    const currentTier = isRecord(payload.currentTier)
      ? payload.currentTier
      : isRecord(payload.current_tier)
        ? payload.current_tier
        : null;
    const paidTier = isRecord(payload.paidTier)
      ? payload.paidTier
      : isRecord(payload.paid_tier)
        ? payload.paid_tier
        : null;
    const effectiveTier = readString(paidTier, 'id') ? paidTier : currentTier;
    const tierId = readString(effectiveTier, 'id').toLowerCase();
    const tierName = readString(effectiveTier, 'name');
    const knownPlans: Record<string, string> = {
      'free-tier': 'Free',
      'g1-pro-tier': 'Pro',
      'g1-ultra-tier': 'Ultra',
      'g1-ultra-lite-tier': 'Ultra Lite',
    };
    return knownPlans[tierId] || tierName || tierId || undefined;
  } catch {
    return undefined;
  }
};

async function callUpstreamQuota(
  file: AuthFile,
  provider: QuotaProvider,
  resolvedCodexAccountId?: string,
  responseClock?: { serverTimeOffsetMs?: number },
): Promise<unknown> {
  const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
  if (!authIndex) throw new Error(quotaText('quota.service.error.missingAuthIndex'));
  if (provider === 'devin') {
    if (!readString(file, 'name')) throw new Error(quotaText('quota.service.error.missingAuthIndex'));
    return requestQuotaPayload(authIndex, DEVIN_QUOTA_URL, DEVIN_QUOTA_HEADERS, 'POST', DEVIN_QUOTA_DATA, undefined, responseClock);
  }
  const header = { ...headersByProvider[provider] };
  if (provider === 'codex') {
    const accountId = resolvedCodexAccountId ?? await resolveCodexAccountId(file);
    if (accountId) header['Chatgpt-Account-Id'] = accountId;
  }
  const project = provider === 'antigravity' ? await resolveProjectId(file) : '';
  if (provider === 'antigravity' && !project) {
    throw new Error(quotaText('quota.service.error.missingProject'));
  }
  const urls = provider === 'antigravity'
    ? [endpointByProvider.antigravity, 'https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:retrieveUserQuotaSummary', 'https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary']
    : [endpointByProvider[provider]];
  let lastError = '';
  let hadSuccessfulResponse = false;
  for (const url of urls) {
    try {
      const payload = await requestQuotaPayload(
        authIndex,
        url,
        header,
        provider === 'antigravity' ? 'POST' : 'GET',
        project ? JSON.stringify({ project }) : undefined,
        undefined,
        provider === 'antigravity' ? responseClock : undefined,
      );
      if (provider === 'antigravity') {
        hadSuccessfulResponse = true;
        if (quotaRowsFor('antigravity', payload).length === 0) {
          lastError = quotaText('quota.service.error.antigravityEmpty');
          continue;
        }
      }
      return payload;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
  }
  throw new Error(
    lastError || quotaText(
      hadSuccessfulResponse
        ? 'quota.service.error.upstreamEmpty'
        : 'quota.service.error.noResponse',
    ),
  );
}

const callCodexResetCredits = async (
  file: AuthFile,
  accountId: string,
): Promise<ReturnType<typeof codexResetCreditDetailsFor>> => {
  const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
  if (!authIndex) throw new Error(quotaText('quota.service.error.missingResetAuthIndex'));
  const header: Record<string, string> = {
    ...headersByProvider.codex,
    Accept: 'application/json',
    'OpenAI-Beta': 'codex-1',
    Originator: 'Codex Desktop',
  };
  if (accountId) header['Chatgpt-Account-Id'] = accountId;
  const payload = await requestQuotaPayload(authIndex, CODEX_RESET_CREDITS_URL, header, 'GET', undefined, 8_000);
  if (!isRecord(payload) || !['credits', 'available_count', 'availableCount', 'applicable_available_count', 'applicableAvailableCount'].some((key) => key in payload)) {
    throw new Error(quotaText('quota.service.error.resetCreditsInvalid'));
  }
  return codexResetCreditDetailsFor(payload);
};

async function loadQuotaSnapshot(file: AuthFile): Promise<QuotaState> {
  const provider = providerForFile(file);
  if (!provider) {
    return {
      status: 'error',
      rows: [],
      error: quotaText('quota.service.error.unsupportedProvider'),
    };
  }
  try {
    if (booleanValue(file.disabled) === true) throw new Error(quotaText('quota.fileDisabled'));
    const codexMetadata = provider === 'codex' ? codexMetadataFor(file) : undefined;
    const codexAccountId = codexMetadata?.accountId || '';
    const responseClock: { serverTimeOffsetMs?: number } = {};
    const payloadPromise = provider === 'xai'
      ? callXaiQuota(file)
      : callUpstreamQuota(file, provider, codexAccountId, responseClock);
    const planPromise = provider === 'claude'
      ? loadClaudePlan(file)
      : provider === 'antigravity'
        ? loadAntigravityPlan(file)
        : Promise.resolve(undefined);
    let resetCreditsError: string | undefined;
    const resetCreditsPromise = provider === 'codex'
      ? callCodexResetCredits(file, codexAccountId).catch((error) => {
        resetCreditsError = error instanceof Error ? error.message : String(error);
        return null;
      })
      : Promise.resolve(null);
    const claudeGrantsPromise = provider === 'claude'
      ? readClaudeResetGrants(normalizeAuthIndex(file.auth_index ?? file.authIndex)).catch(() => {
        resetCreditsError = quotaText('quota.claude.readError');
        return undefined;
      }) : Promise.resolve(undefined);
    const [payload, detectedPlan, resetCreditDetails, claudeResetGrants] = await Promise.all([
      payloadPromise,
      planPromise,
      resetCreditsPromise,
      claudeGrantsPromise,
    ]);
    const rows = quotaRowsFor(provider, payload);
    if (rows.length === 0) {
      return {
        status: 'error',
        rows: [],
        error: quotaText('quota.service.error.unrecognized'),
      };
    }
    const resetCredits = provider === 'codex'
      ? resetCreditDetails?.availableCount ?? codexResetCreditsFor(payload)
      : claudeResetGrants?.grants.reduce((sum, grant) => sum + grant.resetsLeft, 0);
    const usageCreditDetails = provider === 'codex' && isRecord(payload)
      ? codexResetCreditDetailsFor(payload.rate_limit_reset_credits ?? payload.rateLimitResetCredits)
      : {};
    const accountCredits = provider === 'codex' ? codexAccountCreditsFor(payload) : { unlimited: false };
    return {
      status: 'success',
      rows,
      plan: (provider === 'devin' ? readDevinQuota(payload).plan : detectedPlan)
        ?? (provider === 'xai' ? xaiBillingPlan(payload) : undefined)
        ?? (readString(isRecord(payload) ? payload : {}, 'plan_type', 'planType') || codexMetadata?.plan),
      creditBalance: accountCredits.balance,
      creditsUnlimited: accountCredits.unlimited,
      subscriptionActiveUntil: provider === 'devin'
        ? readDevinQuota(payload).subscriptionActiveUntil : codexMetadata?.subscriptionActiveUntil,
      claudeResetGrants,
      resetCreditsError,
      resetCreditsApplicable: provider === 'claude' ? undefined : usageCreditDetails.applicableAvailableCount
        ?? resetCreditDetails?.applicableAvailableCount ?? resetCredits,
      resetCredits,
      resetCreditExpiries: resetCreditDetails?.expiries ?? usageCreditDetails.expiries,
      resetCreditsEarliestExpiry: resetCreditDetails?.earliestExpiry ?? claudeResetGrants?.grants.filter(grant => grant.resetsLeft > 0 && grant.endsAt).map(grant => grant.endsAt!).sort((a, b) => Date.parse(a) - Date.parse(b))[0],
      serverTimeOffsetMs: responseClock.serverTimeOffsetMs,
      fetchedAt: Date.now(),
    };
  } catch (error) {
    return {
      status: 'error',
      rows: [],
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

const createRedeemRequestId = () => {
  if (typeof globalThis.crypto?.randomUUID === 'function') return globalThis.crypto.randomUUID();
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (character) => {
    const random = Math.floor(Math.random() * 16);
    const value = character === 'x' ? random : (random & 0x3) | 0x8;
    return value.toString(16);
  });
};

async function consumeCodexResetCreditSnapshot(file: AuthFile): Promise<QuotaState> {
  if (providerForFile(file) !== 'codex') {
    throw new Error(quotaText('quota.service.error.codexResetOnly'));
  }
  if (booleanValue(file.disabled) === true) throw new Error(quotaText('quota.fileDisabled'));
  const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
  if (!authIndex) throw new Error(quotaText('quota.service.error.missingConsumeAuthIndex'));
  const header = {
    ...headersByProvider.codex,
  };
  const accountId = await resolveCodexAccountId(file);
  if (accountId) header['Chatgpt-Account-Id'] = accountId;
  await requestQuotaPayload(
    authIndex,
    CODEX_RESET_CREDITS_CONSUME_URL,
    header,
    'POST',
    JSON.stringify({ redeem_request_id: createRedeemRequestId() }),
  );
  return loadQuotaSnapshot(file);
}

const quotaRequests = new Map<string, Promise<QuotaState>>();
const quotaMutationRequests = new Map<string, Promise<QuotaState>>();

export function loadQuota(file: AuthFile): Promise<QuotaState> {
  const key = quotaKey(file);
  const reset = quotaMutationRequests.get(key);
  if (reset) return reset.catch((error): QuotaState => ({
    status: 'error', rows: [], error: error instanceof Error ? error.message : String(error),
  }));
  const existing = quotaRequests.get(key);
  if (existing) return existing;
  const request = loadQuotaSnapshot(file).finally(() => {
    if (quotaRequests.get(key) === request) quotaRequests.delete(key);
  });
  quotaRequests.set(key, request);
  return request;
}

function runQuotaMutation(file: AuthFile, mutate: () => Promise<QuotaState>): Promise<QuotaState> {
  const key = quotaKey(file);
  const existing = quotaMutationRequests.get(key);
  if (existing) return existing;
  const request = (async () => {
    await quotaRequests.get(key);
    return mutate();
  })().finally(() => {
    if (quotaMutationRequests.get(key) === request) quotaMutationRequests.delete(key);
  });
  quotaMutationRequests.set(key, request);
  return request;
}

export function consumeCodexResetCredit(file: AuthFile): Promise<QuotaState> {
  return runQuotaMutation(file, () => consumeCodexResetCreditSnapshot(file));
}

export function consumeClaudeResetCredit(file: AuthFile): Promise<QuotaState> {
  const generation = captureQuotaCacheGeneration();
  return runQuotaMutation(file, async () => {
    if (captureQuotaCacheGeneration() !== generation) throw new Error(quotaText('quota.claude.blocked'));
    if (providerForFile(file) !== 'claude' || booleanValue(file.disabled) === true)
      throw new Error(quotaText('quota.claude.blocked'));
    const authIndex = normalizeAuthIndex(file.auth_index ?? file.authIndex);
    if (!authIndex) throw new Error(quotaText('quota.claude.blocked'));
    const key = quotaKey(file);
    const pending = resetGrantOperations.inspect(key);
    const grantId = pending && !pending.code ? pending.grantId
      : selectResetGrant(await readClaudeResetGrants(authIndex), Date.now())?.id;
    if (!grantId || captureQuotaCacheGeneration() !== generation) throw new Error(quotaText('quota.claude.blocked'));
    try {
      const answer = await resetGrantOperations.run(key, authIndex, grantId);
      if (answer.unresolved) throw new Error(quotaText('quota.claude.unknown'));
      if (answer.code !== 'reset' && answer.code !== 'already_used')
        throw new Error(quotaText(`quota.claude.${answer.code}`));
    } catch (error) {
      const operation = resetGrantOperations.inspect(key);
      if (operation && !operation.code) throw new Error(quotaText(Date.now() - operation.createdAt >= RETRY_WINDOW_MS ? 'quota.claude.expired' : 'quota.claude.unknown'));
      throw error;
    }
    return loadQuotaSnapshot(file);
  });
}

export function quotaResetExpiries(quota: QuotaState): string[] {
  return (quota.resetCreditExpiries ?? (quota.resetCreditsEarliestExpiry ? [quota.resetCreditsEarliestExpiry] : []))
    .slice().sort((left, right) => (quotaResetInstant(left) ?? Infinity) - (quotaResetInstant(right) ?? Infinity));
}
