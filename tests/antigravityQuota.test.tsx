import { describe, expect, it } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nProvider } from '../src/i18n';
import { QuotaCard } from '../src/pages/QuotaPage';
import { quotaRowsFor } from '../src/services/quotaService';
import { antigravityDescription } from '../src/services/antigravityQuotaLabels';

describe('Antigravity current quota response', () => {
  const rowsFor = (weekly: number) => quotaRowsFor('antigravity', { groups: [{
    displayName: 'Gemini Models',
    description: 'Models within this group: Gemini Flash, Gemini Pro',
    buckets: [
      { displayName: 'Weekly Limit Remaining', remainingFraction: weekly },
      { displayName: 'Five Hour Limit Remaining', remainingFraction: 1, resetTime: '2030-01-01T00:00:00Z' },
    ],
  }] });

  it('recognizes new labels without window fields and preserves model details', () => {
    const rows = rowsFor(0.4);
    expect(rows.map(row => row.label)).toEqual(['Gemini models · 5-hour limit', 'Gemini models · Weekly limit']);
    expect(rows[0].quotaAvailable).toBe(true);
    expect(rows[0].resetAtMs).toBe(Date.parse('2030-01-01T00:00:00Z'));
    expect(rows[0].detail).toContain('Gemini Flash, Gemini Pro, Gemini Argon');
    expect(antigravityDescription('Gemini Models', 'Models within this group: gemini argon')).toBe('Models within this group: gemini argon');
    expect(antigravityDescription('Unknown models', 'Custom description')).toBe('Custom description');
  });

  it('does not claim availability when the weekly quota is exhausted', () => {
    const rows = rowsFor(0);
    expect(rows[0].remainingPercent).toBe(100);
    expect(rows[0].quotaAvailable).toBe(false);
    const render = (weekly: number) => renderToStaticMarkup(
      <I18nProvider><QuotaCard file={{ name: 'test.json', provider: 'antigravity' }}
        quota={{ status: 'success', rows: rowsFor(weekly) }} onRefresh={() => {}} onReset={() => {}} /></I18nProvider>,
    );
    expect(render(0)).not.toContain('Quota available');
    expect(render(0)).toContain('100% remaining');
    expect(render(0.4)).toContain('Quota available');
  });
});
