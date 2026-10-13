import { applyTemplateChanges, readTemplatePath, sameTemplateValue, type TemplateConfigChange } from '../services/templateConfig';
import { createPluginMock } from './pluginMock';
import { createQuotaMock, createQuotaMockFiles } from './quotaMock';

export type BrowserMockScenario = 'running' | 'stopped' | 'empty' | 'error';
export type BrowserMockMode = BrowserMockScenario | 'off';

type JsonObject = Record<string, unknown>;
type EmitMockEvent = (event: string, payload: unknown) => void;

export type BrowserMockOptions = {
  mode: BrowserMockMode;
  delayMs: number;
};

export type BrowserMockRuntime = {
  scenario: BrowserMockScenario;
  invoke: (command: string, payload?: unknown) => Promise<unknown>;
};

const SCENARIOS = new Set<BrowserMockScenario>(['running', 'stopped', 'empty', 'error']);
const BUNDLED_CORE = { version: '7.3.15', assetName: 'CLIProxyAPI_7.3.15_windows_amd64.zip' };
const PROVIDER_SECTIONS = [
  'gemini-api-key',
  'interactions-api-key',
  'vertex-api-key',
  'codex-api-key',
  'claude-api-key',
  'xai-api-key',
  'meta-api-key',
  'openai-compatibility',
] as const;
type ProviderSection = (typeof PROVIDER_SECTIONS)[number];
const V8_PROVIDER_BY_SECTION = {
  'gemini-api-key': 'gemini',
  'interactions-api-key': 'interactions',
  'vertex-api-key': 'vertex',
  'codex-api-key': 'codex',
  'claude-api-key': 'claude',
  'xai-api-key': 'xai',
  'meta-api-key': 'meta',
  'openai-compatibility': 'openai-compatibility',
} as const;
const SHARED_PROVIDER_FIELDS = new Set([
  'priority', 'prefix', 'proxy-url', 'headers', 'models', 'excluded-models',
  'disable-cooling', 'request-retry', 'request-scoped-errors',
]);

const clone = <Value,>(value: Value): Value => structuredClone(value);
const sleep = (delayMs: number) => delayMs > 0
  ? new Promise<void>((resolve) => globalThis.setTimeout(resolve, delayMs))
  : Promise.resolve();

const asObject = (value: unknown): JsonObject => (
  typeof value === 'object' && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : {}
);

const asArray = (value: unknown): unknown[] => Array.isArray(value) ? value : [];
const readString = (value: unknown) => typeof value === 'string' ? value : '';
const readNumber = (value: unknown, fallback = 0) => {
  const parsed = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
};

function groupProviderRecords(section: ProviderSection, records: JsonObject[]): JsonObject[] {
  return records.map((record, index) => {
    if (section === 'openai-compatibility') {
      const group: JsonObject = { ...clone(record), keys: asArray(record['api-key-entries']).map((item) => clone(asObject(item))) };
      delete group['api-key-entries'];
      return group;
    }
    const group: JsonObject = { name: readString(record.name).trim() || `${V8_PROVIDER_BY_SECTION[section]}-${index + 1}` };
    const key: JsonObject = {};
    Object.entries(record).forEach(([field, value]) => {
      if (field === 'base-url' || SHARED_PROVIDER_FIELDS.has(field)) group[field] = clone(value);
      else key[field] = clone(value);
    });
    group.keys = [key];
    return group;
  });
}

const isoHoursAgo = (hours: number) => new Date(Date.now() - hours * 60 * 60 * 1000).toISOString();
const localHourKey = (date: Date) => [
  date.getFullYear(),
  String(date.getMonth() + 1).padStart(2, '0'),
  String(date.getDate()).padStart(2, '0'),
  String(date.getHours()).padStart(2, '0'),
].join('-');

export function resolveBrowserMockOptions(
  search: string,
  storedScenario?: string | null,
): BrowserMockOptions {
  const params = new URLSearchParams(search);
  const requested = params.get('mock');
  const mode: BrowserMockMode = requested === 'off'
    ? 'off'
    : SCENARIOS.has(requested as BrowserMockScenario)
      ? requested as BrowserMockScenario
      : SCENARIOS.has(storedScenario as BrowserMockScenario)
        ? storedScenario as BrowserMockScenario
        : 'running';
  const delay = Number(params.get('mockDelay') ?? 0);
  return {
    mode,
    delayMs: Number.isFinite(delay) ? Math.max(0, Math.min(5_000, Math.round(delay))) : 0,
  };
}

type BrowserMockState = ReturnType<typeof createState>;

function createCoreStatus(scenario: BrowserMockScenario) {
  const installed = scenario !== 'empty';
  const running = scenario === 'running';
  return {
    installed,
    running,
    ready: running,
    starting: false,
    managed: running,
    processId: running ? 42817 : null,
    currentVersion: installed ? '7.3.15' : null,
    installDir: 'C:\\EasyCLIProxyAPI\\cpa-core',
    binaryPath: installed ? 'C:\\EasyCLIProxyAPI\\cpa-core\\cli-proxy-api.exe' : null,
    message: running ? 'Browser Mock core is running' : installed ? 'Browser Mock core is stopped' : 'Browser Mock core is not installed',
  };
}

function createTimeline() {
  return Array.from({ length: 24 }, (_, index) => {
    const date = new Date(Date.now() - (23 - index) * 60 * 60 * 1000);
    const requests = 6 + (index % 5) * 3;
    const failure = index % 7 === 0 ? 1 : 0;
    const canceled = index % 11 === 0 ? 1 : 0;
    const success = Math.max(0, requests - failure - canceled);
    const codexTokens = 18_000 + index * 1_350;
    const claudeTokens = 9_000 + (index % 6) * 2_100;
    return {
      hour: localHourKey(date),
      requests,
      success,
      failure,
      canceled,
      tokens: codexTokens + claudeTokens,
      models: [
        { key: 'gpt-5.2-codex', label: 'GPT-5.2 Codex', tokens: codexTokens, requests: Math.ceil(requests * 0.65) },
        { key: 'claude-sonnet-4-6', label: 'Claude Sonnet 4.6', tokens: claudeTokens, requests: Math.floor(requests * 0.35) },
      ],
    };
  });
}

function createUsageEvents() {
  const models = ['gpt-5.2-codex', 'claude-sonnet-4-6', 'gemini-3-pro'];
  const longError = 'Mock upstream rate limit exceeded: this preview account reached its per-minute request allowance while several parallel jobs were active. The proxy retried the request three times, but every attempt received HTTP 429. Retry after 45 seconds or reduce the number of concurrent requests. If this keeps happening, check the provider quota and the client retry policy. Diagnostic context: route=/v1/responses, attempt=3/3, window=60s, trace=preview_long_error_0123456789abcdefghijklmnopqrstuvwxyz_0123456789abcdefghijklmnopqrstuvwxyz.';
  return Array.from({ length: 18 }, (_, index) => {
    const failed = index === 5 || index === 13;
    const input = 1_200 + index * 137;
    const output = failed ? 0 : 380 + index * 31;
    const reasoning = failed ? 0 : 160 + index * 19;
    const cacheRead = index % 3 === 0 ? 640 : 0;
    return {
      id: `mock-event-${index + 1}`,
      row_id: String(index + 1),
      timestamp: isoHoursAgo(index / 2),
      latency_ms: 620 + index * 47,
      ttft_ms: failed ? null : 180 + index * 9,
      source: index % 2 === 0 ? 'codex' : 'claude-code',
      source_display: index % 2 === 0 ? 'Codex' : 'Claude Code',
      failed,
      canceled: index === 9,
      failure_status: failed ? 429 : 0,
      failure_body: index === 5 ? longError : failed ? 'Mock rate limit exceeded' : '',
      provider: index % 2 === 0 ? 'openai-oauth' : 'claude-oauth',
      model: models[index % models.length],
      alias: '',
      reasoning_effort: index < 3 ? 'medium' : index % 2 === 0 ? 'high' : 'medium',
      service_tier: index === 0 ? 'priority' : index === 1 ? 'fast' : index === 2 ? 'default' : 'auto',
      response_service_tier: index === 0 ? 'priority' : index === 1 ? 'fast' : 'default',
      endpoint: '/v1/responses',
      api_key_hash: 'mock-key-hash',
      api_key_display: 'sk-mock••••••••2026',
      api_key_remark: 'Browser Mock key',
      tokens: {
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cacheRead,
        cache_creation_tokens: 0,
        total_tokens: input + output + reasoning + cacheRead,
      },
    };
  });
}

