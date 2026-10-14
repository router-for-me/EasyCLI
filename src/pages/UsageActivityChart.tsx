import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { useI18n } from '../i18n';
import {
  buildUsageTrendSeries,
  formatTrendAxisLabel,
  formatTrendRangeLabel,
  niceCeiling,
  trendAxisTicks,
  trendPointIndexAtRatio,
  trendTimeAxisTicks,
  trendTimePosition,
  type PreparedTrendPoint,
  type UsageTimelinePoint,
} from '../services/usageTrend';
import './UsageActivityChart.css';

type UsageActivityChartProps = {
  timeline: UsageTimelinePoint[];
  range?: { start?: string; end?: string };
};

// Merge neighboring intervals on narrow screens without dropping their requests.
function mergeIntervals(points: PreparedTrendPoint[], limit: number): PreparedTrendPoint[] {
  const groupSize = Math.max(1, Math.ceil(points.length / limit));
  if (groupSize === 1) return points;
  const merged: PreparedTrendPoint[] = [];
  for (let index = 0; index < points.length; index += groupSize) {
    const group = points.slice(index, index + groupSize);
    merged.push({
      ...group[0],
      end: group[group.length - 1].end,
      requests: group.reduce((sum, point) => sum + point.requests, 0),
      success: group.reduce((sum, point) => sum + point.success, 0),
      failure: group.reduce((sum, point) => sum + point.failure, 0),
      canceled: group.reduce((sum, point) => sum + point.canceled, 0),
      tokens: group.reduce((sum, point) => sum + point.tokens, 0),
      models: {},
    });
  }
  return merged;
}

