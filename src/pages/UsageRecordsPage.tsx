import { usagePreferences } from '../services/usagePreferences';
import { useConfirmation } from '../components/ConfirmationDialog';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  Activity,
  Database,
  Gauge,
  Pencil,
  RefreshCw,
  Trash2,
  TriangleAlert,
  Wallet,
  Wrench,
  X,
} from 'lucide-react';
import { getCurrentLocale, useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';
import { MessageNotice, FloatingNotice, useAppNotice } from '../appNotice';
import { calculateTokenComposition } from '../services/usageMetrics';
import { formatDuration, formatUsageNumber } from '../services/usageNumber';
import { handleHorizontalTabKey } from '../components/tabKeyboardNavigation';
import {
  OTHER_TREND_MODEL_KEY,
  buildUsageTrendSeries,
  clampTrendRatio,
  formatTrendAxisLabel,
  formatTrendRangeLabel,
  isClientPointInsideRect,
  niceCeiling,
  trendAxisTicks,
  trendPointIndexAtRatio,
  trendTimeAxisTicks,
  trendTimePosition,
  stackModelTokens,
  type UsageTimelinePoint,
} from '../services/usageTrend';
import { createRefreshScheduler } from '../services/refreshScheduler';
import { usageViewScopeKey } from '../services/usageViewScope';
import { EventsView, type UsageEventPage } from './UsageEventsView';
import { SelectMenu } from '../components/SelectMenu';
import { UsageAnalysisView, type UsageAnalysis, type UsageCategory } from './UsageAnalysisView';

type UsageTab = 'overview' | 'analysis' | 'events' | 'pricing' | 'data-management';
type UsageRange = '4h' | '24h' | 'today' | '7d' | '30d' | 'all' | 'custom';

const USAGE_TABS: readonly UsageTab[] = ['overview', 'analysis', 'events', 'pricing', 'data-management'];

type CollectorStatus = {
  state: 'waiting-core' | 'collecting' | 'error';
  message: string;
  lastCollectedAt: string | null;
  totalRecords: number;
};

type TimelinePoint = UsageTimelinePoint;

type UsageOverview = {
  totalRequests: number;
  successCount: number;
  failureCount: number;
  canceledCount: number;
  successRate: number;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalTokens: number;
  rpm: number;
  tpm: number;
  tps: number;
  tpsSampleCount: number;
  averageLatencyMs: number;
  cacheHitRate: number;
  estimatedCost: number;
  pricedRequests: number;
  timeline: TimelinePoint[];
};

type ModelPrice = {
  model: string;
  prompt: number;
  completion: number;
  cacheRead: number;
  cacheCreation: number;
  promptConfigured: boolean;
  completionConfigured: boolean;
  cacheReadConfigured: boolean;
  cacheCreationConfigured: boolean;
  source: string;
  sourceModelId: string;
  updatedAtMs: number;
};

type UsagePriceRow = {
  model: string;
  requests: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalTokens: number;
  estimatedCost: number;
  price: ModelPrice | null;
};

type UsagePricing = {
  rows: UsagePriceRow[];
  totalCost: number;
  totalRequests: number;
  pricedRequests: number;
  savedPrices: number;
};

type UsageRepairResult = {
  scanned: number;
  repaired: number;
  deleted: number;
  backupPath: string | null;
};

type UsageStorageSettings = {
  maxDatabaseSizeMb: number;
  databaseSizeBytes: number;
  totalRecords: number;
  deletedRecords: number;
};

type ModelPriceSyncResult = {
  imported: number;
};

type ModelPriceSyncPreview = {
  source: string;
  sourceUrl: string;
  matches: ModelPrice[];
  unmatched: string[];
};

type SyncPriceDraft = {
  selected: boolean;
  price: ModelPrice;
  prompt: string;
  completion: string;
  cacheRead: string;
  cacheCreation: string;
};

type UsageQuery = {
  start?: string;
  end?: string;
  model?: string;
  provider?: string;
  source?: string;
  upstream?: string;
  api_key_hash?: string;
  failed?: boolean;
  canceled?: boolean;
  page?: number;
  page_size?: number;
};

const TAB_KEY = 'cpa-gui.usage-records-tab.v1';
const RANGE_KEY = 'cpa-gui.usage-records-range.v1';
const PAGE_SIZE_KEY = 'cpa-gui.usage-events-page-size.v1';
const EVENT_PAGE_SIZES = [20, 50, 100, 200] as const;

const loadPageSize = (): number => {
  try {
    const saved = Number(usagePreferences.getItem(PAGE_SIZE_KEY));
    return EVENT_PAGE_SIZES.includes(saved as (typeof EVENT_PAGE_SIZES)[number]) ? saved : 50;
  } catch {
    return 50;
  }
};
const emptyAnalysis: UsageAnalysis = { models: [], providers: [], sources: [], apiKeys: [] };

const loadTab = (): UsageTab => {
  try {
    const saved = usagePreferences.getItem(TAB_KEY);
    return saved === 'analysis' || saved === 'events' || saved === 'pricing' || saved === 'data-management'
      ? saved
      : 'overview';
  } catch {
    return 'overview';
  }
};

const loadRange = (): UsageRange => {
  try {
    const saved = usagePreferences.getItem(RANGE_KEY) as UsageRange | null;
    return ['4h', '24h', 'today', '7d', '30d', 'all', 'custom'].includes(saved ?? '')
      ? (saved as UsageRange)
      : '24h';
  } catch {
    return '24h';
  }
};

const rangeQuery = (range: UsageRange, customStart: string, customEnd: string): Pick<UsageQuery, 'start' | 'end'> => {
  const now = new Date();
  if (range === 'all') return {};
  if (range === 'custom') {
    const start = customStart ? new Date(customStart) : null;
    const end = customEnd ? new Date(customEnd) : null;
    return {
      start: start && !Number.isNaN(start.getTime()) ? start.toISOString() : undefined,
      end: end && !Number.isNaN(end.getTime()) ? end.toISOString() : undefined,
    };
  }
  if (range === 'today') {
    const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    return { start: start.toISOString(), end: now.toISOString() };
  }
  const hours = range === '4h' ? 4 : range === '24h' ? 24 : range === '7d' ? 24 * 7 : 24 * 30;
  return {
    start: new Date(now.getTime() - hours * 60 * 60 * 1000).toISOString(),
    end: now.toISOString(),
  };
};

const compactNumber = (value: number) => formatUsageNumber(value, getCurrentLocale());
const compactDuration = (value: number) => formatDuration(value, getCurrentLocale());

const formatStorageBytes = (value: number) => {
  const bytes = Number.isFinite(value) ? Math.max(0, value) : 0;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
};

const formatUsd = (amount: number) => {
  if (!Number.isFinite(amount) || amount <= 0) return '$0.00';
  const maximumFractionDigits =
    amount >= 100
      ? 2
      : amount >= 1
      ? 3
      : amount >= 0.01
      ? 4
      : amount >= 0.0001
      ? 6
      : 8;
  return `$${new Intl.NumberFormat(getCurrentLocale(), {
    minimumFractionDigits: 2,
    maximumFractionDigits,
  }).format(amount)}`;
};

const filterOptions = (items: UsageCategory[]) => items.filter((item) => item.key && item.label);

export function UsageRecordsPage() {
  const { t } = useI18n();
  const privacy = usePrivacyMask();
  const [activeTab, setActiveTab] = useState<UsageTab>(loadTab);
  const [range, setRange] = useState<UsageRange>(loadRange);
  const [customStart, setCustomStart] = useState('');
  const [customEnd, setCustomEnd] = useState('');
  const [model, setModel] = useState('');

  const [source, setSource] = useState('');
  const [apiKeyHash, setApiKeyHash] = useState('');
  const [result, setResult] = useState('all');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(loadPageSize);
  const [status, setStatus] = useState<CollectorStatus | null>(null);
  const [overview, setOverview] = useState<UsageOverview | null>(null);
  const [overviewRange, setOverviewRange] = useState<Pick<UsageQuery, 'start' | 'end'>>({});
  const [analysis, setAnalysis] = useState<UsageAnalysis>(emptyAnalysis);
  const [optionsAnalysis, setOptionsAnalysis] = useState<UsageAnalysis>(emptyAnalysis);
  const [events, setEvents] = useState<UsageEventPage | null>(null);
  const [pricing, setPricing] = useState<UsagePricing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const loadedFilterScopeKeyRef = useRef('');
  const optionsLoadedAtRef = useRef(0);
  const requestIdRef = useRef(0);
  const schedulerRef = useRef<ReturnType<typeof createRefreshScheduler> | null>(null);
  if (!schedulerRef.current) schedulerRef.current = createRefreshScheduler(250);

  useEffect(() => {
    try {
      usagePreferences.setItem(TAB_KEY, activeTab);
    } catch {
    }
  }, [activeTab]);

  useEffect(() => {
    try {
      usagePreferences.setItem(RANGE_KEY, range);
    } catch {
    }
  }, [range]);

  useEffect(() => {
    try {
      usagePreferences.setItem(PAGE_SIZE_KEY, String(pageSize));
    } catch {
    }
  }, [pageSize]);

  const buildQueries = useCallback(() => {
    const nextTimeQuery = rangeQuery(range, customStart, customEnd);
    return {
      timeQuery: nextTimeQuery,
      query: {
        ...nextTimeQuery,
        model: model || undefined,
        upstream: source || undefined,
        api_key_hash: apiKeyHash || undefined,
        failed: result === 'failed' ? true : result === 'success' ? false : undefined,
        canceled: result === 'canceled' ? true : result === 'failed' ? false : undefined,
      } satisfies UsageQuery,
    };
  }, [apiKeyHash, customEnd, customStart, model, range, result, source]);

  const filterScopeKey = useMemo(() => usageViewScopeKey({
    tab: 'overview',
    range,
    customStart,
    customEnd,
    model: '',
    provider: '',
    source: '',
    apiKeyHash: '',
    result: 'all',
    page: 1,
    pageSize: 1,
  }), [customEnd, customStart, range]);

  const executeLoadData = useCallback(
    async (quiet = false) => {
      const requestId = ++requestIdRef.current;
      const { timeQuery, query } = buildQueries();
      if (!quiet) setLoading(true);
      try {
        const statusRequest = invoke<CollectorStatus>('get_usage_collector_status');
        const filtersChanged = loadedFilterScopeKeyRef.current !== filterScopeKey;
        // Refresh explicitly and periodically: a fixed time range can still gain
        // new keys, and configured remarks can change without new usage records.
        const refreshOptions = filtersChanged || !quiet || Date.now() - optionsLoadedAtRef.current >= 30_000;
        const optionsRequest = refreshOptions
          ? invoke<UsageAnalysis>('get_usage_analysis', { query: timeQuery })
          : Promise.resolve(null);
        if (activeTab === 'overview') {
          const [nextStatus, nextOptions, nextOverview] = await Promise.all([
            statusRequest,
            optionsRequest,
            invoke<UsageOverview>('get_usage_overview', { query }),
          ]);
          if (requestId !== requestIdRef.current) return;
          setStatus(nextStatus);
          if (nextOptions) setOptionsAnalysis(nextOptions);
          setOverview(nextOverview);
          setOverviewRange(timeQuery);
        } else if (activeTab === 'analysis') {
          const [nextStatus, nextOptions, nextOverview, nextAnalysis] = await Promise.all([
            statusRequest,
            optionsRequest,
            invoke<UsageOverview>('get_usage_overview', { query }),
            model || source || apiKeyHash || result !== 'all' || !refreshOptions
              ? invoke<UsageAnalysis>('get_usage_analysis', { query })
              : optionsRequest.then((analysis) => analysis ?? emptyAnalysis),
          ]);
          if (requestId !== requestIdRef.current) return;
          setStatus(nextStatus);
          if (nextOptions) setOptionsAnalysis(nextOptions);
          setOverview(nextOverview);
          setOverviewRange(timeQuery);
          setAnalysis(nextAnalysis);
        } else if (activeTab === 'events') {
          const [nextStatus, nextOptions, nextEvents] = await Promise.all([
            statusRequest,
            optionsRequest,
            invoke<UsageEventPage>('get_usage_events', {
              query: { ...query, page, page_size: pageSize },
            }),
          ]);
          if (requestId !== requestIdRef.current) return;
          setStatus(nextStatus);
          if (nextOptions) setOptionsAnalysis(nextOptions);
          setEvents(nextEvents);
        } else if (activeTab === 'pricing') {
          const [nextStatus, nextOptions, nextPricing] = await Promise.all([
            statusRequest,
            optionsRequest,
            invoke<UsagePricing>('get_usage_pricing', { query }),
          ]);
          if (requestId !== requestIdRef.current) return;
          setStatus(nextStatus);
          if (nextOptions) setOptionsAnalysis(nextOptions);
          setPricing(nextPricing);
        } else {
          const [nextStatus, nextOptions] = await Promise.all([statusRequest, optionsRequest]);
          if (requestId !== requestIdRef.current) return;
          setStatus(nextStatus);
          if (nextOptions) setOptionsAnalysis(nextOptions);
        }
        if (filtersChanged) loadedFilterScopeKeyRef.current = filterScopeKey;
        if (refreshOptions) optionsLoadedAtRef.current = Date.now();
        setError('');
      } catch (requestError) {
        if (requestId === requestIdRef.current) setError(String(requestError));
      } finally {
        if (requestId === requestIdRef.current) setLoading(false);
      }
    },
    [activeTab, buildQueries, filterScopeKey, page, pageSize, model, source, apiKeyHash, result]
  );

  const loadData = useCallback(
    (quiet = false, immediate = !quiet) => {
      if (!quiet) setLoading(true);
      if (!quiet) return schedulerRef.current!.runForeground(() => executeLoadData(false));
      return schedulerRef.current!.schedule(() => executeLoadData(quiet), immediate);
    },
    [executeLoadData],
  );

  useEffect(() => {
    void loadData();
    return () => {
      ++requestIdRef.current;
      schedulerRef.current?.cancelPending();
    };
  }, [loadData]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    const refresh = () => {
      // WebView2 may classify an unfocused or occluded window on another monitor
      // as hidden. Keep usage refreshes independent of Page Visibility so both
      // record events and the fallback poll continue to update the current view.
      if (!disposed) void loadData(true, true);
    };
    listen('usage-records-updated', refresh)
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    const timer = window.setInterval(refresh, 1_000);
    const refreshWhenVisible = () => {
      if (!document.hidden) refresh();
    };
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refreshWhenVisible);
    return () => {
      disposed = true;
      unlisten?.();
      window.clearInterval(timer);
      window.removeEventListener('focus', refresh);
      document.removeEventListener('visibilitychange', refreshWhenVisible);
    };
  }, [loadData]);

  const changeFilter = (setter: (value: string) => void, value: string) => {
    setter(value);
    setPage(1);
  };

  const collectorTone = status?.state === 'error' ? 'error' : status?.state === 'collecting' ? 'success' : '';
  const showInitialLoading =
    activeTab !== 'data-management' &&
    !error &&
    loading &&
    ((activeTab === 'overview' && !overview) ||
      (activeTab === 'analysis' && !overview && !analysis) ||
      (activeTab === 'events' && !events) ||
      (activeTab === 'pricing' && !pricing));

  const handleTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, tab: UsageTab) => {
    handleHorizontalTabKey(
      event,
      USAGE_TABS,
      tab,
      setActiveTab,
      (next) => document.getElementById(`usage-tab-${next}`),
    );
  };

  const modelOptions = filterOptions(optionsAnalysis.models);
  const sourceOptions = [...(optionsAnalysis.upstreams ?? [])].sort((a, b) => a.label.localeCompare(b.label));
  const keyOptions = filterOptions(optionsAnalysis.apiKeys);
  const filterPanel = (
    <section className="panel usage-filter-panel">
      <div className="usage-filter-row">
        <div className="usage-filter-group">
          <div className="usage-filter-item">
            <SelectMenu
              value={range}
              ariaLabel={t('usage.filter.timeRange')}
              onChange={(value) => {
                setRange(value as UsageRange);
                setPage(1);
              }}
              options={[
                { value: '4h', label: t('usage.range.4h') },
                { value: '24h', label: t('usage.range.24h') },
                { value: 'today', label: t('usage.range.today') },
                { value: '7d', label: t('usage.range.7d') },
                { value: '30d', label: t('usage.range.30d') },
                { value: 'all', label: t('usage.range.all') },
                { value: 'custom', label: t('usage.range.custom') },
              ]}
            />
          </div>
          <div className="usage-filter-item">
            <SelectMenu
              value={model}
              ariaLabel={t('usage.filter.model')}
              onChange={(value) => changeFilter(setModel, value)}
              options={[{ value: '', label: t('usage.filter.allModels') }, ...modelOptions.map((item) => ({ value: item.key, label: item.label }))]}
            />
          </div>
          <div className="usage-filter-item">
            <SelectMenu
              value={source}
              ariaLabel={t('usage.filter.source')}
              onChange={(value) => changeFilter(setSource, value)}
              options={[{ value: '', label: t('usage.filter.allSources') }, ...sourceOptions.map((item) => ({ value: item.key, label: privacy.text(item.label) }))]}
            />
          </div>
          <div className="usage-filter-item">
            <SelectMenu
              value={apiKeyHash}
              ariaLabel={t('usage.column.key')}
              onChange={(value) => changeFilter(setApiKeyHash, value)}
              options={[{ value: '', label: t('usage.filter.allKeys') }, ...keyOptions.map((item) => ({ value: item.key, label: item.label }))]}
            />
          </div>
          <div className="usage-filter-item">
            <SelectMenu
              value={result}
              ariaLabel={t('usage.filter.result')}
              onChange={(value) => changeFilter(setResult, value)}
              options={[
                { value: 'all', label: t('usage.filter.allResults') },
                { value: 'success', label: t('usage.result.success') },
                { value: 'failed', label: t('usage.result.failed') },
                { value: 'canceled', label: t('usage.result.canceled') },
              ]}
            />
          </div>
        </div>
      </div>

      {range === 'custom' ? (
        <div className="usage-custom-range">
          <input
            type="datetime-local"
            value={customStart}
            onChange={(event) => setCustomStart(event.currentTarget.value)}
            aria-label={t('usage.filter.startTime')}
          />
          <span>{t('usage.filter.to')}</span>
          <input
            type="datetime-local"
            value={customEnd}
            onChange={(event) => setCustomEnd(event.currentTarget.value)}
            aria-label={t('usage.filter.endTime')}
          />
        </div>
      ) : null}
    </section>
  );

  return (
    <section className="page management-page usage-records-page" data-active-tab={activeTab}>
      {error ? <MessageNotice message={error} onDismiss={() => setError('')} /> : null}

      <div className="usage-topbar">
        <div className="usage-page-navigation">
          <h1 className="sr-only">{t('usage.title')}</h1>
          <div className="usage-tabs" role="tablist" aria-label={t('usage.pageLabel')}>
          <button
            type="button"
            id="usage-tab-overview"
            role="tab"
            className={activeTab === 'overview' ? 'active' : ''}
            aria-selected={activeTab === 'overview'}
            aria-controls="usage-tab-panel"
            tabIndex={activeTab === 'overview' ? 0 : -1}
            onClick={() => setActiveTab('overview')}
            onKeyDown={(event) => handleTabKeyDown(event, 'overview')}
          >
            <span>{t('usage.tab.overview')}</span>
          </button>
          <button
            type="button"
            id="usage-tab-analysis"
            role="tab"
            className={activeTab === 'analysis' ? 'active' : ''}
            aria-selected={activeTab === 'analysis'}
            aria-controls="usage-tab-panel"
            tabIndex={activeTab === 'analysis' ? 0 : -1}
            onClick={() => setActiveTab('analysis')}
            onKeyDown={(event) => handleTabKeyDown(event, 'analysis')}
          >
            <span>{t('usage.tab.analysis')}</span>
          </button>
          <button
            type="button"
            id="usage-tab-events"
            role="tab"
            className={activeTab === 'events' ? 'active' : ''}
            aria-selected={activeTab === 'events'}
            aria-controls="usage-tab-panel"
            tabIndex={activeTab === 'events' ? 0 : -1}
            onClick={() => setActiveTab('events')}
            onKeyDown={(event) => handleTabKeyDown(event, 'events')}
          >
            <span>{t('usage.tab.events')}</span>
          </button>
          <button
            type="button"
            id="usage-tab-pricing"
            role="tab"
            className={activeTab === 'pricing' ? 'active' : ''}
            aria-selected={activeTab === 'pricing'}
            aria-controls="usage-tab-panel"
            tabIndex={activeTab === 'pricing' ? 0 : -1}
            onClick={() => setActiveTab('pricing')}
            onKeyDown={(event) => handleTabKeyDown(event, 'pricing')}
          >
            <span>{t('usage.tab.pricing')}</span>
          </button>
          <button
            type="button"
            id="usage-tab-data-management"
            role="tab"
            className={activeTab === 'data-management' ? 'active' : ''}
            aria-selected={activeTab === 'data-management'}
            aria-controls="usage-tab-panel"
            tabIndex={activeTab === 'data-management' ? 0 : -1}
            onClick={() => setActiveTab('data-management')}
            onKeyDown={(event) => handleTabKeyDown(event, 'data-management')}
          >
            <span>{t('usage.tab.dataManagement')}</span>
          </button>
          </div>
        </div>

        <div className="usage-topbar-actions">
          <div className={`usage-collector-state ${collectorTone}`} title={status?.message}>
            <span className="status-dot" />
            <strong>
              {status?.state === 'collecting'
                ? t('usage.collector.collecting')
                : status?.state === 'error'
                ? t('usage.collector.error')
                : t('usage.collector.waiting')}
            </strong>
            <span>{t('usage.longTermRecords', { count: compactNumber(status?.totalRecords ?? 0) })}</span>
          </div>
          <button
            type="button"
            className="icon-button usage-refresh-btn"
            onClick={() => void loadData(false)}
            disabled={loading}
            title={t('usage.refresh')}
            aria-label={t('usage.refresh')}
          >
            <RefreshCw size={15} className={loading ? 'spin' : ''} />
          </button>
        </div>
      </div>

      <div
        className="usage-tab-panel"
        id="usage-tab-panel"
        role="tabpanel"
        aria-labelledby={`usage-tab-${activeTab}`}
      >
      {activeTab !== 'data-management' ? filterPanel : null}

      {showInitialLoading && activeTab !== 'events' ? (
        <div className="usage-initial-loading">
          <Database size={22} />
          <span>{t('usage.loading')}</span>
        </div>
      ) : null}

      {activeTab === 'overview' && overview ? <OverviewView overview={overview} range={overviewRange} /> : null}
      {activeTab === 'analysis' && (analysis || overview) ? <UsageAnalysisView analysis={analysis} overview={overview} range={overviewRange} /> : null}
      {activeTab === 'events' ? (
        <EventsView
          events={events ?? { items: [], total: 0, page, pageSize, totalPages: 1 }}
          loading={!events}
          pageSize={pageSize}
          query={buildQueries().query}
          onPage={setPage}
          onPageSizeChange={(size) => {
            setPageSize(size);
            setPage(1);
          }}
        />
      ) : null}
      {activeTab === 'pricing' && pricing ? (
        <PricingView pricing={pricing} query={buildQueries().query} onChanged={() => loadData(true)} />
      ) : null}
      {activeTab === 'data-management' ? <UsageDataManagementView /> : null}
      </div>
    </section>
  );
}