function createAgentStatuses() {
  const ids = [
    ['claude-code', 'Claude Code'],
    ['claude-desktop', 'Claude Desktop'],
    ['codex', 'Codex'],
    ['opencode', 'OpenCode'],
    ['openclaw', 'OpenClaw'],
    ['hermes', 'Hermes Agent'],
    ['deepseek-harness', 'DeepSeek Harness'],
    ['zcode', 'ZCode'],
    ['workbuddy', 'WorkBuddy'],
    ['antigravity-cli', 'Antigravity CLI'],
    ['kimi-code', 'Kimi Code'],
    ['grok-build', 'Grok Build'],
    ['omp', 'Oh My Pi'],
    ['pi', 'Pi'],
  ] as const;
  const claudeMappings = {
    opus: 'claude-opus-4-6',
    sonnet: 'claude-sonnet-4-6',
    haiku: 'claude-haiku-4-5',
    opus1m: false,
    sonnet1m: false,
    haiku1m: false,
    maxContextTokens: 200_000,
    autoCompactPct: 90,
    disableAutoCompact: false,
  };
  return ids.map(([id, name], index) => ({
    id,
    name,
    supportedPlatform: true,
    installed: id !== 'openclaw',
    pluginInstalled: id === 'pi',
    launchTargets: id === 'codex' || id === 'opencode'
      ? [
          { id: 'app', label: 'Desktop App', detail: 'Browser Mock desktop entry point' },
          { id: 'cli', label: 'CLI', detail: 'Browser Mock command-line entry point' },
        ]
      : [{ id: id === 'claude-desktop' ? 'app' : 'cli', label: 'Launch', detail: 'Browser Mock launch target' }],
    version: `1.${index + 4}.0`,
    cliVersion: id === 'claude-desktop' ? null : `1.${index + 4}.0`,
    appVersion: id === 'claude-desktop' ? '0.12.8' : null,
    pluginVersion: id === 'pi' ? '0.4.2' : null,
    configExists: true,
    configValid: true,
    configured: id !== 'openclaw',
    connectionState: id === 'openclaw' ? 'not-configured' : 'configured',
    configurationSynchronized: true,
    currentModel: id.startsWith('claude') ? 'claude-sonnet-4-6' : 'gpt-5.2-codex',
    oauthConfiguration: id === 'codex',
    codexNativeOauth: false,
    modificationEnabled: id !== 'openclaw',
    modificationState: id === 'openclaw' ? 'unconfigured' : 'applied',
    backupAvailable: true,
    appliedModel: id.startsWith('claude') ? 'claude-sonnet-4-6' : 'gpt-5.2-codex',
    claudeCodeModelMappings: id === 'claude-code' ? clone(claudeMappings) : null,
    claudeDesktopModelMappings: id === 'claude-desktop'
      ? {
          ...clone(claudeMappings),
          desktopModels: [
            { id: 'claude-opus-5-cpa', alias: 'Claude Opus', model: 'claude-opus-4-6', enabled: true },
            { id: 'claude-sonnet-5-cpa', alias: 'Claude Sonnet', model: 'claude-sonnet-4-6', enabled: true },
            { id: 'claude-haiku-4-5-cpa', alias: 'Claude Haiku', model: 'claude-haiku-4-5', enabled: true },
          ],
        }
      : null,
    warnings: id === 'openclaw' ? ['Mock: configuration has not been applied'] : [],
    error: null,
  }));
}