export function UsageActivityChart({ timeline, range }: UsageActivityChartProps) {
  const { t, locale, formatNumber } = useI18n();
  const titleId = useId();
  const detailId = useId();
  const plotRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(760);
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const [hasFocus, setHasFocus] = useState(false);
  const series = useMemo(
    () => buildUsageTrendSeries(timeline, range),
    [timeline, range?.start, range?.end],
  );

  useLayoutEffect(() => {
    const plot = plotRef.current;
    if (!plot) return;
    const measure = () => setWidth(Math.max(180, Math.round(plot.getBoundingClientRect().width)));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(plot);
    return () => observer.disconnect();
  }, []);

  const chart = useMemo(() => {
    const left = 40;
    const right = width - 8;
    const top = 14;
    const baseline = 128;
    const plotWidth = right - left;
    const points = mergeIntervals(series.points, Math.max(12, Math.min(72, Math.floor(plotWidth / 10))));
    const start = points[0]?.start ?? new Date(0);
    const end = points[points.length - 1]?.end ?? start;
    const maximum = niceCeiling(points.reduce((value, point) => Math.max(value, point.requests, point.success + point.failure + point.canceled), 1));
    const peakIndex = points.reduce((best, point, index) => point.requests > (points[best]?.requests ?? -1) ? index : best, 0);
    const compactSameDay = start.toDateString() === end.toDateString();
    const timeTicks = points.length ? trendTimeAxisTicks(start, end, plotWidth, compactSameDay ? 68 : 110) : [];
    const showTime = timeTicks.length > 1 && timeTicks[1].getTime() - timeTicks[0].getTime() < 86400000;
    const bars = points.map((point) => {
      const x0 = left + trendTimePosition(point.start, start, end) * plotWidth;
      const x1 = left + trendTimePosition(point.end, start, end) * plotWidth;
      const gap = Math.min(6, (x1 - x0) * 0.23);
      const barWidth = Math.max(1, Math.min(40, x1 - x0 - gap));
      let sum = 0;
      const layers = [
        { status: 'success', value: point.success },
        { status: 'failed', value: point.failure },
        { status: 'canceled', value: point.canceled },
      ].map((layer) => {
        sum += layer.value;
        return { ...layer, y: baseline - sum / maximum * (baseline - top), height: layer.value / maximum * (baseline - top) };
      });
      // A partially selected edge interval can be narrower than our 1px bar.
      // Keep that visible minimum inside the plot instead of crossing its edge.
      const x = Math.max(left, Math.min(right - barWidth, (x0 + x1 - barWidth) / 2));
      return { x, width: barWidth, center: (x0 + x1) / 2, left: x0, right: x1, layers };
    });
    return { points, left, right, top, baseline, plotWidth, start, end, maximum, peakIndex, compactSameDay, timeTicks, showTime, bars };
  }, [series, width]);

  useEffect(() => setActiveIndex(null), [series, chart.points.length]);

  const active = activeIndex === null ? null : chart.points[activeIndex];
  const activeBar = activeIndex === null ? null : chart.bars[activeIndex];
  const statusItems = [
    { status: 'success', label: t('usage.result.success'), count: active?.success ?? series.totals.success },
    { status: 'failed', label: t('usage.result.failed'), count: active?.failure ?? series.totals.failures },
    { status: 'canceled', label: t('usage.result.canceled'), count: active?.canceled ?? series.totals.canceled },
  ];
  const detailText = active
    ? `${formatTrendRangeLabel(active, locale, series.bucket)} · ${t('usage.analysis.activityTotal', { count: formatNumber(active.requests) })} · ${statusItems.map((item) => `${item.label} ${formatNumber(item.count)}`).join(' · ')}`
    : t('usage.analysis.activityTotal', { count: formatNumber(series.totals.requests) });
  const handlePointerMove = (event: PointerEvent<SVGSVGElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const x = (event.clientX - bounds.left) / bounds.width * width;
    if (x < chart.left || x > chart.right || !chart.points.length) {
      if (!hasFocus) setActiveIndex(null);
      return;
    }
    setActiveIndex(trendPointIndexAtRatio(chart.points, chart.start, chart.end, (x - chart.left) / chart.plotWidth));
  };
  const handleKeyDown = (event: KeyboardEvent<SVGSVGElement>) => {
    if (!chart.points.length) return;
    let next: number;
    if (event.key === 'ArrowRight') next = Math.min(chart.points.length - 1, activeIndex === null ? 0 : activeIndex + 1);
    else if (event.key === 'ArrowLeft') next = Math.max(0, activeIndex === null ? chart.points.length - 1 : activeIndex - 1);
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = chart.points.length - 1;
    else if (event.key === 'Escape') {
      setActiveIndex(null);
      return;
    } else return;
    event.preventDefault();
    setActiveIndex(next);
  };

  return (
    <section className="usage-activity-card" aria-labelledby={titleId}>
      <div className="usage-activity-heading">
        <div className="usage-activity-title">
          <h3 id={titleId}>{t('usage.analysis.activityTitle')}</h3>
          <p>{t('usage.analysis.activityHint')}</p>
        </div>
        <div className="usage-activity-summary">
          <strong>{t('usage.analysis.activityTotal', { count: formatNumber(series.totals.requests) })}</strong>
          <div className="usage-activity-legend" aria-hidden="true">
            {statusItems.map((item) => <span key={item.status}><i className={`usage-activity-dot usage-activity-${item.status}`} />{item.label}</span>)}
          </div>
        </div>
      </div>
      <div className="usage-activity-plot" ref={plotRef}>
        <svg
          className="usage-activity-svg"
          viewBox={`0 0 ${width} 154`}
          role="img"
          tabIndex={chart.points.length ? 0 : undefined}
          aria-label={t('usage.analysis.chartLabel')}
          aria-describedby={detailId}
          onPointerMove={handlePointerMove}
          onPointerLeave={() => { if (!hasFocus) setActiveIndex(null); }}
          onFocus={() => { setHasFocus(true); setActiveIndex((current) => current ?? chart.peakIndex); }}
          onBlur={() => { setHasFocus(false); setActiveIndex(null); }}
          onKeyDown={handleKeyDown}
        >
          {trendAxisTicks(chart.maximum, 3).map((tick) => {
            const y = chart.baseline - tick / chart.maximum * (chart.baseline - chart.top);
            return <g key={tick} className="usage-activity-axis"><line x1={chart.left} x2={chart.right} y1={y} y2={y} /><text x={chart.left - 10} y={y + 3} textAnchor="end">{formatNumber(tick, { notation: 'compact', maximumFractionDigits: 1 })}</text></g>;
          })}
          {activeBar && <rect className="usage-activity-highlight" x={activeBar.left} y={chart.top - 5} width={Math.max(0, activeBar.right - activeBar.left)} height={chart.baseline - chart.top + 5} rx="3" />}
          {chart.bars.map((bar, index) => <g key={chart.points[index].hour} className="usage-activity-bar" data-requests={chart.points[index].requests} data-success={chart.points[index].success} data-failure={chart.points[index].failure} data-canceled={chart.points[index].canceled}>
            {bar.layers.filter((layer) => layer.value > 0).map((layer) => <rect key={layer.status} className={`usage-activity-${layer.status}`} x={bar.x} y={layer.y} width={bar.width} height={layer.height} rx="1" />)}
          </g>)}
          {chart.timeTicks.map((tick, index) => <text key={tick.getTime()} className="usage-activity-time" x={chart.left + trendTimePosition(tick, chart.start, chart.end) * chart.plotWidth} y="149" textAnchor={index === 0 ? 'start' : index === chart.timeTicks.length - 1 ? 'end' : 'middle'}>{formatTrendAxisLabel({ start: tick }, series.bucket, locale, { compactSameDay: chart.compactSameDay, showTime: chart.showTime })}</text>)}
        </svg>
        {series.totals.requests === 0 && <div className="usage-activity-empty">{t('usage.empty')}</div>}
        {active && activeBar && <div className="usage-activity-tooltip" data-theme-surface="overlay" style={{ left: `${Math.max(0, Math.min(width - Math.min(224, width), activeBar.center - 112))}px` }} aria-hidden="true">
          <strong>{formatTrendRangeLabel(active, locale, series.bucket)}</strong>
          <span className="usage-activity-tooltip-total">{t('usage.analysis.activityTotal', { count: formatNumber(active.requests) })}</span>
          {statusItems.map((item) => <div key={item.status}><span><i className={`usage-activity-dot usage-activity-${item.status}`} />{item.label}</span><b>{formatNumber(item.count)}</b></div>)}
        </div>}
      </div>
      <span id={detailId} className="sr-only" aria-live="polite">{detailText}</span>
    </section>
  );
}
