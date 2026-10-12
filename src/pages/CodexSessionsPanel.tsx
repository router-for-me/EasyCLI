import { MessageNotice } from '../appNotice';
import { CodexSessionContextEditor } from './CodexSessionContextEditor';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  ArrowLeft,
  ArrowRight,
  Archive,
  Database,
  FileText,
  LoaderCircle,
  RefreshCw,
  ScanSearch,
  ShieldCheck,
  Trash2,
  TriangleAlert,
  X,
} from 'lucide-react';
import { useI18n } from '../i18n';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import {
  codexSessionPageCounts,
  retainVisibleCodexSessionIds,
  type CodexSessionDeleteBatchResult,
  type CodexSessionPage,
  type CodexSessionRepairProgress,
  type CodexSessionRepairResult,
  type CodexSessionSummary,
  type SessionIndexCleanupPreview,
  type SessionIndexCleanupResult,
} from '../services/codexSessionState';

type Notice = {
  kind: 'success' | 'warning' | 'error';
  message: string;
};

type DeleteConfirmation = {
  ids: string[];
  title: string;
  description: string;
};

const PAGE_SIZE = 50;

let sessionViewCache = {
  offset: 0,
  selectionMode: false,
  selectedIds: new Set<string>(),
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function CodexSessionsPanel() {
  const { formatDate, t } = useI18n();
  const [page, setPage] = useState<CodexSessionPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [operation, setOperation] = useState<'delete' | 'repair' | 'preview' | 'cleanup' | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [selectionMode, setSelectionMode] = useState(() => sessionViewCache.selectionMode);
  const [selectedIds, setSelectedIds] = useState(() => new Set(sessionViewCache.selectedIds));
  const [deleteConfirmation, setDeleteConfirmation] = useState<DeleteConfirmation | null>(null);
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [cleanupPreview, setCleanupPreview] = useState<SessionIndexCleanupPreview | null>(null);
  const [cleanupSelectedIds, setCleanupSelectedIds] = useState<Set<string>>(() => new Set());
  const [repairProgress, setRepairProgress] = useState<CodexSessionRepairProgress>({
    phase: 'scanning',
    percent: 0,
    processed: 0,
    total: 0,
  });
  const mountedRef = useRef(false);
  const loadRequestRef = useRef(0);
  const initialOffsetRef = useRef(sessionViewCache.offset);
  const deleteDialogRef = useDialogFocusTrap<HTMLElement>({
    active: Boolean(deleteConfirmation),
    onEscape: operation === 'delete' ? undefined : () => setDeleteConfirmation(null),
    preventEscape: operation === 'delete',
  });
  const cleanupDialogRef = useDialogFocusTrap<HTMLElement>({
    active: Boolean(cleanupPreview),
    onEscape: operation === 'cleanup' ? undefined : () => setCleanupPreview(null),
    preventEscape: operation === 'cleanup',
  });

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      loadRequestRef.current += 1;
    };
  }, []);

  useEffect(() => {
    sessionViewCache = {
      offset: page?.offset ?? initialOffsetRef.current,
      selectionMode,
      selectedIds: new Set(selectedIds),
    };
  }, [page, selectionMode, selectedIds]);

  const loadPage = useCallback(async (offset = 0, silent = false) => {
    if (!mountedRef.current) return null;
    const requestId = ++loadRequestRef.current;
    if (!silent) {
      setLoading(true);
      setNotice((current) => current?.kind === 'error' ? null : current);
    }
    try {
      const result = await invoke<CodexSessionPage>('list_codex_sessions', {
        request: { offset, limit: PAGE_SIZE },
      });
      if (!mountedRef.current || requestId !== loadRequestRef.current) return null;
      if (result.sessions.length === 0 && result.offset > 0) {
        return await loadPage(Math.max(0, result.offset - result.limit), silent);
      }
      setPage(result);
      setSelectedIds((current) => retainVisibleCodexSessionIds(current, result.sessions));
      if (!silent && result.warnings.length > 0) {
        setNotice({ kind: 'warning', message: result.warnings.join('；') });
      }
      return result;
    } catch (error) {
      if (!mountedRef.current || requestId !== loadRequestRef.current) return null;
      setNotice({ kind: 'error', message: t('agents.sessions.loadFailed', { error: errorMessage(error) }) });
      return null;
    } finally {
      if (!silent && mountedRef.current && requestId === loadRequestRef.current) setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadPage(sessionViewCache.offset);
  }, [loadPage]);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<CodexSessionRepairProgress>('codex-session-repair-progress', (event) => {
      if (!disposed) setRepairProgress(event.payload);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    }).catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  const sessions = page?.sessions ?? [];
  const pageCounts = codexSessionPageCounts(sessions);
  const currentOffset = page?.offset ?? initialOffsetRef.current;
  const currentPage = Math.floor(currentOffset / (page?.limit ?? PAGE_SIZE)) + 1;
  const selectedSessions = useMemo(
    () => sessions.filter((session) => selectedIds.has(session.id)),
    [selectedIds, sessions],
  );
  const allSelected = sessions.length > 0 && selectedSessions.length === sessions.length;
  const busy = operation !== null;

  const toggleSelected = (id: string, selected: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const requestDelete = (targets: CodexSessionSummary[]) => {
    if (targets.length === 0) return;
    const preview = targets
      .slice(0, 6)
      .map((session) => session.title || session.id)
      .join('、');
    const extra = Math.max(0, targets.length - 6);
    setDeleteConfirmation({
      ids: targets.map((session) => session.id),
      title: targets.length === 1
        ? t('agents.sessions.deleteOneTitle')
        : t('agents.sessions.deleteManyTitle', { count: targets.length }),
      description: targets.length === 1
        ? t('agents.sessions.deleteOneDescription', { title: preview })
        : t('agents.sessions.deleteManyDescription', {
            count: targets.length,
            preview,
            extra: extra > 0 ? t('agents.sessions.deleteMore', { count: extra }) : '',
          }),
    });
  };

  const confirmDelete = async () => {
    if (!deleteConfirmation) return;
    const request = deleteConfirmation;
    setDeleteConfirmation(null);
    setOperation('delete');
    setNotice(null);
    try {
      const result = await invoke<CodexSessionDeleteBatchResult>('delete_codex_sessions', {
        request: { sessionIds: request.ids },
      });
      const failed = result.results.filter((item) => !['deleted', 'partial'].includes(item.status));
      const partial = result.results.filter((item) => item.status === 'partial');
      const backupPaths = result.results
        .map((item) => item.backupPath)
        .filter((path): path is string => Boolean(path));
      const details = [...partial, ...failed].map((item) => `${item.sessionId}: ${item.message}`).join('；');
      setNotice({
        kind: failed.length > 0 ? (result.deletedCount > 0 ? 'warning' : 'error') : partial.length > 0 ? 'warning' : 'success',
        message: t('agents.sessions.deleteResult', {
          deleted: result.deletedCount,
          failed: result.failedCount,
          details: details ? ` ${details}` : '',
          backup: backupPaths[0] ? t('agents.sessions.backupAt', { path: backupPaths[0] }) : '',
        }),
      });
      setSelectedIds(new Set());
      await loadPage(currentOffset, true);
    } catch (error) {
      setNotice({ kind: 'error', message: t('agents.sessions.deleteFailed', { error: errorMessage(error) }) });
    } finally {
      setOperation(null);
    }
  };

  const repairSessions = async () => {
    setOperation('repair');
    setNotice(null);
    setRepairProgress({ phase: 'scanning', percent: 4, processed: 0, total: 0 });
    try {
      const result = await invoke<CodexSessionRepairResult>('repair_codex_session_metadata');
      const messages = [
        t('agents.sessions.repairResult', {
          provider: result.targetProvider,
          files: result.changedRolloutFiles,
          rows: result.sqliteRowsUpdated,
          skipped: result.skippedLockedFiles.length,
        }),
      ];
      if (result.encryptedContentWarning) messages.push(result.encryptedContentWarning);
      if (result.warnings.length > 0) messages.push(result.warnings.join('；'));
      if (result.backupPath) messages.push(t('agents.sessions.backupAt', { path: result.backupPath }));
      setNotice({
        kind: result.encryptedContentWarning || result.warnings.length > 0 ? 'warning' : 'success',
        message: messages.join(' '),
      });
      await loadPage(currentOffset, true);
    } catch (error) {
      setNotice({ kind: 'error', message: t('agents.sessions.repairFailed', { error: errorMessage(error) }) });
    } finally {
      setOperation(null);
    }
  };

  const previewCleanup = async () => {
    setOperation('preview');
    setNotice(null);
    try {
      const preview = await invoke<SessionIndexCleanupPreview>('preview_codex_session_index_cleanup');
      setCleanupSelectedIds(new Set());
      if (preview.candidates.length === 0) {
        setCleanupPreview(null);
        setNotice({ kind: 'success', message: t('agents.sessions.cleanupNoCandidates') });
      } else {
        setCleanupPreview(preview);
      }
    } catch (error) {
      setNotice({
        kind: 'error',
        message: t('agents.sessions.cleanupPreviewFailed', { error: errorMessage(error) }),
      });
    } finally {
      setOperation(null);
    }
  };

  const applyCleanup = async () => {
    if (!cleanupPreview || cleanupSelectedIds.size === 0) return;
    setOperation('cleanup');
    setNotice(null);
    try {
      const result = await invoke<SessionIndexCleanupResult>('apply_codex_session_index_cleanup', {
        request: {
          snapshotSha256: cleanupPreview.snapshotSha256,
          threadIds: Array.from(cleanupSelectedIds),
        },
      });
      setCleanupPreview(null);
      setCleanupSelectedIds(new Set());
      setNotice({
        kind: 'success',
        message: t('agents.sessions.cleanupResult', {
          count: result.prunedEntries,
          backup: result.backupPath ? t('agents.sessions.backupAt', { path: result.backupPath }) : '',
        }),
      });
    } catch (error) {
      setCleanupPreview(null);
      setCleanupSelectedIds(new Set());
      setNotice({ kind: 'error', message: t('agents.sessions.cleanupFailed', { error: errorMessage(error) }) });
    } finally {
      setOperation(null);
    }
  };

  const repairPhaseText = t(`agents.sessions.progress.${repairProgress.phase}` as
    | 'agents.sessions.progress.scanning'
    | 'agents.sessions.progress.backingUp'
    | 'agents.sessions.progress.rewriting'
    | 'agents.sessions.progress.updatingDatabase'
    | 'agents.sessions.progress.complete');

  if (editingSessionId) {
    return (
      <CodexSessionContextEditor
        sessionId={editingSessionId}
        onBack={() => setEditingSessionId(null)}
        onSessionUpdated={() => void loadPage(currentOffset, true)}
      />
    );
  }

  return (
    <div className="codex-sessions-page">
      <section className="codex-session-overview">
        <div className="codex-session-overview-head">
          <div>
            <strong>{t('agents.sessions.title')}</strong>
            <span>{t('agents.sessions.description')}</span>
          </div>
          <button
            type="button"
            className="secondary-button compact-button"
            disabled={busy || loading}
            onClick={() => void loadPage(currentOffset)}
          >
            <RefreshCw size={15} className={loading ? 'spin' : ''} />
            {t('agents.sessions.refresh')}
          </button>
        </div>

        <div className="codex-session-metrics">
          <div><span>{t('agents.sessions.total')}</span><strong>{page?.totalCount ?? sessions.length}</strong></div>
          <div><span>{t('agents.sessions.active')}</span><strong>{pageCounts.active}</strong></div>
          <div><span>{t('agents.sessions.archived')}</span><strong>{pageCounts.archived}</strong></div>
          <div
            className="codex-session-database-metric"
            title={page?.databasePaths.join('\n') || page?.codexHome}
          >
            <span><Database size={13} />{t('agents.sessions.database')}</span>
            <strong>{page?.databasePaths[0] || t('agents.sessions.databaseMissing')}</strong>
          </div>
        </div>

        <div className="codex-session-repair-panel">
          <div className="codex-session-repair-copy">
            <ShieldCheck size={18} />
            <div>
              <strong>{t('agents.sessions.repairTitle')}</strong>
              <span>{t('agents.sessions.repairDescription')}</span>
            </div>
          </div>
          <div className="codex-session-repair-actions">
            <button
              type="button"
              className="secondary-button compact-button"
              disabled={busy || loading}
              onClick={() => void previewCleanup()}
            >
              {operation === 'preview' ? <LoaderCircle size={15} className="spin" /> : <ScanSearch size={15} />}
              {operation === 'preview' ? t('agents.sessions.scanningIndex') : t('agents.sessions.scanIndex')}
            </button>
            <button
              type="button"
              className="secondary-button compact-button"
              disabled={busy}
              onClick={() => void repairSessions()}
            >
              {operation === 'repair' ? <LoaderCircle size={15} className="spin" /> : <ShieldCheck size={15} />}
              {operation === 'repair' ? t('agents.sessions.repairing') : t('agents.sessions.repairNow')}
            </button>
          </div>
          {operation === 'repair' ? (
            <div className="codex-session-repair-progress">
              <div>
                <span>
                  {repairPhaseText}
                  {repairProgress.total > 0
                    ? ` · ${t('agents.sessions.progressCount', { processed: repairProgress.processed, total: repairProgress.total })}`
                    : ''}
                </span>
                <strong>{repairProgress.percent}%</strong>
              </div>
              <div
                className="codex-session-progress-track"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={repairProgress.percent}
              >
                <i style={{ width: `${repairProgress.percent}%` }} />
              </div>
            </div>
          ) : null}
        </div>

      </section>

      <MessageNotice message={notice?.message} tone={notice?.kind === 'warning' ? 'info' : notice?.kind ?? 'info'} onDismiss={() => setNotice(null)} />

      <section className="codex-session-list-section">
        <div className="codex-session-list-head">
          <div>
            <strong>{t('agents.sessions.localSessions')}</strong>
            <span>{t('agents.sessions.pageDescription', { page: currentPage, size: page?.limit ?? PAGE_SIZE })}</span>
          </div>
          <div className="codex-session-selection-actions">
            {selectionMode ? <span>{t('agents.sessions.selected', { count: selectedSessions.length })}</span> : null}
            {selectionMode ? (
              <>
                <button type="button" className="secondary-button compact-button" disabled={busy || allSelected} onClick={() => setSelectedIds(new Set(sessions.map((session) => session.id)))}>{t('agents.sessions.selectAll')}</button>
                <button type="button" className="secondary-button compact-button" disabled={busy || selectedIds.size === 0} onClick={() => setSelectedIds(new Set())}>{t('agents.sessions.clearSelection')}</button>
                <button type="button" className="danger-button compact-button" disabled={busy || selectedSessions.length === 0} onClick={() => requestDelete(selectedSessions)}><Trash2 size={14} />{t('agents.sessions.deleteSelected')}</button>
              </>
            ) : null}
            <button
              type="button"
              className="secondary-button compact-button"
              disabled={busy || sessions.length === 0}
              onClick={() => {
                setSelectionMode((current) => !current);
                setSelectedIds(new Set());
              }}
            >
              {selectionMode ? t('common.cancel') : t('agents.sessions.multiSelect')}
            </button>
          </div>
        </div>

        {loading ? (
          <div className="codex-session-empty"><LoaderCircle size={22} className="spin" /><span>{t('agents.sessions.loading')}</span></div>
        ) : sessions.length === 0 ? (
          <div className="codex-session-empty"><Archive size={22} /><span>{t('agents.sessions.empty')}</span></div>
        ) : (
          <div className="codex-session-list">
            {sessions.map((session) => (
              <article className={`codex-session-row ${selectedIds.has(session.id) ? 'selected' : ''}`} key={session.id}>
                {selectionMode ? (
                  <label className="codex-session-checkbox">
                    <input
                      type="checkbox"
                      checked={selectedIds.has(session.id)}
                      aria-label={t('agents.sessions.selectSession', { title: session.title || session.id })}
                      onChange={(event) => toggleSelected(session.id, event.currentTarget.checked)}
                    />
                  </label>
                ) : null}
                <div className="codex-session-main">
                  <strong>{session.title || t('agents.sessions.untitled')}</strong>
                  <code>{session.id}</code>
                  <span title={session.cwd}>{session.cwd || t('agents.sessions.noProject')}</span>
                </div>
                <div className="codex-session-meta">
                  <span className={session.archived ? 'archived' : 'active'}>{session.archived ? t('agents.sessions.archivedBadge') : t('agents.sessions.activeBadge')}</span>
                  <small>{session.modelProvider || t('agents.sessions.noProvider')}</small>
                  <time>{session.updatedAtMs ? formatDate(session.updatedAtMs, { dateStyle: 'medium', timeStyle: 'short' }) : t('agents.sessions.noTime')}</time>
                </div>
                <div className="codex-session-actions">
                  <button
                    type="button"
                    className="secondary-button compact-button codex-session-context-button"
                    disabled={busy}
                    onClick={() => setEditingSessionId(session.id)}
                    title={t('agents.sessions.editContext')}
                  >
                    <FileText size={14} />
                    <span>{t('agents.sessions.editContext')}</span>
                  </button>
                  <button
                    type="button"
                    className="danger-button compact-button codex-session-delete-button"
                    disabled={busy}
                    onClick={() => requestDelete([session])}
                  >
                    <Trash2 size={14} />
                    {t('common.delete')}
                  </button>
                </div>
              </article>
            ))}
          </div>
        )}

        <div className="codex-session-pagination">
          <button
            type="button"
            className="secondary-button"
            aria-label={t('agents.sessions.previousPage')}
            title={t('agents.sessions.previousPage')}
            disabled={busy || loading || !page || page.offset === 0}
            onClick={() => void loadPage(Math.max(0, currentOffset - (page?.limit ?? PAGE_SIZE)))}
          ><ArrowLeft size={15} /></button>
          <span>{t('agents.sessions.pageNumber', { page: currentPage })}</span>
          <button
            type="button"
            className="secondary-button"
            aria-label={t('agents.sessions.nextPage')}
            title={t('agents.sessions.nextPage')}
            disabled={busy || loading || !page?.hasMore}
            onClick={() => void loadPage(currentOffset + (page?.limit ?? PAGE_SIZE))}
          ><ArrowRight size={15} /></button>
        </div>
      </section>

      {deleteConfirmation ? (
        <div className="config-dialog-backdrop">
          <section ref={deleteDialogRef} className="config-dialog codex-session-dialog" role="alertdialog" aria-modal="true" aria-labelledby="codex-session-delete-title">
            <div className="config-dialog-heading"><div><TriangleAlert size={19} /><h2 id="codex-session-delete-title">{deleteConfirmation.title}</h2></div></div>
            <p>{deleteConfirmation.description}</p>
            <div className="config-dialog-actions two-actions">
              <button type="button" className="secondary-button" onClick={() => setDeleteConfirmation(null)}>{t('common.cancel')}</button>
              <button type="button" className="danger-button" onClick={() => void confirmDelete()}><Trash2 size={15} />{t('agents.sessions.confirmDelete')}</button>
            </div>
          </section>
        </div>
      ) : null}

      {cleanupPreview ? (
        <div className="config-dialog-backdrop">
          <section ref={cleanupDialogRef} className="config-dialog codex-session-cleanup-dialog" role="dialog" aria-modal="true" aria-labelledby="codex-session-cleanup-title">
            <div className="config-dialog-heading">
              <div><TriangleAlert size={19} /><h2 id="codex-session-cleanup-title">{t('agents.sessions.cleanupTitle')}</h2></div>
              <button type="button" className="codex-session-dialog-close" aria-label={t('common.close')} onClick={() => setCleanupPreview(null)}><X size={16} /></button>
            </div>
            <p>{t('agents.sessions.cleanupDescription', { count: cleanupPreview.candidates.length })}</p>
            <label className="codex-session-cleanup-select-all">
              <input
                type="checkbox"
                checked={cleanupSelectedIds.size === cleanupPreview.candidates.length && cleanupPreview.candidates.length > 0}
                onChange={(event) => setCleanupSelectedIds(event.currentTarget.checked ? new Set(cleanupPreview.candidates.map((candidate) => candidate.id)) : new Set())}
              />
              <span>{t('agents.sessions.cleanupSelectAll')}</span>
            </label>
            <div className="codex-session-cleanup-list">
              {cleanupPreview.candidates.map((candidate) => (
                <label key={candidate.id}>
                  <input
                    type="checkbox"
                    checked={cleanupSelectedIds.has(candidate.id)}
                    onChange={(event) => {
                      const checked = event.currentTarget.checked;
                      setCleanupSelectedIds((current) => {
                        const next = new Set(current);
                        if (checked) next.add(candidate.id);
                        else next.delete(candidate.id);
                        return next;
                      });
                    }}
                  />
                  <span><strong>{candidate.threadName || t('agents.sessions.untitled')}</strong><code>{candidate.id}</code><small>{candidate.updatedAt}</small></span>
                </label>
              ))}
            </div>
            <div className="config-dialog-actions two-actions">
              <button type="button" className="secondary-button" disabled={operation === 'cleanup'} onClick={() => setCleanupPreview(null)}>{t('common.cancel')}</button>
              <button type="button" className="danger-button" disabled={operation === 'cleanup' || cleanupSelectedIds.size === 0} onClick={() => void applyCleanup()}>
                {operation === 'cleanup' ? <LoaderCircle size={15} className="spin" /> : <Trash2 size={15} />}
                {t('agents.sessions.cleanupConfirm', { count: cleanupSelectedIds.size })}
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </div>
  );
}