function createState(scenario: BrowserMockScenario) {
  const timeline = createTimeline();
  const events = createUsageEvents();
  const authFiles: JsonObject[] = [
    {
      name: 'codex-personal.json',
      provider: 'codex',
      type: 'codex',
      source: 'file',
      path: 'C:\\EasyCLIProxyAPI\\oauth\\codex-personal.json',
      auth_index: 'mock-codex-1',
      account_id: 'acct_mock_codex',
      email: 'codex.mock@example.com',
      disabled: false,
      priority: 0,
      success: 138,
      failed: 4,
      modtime: Date.now() - 20 * 60_000,
      recent_requests: Array.from({ length: 20 }, (_, index) => ({
        time: isoHoursAgo((19 - index) / 3),
        success: 3 + index % 4,
        failed: index % 8 === 0 ? 1 : 0,
      })),
      excluded_models: [],
    },
    {
      name: 'claude-team.json',
      provider: 'claude',
      type: 'anthropic',
      source: 'file',
      path: 'C:\\EasyCLIProxyAPI\\oauth\\claude-team.json',
      auth_index: 'mock-claude-1',
      email: 'claude.mock@example.com',
      disabled: false,
      priority: 0,
      success: 92,
      failed: 1,
      modtime: Date.now() - 45 * 60_000,
      excluded_models: ['claude-legacy-*'],
    },
    ...createQuotaMockFiles(),
    {
      name: 'gemini-runtime',
      provider: 'gemini',
      type: 'gemini',
      source: 'runtime',
      runtime_only: true,
      auth_index: 'mock-gemini-runtime',
      disabled: false,
      success: 27,
      failed: 0,
    },
  ];
  const legacyProviderConfig: Record<ProviderSection, JsonObject[]> = {
    'interactions-api-key': [],
    'vertex-api-key': [],
    'xai-api-key': [],
    'meta-api-key': [],
    'codex-api-key': [
      {
        name: 'Codex Direct',
        'api-key': 'sk-mock-codex-direct',
        'base-url': 'https://api.openai.com/v1',
        priority: 10,
        models: [
          { name: 'gpt-5.2-codex', alias: 'codex-latest', thinking: { levels: ['low', 'medium', 'high', 'xhigh'] } },
          { name: 'gpt-5.1-codex', thinking: { levels: ['low', 'medium', 'high'] } },
        ],
      },
      {
        name: 'DeepSeek',
        'api-key': 'sk-mock-deepseek',
        'base-url': 'https://api.deepseek.com',
        priority: 20,
        models: [
          { name: 'deepseek-chat' },
          { name: 'deepseek-reasoner', thinking: { levels: ['high'] } },
        ],
      },
    ],
    'claude-api-key': [{
      name: 'Claude Direct',
      'api-key': 'sk-ant-mock-claude',
      'base-url': 'https://api.anthropic.com',
      models: [
        { name: 'claude-opus-4-6' },
        { name: 'claude-sonnet-4-6' },
        { name: 'claude-haiku-4-5' },
      ],
    }],
    'gemini-api-key': [{
      name: 'Gemini AI Studio',
      'api-key': 'AIza-mock-gemini',
      'base-url': 'https://generativelanguage.googleapis.com',
      models: [{ name: 'gemini-3-pro' }, { name: 'gemini-3-flash' }],
    }],
    'openai-compatibility': [{
      name: 'Mock OpenAI Compatible',
      'base-url': 'https://mock-provider.example.com/v1',
      'api-key-entries': [
        { 'api-key': 'sk-mock-compatible-a', 'auth-index': 'mock-compatible-a' },
        { 'api-key': 'sk-mock-compatible-b', 'auth-index': 'mock-compatible-b' },
      ],
      models: [
        { name: 'mock-large', alias: 'mock-default' },
        { name: 'mock-fast' },
      ],
      priority: 30,
      disabled: false,
    }],
  };
  const analysis = {
    models: [
      { key: 'gpt-5.2-codex', label: 'GPT-5.2 Codex', requests: 182, failures: 4, tokens: 1_284_600 },
      { key: 'claude-sonnet-4-6', label: 'Claude Sonnet 4.6', requests: 96, failures: 2, tokens: 684_200 },
      { key: 'gemini-3-pro', label: 'Gemini 3 Pro', requests: 34, failures: 1, tokens: 218_500 },
    ],
    providers: [
      { key: 'openai-oauth', label: 'OpenAI OAuth', requests: 182, failures: 4, tokens: 1_284_600 },
      { key: 'claude-oauth', label: 'Claude OAuth', requests: 96, failures: 2, tokens: 684_200 },
      { key: 'gemini-api', label: 'Gemini API', requests: 34, failures: 1, tokens: 218_500 },
    ],
    sources: [
      { key: 'codex', label: 'Codex', requests: 164, failures: 4, tokens: 1_102_000 },
      { key: 'claude-code', label: 'Claude Code', requests: 114, failures: 2, tokens: 752_000 },
      { key: 'other', label: 'Other', requests: 34, failures: 1, tokens: 333_300 },
    ],
    apiKeys: [
      { key: 'mock-key-hash', label: 'Browser Mock key', requests: 312, failures: 7, tokens: 2_187_300 },
    ],
  };
  return {
    scenario,
    coreStatus: createCoreStatus(scenario),
    guiSettings: {
      host: '127.0.0.1',
      port: 8317,
      runOnStartup: true,
      closeBehavior: 'ask',
    },
    coreConfig: {
      apiKeys: [
        { apiKey: 'sk-browser-mock-2026', remark: 'Browser Mock key' },
        { apiKey: 'sk-browser-demo-secondary', remark: 'Secondary demo key' },
      ],
      debug: false,
      commercialMode: false,
      loggingToFile: true,
      logsMaxTotalSizeMb: 256,
      errorLogsMaxFiles: 10,
      usageStatisticsEnabled: true,
      redisUsageQueueRetentionSeconds: 60,
      requestLog: false,
      pluginsEnabled: false,
      managementSecretConfigured: true,
      host: '127.0.0.1',
      port: 8317,
      allowLan: false,
      routingStrategy: 'round-robin',
      proxyUrl: '',
      proxyOverride: false,
      routingSessionAffinity: true,
      routingSessionAffinityTtl: '30m',
      disableCooling: false,
      requestRetry: 3,
      maxRetryCredentials: 0,
      maxRetryInterval: 30,
      streamingBootstrapRetries: 0,
    },
    extendedConfig: {
      'config-version': 8,
      server: { host: '127.0.0.1', port: 8317, 'commercial-mode': false, tls: { enable: false, cert: '', key: '' } },
      management: { 'allow-remote': false, 'disable-control-panel': false },
      routing: {
        strategy: 'round-robin', 'session-affinity': true, 'session-affinity-ttl': '30m',
        retry: { 'request-retry': 3, 'max-retry-credentials': 0, 'max-retry-interval': 30 },
        cooldown: { 'disable-cooling': false },
      },
      requests: { 'proxy-url': '', streaming: { 'bootstrap-retries': 0 } },
      observability: { logs: { debug: false, 'logging-to-file': true, 'logs-max-total-size-mb': 256, 'error-logs-max-files': 10 }, usage: { 'usage-statistics-enabled': true, 'redis-usage-queue-retention-seconds': 60 } },
    } as JsonObject,
    softwareSettings: {
      closeBehavior: 'ask',
      autostartEnabled: false,
      startCoreOnLaunch: true,
      silentStartEnabled: false,
      defaultTerminal: 'auto',
      availableTerminals: [
        { id: 'auto', label: 'Automatic selection' },
        { id: 'windows-terminal', label: 'Windows Terminal' },
        { id: 'powershell', label: 'PowerShell' },
      ],
    },
    tlsSettings: { enabled: false, cert: '', key: '' },
    sensitiveWords: {
      antigravitySensitiveWords: ['internal-only', 'mock-secret'],
      devinSensitiveWords: ['private-repository'],
    },
    appUpdateInfo: {
      currentVersion: '0.2.97',
      latestVersion: '0.3.0',
      updateAvailable: true,
      releaseUrl: 'https://github.com/router-for-me/EasyCLIProxyAPI/releases/latest',
      releaseNotes: {
        'zh-CN': '## Browser Mock 更新示例\n\n- 新增界面调试模式\n- 改进启动体验',
        en: '## Browser Mock release\n\n- Added UI mock mode',
      },
      publishedAt: isoHoursAgo(18),
      autoUpdateSupported: true,
      downloadSizeBytes: 48 * 1024 * 1024,
      unsupportedReason: null,
    },
    appUpdateTask: {
      running: false,
      cancellable: false,
      phase: 'idle',
      targetVersion: null,
      downloadedBytes: 0,
      totalBytes: null,
      percent: null,
      message: null,
    },
    coreLatest: { version: '7.3.16', assetName: 'CLIProxyAPI_7.3.16_windows_amd64.zip' },
    coreInstallTask: {
      running: false,
      cancellable: false,
      phase: 'idle',
      downloaded: 0,
      total: null,
      percent: null,
      message: null,
      result: null,
    },
    versionSource: { source: 'github', gitcodeAvailable: true, customMirrors: ['https://gh-proxy.example.com/'] },
    // Native groups are authoritative; legacy routes only adapt their own view.
    providerConfig: Object.fromEntries(PROVIDER_SECTIONS.map((section) => [
      section, scenario === 'empty' ? [] : groupProviderRecords(section, legacyProviderConfig[section]),
    ])) as Record<ProviderSection, JsonObject[]>,
    authFiles: scenario === 'empty' ? [] : authFiles,
    quotaApiCall: createQuotaMock(),
    oauthExcludedModels: { codex: [], claude: ['claude-legacy-*'], gemini: [] } as Record<string, string[]>,
    oauthSessions: new Map<string, string>(),
    analysis,
    usageOverview: {
      totalRequests: 312,
      successCount: 301,
      failureCount: 7,
      canceledCount: 4,
      successRate: 96.47,
      inputTokens: 1_264_000,
      outputTokens: 486_500,
      reasoningTokens: 286_800,
      cacheReadTokens: 132_000,
      cacheCreationTokens: 18_000,
      totalTokens: 2_187_300,
      rpm: 4.8,
      tpm: 31_450,
      tps: 58.4,
      tpsSampleCount: 284,
      averageLatencyMs: 1_284,
      cacheHitRate: 0.386,
      estimatedCost: 3.8421,
      pricedRequests: 296,
      timeline,
    },
    usageEvents: events,
    usagePrices: [
      {
        model: 'gpt-5.2-codex', prompt: 1.25, completion: 10, cacheRead: 0.125, cacheCreation: 0,
        promptConfigured: true, completionConfigured: true, cacheReadConfigured: true, cacheCreationConfigured: false,
        source: 'mock', sourceModelId: 'gpt-5.2-codex', updatedAtMs: Date.now(),
      },
      {
        model: 'claude-sonnet-4-6', prompt: 3, completion: 15, cacheRead: 0.3, cacheCreation: 3.75,
        promptConfigured: true, completionConfigured: true, cacheReadConfigured: true, cacheCreationConfigured: true,
        source: 'mock', sourceModelId: 'claude-sonnet-4-6', updatedAtMs: Date.now(),
      },
    ],
    usageStorage: { maxDatabaseSizeMb: 512, databaseSizeBytes: 84 * 1024 * 1024, totalRecords: 312, deletedRecords: 0 },
    agentStatuses: createAgentStatuses(),
    deepSeekStatus: {
      running: false,
      pid: null as number | null,
      mode: null as string | null,
    },
    thinkingAliases: [
      {
        position: { section: 'oauth-model-alias', providerIndex: null, modelIndex: 0 },
        sourceModel: 'gpt-5.2-codex',
        alias: 'codex-high',
        effort: 'high' as string | null,
        provider: 'Codex OAuth',
        kind: 'codex-oauth',
        oauthChannel: 'codex' as string | null,
      },
    ],
    speedAliases: [
      {
        position: { section: 'oauth-model-alias', providerIndex: null, modelIndex: 0 },
        sourceModel: 'gpt-5.2-codex',
        alias: 'codex-fast',
        serviceTier: 'priority',
        provider: 'Codex OAuth',
        kind: 'codex-oauth',
        oauthChannel: 'codex' as string | null,
      },
    ],
    aliasSources: [
      { id: 'codex:gpt-5.2-codex', model: 'gpt-5.2-codex', displayName: 'GPT-5.2 Codex', provider: 'Codex OAuth', kind: 'codex-oauth', protocol: 'openai', reasoningLevels: ['low', 'medium', 'high', 'xhigh'] },
      { id: 'claude:claude-sonnet-4-6', model: 'claude-sonnet-4-6', displayName: 'Claude Sonnet 4.6', provider: 'Claude OAuth', kind: 'claude-oauth', protocol: 'anthropic', reasoningLevels: ['low', 'medium', 'high'] },
    ],
    codexCatalog: {
      revision: 'mock-catalog-r1',
      models: ['gpt-5.2-codex', 'gpt-5.1-codex'].map((slug) => {
        const configuration = {
          display_name: slug === 'gpt-5.2-codex' ? 'GPT-5.2 Codex' : 'GPT-5.1 Codex',
          description: 'Browser Mock model catalog',
          context_window: 272_000,
          max_context_window: 400_000,
          effective_context_window_percent: 90,
          auto_compact_token_limit: 244_800,
          default_reasoning_level: 'medium',
          supported_reasoning_levels: ['low', 'medium', 'high', 'xhigh'].map((effort) => ({ effort, description: `${effort} reasoning effort` })),
          input_modalities: ['text', 'image'],
          visibility: 'list',
          supports_parallel_tool_calls: true,
          base_instructions: 'You are Codex, a coding assistant.',
        };
        return {
          slug,
          hasOfficialTemplate: true,
          contextSource: 'template',
          customized: false,
          configuration,
          defaults: clone(configuration),
        };
      }),
    },
    harnessCatalog: {
      revision: 'mock-harness-r1',
      models: [
        { id: 'deepseek-chat', defaults: { name: 'DeepSeek Chat', input: ['text'], contextWindow: 64_000, maxTokens: 8_192 }, configuration: {} },
        { id: 'deepseek-reasoner', defaults: { name: 'DeepSeek Reasoner', input: ['text'], contextWindow: 64_000, maxTokens: 16_384 }, configuration: { reasoningEfforts: { high: 'high' } } },
      ],
      provider: { api: 'openai-completions', defaultContextWindow: 64_000 },
      baseUrl: 'http://127.0.0.1:8317/v1',
      defaultModel: 'deepseek-chat',
      configured: true,
    },
    codexSessionContexts: {} as Record<string, MockCodexSessionContext>,
    codexSessions: [
      { id: 'mock-session-1', title: 'Implement Browser Mock', cwd: 'E:\\projects\\mock-demo', modelProvider: 'cpa-gui', archived: false, updatedAtMs: Date.now() - 20 * 60_000, databasePath: 'C:\\Users\\Mock\\.codex\\state.sqlite' },
      { id: 'mock-session-2', title: 'Learn TypeScript types', cwd: 'E:\\projects\\typescript-study', modelProvider: 'openai', archived: false, updatedAtMs: Date.now() - 3 * 60 * 60_000, databasePath: 'C:\\Users\\Mock\\.codex\\state.sqlite' },
      { id: 'mock-session-3', title: 'Review legacy UI', cwd: 'E:\\projects\\legacy-ui', modelProvider: 'cpa-gui', archived: true, updatedAtMs: Date.now() - 3 * 24 * 60 * 60_000, databasePath: 'C:\\Users\\Mock\\.codex\\state.sqlite' },
    ],
  };
}

