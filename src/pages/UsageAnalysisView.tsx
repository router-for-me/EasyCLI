import { useState, type CSSProperties } from 'react';
import { BarChart3, CheckCircle2, ChevronDown, ChevronUp } from 'lucide-react';
import { useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';
import { formatUsageNumber } from '../services/usageNumber';
import type { UsageTimelinePoint } from '../services/usageTrend';
import { UsageActivityChart } from './UsageActivityChart';
import './UsageAnalysisView.css';

export type UsageCategory = {
  key: string;
  label: string;
  requests: number;
  failures: number;
  tokens: number;
};

export type UsageAnalysis = {
  models: UsageCategory[];
  providers: UsageCategory[];
  sources: UsageCategory[];
  apiKeys: UsageCategory[];
};

type AnalysisOverview = {
  totalRequests: number;
  successCount: number;
  failureCount: number;
  canceledCount: number;
  timeline: UsageTimelinePoint[];
};

const MODEL_COLORS = ['#528bdf', '#8870d8', '#2aa88b', '#e5a136', '#d579a6'];
const sum = (items: UsageCategory[], metric: 'tokens' | 'requests' | 'failures') =>
  items.reduce((total, item) => total + item[metric], 0);

export function UsageAnalysisView({ analysis, overview, range }: {
  analysis: UsageAnalysis;
  overview: AnalysisOverview | null;
  range: { start?: string; end?: string };
}) {
  const { t } = useI18n();
  if (!overview?.totalRequests) {
    return <div className="usage-analysis-grid usage-analysis-view usage-analysis-empty">
      <BarChart3 size={32} aria-hidden="true" /><p>{t('usage.empty')}</p>
    </div>;
  }
  return (
    <div className="usage-analysis-grid usage-analysis-view">
      <div className="usage-analysis-primary">
        <ModelComparison models={analysis.models} />
        <RequestResults overview={overview} models={analysis.models} />
      </div>
      <UsageActivityChart timeline={overview.timeline} range={range} />
      <section className="usage-analysis-context">
        <CategorySummary title={t('usage.analysis.sources')} items={analysis.sources} />
        <CategorySummary title={t('usage.analysis.keys')} items={analysis.apiKeys} />
      </section>
    </div>
  );
}

function ModelComparison({ models }: { models: UsageCategory[] }) {
  const { t, locale, formatNumber } = useI18n();
  const [metric, setMetric] = useState<'tokens' | 'requests'>('tokens');
  const [expanded, setExpanded] = useState(false);
  const ranked = [...models].sort((left, right) => right[metric] - left[metric]
    || right.requests - left.requests || left.key.localeCompare(right.key));
  const total = sum(models, metric);
  const remaining = ranked.slice(5);
  const visible: Array<UsageCategory & { aggregate?: boolean }> = !expanded && ranked.length > 6
    ? [...ranked.slice(0, 5), {
      key: '', label: t('usage.analysis.otherModels', { count: remaining.length }),
      tokens: sum(remaining, 'tokens'), requests: sum(remaining, 'requests'),
      failures: sum(remaining, 'failures'), aggregate: true,
    }]
    : ranked;
  const colorOrder = [...models].sort((a, b) => b.tokens - a.tokens || a.key.localeCompare(b.key));
  return (
    <section className="usage-analysis-card usage-model-comparison">
      <header className="usage-analysis-heading">
        <div><h2>{t('usage.analysis.modelTitle')}</h2><p>{t('usage.analysis.modelHint')}</p></div>
        <div className="usage-analysis-metric" role="group" aria-label={t('usage.analysis.metricLabel')}>
          {(['tokens', 'requests'] as const).map(value => <button key={value} type="button"
            aria-pressed={metric === value} onClick={() => setMetric(value)}>
            {t(value === 'tokens' ? 'usage.analysis.metricTokens' : 'usage.analysis.metricRequests')}
          </button>)}
        </div>
      </header>
      <div className="usage-model-bars">
        {visible.map(item => {
          const share = total > 0 ? item[metric] / total * 100 : 0;
          const color = item.aggregate ? '#94a3b8' : MODEL_COLORS[colorOrder.findIndex(model => model.key === item.key) % MODEL_COLORS.length];
          return <div className="usage-model-bar-row" key={item.aggregate ? 'aggregate' : `model:${item.key}`}
            data-model-key={item.aggregate ? undefined : item.key} data-metric-value={item[metric]}
            style={{ '--model-color': color } as CSSProperties}>
            <div className="usage-model-bar-label">
              <span className="usage-model-name" title={item.label}><i aria-hidden="true" /><span>{item.label}</span></span>
              <strong title={formatNumber(item[metric])}>{formatUsageNumber(item[metric], locale)}</strong>
            </div>
            <div className="usage-model-bar-detail">
              <span className="usage-model-request-count">{metric === 'tokens'
                ? t('usage.analysis.requestCount', { count: formatUsageNumber(item.requests, locale) })
                : `${formatUsageNumber(item.tokens, locale)} Token`}</span>
              <span className="usage-model-bar-track" aria-hidden="true"><span style={{ width: `${share}%` }} /></span>
              <span className="usage-model-share" aria-label={t('usage.analysis.shareLabel')}>{formatNumber(share, { maximumFractionDigits: 1 })}%</span>
            </div>
          </div>;
        })}
        {!models.length && <p className="usage-analysis-muted">{t('usage.empty')}</p>}
      </div>
      {models.length > 6 && <button className="usage-analysis-expand" type="button" aria-expanded={expanded}
        onClick={() => setExpanded(value => !value)}>
        {t(expanded ? 'usage.analysis.showLess' : 'usage.analysis.showAll', { count: models.length })}
        {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
      </button>}
    </section>
  );
}

function RequestResults({ overview, models }: { overview: AnalysisOverview; models: UsageCategory[] }) {
  const { t, locale, formatNumber } = useI18n();
  const statuses = [
    { key: 'success', label: t('usage.result.success'), value: overview.successCount, color: 'var(--analysis-success)' },
    { key: 'failed', label: t('usage.result.failed'), value: overview.failureCount, color: 'var(--analysis-failed)' },
    { key: 'canceled', label: t('usage.result.canceled'), value: overview.canceledCount, color: 'var(--analysis-canceled)' },
  ];
  const total = statuses.reduce((value, status) => value + status.value, 0);
  const completed = overview.successCount + overview.failureCount;
  const rate = completed > 0 ? overview.successCount / completed * 100 : null;
  const failures = models.filter(model => model.failures > 0).sort((a, b) => b.failures - a.failures || b.requests - a.requests).slice(0, 3);
  const circumference = 2 * Math.PI * 46;
  let offset = 0;
  return (
    <section className="usage-analysis-card usage-request-results">
      <header className="usage-analysis-heading">
        <div><h2>{t('usage.analysis.resultsTitle')}</h2><p>{t('usage.analysis.resultsHint')}</p></div>
        <span className="usage-analysis-total">{t('usage.analysis.requestCount', { count: formatUsageNumber(overview.totalRequests, locale) })}</span>
      </header>
      <div className="usage-results-chart">
        <div className="usage-results-ring">
          <svg viewBox="0 0 120 120" role="img" aria-label={t('usage.analysis.resultChart', {
            success: formatNumber(overview.successCount), failed: formatNumber(overview.failureCount), canceled: formatNumber(overview.canceledCount),
          })}>
            <circle cx="60" cy="60" r="46" fill="none" stroke="var(--bg-tertiary)" strokeWidth="11" />
            {statuses.map(status => {
              const length = total > 0 ? status.value / total * circumference : 0;
              const start = offset;
              offset += length;
              return length > 0 && <circle key={status.key} cx="60" cy="60" r="46" fill="none"
                stroke={status.color} strokeWidth="11" strokeDasharray={`${length} ${circumference - length}`}
                strokeDashoffset={-start} />;
            })}
          </svg>
          <div className="usage-results-center"><span>{t('usage.analysis.successRate')}</span>
            <strong>{rate === null ? '—' : `${formatNumber(rate, { maximumFractionDigits: 1 })}%`}</strong>
            <small>{t('usage.analysis.excludingCanceled')}</small></div>
        </div>
        <dl className="usage-results-legend">
          {statuses.map(status => <div key={status.key}>
            <dt><i style={{ background: status.color }} aria-hidden="true" />{status.label}</dt>
            <dd title={formatNumber(status.value)}>{formatUsageNumber(status.value, locale)}<span>{formatNumber(total > 0 ? status.value / total * 100 : 0, { maximumFractionDigits: 1 })}%</span></dd>
          </div>)}
        </dl>
      </div>
      <div className="usage-failure-models">
        {failures.length ? <>
          <div className="usage-failure-heading"><h3>{t('usage.analysis.failureModels')}</h3><span>{t('usage.result.failed')} / {t('usage.analysis.metricRequests')}</span></div>
          {failures.map(model => <div className="usage-failure-model" key={model.key}>
            <span title={model.label}>{model.label}</span><strong title={t('usage.analysis.failureMeta', {
              count: formatNumber(model.failures), rate: formatNumber(model.requests ? model.failures / model.requests * 100 : 0, { maximumFractionDigits: 1 }),
            })}>{formatNumber(model.failures)}<small> / {formatNumber(model.requests)}</small></strong>
          </div>)}
        </> : <div className="usage-analysis-no-failures"><CheckCircle2 size={21} aria-hidden="true" />
          <div><strong>{t('usage.analysis.noFailures')}</strong><p>{t('usage.analysis.noFailuresHint')}</p></div>
        </div>}
      </div>
    </section>
  );
}

function CategorySummary({ title, items }: { title: string; items: UsageCategory[] }) {
  const { t, locale, formatNumber } = useI18n();
  const privacy = usePrivacyMask();
  const ranked = [...items].sort((a, b) => b.requests - a.requests || b.tokens - a.tokens)
    .map((item) => ({ ...item, label: privacy.text(item.label) }));
  const total = sum(items, 'requests');
  const first = ranked[0];
  return <details className="usage-context-detail">
    <summary>
      <div className="usage-context-heading"><span>{title}</span><small>{t('usage.analysis.categoryCount', { count: items.length })}</small></div>
      <strong title={first?.label}>{first?.label || '—'}</strong>
      <div className="usage-context-caption"><span>{items.length === 1 ? t('usage.analysis.singleCategory')
        : first ? t('usage.analysis.leadingShare', { percent: formatNumber(total ? first.requests / total * 100 : 0, { maximumFractionDigits: 1 }) }) : '—'}</span><ChevronDown size={14} aria-hidden="true" /></div>
    </summary>
    <ul>{ranked.map(item => <li key={item.key}><span title={item.label}>{item.label}</span>
      <strong title={formatNumber(item.requests)}>{t('usage.analysis.requestCount', { count: formatUsageNumber(item.requests, locale) })}</strong></li>)}</ul>
  </details>;
}
