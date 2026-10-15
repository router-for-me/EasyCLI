import { useSyncExternalStore } from 'react';
import { getCurrentLocale, translate, type AppLocale } from '../i18n';

export const quotaResetInstant = (value: unknown): number | undefined => {
  if (typeof value !== 'number' && typeof value !== 'string') return undefined;
  if (typeof value === 'string' && !value.trim()) return undefined;
  const numeric = Number(value);
  const ms = Number.isFinite(numeric)
    ? numeric > 0 ? numeric < 1e12 ? numeric * 1000 : numeric : NaN
    : Date.parse(String(value));
  return Number.isFinite(ms) && Number.isFinite(new Date(ms).getTime()) ? ms : undefined;
};

export const quotaResetFor = (
  record: Record<string, unknown>,
  absoluteKeys: string[],
  relativeKeys: string[] = [],
  nowMs = Date.now(),
): number | undefined => {
  for (const key of absoluteKeys) {
    const ms = quotaResetInstant(record[key]);
    if (ms !== undefined) return ms;
  }
  for (const key of relativeKeys) {
    const raw = record[key];
    if (typeof raw !== 'number' && typeof raw !== 'string') continue;
    if (typeof raw === 'string' && !raw.trim()) continue;
    const seconds = Number(raw);
    const ms = nowMs + seconds * 1000;
    if (Number.isFinite(seconds) && seconds >= 0 && Number.isFinite(new Date(ms).getTime())) return ms;
  }
  return undefined;
};

export const formatQuotaReset = (
  resetAtMs: number | undefined,
  fallback?: string,
  locale: AppLocale = getCurrentLocale(),
  nowMs = Date.now(),
): string => {
  if (resetAtMs === undefined || !Number.isFinite(resetAtMs)) return fallback || '';
  const absolute = new Intl.DateTimeFormat(locale, {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false,
  }).format(resetAtMs);
  const delta = resetAtMs - nowMs;
  if (delta <= 0) return `${absolute} · ${translate(locale, 'quota.resetPassed')}`;
  const minutes = Math.max(1, Math.floor(delta / 60000));
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: 'always' });
  const label = minutes >= 1440 ? relative.format(Math.floor(minutes / 1440), 'day')
    : minutes >= 60 ? relative.format(Math.floor(minutes / 60), 'hour')
      : relative.format(minutes, 'minute');
  return `${absolute} · ${label}`;
};

let now = Date.now();
let timer: ReturnType<typeof setInterval> | undefined;
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  if (!timer) {
    now = Date.now();
    timer = setInterval(() => {
      now = Date.now();
      listeners.forEach((notify) => notify());
    }, 60_000);
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      if (timer !== undefined) clearInterval(timer);
      timer = undefined;
    }
  };
};
export const useQuotaClock = () => useSyncExternalStore(subscribe, () => now, () => now);