type MockCodexSessionMessage = {
  id: string;
  lineNumber: number;
  role: string;
  timestamp: string;
  content: string;
  rawType: string;
  model: string;
  callId: string | null;
};

type MockCodexSessionContext = {
  id: string;
  title: string;
  cwd: string;
  modelProvider: string;
  model: string;
  archived: boolean;
  updatedAtMs: number;
  rolloutPath: string;
  databasePath: string;
  messages: MockCodexSessionMessage[];
  stats: {
    totalLines: number;
    messageCount: number;
    userMessageCount: number;
    assistantMessageCount: number;
    toolCount: number;
    fileSizeBytes: number;
  };
  rawJsonlAvailable: boolean;
  rolloutSha256: string;
  rawJsonl: string;
};

function mockSessionRawJsonl(context: MockCodexSessionContext): string {
  const lines = [
    JSON.stringify({ type: 'session_meta', payload: { id: context.id, title: context.title, cwd: context.cwd, model_provider: context.modelProvider } }),
    JSON.stringify({ type: 'turn_context', payload: { model: context.model, cwd: context.cwd } }),
  ];
  for (const message of context.messages) {
    if (message.role === 'tool') {
      lines.push(JSON.stringify({
        type: 'response_item',
        payload: { type: 'function_call', name: 'exec_command', arguments: message.content, call_id: message.callId },
      }));
      continue;
    }
    lines.push(JSON.stringify({
      type: 'response_item',
      payload: {
        type: 'message',
        role: message.role,
        content: [{ type: message.role === 'assistant' ? 'output_text' : 'input_text', text: message.content }],
      },
    }));
  }
  return lines.join('\n') + '\n';
}

function mockSessionStats(context: MockCodexSessionContext) {
  const lines = context.rawJsonl.split('\n').filter((line) => line.trim().length > 0);
  return {
    totalLines: lines.length,
    messageCount: context.messages.filter((message) => message.role !== 'tool').length,
    userMessageCount: context.messages.filter((message) => message.role === 'user').length,
    assistantMessageCount: context.messages.filter((message) => message.role === 'assistant').length,
    toolCount: context.messages.filter((message) => message.role === 'tool').length,
    fileSizeBytes: new TextEncoder().encode(context.rawJsonl).length,
  };
}

function mockSessionContext(state: BrowserMockState, sessionId: string): MockCodexSessionContext {
  const cached = state.codexSessionContexts[sessionId];
  if (cached) return cached;
  const session = state.codexSessions.find((item) => item.id === sessionId);
  const context: MockCodexSessionContext = {
    id: sessionId,
    title: session?.title ?? 'Mock Session Title',
    cwd: session?.cwd ?? 'C:\\Workspace\\project',
    modelProvider: session?.modelProvider ?? 'cpa-gui',
    model: 'deepseek-v4.1',
    archived: session?.archived ?? false,
    updatedAtMs: session?.updatedAtMs ?? Date.now(),
    rolloutPath: `C:\\Users\\Mock\\.codex\\sessions\\rollout-${sessionId}.jsonl`,
    databasePath: session?.databasePath ?? 'C:\\Users\\Mock\\.codex\\state.sqlite',
    messages: [
      {
        id: 'msg-2',
        lineNumber: 2,
        role: 'user',
        timestamp: new Date(Date.now() - 3_600_000).toISOString(),
        content: 'Hello Codex, can you help me refactor the database module?',
        rawType: 'response_item',
        model: 'deepseek-v4.1',
        callId: null,
      },
      {
        id: 'msg-3',
        lineNumber: 3,
        role: 'assistant',
        timestamp: new Date(Date.now() - 3_500_000).toISOString(),
        content: 'Sure, I will inspect the current database schema and transactions first.',
        rawType: 'response_item',
        model: 'deepseek-v4.1',
        callId: null,
      },
      {
        id: 'tool-4',
        lineNumber: 4,
        role: 'tool',
        timestamp: new Date(Date.now() - 3_400_000).toISOString(),
        content: 'exec_command(cmd="Get-ChildItem -Directory")',
        rawType: 'response_item',
        model: 'deepseek-v4.1',
        callId: 'mock-call-1',
      },
    ],
    stats: {
      totalLines: 0,
      messageCount: 0,
      userMessageCount: 0,
      assistantMessageCount: 0,
      toolCount: 0,
      fileSizeBytes: 0,
    },
    rawJsonlAvailable: true,
    rolloutSha256: '0'.repeat(64),
    rawJsonl: '',
  };
  context.rawJsonl = mockSessionRawJsonl(context);
  context.stats = mockSessionStats(context);
  state.codexSessionContexts[sessionId] = context;
  return context;
}

function findAuthFile(state: BrowserMockState, name: string) {
  return state.authFiles.find((file) => file.name === name);
}

function mockModelDefinitions() {
  return {
    models: [
      { id: 'gpt-5.2-codex', display_name: 'GPT-5.2 Codex' },
      { id: 'claude-sonnet-4-6', display_name: 'Claude Sonnet 4.6' },
      { id: 'gemini-3-pro', display_name: 'Gemini 3 Pro' },
      { id: 'deepseek-chat', display_name: 'DeepSeek Chat' },
    ],
  };
}

function mockApiCall(state: BrowserMockState, body: JsonObject) {
  const quota = state.quotaApiCall(body);
  if (quota) return quota;
  const url = readString(body.url).toLowerCase();
  if (url.includes('/models')) {
    return {
      status_code: 200,
      body: {
        data: [
          { id: 'mock-large', display_name: 'Mock Large' },
          { id: 'mock-fast', display_name: 'Mock Fast' },
          { id: 'mock-reasoner', display_name: 'Mock Reasoner' },
        ],
      },
    };
  }
  if (!/\/(chat\/completions|responses|messages)(\?|$)/.test(url)) {
    throw new Error(`Browser Mock upstream request not implemented: ${readString(body.method)} ${readString(body.url)}`);
  }
  return {
    status_code: 200,
    body: {
      id: 'mock-response',
      choices: [{ message: { content: 'Browser Mock response' } }],
      content: [{ type: 'text', text: 'Browser Mock response' }],
    },
  };
}

function managementResponse(state: BrowserMockState, payload: JsonObject) {
  const request = asObject(payload.request);
  const method = readString(request.method).toUpperCase();
  const path = readString(request.path);
  const query = asObject(request.query);
  const body = request.body;

  const v8Provider = Object.entries(V8_PROVIDER_BY_SECTION).find(
    ([, provider]) => path === `/config/api-keys/${provider}`,
  );
  if (method === 'GET' && path === '/config') {
    return {
      ...clone(state.extendedConfig),
      'config-version': 8,
      'api-keys': Object.fromEntries(PROVIDER_SECTIONS.map((section) => [
        V8_PROVIDER_BY_SECTION[section],
        clone(state.providerConfig[section]),
      ])),
    };
  }
  if (method === 'GET' && path === '/config/api-keys') {
    return Object.fromEntries(PROVIDER_SECTIONS.map(section => [V8_PROVIDER_BY_SECTION[section], clone(state.providerConfig[section])]));
  }
  const legacySection = PROVIDER_SECTIONS.find(section => path === `/${section}`);
  if (method === 'GET' && legacySection) {
    return state.providerConfig[legacySection].flatMap(group => {
      const { keys, ...shared } = group;
      return legacySection === 'openai-compatibility'
        ? [{ ...clone(shared), 'api-key-entries': clone(keys) }]
        : asArray(keys).map(key => ({ ...clone(shared), ...clone(asObject(key)) }));
    });
  }
  if (v8Provider) {
    const section = v8Provider[0] as ProviderSection;
    if (method === 'GET') return clone(state.providerConfig[section]);
    if (method === 'PUT') {
      state.providerConfig[section] = asArray(body).map((item) => clone(asObject(item)));
      return { status: 'ok', 'config-version': 8 };
    }
  }
  if (method === 'GET' && path === '/credentials') {
    return { files: clone(state.authFiles), observed_at: new Date().toISOString() };
  }
  if (method === 'GET' && path === '/credentials/download') {
    const file = findAuthFile(state, readString(query.name));
    if (!file) throw new Error(`Browser Mock credential not found: ${readString(query.name)}`);
    return clone(file);
  }
  if (method === 'GET' && path === '/credentials/models') return mockModelDefinitions();
  if (method === 'GET' && path.startsWith('/routing/model-definitions/')) return mockModelDefinitions();
  if (method === 'GET' && path === '/config/oauth/excluded-models') {
    return clone(state.oauthExcludedModels);
  }
  if (method === 'PUT' && path === '/config/oauth/excluded-models') {
    state.oauthExcludedModels = clone(asObject(body)) as Record<string, string[]>;
    return { status: 'ok', 'config-version': 8 };
  }
  if (method === 'PATCH' && (path === '/credentials/fields' || path === '/credentials/status')) {
    const patch = asObject(body);
    const file = findAuthFile(state, readString(patch.name));
    if (!file) throw new Error(`Browser Mock credential not found: ${readString(patch.name)}`);
    Object.assign(file, clone(patch));
    return { status: 'ok' };
  }
  if (method === 'POST' && path === '/routing/cooldown/reset') {
    const authIndex = readString(asObject(body).auth_index);
    const file = state.authFiles.find(file => file.auth_index === authIndex);
    if (!file) throw new Error(`Browser Mock credential not found: ${authIndex}`);
    const models = asArray(file.cooldowns).map(item => readString(asObject(item).model_key)).filter(Boolean);
    file.cooldowns = [];
    return { status: 'ok', auth_index: authIndex, models };
  }
  if (method === 'DELETE' && path === '/credentials') {
    const index = state.authFiles.findIndex((file) => file.name === query.name);
    if (index < 0) throw new Error(`Browser Mock credential not found: ${readString(query.name)}`);
    state.authFiles.splice(index, 1);
    return null;
  }
  if (method === 'POST' && path === '/requests/api-call') return mockApiCall(state, asObject(body));
  if (method === 'DELETE' && path === '/oauth/session') {
    state.oauthSessions.delete(readString(query.state));
    return null;
  }
  throw new Error(`Browser Mock management request not implemented: ${method} ${path}`);
}

