import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { I18nProvider } from '../../src/i18n';
import { UsageRecordsPage } from '../../src/pages/UsageRecordsPage';
import '../../src/styles/index.css';

const params = new URLSearchParams(location.search);
localStorage.setItem('easy-cli-proxy-api.locale', params.get('locale') || 'zh-TW');
localStorage.setItem('cpa-gui.usage-records-tab.v1', params.get('tab') || 'overview');
localStorage.setItem('cpa-gui.usage-records-range.v1', '4h');
document.documentElement.dataset.theme = params.get('theme') || 'dark';

const hourKey = (date: Date) => [
  date.getFullYear(),
  String(date.getMonth() + 1).padStart(2, '0'),
  String(date.getDate()).padStart(2, '0'),
  String(date.getHours()).padStart(2, '0'),
].join('-');
const now = new Date();
const trendModels = [
  'gpt-test',
  'claude-test',
  'gemini-test',
  'deepseek-test',
  'grok-test',
  'qwen-test',
  'mistral-test',
  'llama-test',
];
const timeline = Array.from({ length: 4 }, (_, index) => ({
  hour: hourKey(new Date(now.getTime() - (3 - index) * 60 * 60 * 1000)),
  requests: 12 + index,
  success: 11 + index,
  failure: 1,
  canceled: 0,
  tokens: 22_000 + index * 2_000,
  models: trendModels.map((key, modelIndex) => ({
    key,
    label: key,
    tokens: 1_700 + modelIndex * 300 + index * 250,
  })),
}));

const records = Array.from({ length: 400 }, (_, index) => ({
  id: `record-${index + 1}`,
  row_id: String(index + 1),
  timestamp: new Date(now.getTime() - index * 1000).toISOString(),
  latency_ms: 2000,
  ttft_ms: 200,
  source: index % 2 ? 'secondary-source' : 'test-source',
  auth_index: index % 2 ? 'auth-secondary' : 'auth-primary',
  source_display: index % 2 ? 'Secondary source' : 'Test source',
  failed: index % 4 === 0,
  canceled: false,
  failure_status: index % 4 === 0 ? 429 : 0,
  failure_body: index % 4 === 0 ? 'Rate limit exceeded' : '',
  provider: 'test-provider',
  auth_type: index % 2 ? 'oauth' : 'apikey',
  model: index % 3 ? 'test-model' : 'secondary-model',
  alias: '',
  reasoning_effort: 'high',
  endpoint: '/v1/responses',
  api_key_hash: 'test-key',
  api_key_display: params.has('noRequestKey') ? '' : 'sk-test',
  api_key_remark: params.has('noRequestKey') ? '' : 'Test key, "local"',
  tokens: {
    input_tokens: 1000,
    output_tokens: 200,
    reasoning_tokens: 100,
    cache_read_tokens: 400,
    cache_creation_tokens: 50,
    total_tokens: 1200,
  },
}));

const category = (key: string, label = key) => ({ key, label, requests: 200, failures: 25, tokens: 24_000 });
const usageKeyFixture = {
  apiKeys: [category('test-key', 'Test key')],
  lastEventKey: '',
};
Object.assign(window, { usageKeyFixture });

mockIPC(async (cmd, args) => {
  if (cmd === 'plugin:event|listen') return 1;
  if (cmd === 'plugin:event|unlisten' || cmd === 'set_app_locale') return null;
  if (cmd === 'get_usage_collector_status') {
    return { state: 'collecting', message: '', lastCollectedAt: now.toISOString(), totalRecords: 54 };
  }
  if (cmd === 'get_usage_analysis') {
    return {
      models: [category('test-model'), category('secondary-model')],
      providers: [category('test-provider')],
      sources: [category('test-source', 'Test source'), category('secondary-source', 'Secondary source')],
      apiKeys: usageKeyFixture.apiKeys,
      upstreams: [
        { ...category('auth:auth-primary', 'Primary credential'), provider: 'codex' },
        { ...category('auth:auth-secondary', 'Secondary credential'), provider: 'claude' },
      ],
    };
  }
  if (cmd === 'get_usage_events') {
    const query = args?.query as {
      page: number; page_size: number; model?: string; source?: string;
      provider?: string; upstream?: string; api_key_hash?: string; failed?: boolean; canceled?: boolean;
    };
    const filtered = records.filter(record =>
      (!query.model || record.model === query.model)
      && (!query.upstream || `auth:${record.auth_index}` === query.upstream)
      && (!query.source || record.source === query.source)
      && (!query.provider || record.provider === query.provider)
      && (!query.api_key_hash || record.api_key_hash === query.api_key_hash)
      && (query.failed === undefined || record.failed === query.failed)
      && (query.canceled === undefined || record.canceled === query.canceled));
    usageKeyFixture.lastEventKey = query.api_key_hash || '';
    const start = (query.page - 1) * query.page_size;
    return {
      page: query.page,
      pageSize: query.page_size,
      total: filtered.length,
      totalPages: Math.max(1, Math.ceil(filtered.length / query.page_size)),
      items: filtered.slice(start, start + query.page_size),
    };
  }
  if (cmd === 'get_usage_overview') {
    return {
      totalRequests: 54,
      successCount: 50,
      failureCount: 4,
      canceledCount: 0,
      successRate: 92.6,
      inputTokens: 52_000,
      outputTokens: 24_000,
      reasoningTokens: 8_000,
      cacheReadTokens: 16_000,
      cacheCreationTokens: 0,
      totalTokens: 100_000,
      rpm: 0.23,
      tpm: 416.7,
      tps: 31.2,
      tpsSampleCount: 54,
      averageLatencyMs: 2_840,
      cacheHitRate: 0.307,
      estimatedCost: 1.2345,
      pricedRequests: 54,
      timeline,
    };
  }
  throw new Error(`Unhandled fixture command: ${cmd}`);
});

createRoot(document.getElementById('root')!).render(
  <I18nProvider>
    <div className="app-shell">
      <aside className="sidebar" aria-hidden="true" />
      <div className="workspace">
        <main className="content">
          <UsageRecordsPage />
        </main>
      </div>
    </div>
  </I18nProvider>,
);
