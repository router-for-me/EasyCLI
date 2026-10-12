import { isRecord, managementApi, readBoolean, readString } from './managementApi';
import { getCurrentLocale, translate } from '../i18n';

export type AuthFileRecord = Record<string, unknown>;
export type AuthFileSnapshot = Map<string, string>;

export type AuthFileCooldownResetResponse = {
  status: 'ok';
  auth_index: string;
  models: string[];
};

export const authFileCooldownResetIndex = (file: AuthFileRecord): string | undefined => {
  const value = file.auth_index ?? file.authIndex;
  if (typeof value === 'string') return value.trim() || undefined;
  if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) return String(value);
  return undefined;
};

export const resetAuthFileCooldown = async (
  authIndex: string,
  api: { post: (path: string, body: Record<string, unknown>) => Promise<unknown> } = managementApi,
): Promise<AuthFileCooldownResetResponse> => {
  const index = authFileCooldownResetIndex({ auth_index: authIndex });
  if (!index) throw new Error(translate(getCurrentLocale(), 'authFiles.cooldown.missingIndex'));
  const response = await api.post('/routing/cooldown/reset', { auth_index: index });
  // Only an acknowledgement for this index confirms the operation. The caller
  // reloads the list to obtain authoritative health and disabled state afterward.
  if (!isRecord(response) || response.status !== 'ok' || response.auth_index !== index
    || (response.models != null && (!Array.isArray(response.models)
      || response.models.some((model) => typeof model !== 'string')))) {
    throw new Error(translate(getCurrentLocale(), 'authFiles.cooldown.invalidResponse'));
  }
  return { status: 'ok', auth_index: index, models: (response.models ?? []) as string[] };
};

export const authFileName = (file: AuthFileRecord) =>
  readString(file, 'name') || translate(getCurrentLocale(), 'authFiles.unnamed');

/** Account identifiers that privacy mode masks wherever this credential is shown. */
export const authFileIdentities = (file: AuthFileRecord): string[] =>
  [...new Set(['email', 'account', 'account_id', 'project_id', 'projectId']
    .map((key) => readString(file, key).trim())
    .filter(Boolean))];

/**
 * Present credentials in the same order used by routing: higher explicit
 * priority first, then a deterministic case-insensitive filename order.
 * Return a new array so callers do not mutate the API response or snapshots.
 */
export const sortAuthFilesByPriority = <File extends AuthFileRecord>(files: File[]): File[] =>
  [...files].sort((left, right) => {
    const priorityDelta = (parseAuthFilePriority(right.priority) ?? 0)
      - (parseAuthFilePriority(left.priority) ?? 0);
    if (priorityDelta !== 0) return priorityDelta;
    return authFileName(left).localeCompare(authFileName(right), undefined, { sensitivity: 'base' });
  });

export const isRuntimeOnlyAuthFile = (file: AuthFileRecord) =>
  readBoolean(file, 'runtime_only', 'runtimeOnly');

export const isOAuthCredentialFile = (file: AuthFileRecord): boolean => {
  if (!readString(file, 'name').toLowerCase().endsWith('.json') || isRuntimeOnlyAuthFile(file)) return false;
  const kinds = ['account_type', 'auth_kind', 'authKind'].map((key) =>
    readString(file, key).toLowerCase().replace(/[-_]/g, ''));
  if (kinds.includes('apikey')) return false;
  const source = readString(file, 'source').toLowerCase();
  return !source || source === 'file';
};

export const setOAuthCredentialFileDisabled = async (
  file: AuthFileRecord,
  disabled: boolean,
  api: { patch: (path: string, body: Record<string, unknown>) => Promise<unknown> } = managementApi,
): Promise<void> => {
  if (!isOAuthCredentialFile(file)) {
    throw new Error(translate(getCurrentLocale(), 'authFiles.fileOnly'));
  }
  await api.patch('/credentials/status', { name: readString(file, 'name'), disabled });
};

export const parseAuthFilePriority = (value: unknown): number | undefined => {
  if (typeof value === 'number') {
    return Number.isSafeInteger(value) ? value : undefined;
  }
  if (typeof value !== 'string') return undefined;
  const normalized = value.trim();
  if (!/^-?\d+$/.test(normalized)) return undefined;
  const priority = Number(normalized);
  return Number.isSafeInteger(priority) ? priority : undefined;
};

export const normalizeAuthFilePriorityInput = (value: string): number | null => {
  const normalized = value.trim();
  if (!normalized) return 0;
  return parseAuthFilePriority(normalized) ?? null;
};

export const normalizeOAuthProvider = (value: string) => {
  const provider = value.trim().toLowerCase();
  if (provider === 'cognition') return 'devin';
  if (provider === 'anthropic') return 'claude';
  if (provider === 'anti-gravity') return 'antigravity';
  if (provider === 'openai') return 'codex';
  if (provider === 'muse') return 'meta';
  return provider;
};

const authFileProvider = (file: AuthFileRecord) =>
  normalizeOAuthProvider(readString(file, 'provider', 'type'));

export const oauthModelProvidersFromAuthFiles = (files: AuthFileRecord[]): string[] =>
  [...new Set(files.filter(isOAuthCredentialFile).map(authFileProvider).filter(Boolean))].sort();

export type OAuthAccountSummary = {
  name: string;
  label: string;
  disabled: boolean;
  /** Account identifiers that privacy mode masks inside `label`. */
  identities: string[];
};

