import { describe, expect, it } from 'bun:test';
import {
  authFileCooldownResetIndex,
  changedOAuthAuthFileNames,
  dedupeAuthFiles,
  groupOAuthAccountsByProvider,
  isOAuthCredentialFile,
  normalizeAuthFilePriorityInput,
  normalizeOAuthProvider,
  oauthModelProvidersFromAuthFiles,
  parseAuthFilePriority,
  setOAuthCredentialFileDisabled,
  sortAuthFilesByPriority,
  snapshotAuthFiles,
} from '../src/services/authFiles';

describe('认证文件列表规范化', () => {
  it('合并同名的磁盘和运行时记录并优先保留磁盘状态', () => {
    const files = dedupeAuthFiles([
      {
        name: 'codex-user.json',
        provider: 'codex',
        runtime_only: true,
        account_type: 'api_key',
        auth_index: 'runtime-index',
        email: 'user@example.com',
      },
      {
        name: 'codex-user.json',
        provider: 'codex',
        source: 'file',
        path: '/tmp/codex-user.json',
        disabled: false,
        modtime: 100,
      },
    ]);

    expect(files).toHaveLength(1);
    expect(files[0].source).toBe('file');
    expect(files[0].path).toBe('/tmp/codex-user.json');
    expect(files[0].email).toBe('user@example.com');
    expect(files[0].auth_index).toBe('runtime-index');
    expect(files[0].runtime_only).toBeUndefined();
    expect(files[0].account_type).toBeUndefined();
    expect(isOAuthCredentialFile(files[0])).toBe(true);
  });

  const cooldown = {
    scope: 'model', model_key: 'model-b', reason: 'quota',
    retry_at: '2040-01-01T00:00:00Z', remaining_seconds: 30,
  };

  it('keeps different auth indexes separate when filenames match', () => {
    const disk = { name: 'same.json', auth_index: 'A', source: 'file', path: '/synthetic/a', disabled: false };
    const runtime = { name: 'same.json', auth_index: 'B', runtime_only: true, cooldowns: [cooldown] };
    for (const entries of [[disk, runtime], [runtime, disk]]) {
      const files = dedupeAuthFiles(entries);
      expect(files).toHaveLength(2);
      expect(files.find((file) => authFileCooldownResetIndex(file) === 'A')).toEqual(disk);
      expect(files.find((file) => authFileCooldownResetIndex(file) === 'B')).toEqual(runtime);
    }
  });

  it('merges the same normalized index and preserves atomic empty or unknown cooldowns', () => {
    for (const cooldowns of [[], null]) {
      const files = dedupeAuthFiles([
        { name: 'same.json', auth_index: 0, source: 'file', path: '/synthetic/a', cooldowns },
        { name: 'same.json', authIndex: ' 0 ', runtime_only: true, email: 'account@example.test', cooldowns: [cooldown] },
        { name: 'same.json', auth_index: 'B', runtime_only: true, cooldowns: [cooldown] },
      ]);
      expect(files).toHaveLength(2);
      const sameIdentity = files.find((file) => authFileCooldownResetIndex(file) === '0')!;
      expect(sameIdentity.path).toBe('/synthetic/a');
      expect(sameIdentity.email).toBe('account@example.test');
      expect(sameIdentity.runtime_only).toBeUndefined();
      expect(sameIdentity.cooldowns).toEqual(cooldowns);
      expect(files.find((file) => authFileCooldownResetIndex(file) === 'B')?.cooldowns).toEqual([cooldown]);
    }
  });

  it('keeps unindexed records separate when the filename has conflicting identities', () => {
    const files = dedupeAuthFiles([
      { name: 'same.json', source: 'file', path: '/synthetic/unknown', cooldowns: [cooldown] },
      { name: 'same.json', auth_index: 'A', runtime_only: true, cooldowns: [] },
      { name: 'same.json', authIndex: 'B', runtime_only: true, cooldowns: null },
    ]);
    expect(files).toHaveLength(3);
    const unknown = files.find((file) => authFileCooldownResetIndex(file) === undefined)!;
    expect(unknown.path).toBe('/synthetic/unknown');
    expect(unknown.cooldowns).toEqual([cooldown]);
    for (const index of ['A', 'B']) {
      const file = files.find((entry) => authFileCooldownResetIndex(entry) === index)!;
      expect(file.path).toBeUndefined();
      expect(file.runtime_only).toBe(true);
      expect(file.cooldowns).toEqual(index === 'A' ? [] : null);
    }
  });
});