function UsageDataManagementView() {
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const { t } = useI18n();
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<UsageRepairResult | null>(null);
  const [error, setError] = useState('');
  const [storage, setStorage] = useState<UsageStorageSettings | null>(null);
  const [limitDraft, setLimitDraft] = useState('0');
  const [loadingLimit, setLoadingLimit] = useState(true);
  const [savingLimit, setSavingLimit] = useState(false);
  const [shrinkDraft, setShrinkDraft] = useState('');
  const [shrinking, setShrinking] = useState(false);
  const [storageNotice, setStorageNotice] = useState('');

  useEffect(() => {
    let disposed = false;
    invoke<UsageStorageSettings>('get_usage_storage_settings')
      .then((next) => {
        if (disposed) return;
        setStorage(next);
        setLimitDraft(String(next.maxDatabaseSizeMb));
      })
      .catch((requestError) => {
        if (!disposed) setError(String(requestError));
      })
      .finally(() => {
        if (!disposed) setLoadingLimit(false);
      });
    return () => {
      disposed = true;
    };
  }, []);

  const saveStorageLimit = async () => {
    const normalized = limitDraft.trim();
    const maxDatabaseSizeMb = Number(normalized);
    if (!/^\d+$/.test(normalized) || !Number.isSafeInteger(maxDatabaseSizeMb)) {
      setError(t('usage.dataManagement.storageInvalid'));
      return;
    }
    setSavingLimit(true);
    setError('');
    setStorageNotice('');
    try {
      const next = await invoke<UsageStorageSettings>('save_usage_storage_settings', { maxDatabaseSizeMb });
      setStorage(next);
      setLimitDraft(String(next.maxDatabaseSizeMb));
      setStorageNotice(t(
        next.deletedRecords > 0
          ? 'usage.dataManagement.storageSavedWithCleanup'
          : 'usage.dataManagement.storageSaved',
        { deleted: next.deletedRecords.toLocaleString() },
      ));
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setSavingLimit(false);
    }
  };

  const shrinkDatabase = async () => {
    const normalized = shrinkDraft.trim();
    const targetDatabaseSizeMb = Number(normalized);
    if (!/^\d+$/.test(normalized) || !Number.isSafeInteger(targetDatabaseSizeMb) || targetDatabaseSizeMb <= 0) {
      setError(t('usage.dataManagement.shrinkInvalid'));
      return;
    }
    const confirmed = await askConfirmation({
      title: t('usage.dataManagement.shrinkConfirmTitle'),
      message: t('usage.dataManagement.shrinkConfirm', { size: targetDatabaseSizeMb }),
    });
    if (!confirmed) return;
    setShrinking(true);
    setError('');
    setStorageNotice('');
    try {
      const next = await invoke<UsageStorageSettings>('shrink_usage_database', { targetDatabaseSizeMb });
      setStorage(next);
      setStorageNotice(t(
        next.deletedRecords > 0
          ? 'usage.dataManagement.shrinkSuccess'
          : 'usage.dataManagement.shrinkNoCleanup',
        {
          deleted: next.deletedRecords.toLocaleString(),
          size: formatStorageBytes(next.databaseSizeBytes),
        },
      ));
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setShrinking(false);
    }
  };

  const repair = async () => {
    if (!await askConfirmation({ title: t('usage.dataManagement.title'), message: t('usage.dataManagement.confirm') })) return;
    setRunning(true);
    setError('');
    setResult(null);
    try {
      const next = await invoke<UsageRepairResult>('repair_usage_cache_records');
      setResult(next);
      const nextStorage = await invoke<UsageStorageSettings>('get_usage_storage_settings');
      setStorage(nextStorage);
    } catch (requestError) {
      setError(String(requestError));
    } finally {
      setRunning(false);
    }
  };

  return (
    <section className="panel usage-data-management-panel">
      {confirmationDialog}
      <div className="usage-data-management-heading">
        <div>
          <Wrench size={20} aria-hidden="true" />
          <h2>{t('usage.dataManagement.title')}</h2>
        </div>
        <span className="usage-data-management-badge">{t('usage.dataManagement.manualBadge')}</span>
      </div>

      <div className="usage-data-management-action usage-storage-limit-action">
        <div>
          <strong>{t('usage.dataManagement.storageTitle')}</strong>
          <span>{t('usage.dataManagement.storageDescription')}</span>
          {storage ? (
            <small>
              {t('usage.dataManagement.storageCurrent', {
                size: formatStorageBytes(storage.databaseSizeBytes),
                records: compactNumber(storage.totalRecords),
              })}
            </small>
          ) : null}
        </div>
        <div className="usage-storage-limit-editor">
          <label>
            <input
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              value={limitDraft}
              disabled={loadingLimit || savingLimit || shrinking || running}
              onChange={(event) => setLimitDraft(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !loadingLimit && !savingLimit && !shrinking && !running) void saveStorageLimit();
              }}
              aria-label={t('usage.dataManagement.storageInput')}
            />
            <span>{t('usage.dataManagement.storageUnit')}</span>
          </label>
          <button
            type="button"
            className="primary-button"
            onClick={() => void saveStorageLimit()}
            disabled={loadingLimit || savingLimit || shrinking || running}
          >
            {savingLimit ? t('usage.dataManagement.storageSaving') : t('usage.dataManagement.storageSave')}
          </button>
        </div>
      </div>

      <div className="usage-data-management-action">
        <div>
          <strong>{t('usage.dataManagement.shrinkTitle')}</strong>
          <span>{t('usage.dataManagement.shrinkDescription')}</span>
        </div>
        <div className="usage-storage-limit-editor">
          <label>
            <input
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              value={shrinkDraft}
              disabled={savingLimit || shrinking || running}
              onChange={(event) => setShrinkDraft(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !savingLimit && !shrinking && !running) void shrinkDatabase();
              }}
              aria-label={t('usage.dataManagement.shrinkInput')}
            />
            <span>{t('usage.dataManagement.storageUnit')}</span>
          </label>
          <button
            type="button"
            className="primary-button"
            onClick={() => void shrinkDatabase()}
            disabled={savingLimit || shrinking || running}
          >
            {shrinking ? t('usage.dataManagement.shrinking') : t('usage.dataManagement.shrinkRun')}
          </button>
        </div>
      </div>

      {storageNotice ? <MessageNotice tone="success" message={storageNotice} onDismiss={() => setStorageNotice('')} /> : null}

      <div className="usage-data-management-action">
        <div>
          <strong>{t('usage.dataManagement.actionTitle')}</strong>
          <span>{t('usage.dataManagement.actionDescription')}</span>
        </div>
        <button type="button" className="primary-button" onClick={() => void repair()} disabled={running || savingLimit || shrinking}>
          {running ? t('usage.dataManagement.running') : t('usage.dataManagement.run')}
        </button>
      </div>

      {error ? <MessageNotice message={error} onDismiss={() => setError('')} /> : null}
      {result ? (
        <>
          <MessageNotice tone="success" message={t('usage.dataManagement.success', { repaired: result.repaired, deleted: result.deleted })} />
          <div className="usage-data-management-result">
          <div><span>{t('usage.dataManagement.scanned')}</span><strong>{result.scanned.toLocaleString()}</strong></div>
          <div><span>{t('usage.dataManagement.repaired')}</span><strong>{result.repaired.toLocaleString()}</strong></div>
          <div><span>{t('usage.dataManagement.deleted')}</span><strong>{result.deleted.toLocaleString()}</strong></div>
          <div><span>{t('usage.dataManagement.backup')}</span><strong title={result.backupPath ?? undefined}>{result.backupPath ?? '—'}</strong></div>
          </div>
        </>
      ) : null}
    </section>
  );
}

