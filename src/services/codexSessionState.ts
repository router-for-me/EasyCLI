export type CodexSessionSummary = {
  id: string;
  title: string;
  cwd: string;
  modelProvider: string;
  archived: boolean;
  updatedAtMs: number | null;
  databasePath: string;
};

export type CodexSessionPage = {
  codexHome: string;
  databasePaths: string[];
  sessions: CodexSessionSummary[];
  totalCount: number;
  offset: number;
  limit: number;
  hasMore: boolean;
  warnings: string[];
};

export type CodexSessionDeleteResult = {
  sessionId: string;
  status: 'deleted' | 'partial' | 'notFound' | 'failed';
  message: string;
  backupPath: string | null;
};

export type CodexSessionDeleteBatchResult = {
  results: CodexSessionDeleteResult[];
  deletedCount: number;
  failedCount: number;
};

export type CodexSessionRepairProgress = {
  phase: 'scanning' | 'backingUp' | 'rewriting' | 'updatingDatabase' | 'complete';
  percent: number;
  processed: number;
  total: number;
};

export type CodexSessionRepairResult = {
  targetProvider: string;
  changedRolloutFiles: number;
  sqliteRowsUpdated: number;
  skippedLockedFiles: string[];
  backupPath: string | null;
  encryptedContentWarning: string | null;
  warnings: string[];
};

export type SessionIndexCleanupCandidate = {
  id: string;
  threadName: string;
  updatedAt: string;
};

export type SessionIndexCleanupPreview = {
  snapshotSha256: string;
  candidates: SessionIndexCleanupCandidate[];
};

export type SessionIndexCleanupResult = {
  prunedEntries: number;
  backupPath: string | null;
};

export type CodexSessionPageCounts = {
  active: number;
  archived: number;
};

export function codexSessionPageCounts(
  sessions: readonly Pick<CodexSessionSummary, 'archived'>[],
): CodexSessionPageCounts {
  const archived = sessions.reduce(
    (count, session) => count + Number(session.archived),
    0,
  );
  return { active: sessions.length - archived, archived };
}

export function retainVisibleCodexSessionIds(
  selectedIds: ReadonlySet<string>,
  sessions: readonly Pick<CodexSessionSummary, 'id'>[],
): Set<string> {
  const visibleIds = new Set(sessions.map((session) => session.id));
  return new Set(Array.from(selectedIds).filter((id) => visibleIds.has(id)));
}

export type CodexSessionMessageItem = {
  id: string;
  lineNumber: number;
  role: 'user' | 'assistant' | 'system' | 'tool' | 'turn_context' | 'reasoning' | string;
  timestamp: string | null;
  content: string;
  rawType: string;
  model: string | null;
  callId: string | null;
  signatureStripped?: boolean;
};

export type CodexSessionContextStats = {
  totalLines: number;
  messageCount: number;
  userMessageCount: number;
  assistantMessageCount: number;
  toolCount: number;
  reasoningCount?: number;
  fileSizeBytes: number;
};

export type CodexSessionContextDetail = {
  id: string;
  title: string;
  cwd: string;
  modelProvider: string;
  model: string | null;
  archived: boolean;
  updatedAtMs: number | null;
  rolloutPath: string | null;
  databasePath: string | null;
  messages: CodexSessionMessageItem[];
  stats: CodexSessionContextStats;
  rawJsonl: string | null;
  rawJsonlAvailable: boolean;
  rolloutSha256: string | null;
};

export type CodexSessionMessageUpdate = {
  lineNumber: number;
  role?: string;
  content?: string;
  deleted?: boolean;
};

export type CodexSessionNewMessage = {
  role: string;
  content: string;
};

export type SaveCodexSessionContextRequest = {
  sessionId: string;
  expectedRolloutSha256?: string;
  title?: string;
  cwd?: string;
  modelProvider?: string;
  model?: string;
  archived?: boolean;
  messageUpdates?: CodexSessionMessageUpdate[];
  newMessages?: CodexSessionNewMessage[];
};

export type SaveCodexSessionContextResult = {
  success: boolean;
  sessionId: string;
  backupPath: string | null;
  message: string;
};