/** Group saved OAuth credential files by provider so login cards can show existing accounts. */
export const groupOAuthAccountsByProvider = (files: AuthFileRecord[]): Map<string, OAuthAccountSummary[]> => {
  const grouped = new Map<string, OAuthAccountSummary[]>();
  dedupeAuthFiles(files).filter(isOAuthCredentialFile).forEach((file) => {
    const provider = authFileProvider(file);
    if (!provider) return;
    const accounts = grouped.get(provider) ?? [];
    accounts.push({
      name: readString(file, 'name'),
      label: readString(file, 'email', 'account', 'label') || authFileName(file),
      disabled: readBoolean(file, 'disabled'),
      identities: authFileIdentities(file),
    });
    grouped.set(provider, accounts);
  });
  return grouped;
};

export const snapshotAuthFiles = (files: AuthFileRecord[]): AuthFileSnapshot => {
  const grouped = new Map<string, string[]>();
  files.forEach((file) => {
    const name = readString(file, 'name');
    if (!name) return;
    const fingerprints = grouped.get(name) ?? [];
    fingerprints.push(JSON.stringify(file));
    grouped.set(name, fingerprints);
  });
  return new Map(Array.from(grouped, ([name, fingerprints]) => [
    name,
    fingerprints.sort().join('\n'),
  ]));
};

export const changedOAuthAuthFileNames = (
  before: AuthFileSnapshot,
  files: AuthFileRecord[],
  provider: string,
) => {
  const expectedProvider = normalizeOAuthProvider(provider);
  const after = snapshotAuthFiles(files);
  const names = new Set<string>();
  files.forEach((file) => {
    const name = readString(file, 'name');
    if (!name || authFileProvider(file) !== expectedProvider) return;
    const priority = parseAuthFilePriority(file.priority);
    if (priority !== undefined && priority !== 0) return;
    if (before.get(name) !== after.get(name)) names.add(name);
  });
  return Array.from(names);
};

const hasMeaningfulValue = (value: unknown) => {
  if (value === null || value === undefined) return false;
  if (typeof value === 'string') return value.trim().length > 0;
  if (Array.isArray(value)) return value.length > 0;
  return true;
};

const authFileTimestamp = (file: AuthFileRecord) => {
  for (const value of [file.modtime, file.updated_at, file.last_refresh]) {
    if (value === null || value === undefined || value === '') continue;
    const numeric = typeof value === 'number' ? value : Number(value);
    if (Number.isFinite(numeric)) return numeric < 1e12 ? numeric * 1000 : numeric;
    const parsed = new Date(String(value)).getTime();
    if (!Number.isNaN(parsed)) return parsed;
  }
  return 0;
};

const authFilePriority = (file: AuthFileRecord) => {
  let score = 0;
  if (readString(file, 'source').toLowerCase() === 'file') score += 32;
  if (readString(file, 'path')) score += 16;
  if (!isRuntimeOnlyAuthFile(file)) score += 8;
  if (!readBoolean(file, 'disabled')) score += 4;
  if (authFileTimestamp(file) > 0) score += 2;
  return score;
};

const authFileSourceFields = new Set([
  'source', 'path', 'runtime_only', 'runtimeOnly', 'account_type', 'auth_kind', 'authKind',
]);

const mergeDuplicateAuthFiles = (entries: AuthFileRecord[]) => {
  const sorted = [...entries].sort((left, right) => {
    const priority = authFilePriority(right) - authFilePriority(left);
    if (priority !== 0) return priority;
    const timestamp = authFileTimestamp(right) - authFileTimestamp(left);
    if (timestamp !== 0) return timestamp;
    return Object.values(right).filter(hasMeaningfulValue).length
      - Object.values(left).filter(hasMeaningfulValue).length;
  });
  const merged = { ...sorted[0] };
  sorted.slice(1).forEach((entry) => {
    Object.entries(entry).forEach(([key, value]) => {
      if (authFileSourceFields.has(key)) return;
      if (key === 'cooldowns' && Object.prototype.hasOwnProperty.call(merged, key)) return;
      if (!hasMeaningfulValue(merged[key]) && hasMeaningfulValue(value)) merged[key] = value;
    });
  });
  return merged;
};

export const dedupeAuthFiles = (files: AuthFileRecord[]) => {
  const grouped = new Map<string, AuthFileRecord[]>();
  files.forEach((file, index) => {
    const key = authFileName(file) || `unnamed-${index}`;
    const entries = grouped.get(key) ?? [];
    entries.push(file);
    grouped.set(key, entries);
  });
  return Array.from(grouped.values())
    .flatMap((entries) => {
      const byAuthIndex = new Map<string | undefined, AuthFileRecord[]>();
      entries.forEach((entry) => {
        const authIndex = authFileCooldownResetIndex(entry);
        const matches = byAuthIndex.get(authIndex) ?? [];
        matches.push(entry);
        byAuthIndex.set(authIndex, matches);
      });
      const knownIndexCount = byAuthIndex.size - (byAuthIndex.has(undefined) ? 1 : 0);
      // Legacy disk entries can enrich one indexed runtime record. Conflicting
      // indexes identify different credentials; unindexed entries stay separate.
      return knownIndexCount <= 1
        ? [mergeDuplicateAuthFiles(entries)]
        : Array.from(byAuthIndex.values()).map(mergeDuplicateAuthFiles);
    })
    .sort((left, right) => {
      const nameOrder = authFileName(left).localeCompare(authFileName(right), undefined, { sensitivity: 'base' });
      return nameOrder || (authFileCooldownResetIndex(left) ?? '')
        .localeCompare(authFileCooldownResetIndex(right) ?? '');
    });
};