const oauthFile = { name: 'codex-user.json', provider: 'codex', source: 'file', account_type: 'oauth' };
const nonOAuthFiles = [
  { ...oauthFile, runtime_only: true },
  { ...oauthFile, runtimeOnly: true },
  { ...oauthFile, account_type: 'api_key' },
  { ...oauthFile, account_type: 'API-KEY' },
  { ...oauthFile, auth_kind: 'apikey' },
  { ...oauthFile, authKind: 'api_key' },
  { ...oauthFile, source: 'memory' },
  { ...oauthFile, source: 'config:codex[key]' },
  { ...oauthFile, name: 'codex:apikey:runtime-id' },
  { ...oauthFile, name: '' },
];

describe('OAuth credential file boundaries', () => {
  it('accepts disk-backed OAuth files, disabled files, and legacy disk listings', () => {
    expect(isOAuthCredentialFile(oauthFile)).toBe(true);
    expect(isOAuthCredentialFile({ ...oauthFile, disabled: true })).toBe(true);
    expect(isOAuthCredentialFile({ name: 'legacy.JSON', type: 'codex' })).toBe(true);
    expect(isOAuthCredentialFile({})).toBe(false);
  });

  it('rejects API-key and runtime records even when they use an OAuth provider or JSON name', () => {
    for (const file of nonOAuthFiles) expect(isOAuthCredentialFile(file)).toBe(false);
  });

  it('offers only providers with OAuth credential files, preserving aliases and plugin providers', () => {
    expect(oauthModelProvidersFromAuthFiles([
      ...nonOAuthFiles.map((file) => ({ ...file, provider: 'api-only' })),
      oauthFile,
      { ...oauthFile, name: 'second.json' },
      { name: 'claude.json', type: 'anthropic' },
      { name: 'devin.json', type: 'cognition' },
      { name: 'antigravity.json', type: 'anti-gravity' },
      { name: 'openai.json', type: 'openai' },
      { name: 'muse.json', type: 'muse' },
      { name: 'plugin.json', provider: 'custom-oauth', source: 'file' },
      { name: 'unknown.json' },
    ])).toEqual(['antigravity', 'claude', 'codex', 'custom-oauth', 'devin', 'meta']);
    expect(oauthModelProvidersFromAuthFiles(nonOAuthFiles)).toEqual([]);
  });

  it('enables and disables only the selected OAuth file without touching API configuration', async () => {
    const writes: unknown[] = [];
    const api = { patch: async (path: string, body: Record<string, unknown>) => { writes.push({ path, body }); } };
    await setOAuthCredentialFileDisabled(oauthFile, true, api);
    await setOAuthCredentialFileDisabled({ ...oauthFile, disabled: true }, false, api);
    expect(writes).toEqual([
      { path: '/credentials/status', body: { name: 'codex-user.json', disabled: true } },
      { path: '/credentials/status', body: { name: 'codex-user.json', disabled: false } },
    ]);
  });

  it('rejects runtime and API-key status changes before any management request', async () => {
    const writes: unknown[] = [];
    const api = { patch: async (path: string, body: Record<string, unknown>) => { writes.push({ path, body }); } };
    for (const file of nonOAuthFiles) {
      for (const disabled of [true, false]) {
        await expect(setOAuthCredentialFileDisabled(file, disabled, api)).rejects.toThrow('OAuth');
      }
    }
    expect(writes).toEqual([]);
  });

  it('normalizes Meta OAuth aliases used by the core and Management Center', () => {
    expect(normalizeOAuthProvider(' Muse ')).toBe('meta');
    expect(normalizeOAuthProvider('meta_ai')).toBe('meta_ai');
    expect(normalizeOAuthProvider('anthropic')).toBe('claude');
  });
});