function OverviewView({ overview, range }: { overview: UsageOverview; range?: Pick<UsageQuery, 'start' | 'end'> }) {
  const { t } = useI18n();
  const cards = [
    {
      key: 'requests',
      tone: 'requests',
      label: t('usage.stat.requests'),
      value: compactNumber(overview.totalRequests),
      metaTitle: t('usage.stat.requestMetaTitle', {
        total: compactNumber(overview.totalRequests),
        success: compactNumber(overview.successCount),
        failed: compactNumber(overview.failureCount),
        canceled: compactNumber(overview.canceledCount),
      }),
    },
    {
      key: 'speed',
      tone: 'speed',
      label: t('usage.stat.tps'),
      value: overview.tpsSampleCount > 0 ? overview.tps.toFixed(1) : '—',
      metaTitle: t('usage.stat.performanceMetaTitle', {
        tps: overview.tpsSampleCount > 0 ? overview.tps.toFixed(1) : '—',
        samples: compactNumber(overview.tpsSampleCount),
        rpm: overview.rpm.toFixed(2),
        latency: Math.round(overview.averageLatencyMs),
      }),
    },
    {
      key: 'tokens',
      tone: 'tokens',
      label: t('usage.stat.tokens'),
      value: compactNumber(overview.totalTokens),
      metaTitle: t('usage.stat.tokenMetaTitle', {
        input: compactNumber(overview.inputTokens),
        output: compactNumber(overview.outputTokens),
        reasoning: compactNumber(overview.reasoningTokens),
        cache: compactNumber(overview.cacheReadTokens),
      }),
    },
    {
      key: 'success',
      tone: 'success',
      label: t('usage.stat.successRate'),
      value: `${overview.successRate.toFixed(1)}%`,
      metaTitle: t('usage.stat.successMetaTitle', {
        success: compactNumber(overview.successCount),
        failed: compactNumber(overview.failureCount),
        canceled: compactNumber(overview.canceledCount),
      }),
    },
    {
      key: 'cache',
      tone: 'cache',
      label: t('usage.stat.cacheHitRate'),
      value: `${(overview.cacheHitRate * 100).toFixed(1)}%`,
      metaTitle: t('usage.stat.cacheHitMetaTitle', {
        rate: (overview.cacheHitRate * 100).toFixed(1),
        hit: compactNumber(overview.cacheReadTokens),
        input: compactNumber(overview.inputTokens),
      }),
    },
    {
      key: 'cost',
      tone: 'cost',
      label: t('usage.stat.estimatedCost'),
      value: formatUsd(overview.estimatedCost),
      metaTitle: t('usage.stat.costMetaTitle', {
        priced: compactNumber(overview.pricedRequests),
        total: compactNumber(overview.totalRequests),
        unpriced: compactNumber(Math.max(overview.totalRequests - overview.pricedRequests, 0)),
      }),
    },
  ];

  return (
    <div className="usage-overview-layout">
      <div className="usage-stat-grid">
        {cards.map(({ key, tone, label, value, metaTitle }) => (
          <article className={`panel usage-stat-card tone-${tone}`} key={key} title={metaTitle}>
            <span className="usage-stat-card-label">{label}</span>
            <strong className="usage-stat-card-value">{value}</strong>
          </article>
        ))}
      </div>
      <UsageOverviewPanels>
        <section className="panel usage-trend-panel">
          <div className="usage-section-heading">
            <div>
              <strong>{t('usage.trend.title')}</strong>
            </div>
          </div>
          <UsageTrend points={overview.timeline} range={range} />
        </section>
        <TokenComposition overview={overview} />
      </UsageOverviewPanels>
    </div>
  );
}

