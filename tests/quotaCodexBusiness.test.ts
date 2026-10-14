import { afterEach, expect, it, spyOn } from 'bun:test';
import { managementApi } from '../src/services/managementApi';
import { loadQuota, quotaRowsFor } from '../src/services/quotaService';
import payload from './fixtures/codex-business-spend-control.json';

afterEach(() => { mock?.mockRestore(); });
let mock: ReturnType<typeof spyOn> | undefined;

it('recognizes the anonymized Business spend-control quota', () => {
  expect(quotaRowsFor('codex', payload)).toMatchObject([
    { remainingPercent: 74, resetAtMs: 1893456000000 },
  ]);
});

it('loads the anonymized Business HTTP 200 response without the unrecognized-data error', async () => {
  mock = spyOn(managementApi, 'post').mockImplementation(async (_path, body) => {
    const url = (body as { url: string }).url;
    return { status_code: 200, body: JSON.stringify(url.endsWith('/usage') ? payload : { available_count: 0 }) } as never;
  });
  const result = await loadQuota({ name: 'business-fixture.json', provider: 'codex', auth_index: 'fixture' });
  expect(result).toMatchObject({ status: 'success', plan: 'business', rows: [{ remainingPercent: 74 }] });
});

it('keeps ordinary Codex windows alongside spend controls', () => {
  const rows = quotaRowsFor('codex', {
    ...payload,
    rate_limit: { primary_window: { used_percent: 10, limit_window_seconds: 18000 } },
  });
  expect(rows.map(row => row.remainingPercent)).toEqual([90, 74]);
});

it('preserves explicit zero remaining rather than falling back to used percent', () => {
  expect(quotaRowsFor('codex', {
    spend_control: { individual_limit: { remaining_percent: 0, used_percent: 30 } },
  })[0].remainingPercent).toBe(0);
});

it('falls back to used percent, then to the numeric credit ratio', () => {
  expect(quotaRowsFor('codex', {
    spend_control: { individual_limit: { used_percent: '26' } },
  })[0].remainingPercent).toBe(74);
  expect(quotaRowsFor('codex', {
    spend_control: { individual_limit: { remaining: '7400.5', limit: '10000' } },
  })[0].remainingPercent).toBeCloseTo(74.005);
});

it('does not turn null balance or absent spend controls into unlimited quota', () => {
  expect(quotaRowsFor('codex', { rate_limit: null, credits: payload.credits, spend_control: null })).toEqual([]);
  expect(quotaRowsFor('codex', {
    spend_control: { individual_limit: { remaining: null, limit: '0' } },
  })[0].remainingPercent).toBeNull();
});

it('shows the reported credit amounts without inventing a window duration', () => {
  const row = quotaRowsFor('codex', payload)[0];
  expect(row.label).toBe('Individual spending limit');
  expect(row.detail).toBe('Used 2,600 / 10,000 credit');
});