describe('authentication file priority', () => {
  it('accepts safe integers from API values', () => {
    expect(parseAuthFilePriority(10)).toBe(10);
    expect(parseAuthFilePriority(' -3 ')).toBe(-3);
    expect(parseAuthFilePriority(1.5)).toBeUndefined();
    expect(parseAuthFilePriority('high')).toBeUndefined();
  });

  it('uses zero to restore the default and rejects invalid input', () => {
    expect(normalizeAuthFilePriorityInput('')).toBe(0);
    expect(normalizeAuthFilePriorityInput('0')).toBe(0);
    expect(normalizeAuthFilePriorityInput('12')).toBe(12);
    expect(normalizeAuthFilePriorityInput('1.5')).toBeNull();
  });

  it('sorts credentials by descending priority, then filename', () => {
    expect(sortAuthFilesByPriority([
      { name: 'zeta.json', priority: 2 },
      { name: 'Beta.json', priority: 5 },
      { name: 'alpha.json', priority: 5 },
      { name: 'default.json' },
      { name: 'negative.json', priority: -1 },
    ]).map((file) => file.name)).toEqual([
      'alpha.json', 'Beta.json', 'zeta.json', 'default.json', 'negative.json',
    ]);
  });

  it('finds only credentials created or updated by the completed OAuth provider', () => {
    const before = snapshotAuthFiles([
      { name: 'codex-old.json', provider: 'codex', modtime: 1, priority: 8 },
      { name: 'codex-custom.json', provider: 'codex', modtime: 1, priority: 5 },
      { name: 'claude-old.json', provider: 'claude', modtime: 1 },
    ]);

    expect(changedOAuthAuthFileNames(before, [
      { name: 'codex-old.json', provider: 'codex', modtime: 2 },
      { name: 'codex-custom.json', provider: 'codex', modtime: 2, priority: 5 },
      { name: 'codex-new.json', type: 'codex', modtime: 2 },
      { name: 'claude-old.json', provider: 'claude', modtime: 2 },
    ], 'codex')).toEqual(['codex-old.json', 'codex-new.json']);
  });

  it('applies the default priority to a newly created Muse credential through the muse alias', () => {
    const before = snapshotAuthFiles([
      { name: 'muse-existing.json', provider: 'muse', modtime: 1 },
    ]);
    expect(changedOAuthAuthFileNames(before, [
      { name: 'muse-existing.json', provider: 'muse', modtime: 2 },
      { name: 'muse-new.json', provider: 'meta', modtime: 2 },
      { name: 'muse-priority.json', provider: 'muse', modtime: 2, priority: 4 },
    ], 'muse')).toEqual(['muse-existing.json', 'muse-new.json']);
  });
});

describe('OAuth login account summary', () => {
  it('groups saved OAuth files by normalized provider and skips runtime or API key entries', () => {
    const grouped = groupOAuthAccountsByProvider([
      { name: 'codex-a.json', provider: 'codex', email: 'a@example.com' },
      { name: 'codex-b.json', provider: 'openai', email: 'b@example.com', disabled: true },
      { name: 'claude-a.json', type: 'anthropic', label: 'Work' },
      { name: 'kimi.json', provider: 'kimi' },
      { name: 'codex-runtime.json', provider: 'codex', runtime_only: true },
      { name: 'codex-key.json', provider: 'codex', account_type: 'api_key' },
      { name: 'codex-a.json', provider: 'codex', email: 'a@example.com', runtime_only: true },
    ]);

    expect(grouped.get('codex')).toEqual([
      { name: 'codex-a.json', label: 'a@example.com', disabled: false, identities: ['a@example.com'] },
      { name: 'codex-b.json', label: 'b@example.com', disabled: true, identities: ['b@example.com'] },
    ]);
    expect(grouped.get('claude')).toEqual([{ name: 'claude-a.json', label: 'Work', disabled: false, identities: [] }]);
    expect(grouped.get('kimi')).toEqual([{ name: 'kimi.json', label: 'kimi.json', disabled: false, identities: [] }]);
  });
});