const CORE_CONFIG_PATHS: Record<string, string> = {
  debug: 'observability.logs.debug', commercialMode: 'server.commercial-mode',
  loggingToFile: 'observability.logs.logging-to-file', logsMaxTotalSizeMb: 'observability.logs.logs-max-total-size-mb',
  errorLogsMaxFiles: 'observability.logs.error-logs-max-files', usageStatisticsEnabled: 'observability.usage.usage-statistics-enabled',
  redisUsageQueueRetentionSeconds: 'observability.usage.redis-usage-queue-retention-seconds',
  requestLog: 'observability.logs.request-log', pluginsEnabled: 'plugins.enabled',
  host: 'server.host', port: 'server.port', routingStrategy: 'routing.strategy', proxyUrl: 'requests.proxy-url',
  routingSessionAffinity: 'routing.session-affinity', routingSessionAffinityTtl: 'routing.session-affinity-ttl',
  disableCooling: 'routing.cooldown.disable-cooling', requestRetry: 'routing.retry.request-retry',
  maxRetryCredentials: 'routing.retry.max-retry-credentials', maxRetryInterval: 'routing.retry.max-retry-interval',
  streamingBootstrapRetries: 'requests.streaming.bootstrap-retries',
};

function updateCoreConfig(state: BrowserMockState, payload: JsonObject) {
  const settings = asObject(payload.settings);
  Object.assign(state.coreConfig, settings);
  const changes: TemplateConfigChange[] = Object.entries(settings).flatMap(([key, value]) => CORE_CONFIG_PATHS[key] ? [{ path: CORE_CONFIG_PATHS[key].split('.'), value, remove: false, expected: null, expectedExists: false }] : []);
  state.extendedConfig = applyTemplateChanges(state.extendedConfig, changes);
  state.guiSettings.host = state.coreConfig.host;
  state.guiSettings.port = state.coreConfig.port;
  return clone(state.coreConfig);
}

function createActionResult(payload: JsonObject) {
  return {
    outcome: 'updated',
    enabled: true,
    model: readString(payload.model) || 'gpt-5.2-codex',
    changedFiles: ['C:\\Users\\Mock\\.config\\agent\\settings.json'],
    conflictFiles: [],
  };
}

const ERROR_SCENARIO_COMMANDS = new Set([
  'get_core_status',
  'get_gui_settings',
  'get_core_config_settings',
  'get_extended_core_config',
  'check_app_update',
  'check_latest_core',
  'management_request',
  'get_usage_overview',
  'get_core_models',
  'core_health_probe',
  'get_agent_config_statuses',
]);

