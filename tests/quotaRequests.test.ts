import { afterEach, beforeEach, describe, expect, it, spyOn } from 'bun:test';
import { managementApi } from '../src/services/managementApi';
import { consumeCodexResetCredit, loadQuota } from '../src/services/quotaService';

const success = (body: unknown) => ({ status_code: 200, body });
const codexUsage = {
  rate_limit: { primary_window: { used_percent: 25, limit_window_seconds: 18000 } },
  credits: { balance: 12.5, unlimited: false },
  rate_limit_reset_credits: { available_count: 2, applicable_available_count: 0 },
};
const codexFile = { name: 'codex-test.json', provider: 'codex', auth_index: 1 };

type Request = { url: string; method: string; header: Record<string, string>; data?: string; authIndex: string };
let post: ReturnType<typeof spyOn>;
let get: ReturnType<typeof spyOn>;
let calls: Request[];
let handler: (request: Request) => unknown | Promise<unknown>;

beforeEach(() => {
  calls = [];
  handler = () => { throw new Error('Unexpected API request'); };
  post = spyOn(managementApi, 'post').mockImplementation(async (path, body) => {
    expect(path).toBe('/requests/api-call');
    const request = body as unknown as Request;
    calls.push(request);
    return await handler(request) as never;
  });
  get = spyOn(managementApi, 'get').mockImplementation(async () => { throw new Error('Unexpected download'); });
});
afterEach(() => {
  post.mockRestore();
  get.mockRestore();
});

describe('quota API compatibility', () => {
  it('Codex 使用新请求头和嵌套账户信息；积分详情失败仍显示用量及 usage 回退次数', async () => {
    handler = (request) => request.url.endsWith('/usage')
      ? success(codexUsage) : { statusCode: 403, bodyText: '{"error":"credits forbidden"}' };
    const result = await loadQuota({
      ...codexFile,
      metadata: { id_token: { chatgpt_account_id: 'account-test', plan_type: 'pro', chatgpt_subscription_active_until: '2030-01-01T00:00:00Z' } },
    });
    expect(result).toMatchObject({ status: 'success', plan: 'pro', creditBalance: '12.5', creditsUnlimited: false, resetCredits: 2, resetCreditsApplicable: 0, resetCreditsError: 'HTTP 403: credits forbidden' });
    expect(result.subscriptionActiveUntil).toBe('2030-01-01T00:00:00Z');
    expect(calls).toHaveLength(2);
    for (const call of calls) {
      expect(call.header['Chatgpt-Account-Id']).toBe('account-test');
      expect(call.header['User-Agent']).toContain('codex-tui/0.149.1');
      expect(call.authIndex).toBe('1');
      expect(call.header.Authorization).toBe('Bearer $TOKEN$');
    }
    const creditCallIndex = calls.findIndex((call) => call.url.endsWith('/rate-limit-reset-credits'));
    expect(post.mock.calls[creditCallIndex][2]).toEqual({ timeoutMs: 8000 });
  });

  it('Codex 保留零次数，usage 的适用次数优先于详情，并优先使用用量接口套餐', async () => {
    handler = (request) => request.url.endsWith('/usage')
      ? success({ ...codexUsage, plan_type: 'team' })
      : success({ availableCount: 0, applicableAvailableCount: 3, credits: [] });
    expect(await loadQuota({ ...codexFile, plan_type: 'pro' })).toMatchObject({
      status: 'success', plan: 'team', resetCredits: 0, resetCreditsApplicable: 0,
    });
  });

  it('Codex 详情空列表不能覆盖 usage 报告的可用次数', async () => {
    handler = (request) => success(request.url.endsWith('/usage') ? codexUsage : { credits: [] });
    expect(await loadQuota(codexFile)).toMatchObject({ status: 'success', resetCredits: 2 });
  });

  it('Codex 详情返回坏数据时报告非致命错误而不是吞掉失败', async () => {
    handler = (request) => success(request.url.endsWith('/usage') ? codexUsage : { unexpected: true });
    const result = await loadQuota(codexFile);
    expect(result.status).toBe('success');
    expect(result.resetCredits).toBe(2);
    expect(result.resetCreditsError).toContain('unrecognized data');
  });

  it('Claude profile 查询失败不影响现代 Fable 额度', async () => {
    handler = (request) => request.url.endsWith('/profile')
      ? { status_code: 500 }
      : success({ limits: [{ kind: 'weekly_scoped', percent: 40, scope: { model: { display_name: 'Fable 5' } } }] });
    expect(await loadQuota({ name: 'claude.json', provider: 'claude', auth_index: 'c' })).toMatchObject({
      status: 'success', rows: [{ label: '7-day Fable window', remainingPercent: 60 }],
    });
  });

  it('Antigravity 使用元数据项目、不下载凭据，空响应后回退并读取服务端时间', async () => {
    let quotaCalls = 0;
    const serverTime = 'Tue, 01 Jan 2030 00:00:00 GMT';
    handler = (request) => {
      if (request.url.endsWith(':loadCodeAssist')) return success({ paidTier: { id: 'g1-ultra-lite-tier' } });
      quotaCalls += 1;
      return quotaCalls === 1 ? success({ groups: [] }) : {
        ...success({ groups: [{ displayName: 'Gemini', buckets: [{ remainingFraction: 0.8, resetTime: '2030-01-01T01:00:00Z' }] }] }),
        header: { Date: [serverTime] },
      };
    };
    const result = await loadQuota({ name: 'anti.json', provider: 'antigravity', auth_index: 'a', attributes: { gemini_virtual_project: 'project-test' } });
    expect(result).toMatchObject({ status: 'success', plan: 'Ultra Lite' });
    expect(get).not.toHaveBeenCalled();
    const requests = calls.filter((request) => request.url.endsWith(':retrieveUserQuotaSummary'));
    expect(requests).toHaveLength(2);
    expect(requests[1].url).toContain('sandbox.googleapis.com');
    expect(calls.filter((request) => request.url.endsWith(':loadCodeAssist'))).toHaveLength(1);
    expect(calls).toHaveLength(3);
    for (const request of calls) {
      expect(request.method).toBe('POST');
      expect(request.header).toMatchObject({
        'User-Agent': 'antigravity/hub/2.23.0 darwin/arm64',
        Authorization: 'Bearer $TOKEN$',
        'Content-Type': 'application/json',
      });
    }
    requests.forEach((request) => expect(JSON.parse(request.data!)).toEqual({ project: 'project-test' }));
    expect(Math.abs((result.serverTimeOffsetMs ?? 0) - (Date.parse(serverTime) - Date.now()))).toBeLessThan(1000);
  });

  it('Antigravity 项目缺失时从下载的 JSON 中提取 installed 项目', async () => {
    get.mockImplementation(async () => JSON.stringify({ installed: { project_id: 'downloaded-project' } }));
    handler = (request) => success(request.url.endsWith(':loadCodeAssist') ? {} : {
      groups: [{ buckets: [{ remainingFraction: 1 }] }],
    });
    expect((await loadQuota({ name: 'anti-download.json', provider: 'antigravity', auth_index: 'a' })).status).toBe('success');
    expect(get).toHaveBeenCalledWith('/credentials/download', { name: 'anti-download.json' });
    expect(calls.find((request) => request.url.endsWith(':retrieveUserQuotaSummary'))?.data).toBe('{"project":"downloaded-project"}');
  });

  it('缺少 auth-index 或禁用的凭据不发起请求', async () => {
    expect((await loadQuota({ provider: 'kimi' })).status).toBe('error');
    expect((await loadQuota({ ...codexFile, disabled: 'true' })).status).toBe('error');
    await expect(consumeCodexResetCredit({ ...codexFile, disabled: true })).rejects.toThrow();
    expect(post).not.toHaveBeenCalled();
  });
});