function UsageOverviewPanels({ children }: { children: ReactNode }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(1);

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    let frame = 0;
    const measure = () => {
      const { width, height } = container.getBoundingClientRect();
      if (width <= 0 || height <= 0) return;
      const nextScale = Math.min(1, width / 900, height / 420);
      setScale((current) => Math.abs(current - nextScale) < 0.001 ? current : nextScale);
    };
    const scheduleMeasure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    };
    measure();
    const observer = new ResizeObserver(scheduleMeasure);
    observer.observe(container);
    window.addEventListener('resize', scheduleMeasure);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener('resize', scheduleMeasure);
    };
  }, []);

  return (
    <div className="usage-overview-panels" ref={containerRef} style={{ '--usage-overview-scale': scale } as CSSProperties}>
      <div className="usage-overview-canvas">{children}</div>
    </div>
  );
}

function UsageTrend({
  points,
  range,
}: {
  points: TimelinePoint[];
  range?: Pick<UsageQuery, 'start' | 'end'>;
}) {
  const { t, locale } = useI18n();
  const [hoveredRatio, setHoveredRatio] = useState<number | null>(null);
  const [hiddenModels, setHiddenModels] = useState<string[]>([]);
  const plotRef = useRef<HTMLDivElement>(null);
  const [plotWidth, setPlotWidth] = useState(0);

  const series = useMemo(
    () => buildUsageTrendSeries(points, range),
    [points, range?.start, range?.end],
  );

  const hiddenKeys = useMemo(() => new Set(hiddenModels), [hiddenModels]);
  useEffect(() => {
    const available = new Set(series.models.map((model) => model.key));
    setHiddenModels((current) => {
      const next = current.filter((key) => available.has(key));
      return next.length === current.length && next.every((key, index) => key === current[index])
        ? current
        : next;
    });
  }, [series.models]);

  const count = series.points.length;

  useLayoutEffect(() => {
    const plot = plotRef.current;
    if (!plot) return;
    let frame = 0;
    const updateWidth = (width: number) => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const next = Math.max(0, Math.round(width));
        setPlotWidth((current) => current === next ? current : next);
      });
    };
    const measure = () => updateWidth(plot.getBoundingClientRect().width);
    measure();
    const observer = typeof ResizeObserver === 'undefined'
      ? null
      : new ResizeObserver(measure);
    observer?.observe(plot);
    window.addEventListener('resize', measure);
    return () => {
      cancelAnimationFrame(frame);
      observer?.disconnect();
      window.removeEventListener('resize', measure);
    };
  }, [count > 0]);

  useEffect(() => {
    if (count === 0) setHoveredRatio(null);
  }, [count === 0]);

  useEffect(() => {
    if (hoveredRatio == null) return undefined;
    const onWindowPointerMove = (event: PointerEvent) => {
      if (event.clientX === 0 && event.clientY === 0) return;
      const plot = plotRef.current;
      if (!plot || !isClientPointInsideRect(event.clientX, event.clientY, plot.getBoundingClientRect())) {
        setHoveredRatio(null);
      }
    };
    window.addEventListener('pointermove', onWindowPointerMove);
    return () => window.removeEventListener('pointermove', onWindowPointerMove);
  }, [hoveredRatio == null]);

  const chart = useMemo(() => {
    const stacked = series.points.map((point) => stackModelTokens(point, series.models, hiddenKeys));
    const maxTokens = niceCeiling(stacked.reduce((max, layers) => Math.max(max, layers[layers.length - 1]?.y1 ?? 0), 1));

    const VIEWBOX_W = 1000;
    const PT = 8;
    const PB = 8;
    const UH = 154 - PT - PB;
    const baseY = PT + UH;

    const calcY = (val: number) => (maxTokens > 0 ? baseY - (val / maxTokens) * UH : baseY);
    const start = series.points[0]?.start ?? new Date(0);
    const end = series.points[count - 1]?.end ?? start;
    const bars = series.points.map((point, index) => {
      const left = trendTimePosition(point.start, start, end) * VIEWBOX_W;
      const right = trendTimePosition(point.end, start, end) * VIEWBOX_W;
      const gap = Math.min((right - left) * 0.2, 6);
      return {
        x: left + gap / 2,
        width: right - left - gap,
        center: (left + right) / 2,
        layers: stacked[index].filter((layer) => layer.tokens > 0).map((layer) => ({
          ...layer,
          y: calcY(layer.y1),
          height: (layer.tokens / maxTokens) * UH,
          color: series.models.find((model) => model.key === layer.key)?.color,
        })),
      };
    });
    const yTicks = trendAxisTicks(maxTokens);
    const compactSameDay = start.toDateString() === end.toDateString();
    const timeTicks = trendTimeAxisTicks(start, end, plotWidth, compactSameDay ? 64 : 112);
    const showAxisTime = timeTicks.length > 1 && timeTicks[1].getTime() - timeTicks[0].getTime() < 24 * 60 * 60 * 1000;

    return {
      maxTokens,
      stacked,
      bars,
      start,
      end,
      yTicks,
      timeTicks,
      compactSameDay,
      showAxisTime,
      baseY,
      PT,
      UH,
    };
  }, [count, hiddenKeys, series, plotWidth]);

  if (count === 0) {
    return <UsageEmpty />;
  }

  const modelLabel = (key: string, fallback: string) =>
    key === OTHER_TREND_MODEL_KEY ? t('usage.trend.other') : fallback;

  const hoveredIndex = hoveredRatio == null
    ? -1
    : trendPointIndexAtRatio(series.points, chart.start, chart.end, hoveredRatio);

  const handlePointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    if (rect.width <= 0 || count === 0) return;
    setHoveredRatio(clampTrendRatio((e.clientX - rect.left) / rect.width));
  };

  const handlePointerLeave = (e: ReactPointerEvent<HTMLDivElement>) => {
    const next = e.relatedTarget;
    if (next instanceof Node && e.currentTarget.contains(next)) return;
    const rect = e.currentTarget.getBoundingClientRect();
    if (isClientPointInsideRect(e.clientX, e.clientY, rect)) return;
    if (e.clientX === 0 && e.clientY === 0) return;
    setHoveredRatio(null);
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (count === 0) return;
    if (e.key === 'ArrowLeft') {
      e.preventDefault();
      const current = hoveredIndex < 0
        ? count - 1
        : (hoveredIndex <= 0 ? count - 1 : hoveredIndex - 1);
      setHoveredRatio((chart.bars[current]?.center ?? 0) / 1000);
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      const current = hoveredIndex < 0
        ? 0
        : (hoveredIndex >= count - 1 ? 0 : hoveredIndex + 1);
      setHoveredRatio((chart.bars[current]?.center ?? 0) / 1000);
    } else if (e.key === 'Home') {
      e.preventDefault();
      setHoveredRatio((chart.bars[0]?.center ?? 0) / 1000);
    } else if (e.key === 'End') {
      e.preventDefault();
      setHoveredRatio((chart.bars[count - 1]?.center ?? 0) / 1000);
    } else if (e.key === 'Escape') {
      setHoveredRatio(null);
    }
  };

  const active = hoveredIndex >= 0 && hoveredIndex < count ? series.points[hoveredIndex] : null;
  const activeStacked = hoveredIndex >= 0 && chart.stacked[hoveredIndex] ? chart.stacked[hoveredIndex] : [];
  const activeViewboxX = hoveredIndex >= 0 ? (chart.bars[hoveredIndex]?.center ?? 0) : 0;
  const activePercent = activeViewboxX / 10;
  const activeLayers = [...activeStacked]
    .filter((l) => l.tokens > 0)
    .sort((a, b) => b.tokens - a.tokens);
  const activeTotal = activeStacked[activeStacked.length - 1]?.y1 ?? 0;

  const srText = active
    ? `${formatTrendRangeLabel(active, locale, series.bucket)}: ${compactNumber(activeTotal)} ${t('usage.unit.tokens')}`
    : '';

  return (
    <div className="usage-trend-wrapper">
      <div className="usage-trend-toolbar">
        <div className="usage-trend-legend" role="group" aria-label={t('usage.trend.aria')}>
          {series.models.map((model) => {
            const isHidden = hiddenKeys.has(model.key);
            return (
              <button
                type="button"
                key={model.key}
                className={`usage-trend-legend-item${isHidden ? ' is-hidden' : ''}`}
                aria-pressed={!isHidden}
                onClick={() =>
                  setHiddenModels((current) =>
                    current.includes(model.key)
                      ? current.filter((key) => key !== model.key)
                      : [...current, model.key],
                  )
                }
              >
                <span className="usage-trend-swatch" style={{ background: model.color }} />
                <span>{modelLabel(model.key, model.label)}</span>
              </button>
            );
          })}
        </div>
      </div>

      <div className="usage-trend-chart">
        <div className="usage-trend-y-axis" aria-hidden="true">
          {chart.yTicks.map((tick) => (
            <span key={tick} style={{ top: `${((chart.baseY - (tick / chart.maxTokens) * chart.UH) / 154) * 100}%` }}>
              {compactNumber(tick)}
            </span>
          ))}
        </div>

        <div
          ref={plotRef}
          className="usage-trend-plot"
          tabIndex={0}
          role="region"
          aria-label={t('usage.trend.aria')}
          onPointerMove={handlePointerMove}
          onPointerLeave={handlePointerLeave}
          onPointerCancel={handlePointerLeave}
          onKeyDown={handleKeyDown}
        >
          <svg
            viewBox="0 0 1000 154"
            preserveAspectRatio="none"
            className="usage-trend-svg"
          >
            {chart.yTicks.map((tick) => {
              const y = chart.baseY - (tick / chart.maxTokens) * chart.UH;
              const isBase = tick === 0;
              return (
                <line
                  key={`grid-${tick}`}
                  x1="0"
                  y1={y}
                  x2="1000"
                  y2={y}
                  className={isBase ? 'usage-trend-baseline' : 'usage-trend-grid'}
                />
              );
            })}

            {chart.bars.map((bar, index) => (
              <g key={series.points[index].hour} className={`usage-trend-bar${active && index === hoveredIndex ? ' is-active' : ''}`}>
                {bar.layers.map((layer) => (
                  <rect
                    key={layer.key}
                    x={bar.x}
                    y={layer.y}
                    width={bar.width}
                    height={layer.height}
                    fill={layer.color}
                  />
                ))}
              </g>
            ))}

            {active ? (
              <g className="usage-trend-active-mark">
                <line
                  x1={activeViewboxX}
                  y1={chart.PT}
                  x2={activeViewboxX}
                  y2={chart.baseY}
                  className="usage-trend-cursor-line"
                />
              </g>
            ) : null}
          </svg>

          {active ? (
            <div
              className={`usage-trend-tooltip${activePercent > 62 ? ' is-left' : ' is-right'}`}
              style={{ left: `${activePercent}%` }}
            >
              <div className="usage-trend-tooltip-header">
                <strong>{formatTrendRangeLabel(active, locale, series.bucket)}</strong>
                <span className="usage-trend-tooltip-total">
                  {compactNumber(activeTotal)} {t('usage.trend.tooltip.tokens')}
                </span>
              </div>
              {activeLayers.length > 0 ? (
                <div className="usage-trend-tooltip-list">
                  {activeLayers.map((layer) => {
                    const model = series.models.find((m) => m.key === layer.key);
                    return (
                      <div key={layer.key} className="usage-trend-tooltip-row">
                        <span className="usage-trend-swatch" style={{ background: model?.color }} />
                        <span className="usage-trend-tooltip-label">
                          {modelLabel(layer.key, model?.label ?? layer.key)}
                        </span>
                        <b>{compactNumber(layer.tokens)}</b>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="usage-trend-tooltip-empty">{t('usage.unit.tokens')}: 0</div>
              )}
            </div>
          ) : null}
        </div>

        <div className="usage-trend-x-axis" aria-hidden="true">
          {chart.timeTicks.map((date, index) => {
            const left = trendTimePosition(date, chart.start, chart.end) * 100;
            const posClass = index === 0 ? 'is-start' : index === chart.timeTicks.length - 1 ? 'is-end' : 'is-mid';
            return (
              <span
                key={date.getTime()}
                className={posClass}
                style={{ left: `${left}%` }}
                title={date.toLocaleString(locale)}
              >
                {formatTrendAxisLabel({ start: date }, series.bucket, locale, {
                  compactSameDay: chart.compactSameDay,
                  showTime: chart.showAxisTime,
                })}
              </span>
            );
          })}
        </div>
      </div>

      <span className="sr-only" aria-live="polite">
        {srText}
      </span>
    </div>
  );
}

function TokenComposition({ overview }: { overview: UsageOverview }) {
  const { t, locale } = useI18n();
  const composition = calculateTokenComposition(overview);
  const unpricedRequests = Math.max(overview.totalRequests - overview.pricedRequests, 0);
  const pricingLabel = unpricedRequests > 0
    ? t('usage.token.pricingNeedsUpdate')
    : t('usage.token.pricing');
  const labels = {
    input: t('usage.token.uncachedInput'),
    'cache-read': t('usage.token.cacheRead'),
    'cache-creation': t('usage.token.cacheWrite'),
    output: t('usage.token.output'),
  };
  return (
    <section className="panel usage-health-panel" aria-labelledby="usage-token-title">
      <div className="usage-section-heading">
        <strong id="usage-token-title">{t('usage.token.title')}</strong>
      </div>
      <div className="usage-token-content">
        <div className="usage-token-composition">
          <div className="usage-token-donut">
            <svg viewBox="0 0 200 200" aria-hidden="true">
              <circle className="usage-token-donut-track" cx="100" cy="100" r="80" />
              {composition.segments.filter((segment) => segment.value > 0).map(({ key, percent, offset }) => {
                const gap = percent === 100 ? 0 : Math.min(0.7, percent / 6);
                return (
                  <circle
                    className={`usage-token-donut-segment tone-${key}`}
                    key={key}
                    cx="100"
                    cy="100"
                    r="80"
                    pathLength="100"
                    strokeDasharray={`${percent - gap} ${100 - percent + gap}`}
                    strokeDashoffset={-offset - gap / 2}
                    transform="rotate(-90 100 100)"
                  />
                );
              })}
            </svg>
            <div className="usage-token-donut-label">
              <strong>{composition.cacheShare.toFixed(1)}%</strong>
              <span>{t('usage.token.cacheShare')}</span>
            </div>
          </div>
          <dl className="usage-token-breakdown">
            {composition.segments.map(({ key, value, percent }) => (
              <div className={`usage-token-row tone-${key}`} key={key}>
                <dt className="usage-token-name"><i aria-hidden="true" />{labels[key]}</dt>
                <dd className="usage-token-measurement">
                  <div className="usage-token-vals">
                    <span className="usage-token-count" title={value.toLocaleString(locale)}>{compactNumber(value)}</span>
                    <span className="usage-token-pct">{percent.toFixed(1)}%</span>
                  </div>
                  <div className="usage-token-bar-track" aria-hidden="true">
                    <div className="usage-token-bar-fill" style={{ width: `${percent}%` }} />
                  </div>
                </dd>
              </div>
            ))}
          </dl>
        </div>
        <div className="usage-token-notes">
          <span>{t('usage.token.input')} <b>{compactNumber(overview.inputTokens)}</b></span>
          <span>{t('usage.token.output')} <b>{compactNumber(overview.outputTokens)}</b></span>
          <span title={t('usage.token.reasoningNote')}>{t('usage.token.reasoning')} <b>{compactNumber(overview.reasoningTokens)}</b></span>
          <span>{t('usage.stat.cacheHitRate')} <b>{(overview.cacheHitRate * 100).toFixed(1)}%</b></span>
        </div>
        <div className="usage-token-context">
          <section className="usage-token-context-group usage-token-results" aria-labelledby="usage-token-results-title">
            <h3 id="usage-token-results-title"><Activity size={14} aria-hidden="true" />{t('usage.token.requestStatus')}</h3>
            <dl className="usage-token-context-metrics">
              <div className="tone-success"><dt>{t('usage.result.success')}</dt><dd>{compactNumber(overview.successCount)}</dd></div>
              <div className="tone-failed"><dt>{t('usage.result.failed')}</dt><dd>{compactNumber(overview.failureCount)}</dd></div>
              <div><dt>{t('usage.result.canceled')}</dt><dd>{compactNumber(overview.canceledCount)}</dd></div>
            </dl>
          </section>
          <section className="usage-token-context-group" aria-labelledby="usage-token-performance-title">
            <h3 id="usage-token-performance-title"><Gauge size={14} aria-hidden="true" />{t('usage.token.performance')}</h3>
            <dl className="usage-token-context-metrics">
              <div><dt>{t('usage.stat.rpm')}</dt><dd>{overview.rpm.toFixed(2)}</dd></div>
              <div><dt>{t('usage.stat.averageLatency')}</dt><dd>{compactDuration(overview.averageLatencyMs)}</dd></div>
            </dl>
          </section>
          <section className="usage-token-context-group" aria-labelledby="usage-token-pricing-title" title={t('usage.stat.costNote')}>
            <h3 id="usage-token-pricing-title"><Wallet size={14} aria-hidden="true" />{pricingLabel}</h3>
            <dl className="usage-token-context-metrics">
              <div><dt>{t('usage.token.priced')}</dt><dd>{compactNumber(overview.pricedRequests)}</dd></div>
              <div><dt>{t('usage.token.unpriced')}</dt><dd>{compactNumber(unpricedRequests)}</dd></div>
            </dl>
          </section>
        </div>
      </div>
    </section>
  );
}

type PriceDraft = {
  model: string;
  prompt: string;
  completion: string;
  cacheRead: string;
  cacheCreation: string;
};

const emptyPriceDraft = (): PriceDraft => ({
  model: '',
  prompt: '',
  completion: '',
  cacheRead: '',
  cacheCreation: '',
});
const priceDraftFor = (model = '', price?: ModelPrice | null): PriceDraft => ({
  model,
  prompt: price ? String(price.prompt) : '',
  completion: price ? String(price.completion) : '',
  cacheRead: price ? String(price.cacheRead) : '',
  cacheCreation: price ? String(price.cacheCreation) : '',
});

const parsePrice = (value: string) => {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
};

const priceUnit = (value: number | undefined) => (Number.isFinite(value) ? `$${Number(value).toFixed(4)}` : '—');

function PricingView({
  pricing,
  query,
  onChanged,
}: {
  pricing: UsagePricing;
  query: UsageQuery;
  onChanged: () => void | Promise<void>;
}) {
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const { t } = useI18n();
  const [search, setSearch] = useState('');
  const [draft, setDraft] = useState<PriceDraft | null>(null);
  const [saving, setSaving] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [applyingSync, setApplyingSync] = useState(false);
  const [syncSource, setSyncSource] = useState<'models-dev' | 'litellm'>(() => {
    try { return usagePreferences.getItem('cpa-gui.pricing-sync-source.v1') === 'litellm' ? 'litellm' : 'models-dev'; }
    catch { return 'models-dev'; }
  });
  const [syncPreview, setSyncPreview] = useState<ModelPriceSyncPreview | null>(null);
  const [syncDrafts, setSyncDrafts] = useState<SyncPriceDraft[]>([]);
  const [syncError, setSyncError] = useState('');
  const syncDialog = useRef<HTMLDialogElement>(null);
  const { notice, revision, showNotice, clearNotice } = useAppNotice();


  const visibleRows = pricing.rows.filter((row) => {
    const keyword = search.trim().toLowerCase();
    return !keyword || row.model.toLowerCase().includes(keyword);
  });

  const savePrice = async () => {
    if (!draft?.model.trim()) {
      showNotice({ key: 'usage.pricing.modelRequired' }, 'error');
      return;
    }
    setSaving(true);
    clearNotice();
    try {
      await invoke('save_usage_model_price', {
        price: {
          model: draft.model.trim(),
          prompt: parsePrice(draft.prompt),
          completion: parsePrice(draft.completion),
          cacheRead: parsePrice(draft.cacheRead),
          cacheCreation: parsePrice(draft.cacheCreation),
          promptConfigured: draft.prompt.trim() !== '',
          completionConfigured: draft.completion.trim() !== '',
          cacheReadConfigured: draft.cacheRead.trim() !== '',
          cacheCreationConfigured: draft.cacheCreation.trim() !== '',
          source: 'manual',
          sourceModelId: '',
          updatedAtMs: 0,
        } satisfies ModelPrice,
      });
      setDraft(null);
      showNotice({ key: 'usage.pricing.saved' });
      await onChanged();
    } catch (saveError) {
      showNotice(String(saveError), 'error');
    } finally {
      setSaving(false);
    }
  };

  const deletePrice = async (model: string) => {
    if (!await askConfirmation({ title: t('common.delete'), message: t('usage.pricing.deleteConfirm', { model }), confirmText: t('common.delete'), variant: 'danger' })) return;
    clearNotice();
    try {
      await invoke('delete_usage_model_price', { model });
      showNotice({ key: 'usage.pricing.deleted' });
      await onChanged();
    } catch (deleteError) {
      showNotice(String(deleteError), 'error');
    }
  };

  const previewPrices = async () => {
    setSyncing(true);
    setSyncError('');
    clearNotice();
    try {
      const preview = await invoke<ModelPriceSyncPreview>('preview_usage_model_prices', { query, source: syncSource });
      const existing = new Map(pricing.rows.map((row) => [row.model.toLowerCase(), row.price]));
      setSyncPreview(preview);
      setSyncDrafts(preview.matches.map((price) => ({
        selected: existing.get(price.model.toLowerCase())?.source !== 'manual',
        price,
        prompt: String(price.prompt),
        completion: String(price.completion),
        cacheRead: String(price.cacheRead),
        cacheCreation: String(price.cacheCreation),
      })));
      syncDialog.current?.showModal();
    } catch (syncError) {
      showNotice(String(syncError), 'error');
    } finally {
      setSyncing(false);
    }
  };

  const applySyncPrices = async () => {
    const selected = syncDrafts.filter((item) => item.selected);
    if (!selected.length) return;
    const valid = (value: string) => value.trim() !== '' && Number.isFinite(Number(value)) && Number(value) >= 0;
    if (selected.some((item) => !valid(item.prompt) || !valid(item.completion)
      || (item.cacheRead.trim() !== '' && !valid(item.cacheRead))
      || (item.cacheCreation.trim() !== '' && !valid(item.cacheCreation)))) {
      setSyncError(t('usage.pricing.syncInvalid'));
      return;
    }
    const prices = selected.map((item): ModelPrice => ({
      ...item.price,
      prompt: Number(item.prompt),
      completion: Number(item.completion),
      cacheRead: item.cacheRead.trim() === '' ? 0 : Number(item.cacheRead),
      cacheCreation: item.cacheCreation.trim() === '' ? 0 : Number(item.cacheCreation),
      cacheReadConfigured: item.cacheRead.trim() !== '',
      cacheCreationConfigured: item.cacheCreation.trim() !== '',
    }));
    setApplyingSync(true);
    setSyncError('');
    clearNotice();
    try {
      const result = await invoke<ModelPriceSyncResult>('apply_usage_model_prices', { prices });
      syncDialog.current?.close();
      showNotice({ key: 'usage.pricing.syncResult', variables: { imported: result.imported } });
      await onChanged();
    } catch (syncError) {
      setSyncError(String(syncError));
    } finally {
      setApplyingSync(false);
    }
  };

  const updateSyncDraft = (index: number, update: Partial<SyncPriceDraft>) => {
    setSyncDrafts((current) => current.map((item, currentIndex) => currentIndex === index ? { ...item, ...update } : item));
  };

  const changeSyncSource = (source: 'models-dev' | 'litellm') => {
    setSyncSource(source);
    try { usagePreferences.setItem('cpa-gui.pricing-sync-source.v1', source); } catch {}
  };

  return (
    <section className="panel usage-pricing-panel">
      {confirmationDialog}
      <div className="usage-pricing-toolbar">
        <div className="usage-pricing-summary">
          <strong>{formatUsd(pricing.totalCost)}</strong>
          <span>
            {t('usage.pricing.coverage', {
              priced: compactNumber(pricing.pricedRequests),
              total: compactNumber(pricing.totalRequests),
              saved: compactNumber(pricing.savedPrices),
            })}
          </span>
        </div>
        <div className="usage-pricing-actions">
          <div className="usage-pricing-action-group">
            <input
              value={search}
              onChange={(event) => setSearch(event.currentTarget.value)}
              placeholder={t('usage.pricing.search')}
              aria-label={t('usage.pricing.search')}
            />
            <button type="button" className="secondary-button" onClick={() => setDraft(emptyPriceDraft())}>
              {t('usage.pricing.add')}
            </button>
          </div>
          <div className="usage-pricing-action-group">
            <select value={syncSource} onChange={(event) => changeSyncSource(event.currentTarget.value as 'models-dev' | 'litellm')} aria-label={t('usage.pricing.syncSource')}>
              <option value="models-dev">Models.dev</option>
              <option value="litellm">LiteLLM</option>
            </select>
            <button type="button" className="primary-button" disabled={syncing} onClick={() => void previewPrices()}>
              {syncing ? t('usage.pricing.syncing') : t('usage.pricing.sync')}
            </button>
          </div>
        </div>
      </div>

      <FloatingNotice key={revision} notice={notice} onDismiss={clearNotice} />

      <dialog ref={syncDialog} className="usage-price-sync-dialog" onCancel={(event) => { if (applyingSync) event.preventDefault(); }} onClose={() => setSyncPreview(null)}>
        <div className="usage-price-sync-header">
          <div>
            <h2>{t('usage.pricing.syncPreview')}</h2>
            <a href={syncPreview?.sourceUrl} target="_blank" rel="noreferrer">{syncPreview?.source}</a>
          </div>
          <button type="button" className="icon-button" aria-label={t('common.close')} disabled={applyingSync} onClick={() => syncDialog.current?.close()}><X size={16} /></button>
        </div>
        <p className="usage-price-sync-note">{t('usage.pricing.syncNote')}</p>
        <div className="usage-price-sync-controls">
          <span>{t('usage.pricing.syncCounts', { matched: syncDrafts.length, unmatched: syncPreview?.unmatched.length ?? 0 })}</span>
          <button type="button" className="secondary-button" onClick={() => setSyncDrafts((current) => current.map((item) => ({ ...item, selected: true })))}>{t('usage.pricing.selectAll')}</button>
          <button type="button" className="secondary-button" onClick={() => setSyncDrafts((current) => current.map((item) => ({ ...item, selected: false })))}>{t('usage.pricing.selectNone')}</button>
        </div>
        <div className="usage-price-sync-list">
          {syncDrafts.map((item, index) => (
            <div className="usage-price-sync-row" key={item.price.model}>
              <label className="usage-price-sync-identity">
                <input type="checkbox" checked={item.selected} disabled={applyingSync} onChange={(event) => updateSyncDraft(index, { selected: event.currentTarget.checked })} />
                <span><strong>{item.price.model}</strong><small>{item.price.sourceModelId}</small></span>
              </label>
              {(['prompt', 'completion', 'cacheRead', 'cacheCreation'] as const).map((field) => (
                <label key={field}>
                  <span>{t(`usage.pricing.${field}`)}</span>
                  <input type="number" min="0" step="any" value={item[field]} disabled={applyingSync}
                    onChange={(event) => updateSyncDraft(index, { [field]: event.currentTarget.value })} />
                </label>
              ))}
            </div>
          ))}
          {syncDrafts.length === 0 && <p>{t('usage.pricing.syncNoMatches')}</p>}
        </div>
        {(syncPreview?.unmatched.length ?? 0) > 0 && (
          <details className="usage-price-sync-unmatched">
            <summary>{t('usage.pricing.syncUnmatched', { count: syncPreview?.unmatched.length ?? 0 })}</summary>
            <p>{syncPreview?.unmatched.join(', ')}</p>
          </details>
        )}
        {syncError && <p className="usage-price-sync-error" role="alert">{syncError}</p>}
        <div className="usage-price-sync-footer">
          <button type="button" className="secondary-button" disabled={applyingSync} onClick={() => syncDialog.current?.close()}>{t('common.cancel')}</button>
          <button type="button" className="primary-button" disabled={applyingSync || !syncDrafts.some((item) => item.selected)} onClick={() => void applySyncPrices()}>
            {applyingSync ? t('usage.pricing.saving') : t('usage.pricing.applySelected', { count: syncDrafts.filter((item) => item.selected).length })}
          </button>
        </div>
      </dialog>

      {draft ? (
        <div className="usage-price-editor">
          <label>
            <span>{t('usage.pricing.model')}</span>
            <input
              value={draft.model}
              onChange={(event) => setDraft({ ...draft, model: event.currentTarget.value })}
              placeholder="gpt-5.6-terra"
            />
          </label>
          <label>
            <span>{t('usage.pricing.prompt')}</span>
            <input
              type="number"
              min="0"
              step="0.0001"
              value={draft.prompt}
              onChange={(event) => setDraft({ ...draft, prompt: event.currentTarget.value })}
            />
          </label>
          <label>
            <span>{t('usage.pricing.completion')}</span>
            <input
              type="number"
              min="0"
              step="0.0001"
              value={draft.completion}
              onChange={(event) => setDraft({ ...draft, completion: event.currentTarget.value })}
            />
          </label>
          <label>
            <span>{t('usage.pricing.cacheRead')}</span>
            <input
              type="number"
              min="0"
              step="0.0001"
              value={draft.cacheRead}
              onChange={(event) => setDraft({ ...draft, cacheRead: event.currentTarget.value })}
              placeholder={t('usage.pricing.optional')}
            />
          </label>
          <label>
            <span>{t('usage.pricing.cacheCreation')}</span>
            <input
              type="number"
              min="0"
              step="0.0001"
              value={draft.cacheCreation}
              onChange={(event) => setDraft({ ...draft, cacheCreation: event.currentTarget.value })}
              placeholder={t('usage.pricing.optional')}
            />
          </label>
          <div className="usage-price-editor-actions">
            <button type="button" className="secondary-button" onClick={() => setDraft(null)}>
              <X size={14} />
              {t('common.cancel')}
            </button>
            <button type="button" className="primary-button" disabled={saving} onClick={() => void savePrice()}>
              {saving ? t('usage.pricing.saving') : t('common.save')}
            </button>
          </div>
        </div>
      ) : null}

      {visibleRows.length ? (
        <div className="usage-table-wrap usage-pricing-table-wrap" tabIndex={0} role="region" aria-label={t('usage.tab.pricing')}>
          <table className="usage-pricing-table">
            <thead>
              <tr>
                <th>{t('usage.pricing.model')}</th>
                <th>{t('usage.pricing.calls')}</th>
                <th>{t('usage.unit.tokens')}</th>
                <th>{t('usage.pricing.cost')}</th>
                <th>{t('usage.pricing.prompt')}</th>
                <th>{t('usage.pricing.completion')}</th>
                <th>{t('usage.pricing.cacheRead')}</th>
                <th>{t('usage.pricing.cacheCreation')}</th>
                <th>{t('usage.pricing.actions')}</th>
              </tr>
            </thead>
            <tbody>
              {visibleRows.map((row) => (
                <tr key={row.model}>
                  <td>
                    <strong title={row.model}>{row.model}</strong>
                  </td>
                  <td>{compactNumber(row.requests)}</td>
                  <td>{compactNumber(row.totalTokens)}</td>
                  <td>
                    <strong>{row.price ? formatUsd(row.estimatedCost) : '—'}</strong>
                  </td>
                  <td>{row.price ? priceUnit(row.price.prompt) : '—'}</td>
                  <td>{row.price ? priceUnit(row.price.completion) : '—'}</td>
                  <td>{row.price ? priceUnit(row.price.cacheRead) : '—'}</td>
                  <td>{row.price ? priceUnit(row.price.cacheCreation) : '—'}</td>
                  <td>
                    <div className="usage-price-row-actions">
                      <button
                        type="button"
                        className="icon-button"
                        title={t('common.edit')}
                        aria-label={`${t('common.edit')}: ${row.model}`}
                        onClick={() => setDraft(priceDraftFor(row.model, row.price))}
                      >
                        <Pencil size={14} aria-hidden="true" />
                      </button>
                      {row.price?.source === 'manual' ? (
                        <button
                          type="button"
                          className="icon-button danger"
                          title={t('common.delete')}
                          aria-label={`${t('common.delete')}: ${row.model}`}
                          onClick={() => void deletePrice(row.model)}
                        >
                          <Trash2 size={14} aria-hidden="true" />
                        </button>
                      ) : null}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <UsageEmpty />
      )}
    </section>
  );
}

function UsageEmpty() {
  const { t } = useI18n();
  return (
    <div className="usage-empty">
      <TriangleAlert size={18} />
      <span>{t('usage.empty')}</span>
    </div>
  );
}
