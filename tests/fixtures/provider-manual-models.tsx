import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { I18nProvider } from '../../src/i18n';
import { ApiAccessPage, providerRemarkIdentity, type ApiAccessRemarkLocator, type ProviderSection } from '../../src/pages/ApiAccessPage';
import '../../src/styles/index.css';

localStorage.setItem('easy-cli-proxy-api.locale', 'en');
const fixture = window as typeof window & {
  providerFixture: { records: Record<string, unknown>[]; discoveryCalls: number; writes: { method: string; body: unknown }[] };
};
fixture.providerFixture = {
  records: [{
    name: 'First route', keys: [{ 'api-key': 'shared-test-key' }], 'base-url': 'https://codex.example.test', priority: 10,
    models: [{ name: 'upstream-a', alias: 'codex-a' }],
  }],
  writes: [],
  discoveryCalls: 0,
};
const remarks = new Map<string, string>();
mockIPC((cmd, rawArgs) => {
  if (cmd === 'set_app_locale') return null;
  if (cmd === 'resolve_api_access_remarks') {
    const { queries } = rawArgs as { queries: (ApiAccessRemarkLocator & { providerSection: ProviderSection })[] };
    return queries.map((query) => remarks.get(providerRemarkIdentity(query.providerSection, query)) ?? '');
  }
  if (cmd === 'save_api_access_remark') {
    const { update } = rawArgs as { update: {
      providerSection: ProviderSection; previousRecords: ApiAccessRemarkLocator[];
      records: ApiAccessRemarkLocator[]; remark: string;
    } };
    for (const record of update.previousRecords) remarks.delete(providerRemarkIdentity(update.providerSection, record));
    for (const record of update.records) remarks.set(providerRemarkIdentity(update.providerSection, record), update.remark);
    return null;
  }
  if (cmd === 'management_request') {
    const { request } = rawArgs as { request: { method: string; path: string; body: Record<string, unknown>[] } };
    const section = request.path.slice(1);
    if (request.method === 'POST' && section === 'requests/api-call') {
      fixture.providerFixture.discoveryCalls += 1;
      return { status_code: 403, body: { error: { message: 'Model listing disabled' } } };
    }
    if (request.method === 'GET') {
      const groups = structuredClone(fixture.providerFixture.records);
      if (section === 'config') return { 'api-keys': { codex: groups } };
      return section === 'config/api-keys/codex' ? groups : [];
    }
    fixture.providerFixture.writes.push({ method: request.method, body: structuredClone(request.body) });
    if (request.method !== 'PUT' || section !== 'config/api-keys/codex') throw new Error('Unexpected provider mutation');
    fixture.providerFixture.records = structuredClone(request.body);
    return { status: 'ok' };
  }
  throw new Error(`Unhandled fixture command: ${cmd}`);
});

createRoot(document.getElementById('root')!).render(
  <I18nProvider><div className="app-shell"><aside className="sidebar" /><div className="workspace"><main className="content">
    <ApiAccessPage />
  </main></div></div></I18nProvider>,
);