export function createBrowserMockRuntime(
  scenario: BrowserMockScenario,
  emitEvent: EmitMockEvent = () => {},
  delayMs = 0,
): BrowserMockRuntime {
  const state = createState(scenario);
  if (scenario === 'empty') {
    state.usageEvents = [];
    state.usagePrices = [];
    Object.assign(state.usageOverview, Object.fromEntries(Object.entries(state.usageOverview)
      .filter(([, value]) => typeof value === 'number').map(([key]) => [key, 0])));
    state.usageOverview.timeline = [];
    state.analysis = { models: [], providers: [], sources: [], apiKeys: [] };
    state.usageStorage.databaseSizeBytes = 0;
    state.usageStorage.totalRecords = 0;
    state.coreConfig.apiKeys = [];
  }
  const pluginMock = createPluginMock(state);
  const emit = (event: string, payload: unknown) => emitEvent(event, clone(payload));

  const invoke = async (command: string, rawPayload?: unknown): Promise<unknown> => {
    await sleep(delayMs);
    const payload = asObject(rawPayload);
    if (scenario === 'error' && ERROR_SCENARIO_COMMANDS.has(command)) {
      throw new Error(`Browser Mock error scenario: ${command}`);
    }

    const pluginResponse = pluginMock.handle(command, payload);
    if (pluginResponse.handled) return pluginResponse.value;

    switch (command) {
      case 'plugin:app|version': return '0.2.97-mock';
      case 'plugin:app|name': return 'EasyCLIProxyAPI Browser Mock';
      case 'plugin:app|tauri_version': return '2.x-mock';
      case 'plugin:app|identifier': return 'com.cpa.gui.browser-mock';
      case 'plugin:dialog|open': {
        const options = asObject(payload.options);
        return options.directory
          ? 'C:\\Users\\Mock\\Projects\\demo'
          : 'C:\\Users\\Mock\\Certificates\\mock.pem';
      }
      case 'plugin:dialog|save': return 'C:\\Users\\Mock\\Downloads\\mock-output.json';
      case 'save_usage_events_export': return null;
      case 'detect_core_platform': return { os: 'windows', arch: 'x86_64', assetOs: 'windows', assetArch: 'amd64', archiveKind: 'zip' };
      case 'get_linux_system_theme': return 'light';
      case 'set_app_locale':
      case 'open_external_url':
      case 'open_oauth_url':
      case 'resolve_windows_close_request':
      case 'open_auth_files_directory':
      case 'open_core_logs_directory':
      case 'restart_agent_app':
      case 'restart_codex_app':
      case 'restart_opencode_app':
      case 'check_codex_oauth_login':
      case 'restore_codex_official_config':
      case 'create_agent_config_backup':
      case 'restore_agent_config_backup':
      case 'launch_agent': return null;

      case 'get_core_status': return clone(state.coreStatus);
      case 'start_core_process': {
        if (!state.coreStatus.installed) throw new Error('Browser Mock: install the core first');
        Object.assign(state.coreStatus, { running: true, ready: true, managed: true, starting: false, processId: 42817, message: 'Browser Mock core is running' });
        state.guiSettings.runOnStartup = true;
        emit('core-status-changed', state.coreStatus);
        return clone(state.coreStatus);
      }
      case 'stop_core_process': {
        Object.assign(state.coreStatus, { running: false, ready: false, managed: false, starting: false, processId: null, message: 'Browser Mock core is stopped' });
        state.guiSettings.runOnStartup = false;
        emit('core-status-changed', state.coreStatus);
        return clone(state.coreStatus);
      }
      case 'restart_core_process': {
        if (!state.coreStatus.installed) throw new Error('Browser Mock: install the core first');
        Object.assign(state.coreStatus, { running: true, ready: true, managed: true, starting: false, processId: 42818, message: 'Browser Mock core restarted' });
        emit('core-status-changed', state.coreStatus);
        return clone(state.coreStatus);
      }
      case 'get_gui_settings': return clone(state.guiSettings);
      case 'get_core_config_settings': return clone(state.coreConfig);
      case 'get_extended_core_config': return clone(state.extendedConfig);
      case 'save_extended_core_config': {
        const changes = asArray(payload.changes) as TemplateConfigChange[];
        for (const change of changes) {
          const original = readTemplatePath(state.extendedConfig, change.path);
          if (original.exists !== change.expectedExists || (original.exists && !sameTemplateValue(original.value, change.expected))) throw new Error(`Configuration changed externally: ${change.path.join('.')}. Discard this group's changes and edit the latest values.`);
        }
        const before = state.extendedConfig;
        state.extendedConfig = applyTemplateChanges(before, changes);
        const syncValues: JsonObject = {};
        for (const [key, path] of Object.entries(CORE_CONFIG_PATHS)) {
          if (!changes.some((change) => change.path.join('.') === path)) continue;
          const current = readTemplatePath(state.extendedConfig, path.split('.'));
          if (current.exists) syncValues[key] = current.value;
          else if (key === 'requestLog' || key === 'pluginsEnabled') syncValues[key] = false;
        }
        Object.assign(state.coreConfig, syncValues);
        const restartRequired = changes.some((change) => !sameTemplateValue(readTemplatePath(before, change.path), readTemplatePath(state.extendedConfig, change.path)) && ['server.host', 'server.port', 'server.tls', 'management', 'oauth.auth-dir'].some((prefix) => change.path.join('.') === prefix || change.path.join('.').startsWith(`${prefix}.`)));
        emit('config-files-changed', { paths: ['cpa-core/config.yaml'], errors: [] });
        return { config: clone(state.extendedConfig), restartRequired };
      }
      case 'get_software_settings': return clone(state.softwareSettings);
      case 'get_core_tls_settings': return clone(state.tlsSettings);
      case 'get_core_sensitive_words_settings': return clone(state.sensitiveWords);
      case 'save_core_sensitive_words_settings': {
        state.sensitiveWords = clone(asObject(payload.settings)) as typeof state.sensitiveWords;
        emit('config-files-changed', { paths: ['cpa-core/config.yaml'], errors: [] });
        return clone(state.sensitiveWords);
      }
      case 'save_software_settings': {
        Object.assign(state.softwareSettings, asObject(payload.settings));
        return clone(state.softwareSettings);
      }
      case 'save_core_tls_settings': {
        Object.assign(state.tlsSettings, asObject(payload.settings));
        state.extendedConfig = applyTemplateChanges(state.extendedConfig, [{ path: ['server', 'tls'], value: { enable: state.tlsSettings.enabled, cert: state.tlsSettings.cert, key: state.tlsSettings.key }, remove: false, expected: null, expectedExists: false }]);
        emit('config-files-changed', { paths: ['cpa-core/config.yaml'], errors: [] });
        return clone(state.tlsSettings);
      }
      case 'save_core_logging_settings':
      case 'save_network_endpoint_settings':
      case 'save_retry_settings':
      case 'save_session_routing_settings': {
        const config = updateCoreConfig(state, payload);
        emit('config-files-changed', { paths: ['cpa-core/config.yaml'], errors: [] });
        return config;
      }
      case 'set_core_routing_strategy': {
        const config = updateCoreConfig(state, { settings: { routingStrategy: readString(payload.strategy) } });
        emit('config-files-changed', { paths: ['cpa-core/config.yaml'], errors: [] });
        return config;
      }
      case 'set_core_proxy_url': {
        return updateCoreConfig(state, { settings: { proxyUrl: readString(payload.proxyUrl) } });
      }
      case 'set_core_session_affinity': {
        return updateCoreConfig(state, { settings: { routingSessionAffinity: Boolean(payload.enabled) } });
      }
      case 'set_core_session_affinity_ttl': {
        return updateCoreConfig(state, { settings: { routingSessionAffinityTtl: readString(payload.ttl) } });
      }
      case 'add_core_api_key': {
        state.coreConfig.apiKeys.push({ apiKey: readString(payload.apiKey), remark: readString(payload.remark) });
        return clone(state.coreConfig);
      }
      case 'update_core_api_key': {
        const entry = state.coreConfig.apiKeys.find((item) => item.apiKey === payload.originalApiKey);
        if (entry) Object.assign(entry, { apiKey: readString(payload.apiKey), remark: readString(payload.remark) });
        return clone(state.coreConfig);
      }
      case 'delete_core_api_key': {
        state.coreConfig.apiKeys = state.coreConfig.apiKeys.filter((item) => item.apiKey !== payload.apiKey);
        return clone(state.coreConfig);
      }
      case 'set_core_management_secret_key': {
        state.coreConfig.managementSecretConfigured = true;
        return clone(state.coreConfig);
      }
      case 'clear_core_management_secret_key': {
        state.coreConfig.managementSecretConfigured = false;
        return clone(state.coreConfig);
      }

      case 'check_app_update': return clone(state.appUpdateInfo);
      case 'get_app_update_task': return clone(state.appUpdateTask);
      case 'start_app_update': {
        Object.assign(state.appUpdateTask, {
          running: true,
          cancellable: true,
          phase: 'downloading',
          targetVersion: state.appUpdateInfo.latestVersion,
          downloadedBytes: 18 * 1024 * 1024,
          totalBytes: state.appUpdateInfo.downloadSizeBytes,
          percent: 38,
          message: 'Browser Mock is downloading an update',
        });
        emit('app-update-progress', state.appUpdateTask);
        return null;
      }
      case 'cancel_app_update': {
        Object.assign(state.appUpdateTask, { running: false, cancellable: false, phase: 'cancelled', message: 'Browser Mock update canceled' });
        emit('app-update-progress', state.appUpdateTask);
        return null;
      }
      case 'check_latest_core': return clone(state.coreLatest);
      case 'get_core_install_task': return clone(state.coreInstallTask);
      case 'install_core_version':
      case 'install_bundled_core': {
        const source = command === 'install_bundled_core' ? BUNDLED_CORE : state.coreLatest;
        const result = {
          version: command === 'install_bundled_core' ? source.version : readString(payload.version) || source.version,
          assetName: source.assetName,
          installDir: state.coreStatus.installDir,
          binaryPath: 'C:\\EasyCLIProxyAPI\\cpa-core\\cli-proxy-api.exe',
        };
        Object.assign(state.coreStatus, { installed: true, currentVersion: result.version, binaryPath: result.binaryPath, message: 'Browser Mock core installed' });
        Object.assign(state.coreInstallTask, { running: false, cancellable: false, phase: 'completed', downloaded: 64 * 1024 * 1024, total: 64 * 1024 * 1024, percent: 100, message: 'Browser Mock installation complete', result });
        emit('core-install-progress', state.coreInstallTask);
        emit('core-status-changed', state.coreStatus);
        return clone(result);
      }
      case 'cancel_core_install': return null;
      case 'detect_bundled_core': return clone(BUNDLED_CORE);
      case 'get_version_source_settings': return clone(state.versionSource);
      case 'set_download_source': {
        state.versionSource.source = readString(payload.source) || 'github';
        emit('version-download-source-changed', state.versionSource);
        return clone(state.versionSource);
      }
      case 'add_custom_download_mirror': {
        const url = readString(payload.url);
        if (url && !state.versionSource.customMirrors.includes(url)) state.versionSource.customMirrors.push(url);
        return clone(state.versionSource);
      }
      case 'remove_custom_download_mirror': {
        state.versionSource.customMirrors = state.versionSource.customMirrors.filter((url) => url !== payload.url);
        if (state.versionSource.source === `custom:${payload.url}`) state.versionSource.source = 'github';
        return clone(state.versionSource);
      }
      case 'set_prefer_gitcode_downloads': return clone(state.versionSource);

      case 'management_request': return clone(managementResponse(state, payload));
      case 'import_vertex_credential': {
        const parsed: unknown = JSON.parse(new TextDecoder().decode(Uint8Array.from(asArray(payload.data) as number[])));
        if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('invalid json');
        const serviceAccount = asObject(parsed);
        const projectId = readString(serviceAccount.project_id).trim();
        const email = readString(serviceAccount.client_email).trim();
        if (!projectId || !readString(serviceAccount.private_key).trim()) {
          throw new Error('invalid service account');
        }
        const location = readString(payload.location).trim() || 'us-central1';
        const name = `vertex-${projectId.replace(/[\\/:]/g, '_').replace(/ /g, '-')}.json`;
        const existing = findAuthFile(state, name);
        const file = { name, provider: 'vertex', type: 'vertex', project_id: projectId, email, location,
          service_account: clone(serviceAccount), source: 'file',
          auth_index: existing?.auth_index ?? `mock-vertex-${crypto.randomUUID()}`, modtime: Date.now() };
        if (existing) state.authFiles[state.authFiles.indexOf(existing)] = file; else state.authFiles.push(file);
        return { status: 'ok', project_id: projectId, email, location, 'auth-file': name };
      }
      case 'upload_auth_file': {
        const name = readString(payload.name) || 'uploaded-mock.json';
        const parsed: unknown = JSON.parse(new TextDecoder().decode(Uint8Array.from(asArray(payload.data) as number[])));
        if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('Browser Mock: credential must be a JSON object');
        const content = asObject(parsed);
        const existing = findAuthFile(state, name);
        const file = { ...clone(content), name, provider: readString(content.provider || content.type) || 'codex', source: 'file',
          auth_index: existing?.auth_index ?? `mock-upload-${crypto.randomUUID()}`, modtime: Date.now() };
        if (existing) state.authFiles[state.authFiles.indexOf(existing)] = file; else state.authFiles.push(file);
        return { ok: true, name };
      }
      case 'resolve_api_access_remarks': {
        return asArray(payload.queries).map((query) => {
          const item = asObject(query);
          return readString(item.providerName) || 'Browser Mock Provider';
        });
      }
      case 'save_api_access_remark': return null;
      case 'provider_health_probe': return { firstTokenLatencyMs: 184, responseLatencyMs: 642 };
      case 'core_health_probe': {
        if (!state.coreStatus.ready) throw new Error('Browser Mock: the core is not ready');
        if (!readString(payload.model).trim()) throw new Error('Select a model first');
        return { firstTokenLatencyMs: 184, responseLatencyMs: 642 };
      }

      case 'list_oauth_browsers': return [
        { id: 'default', label: 'System default browser' },
        { id: 'chrome', label: 'Google Chrome (Mock)' },
        { id: 'edge', label: 'Microsoft Edge (Mock)' },
      ];
      case 'start_oauth_login': {
        const provider = readString(payload.provider) || 'codex';
        const session = `mock-oauth-${provider}-${Date.now()}`;
        state.oauthSessions.set(session, provider);
        return { url: `https://example.com/mock-oauth/${provider}?state=${session}`, state: session, opened: true, openError: null };
      }
      case 'get_oauth_status': {
        const session = readString(payload.state);
        const provider = state.oauthSessions.get(session);
        if (!provider) return { status: 'error', error: 'Unknown or canceled mock OAuth session' };
        if (provider && !state.authFiles.some((file) => file.name === `${provider}-mock-login.json`)) {
          state.authFiles.push({
            name: `${provider}-mock-login.json`, provider, type: provider, source: 'file',
            auth_index: `mock-${provider}-${state.authFiles.length + 1}`, priority: 0, disabled: false, modtime: Date.now(),
          });
        }
        return { status: 'ok', error: null };
      }
      case 'submit_oauth_callback': return null;

      case 'get_usage_collector_status': return {
        state: state.coreStatus.ready ? 'collecting' : 'waiting-core',
        message: state.coreStatus.ready ? 'Browser Mock is collecting usage records' : 'Waiting for the core to start',
        lastCollectedAt: new Date().toISOString(),
        totalRecords: state.usageOverview.totalRequests,
      };
      case 'get_usage_overview': return clone(state.usageOverview);
      case 'get_usage_analysis': return clone(state.analysis);
      case 'get_usage_events': {
        const query = asObject(payload.query);
        const page = Math.max(1, readNumber(query.page, 1));
        const pageSize = Math.max(1, readNumber(query.page_size, 50));
        const events = state.usageEvents.filter(event => {
          for (const key of ['model', 'provider', 'source', 'api_key_hash'] as const) {
            if (query[key] && event[key] !== query[key]) return false;
          }
          for (const key of ['failed', 'canceled'] as const) {
            if (typeof query[key] === 'boolean' && event[key] !== query[key]) return false;
          }
          const timestamp = Date.parse(event.timestamp);
          return (!query.start || timestamp >= Date.parse(String(query.start)))
            && (!query.end || timestamp <= Date.parse(String(query.end)));
        });
        const start = (page - 1) * pageSize;
        return {
          items: clone(events.slice(start, start + pageSize)),
          total: events.length,
          page,
          pageSize,
          totalPages: Math.max(1, Math.ceil(events.length / pageSize)),
        };
      }
      case 'get_usage_pricing': {
        const rows = state.usagePrices.map((price, index) => ({
          model: price.model,
          requests: index === 0 ? 182 : 96,
          inputTokens: index === 0 ? 820_000 : 386_000,
          outputTokens: index === 0 ? 312_000 : 142_000,
          cacheReadTokens: index === 0 ? 82_000 : 44_000,
          cacheCreationTokens: index === 0 ? 0 : 18_000,
          totalTokens: index === 0 ? 1_214_000 : 590_000,
          estimatedCost: index === 0 ? 1.92 : 1.71,
          price: clone(price),
        }));
        return { rows, totalCost: rows.length ? 3.63 : 0, totalRequests: rows.length ? 278 : 0, pricedRequests: rows.length ? 278 : 0, savedPrices: state.usagePrices.length };
      }
      case 'get_usage_storage_settings': return clone(state.usageStorage);
      case 'save_usage_storage_settings': {
        state.usageStorage.maxDatabaseSizeMb = readNumber(payload.maxDatabaseSizeMb);
        state.usageStorage.deletedRecords = 0;
        return clone(state.usageStorage);
      }
      case 'shrink_usage_database': {
        const targetMb = readNumber(payload.targetDatabaseSizeMb, 64);
        state.usageStorage.databaseSizeBytes = targetMb * 1024 * 1024;
        state.usageStorage.deletedRecords = 42;
        state.usageStorage.totalRecords = Math.max(0, state.usageStorage.totalRecords - 42);
        return clone(state.usageStorage);
      }
      case 'repair_usage_cache_records': return { scanned: 312, repaired: 3, deleted: 1, backupPath: 'C:\\EasyCLIProxyAPI\\usage-records\\backups\\mock.sqlite' };
      case 'save_usage_model_price': {
        const next = asObject(payload.price ?? payload);
        const model = readString(next.model);
        const current = state.usagePrices.find((price) => price.model === model);
        const normalized = {
          model,
          prompt: readNumber(next.prompt),
          completion: readNumber(next.completion),
          cacheRead: readNumber(next.cacheRead),
          cacheCreation: readNumber(next.cacheCreation),
          promptConfigured: true,
          completionConfigured: true,
          cacheReadConfigured: true,
          cacheCreationConfigured: true,
          source: 'manual',
          sourceModelId: model,
          updatedAtMs: Date.now(),
        };
        if (current) Object.assign(current, normalized); else state.usagePrices.push(normalized);
        return null;
      }
      case 'delete_usage_model_price': {
        state.usagePrices = state.usagePrices.filter((price) => price.model !== payload.model);
        return null;
      }
      case 'preview_usage_model_prices': return {
        source: readString(payload.source) || 'models-dev',
        sourceUrl: 'https://example.com/browser-mock-prices.json',
        matches: clone(state.usagePrices),
        unmatched: ['mock-unknown-model'],
      };
      case 'apply_usage_model_prices': return { imported: asArray(payload.prices).length };

      case 'get_agent_config_statuses':
      case 'refresh_agent_config_statuses': return clone(state.agentStatuses);
      case 'get_core_models': {
        if (!state.coreStatus.ready) throw new Error('Browser Mock: the core is not ready');
        return invoke('get_agent_models', {});
      }
      case 'get_agent_models': return [
        { name: 'gpt-5.2-codex', alias: 'codex-latest', displayName: 'GPT-5.2 Codex', isAlias: true, contextWindow: 272_000, inputModalities: ['text', 'image'] },
        { name: 'claude-opus-4-6', displayName: 'Claude Opus 4.6', contextWindow: 200_000, inputModalities: ['text', 'image'] },
        { name: 'claude-sonnet-4-6', displayName: 'Claude Sonnet 4.6', contextWindow: 200_000, inputModalities: ['text', 'image'] },
        { name: 'claude-haiku-4-5', displayName: 'Claude Haiku 4.5', contextWindow: 200_000, inputModalities: ['text', 'image'] },
        { name: 'gemini-3-pro', displayName: 'Gemini 3 Pro', contextWindow: 1_000_000, inputModalities: ['text', 'image'] },
        { name: 'deepseek-chat', displayName: 'DeepSeek Chat', contextWindow: 64_000, inputModalities: ['text'] },
      ];
      case 'update_agent_config':
      case 'apply_agent_config_template':
      case 'install_pi_provider':
      case 'update_pi_provider':
      case 'repair_pi_provider':
      case 'set_agent_config_enabled':
      case 'close_codex_config_modification': return createActionResult(payload);
      case 'uninstall_pi_provider': return { ...createActionResult(payload), enabled: false, model: null };
      case 'check_pi_provider_update': return { installedVersion: '0.4.2', latestVersion: '0.4.3', updateAvailable: true };
      case 'clear_codex_config': return ['C:\\Users\\Mock\\.codex\\config.toml'];
      case 'preview_agent_config_template': return { revision: 'mock-template-r1', files: ['settings.json', 'models.json'] };
      case 'get_deepseek_harness_process_status': return clone(state.deepSeekStatus);
      case 'stop_deepseek_harness_process': {
        state.deepSeekStatus = { running: false, pid: null, mode: null };
        return clone(state.deepSeekStatus);
      }
      case 'restart_deepseek_harness_process': {
        state.deepSeekStatus = { running: true, pid: 43110, mode: 'web' };
        return clone(state.deepSeekStatus);
      }
      case 'list_agent_config_backups': return {
        versions: [{
          id: 'mock-backup-2026-09-25', createdAt: isoHoursAgo(5), fileCount: 2,
          location: 'C:\\EasyCLIProxyAPI\\agent-backups\\mock-backup-2026-09-25',
          files: [
            { path: 'settings.json', exists: true, size: 1_428 },
            { path: 'models.json', exists: true, size: 3_260 },
          ],
          restorable: true, error: null,
        }],
      };
      case 'preview_agent_config_backup': return {
        revision: 'mock-backup-r1',
        files: [{ path: 'settings.json', exists: true, size: 1_428 }],
        differences: [{ file: 'settings.json', field: 'model', before: 'gpt-5.1-codex', after: 'gpt-5.2-codex' }],
      };
      case 'delete_agent_config_backup': return null;

      case 'get_thinking_aliases': return clone(state.thinkingAliases);
      case 'get_speed_aliases': return clone(state.speedAliases);
      case 'get_model_alias_sources': return clone(state.aliasSources);
      case 'get_thinking_alias_sources': return clone(state.aliasSources);
      case 'get_speed_alias_sources': return clone(state.aliasSources);
      case 'get_model_alias_edit_source': {
        const alias = readString(payload.alias);
        const entry = state.thinkingAliases.find((item) => item.alias === alias);
        const source = state.aliasSources.find((item) => item.model === entry?.sourceModel) ?? state.aliasSources[0];
        return { source: clone(source), effort: entry?.effort ?? null, fast: state.speedAliases.some((item) => item.alias === alias) };
      }
      case 'create_thinking_alias': {
        const request = asObject(payload.request ?? payload);
        const entry = {
          position: { section: 'oauth-model-alias', providerIndex: null, modelIndex: state.thinkingAliases.length },
          sourceModel: readString(request.sourceModel ?? request.model),
          alias: readString(request.alias),
          effort: readString(request.effort) || null,
          provider: readString(request.provider) || 'Browser Mock',
          kind: readString(request.kind) || 'codex-oauth',
          oauthChannel: readString(request.oauthChannel) || null,
        };
        state.thinkingAliases = [...state.thinkingAliases.filter((item) => item.alias !== entry.alias), entry];
        return clone(state.thinkingAliases);
      }
      case 'delete_thinking_alias': {
        state.thinkingAliases = state.thinkingAliases.filter((item) => item.alias !== payload.alias);
        return clone(state.thinkingAliases);
      }
      case 'create_speed_alias': {
        const request = asObject(payload.request ?? payload);
        const entry = {
          position: { section: 'oauth-model-alias', providerIndex: null, modelIndex: state.speedAliases.length },
          sourceModel: readString(request.sourceModel ?? request.model),
          alias: readString(request.alias),
          serviceTier: readString(request.serviceTier) || 'priority',
          provider: readString(request.provider) || 'Browser Mock',
          kind: readString(request.kind) || 'codex-oauth',
          oauthChannel: readString(request.oauthChannel) || null,
        };
        state.speedAliases = [...state.speedAliases.filter((item) => item.alias !== entry.alias), entry];
        return clone(state.speedAliases);
      }
      case 'delete_speed_alias': {
        state.speedAliases = state.speedAliases.filter((item) => item.alias !== payload.alias);
        return clone(state.speedAliases);
      }

      case 'get_codex_model_catalog_editor': return clone(state.codexCatalog);
      case 'save_codex_model_catalog_editor': {
        const request = asObject(payload.request);
        const models = asArray(request.models).map(asObject);
        state.codexCatalog.models = state.codexCatalog.models.map((model) => {
          const update = models.find((item) => item.slug === model.slug);
          return update ? { ...model, configuration: clone(asObject(update.configuration)) as typeof model.configuration, customized: true } : model;
        });
        state.codexCatalog.revision = `mock-catalog-${Date.now()}`;
        return { snapshot: clone(state.codexCatalog), synchronizationError: null };
      }
      case 'get_deepseek_harness_model_catalog_editor': return clone(state.harnessCatalog);
      case 'save_deepseek_harness_model_catalog_editor': {
        const request = asObject(payload.request);
        state.harnessCatalog = { ...state.harnessCatalog, ...clone(request), revision: `mock-harness-${Date.now()}`, configured: true } as typeof state.harnessCatalog;
        return clone(state.harnessCatalog);
      }

      case 'list_codex_sessions': {
        const request = asObject(payload.request);
        const offset = Math.max(0, readNumber(request.offset));
        const limit = Math.max(1, readNumber(request.limit, 50));
        return {
          codexHome: 'C:\\Users\\Mock\\.codex',
          databasePaths: ['C:\\Users\\Mock\\.codex\\state.sqlite'],
          sessions: clone(state.codexSessions.slice(offset, offset + limit)),
          totalCount: state.codexSessions.length,
          offset,
          limit,
          hasMore: offset + limit < state.codexSessions.length,
          warnings: [],
        };
      }
      case 'get_codex_session_context': {
        const request = asObject(payload.request);
        const sessionId = String(request.sessionId || 'mock-session-id');
        return clone(mockSessionContext(state, sessionId));
      }
      case 'open_codex_session_rollout': {
        const sessionId = String(asObject(payload.request).sessionId || '');
        if (!mockSessionContext(state, sessionId).rolloutPath) throw new Error('This session has no rollout file to open');
        return null;
      }
      case 'save_codex_session_context': {
        const request = asObject(payload.request);
        const sessionId = String(request.sessionId || '');
        const context = clone(mockSessionContext(state, sessionId));
        if (typeof request.title === 'string') context.title = request.title;
        if (typeof request.cwd === 'string') context.cwd = request.cwd;
        if (typeof request.model === 'string') context.model = request.model;
        if (typeof request.modelProvider === 'string') context.modelProvider = request.modelProvider;
        if (typeof request.archived === 'boolean') context.archived = request.archived;
        context.updatedAtMs = Date.now();

          for (const entry of asArray(request.messageUpdates)) {
            const update = asObject(entry);
            const index = context.messages.findIndex((message) => message.lineNumber === Number(update.lineNumber));
            if (index < 0) continue;
            if (update.deleted) {
              context.messages.splice(index, 1);
              continue;
            }
            if (typeof update.content === 'string') context.messages[index].content = update.content;
            if (typeof update.role === 'string') context.messages[index].role = update.role;
          }
          let nextLine = context.messages.reduce((max, message) => Math.max(max, message.lineNumber), 0) + 1;
          for (const entry of asArray(request.newMessages)) {
            const message = asObject(entry);
            context.messages.push({
              id: `mock-message-${nextLine}`,
              lineNumber: nextLine,
              role: String(message.role || 'user'),
              timestamp: new Date().toISOString(),
              content: String(message.content || ''),
              rawType: 'response_item',
              model: context.model,
              callId: null,
            });
            nextLine += 1;
          }
          context.messages.forEach((message, index) => {
            message.lineNumber = index + 2;
            message.id = `msg-${message.lineNumber}`;
          });
          context.rawJsonl = mockSessionRawJsonl(context);

        context.stats = mockSessionStats(context);
        state.codexSessionContexts[sessionId] = context;

        const target = state.codexSessions.find((item) => item.id === sessionId);
        if (target) {
          target.title = context.title;
          target.cwd = context.cwd;
          target.modelProvider = context.modelProvider;
          target.archived = context.archived;
          target.updatedAtMs = context.updatedAtMs;
        }

        return {
          success: true,
          sessionId,
          backupPath: `C:\\Users\\Mock\\.codex\\backups\\session-context-edits\\session-${sessionId}`,
          message: 'Session context updated in Mock',
        };
      }
      case 'delete_codex_sessions': {
        const request = asObject(payload.request);
        const ids = asArray(request.sessionIds).map(String);
        state.codexSessions = state.codexSessions.filter((session) => !ids.includes(session.id));
        return {
          results: ids.map((id) => ({ sessionId: id, status: 'deleted', message: 'Deleted by Browser Mock', backupPath: 'C:\\Users\\Mock\\.codex\\backups\\mock.zip' })),
          deletedCount: ids.length,
          failedCount: 0,
        };
      }
      case 'repair_codex_session_metadata': {
        emit('codex-session-repair-progress', { phase: 'complete', percent: 100, processed: state.codexSessions.length, total: state.codexSessions.length });
        return {
          targetProvider: 'cpa-gui', changedRolloutFiles: 2, sqliteRowsUpdated: 2,
          skippedLockedFiles: [], backupPath: 'C:\\Users\\Mock\\.codex\\backups\\repair.zip',
          encryptedContentWarning: null, warnings: [],
        };
      }
      case 'preview_codex_session_index_cleanup': return {
        snapshotSha256: 'mock-snapshot-sha256',
        candidates: [{ id: 'stale-thread-1', threadName: 'Missing session', updatedAt: isoHoursAgo(72) }],
      };
      case 'apply_codex_session_index_cleanup': return { prunedEntries: asArray(asObject(payload.request).threadIds).length, backupPath: 'C:\\Users\\Mock\\.codex\\backups\\index.json' };

      case 'get_lan_ipv4': return ['192.168.100.100'];
      default:
        if (command.startsWith('plugin:window|') || command.startsWith('plugin:webview|')) return null;
        console.warn(`[Browser Mock] Command not implemented: ${command}`, rawPayload);
        throw new Error(`Browser Mock command not implemented: ${command}`);
    }
  };

  return { scenario, invoke };
}