describe('xAI quota queries aligned with Management Center', () => {
  const file = { name: 'xai.json', provider: 'x-ai', auth_index: 'x' };

  it('一个账单失败时保留另一个接口的真实额度，不探测', async () => {
    handler = (request) => request.url.includes('format=credits')
      ? { status_code: 403, body: 'weekly denied' }
      : success({ config: { monthlyLimit: 1000, used: 0 } });
    const result = await loadQuota({ ...file, metadata: { user: { id: 'u' } } });
    expect(result).toMatchObject({ status: 'success', rows: [{ remainingPercent: 100 }] });
    expect(calls).toHaveLength(2);
    calls.forEach((request) => expect(request.header['x-userid']).toBe('u'));
  });

  it('付费账号直接探测，profile 失败不影响聊天成功', async () => {
    handler = (request) => request.url.endsWith('/me')
      ? { status_code: 403, body: 'profile denied' } : success({});
    const result = await loadQuota({ ...file, using_api: true, prefix: 'paid' });
    expect(result).toMatchObject({ status: 'success', plan: 'Paid' });
    expect(result.rows[0].remainingPercent).toBeNull();
    expect(result.rows[0].detail).toContain('Paid API chat is available');
    expect(calls).toHaveLength(2);
    expect(calls.map((request) => request.url)).toEqual([
      'https://api.x.ai/v1/me', 'https://api.x.ai/v1/chat/completions',
    ]);
    expect(JSON.parse(calls[1].data!)).toEqual({
      model: 'grok-4.5', messages: [{ role: 'user', content: 'ping' }],
      max_tokens: 1, stream: false,
    });
    expect(calls[1].method).toBe('POST');
    expect(calls[1].header).toEqual({
      Authorization: 'Bearer $TOKEN$', accept: 'application/json', 'Content-Type': 'application/json',
    });
    post.mock.calls.forEach((call) => expect(call[2]).toEqual({ timeoutMs: 15000 }));
  });

  it('账单为空后探测成功只显示账户可用，不伪造额度', async () => {
    handler = () => success({});
    const result = await loadQuota(file);
    expect(result).toMatchObject({ status: 'success', plan: 'Paid', rows: [{ remainingPercent: null }] });
    expect(calls).toHaveLength(4);
  });

  it('账单和回退都失败时保留原始账单错误', async () => {
    handler = (request) => request.url.includes('cli-chat-proxy')
      ? { status_code: 403, body: 'billing denied' } : { status_code: 429, body: 'paid denied' };
    expect(await loadQuota(file)).toMatchObject({ status: 'error', error: 'HTTP 403: billing denied' });
    expect(calls).toHaveLength(4);
  });

  it('已识别付费账号聊天失败时显示聊天错误', async () => {
    handler = (request) => request.url.endsWith('/me')
      ? success({}) : { status_code: 429, body: 'chat denied' };
    expect(await loadQuota({ ...file, using_api: true, prefix: 'paid' }))
      .toMatchObject({ status: 'error', error: 'HTTP 429: chat denied' });
    expect(calls).toHaveLength(2);
  });

  it('合并缺失百分比与零用量后仍查询成功，不触发探测', async () => {
    handler = (request) => success({ config: request.url.includes('format=credits')
      ? { currentPeriod: { type: 'weekly' } } : { creditUsagePercent: 0 } });
    expect(await loadQuota(file)).toMatchObject({
      status: 'success', rows: [{ remainingPercent: null }],
    });
    expect(calls).toHaveLength(2);
  });

  it('已识别的空周周期或零按量上限不触发付费探测', async () => {
    for (const config of [{ currentPeriod: { type: 'weekly' } }, { onDemandCap: 0 }]) {
      calls = [];
      handler = () => success({ config });
      expect(await loadQuota(file)).toMatchObject({
        status: 'success', rows: [{ remainingPercent: null }],
      });
      expect(calls).toHaveLength(2);
    }
  });
  it('仅有账单周期也不触发付费回退', async () => {
    handler = () => success({ config: { billingPeriodEnd: '2030-01-01T00:00:00Z' } });
    expect(await loadQuota(file)).toMatchObject({
      status: 'success', rows: [{ remainingPercent: null }],
    });
    expect(calls).toHaveLength(2);
  });
});
describe('quota request synchronization', () => {
  it('跨页面重复刷新共用同一个请求', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    handler = async (request) => {
      await gate;
      return success(request.url.endsWith('/usage') ? codexUsage : { available_count: 2 });
    };
    const first = loadQuota(codexFile);
    const second = loadQuota(codexFile);
    expect(second).toBe(first);
    release();
    await Promise.all([first, second]);
    expect(calls).toHaveLength(2);
  });

  it('重置期间刷新等待重置结果，连续点击只消费一次且随后重查用量', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    handler = async (request) => {
      if (request.url.endsWith('/consume')) {
        await gate;
        return success({});
      }
      return success(request.url.endsWith('/usage') ? codexUsage : { available_count: 1 });
    };
    const reset = consumeCodexResetCredit(codexFile);
    const secondReset = consumeCodexResetCredit(codexFile);
    const refresh = loadQuota(codexFile);
    expect(secondReset).toBe(reset);
    release();
    expect((await reset).status).toBe('success');
    expect(await refresh).toEqual(await reset);
    const consumes = calls.filter((request) => request.url.endsWith('/consume'));
    expect(consumes).toHaveLength(1);
    expect(JSON.parse(consumes[0].data!).redeem_request_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(calls.map((request) => request.method)).toEqual(['POST', 'GET', 'GET']);
  });

  it('已有刷新结束后才消费重置积分，避免把旧请求结果当成重置后的额度', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    handler = async (request) => {
      await gate;
      return success(request.url.endsWith('/usage') ? codexUsage : { available_count: 1 });
    };
    const refresh = loadQuota(codexFile);
    const reset = consumeCodexResetCredit(codexFile);
    await Promise.resolve();
    expect(calls.some((request) => request.url.endsWith('/consume'))).toBe(false);
    release();
    await Promise.all([refresh, reset]);
    expect(calls.map((request) => request.method)).toEqual(['GET', 'GET', 'POST', 'GET', 'GET']);
  });

  it('重置失败后释放锁，允许重新刷新', async () => {
    handler = (request) => request.url.endsWith('/consume')
      ? { status_code: 409, body: 'reset denied' }
      : success(request.url.endsWith('/usage') ? codexUsage : { available_count: 1 });
    const reset = consumeCodexResetCredit(codexFile);
    const refresh = loadQuota(codexFile);
    await expect(reset).rejects.toThrow('reset denied');
    expect(await refresh).toMatchObject({ status: 'error', error: 'HTTP 409: reset denied' });
    expect((await loadQuota(codexFile)).status).toBe('success');
  });
});
