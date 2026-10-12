import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  AlertCircle,
  ArrowLeft,
  Bot,
  Brain,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Clock,
  ExternalLink,
  Copy,
  Cpu,
  Download,
  FileText,
  Folder,
  LoaderCircle,
  Plus,
  RefreshCw,
  Save,
  Search,
  ShieldAlert,
  Trash2,
  Undo,
  User,
  Wrench,
  X,
} from 'lucide-react';
import { useI18n } from '../i18n';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import { APP_NAVIGATION_REQUEST_EVENT, type AppNavigationRequest } from '../navigation';
import type {
  CodexSessionContextDetail,
  CodexSessionNewMessage,
  SaveCodexSessionContextRequest,
  SaveCodexSessionContextResult,
} from '../services/codexSessionState';

type Props = {
  sessionId: string;
  onBack: () => void;
  onSessionUpdated?: () => void;
};

type Notice = {
  kind: 'success' | 'warning' | 'error';
  message: string;
};

type MessageEditState = {
  content: string;
  isEditing: boolean;
  isDeleted: boolean;
};

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  const unitIndex = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  return (bytes / Math.pow(1024, unitIndex)).toFixed(unitIndex === 0 ? 0 : 1) + ' ' + units[unitIndex];
}

export function CodexSessionContextEditor({ sessionId, onBack, onSessionUpdated }: Props) {
  const { formatDate, t } = useI18n();
  const displayDate = (value: string | number) => Number.isFinite(new Date(value).getTime()) ? formatDate(value) : String(value);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [detail, setDetail] = useState<CodexSessionContextDetail | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [opening, setOpening] = useState(false);

  const [title, setTitle] = useState('');
  const [cwd, setCwd] = useState('');
  const [model, setModel] = useState('');
  const [modelProvider, setModelProvider] = useState('');
  const [archived, setArchived] = useState(false);
  const [metaExpanded, setMetaExpanded] = useState(false);

  const [messageEdits, setMessageEdits] = useState<Record<number, MessageEditState>>({});
  const [newMessages, setNewMessages] = useState<CodexSessionNewMessage[]>([]);
  const [addingMessage, setAddingMessage] = useState(false);
  const [newRole, setNewRole] = useState<'user' | 'assistant'>('user');
  const [newContent, setNewContent] = useState('');

  const [roleFilter, setRoleFilter] = useState<'all' | 'user' | 'assistant' | 'tool' | 'reasoning'>('all');
  const [searchQuery, setSearchQuery] = useState('');

  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [discardConfirmOpen, setDiscardConfirmOpen] = useState(false);

  const mountedRef = useRef(true);
  const pendingReadRef = useRef<{ sessionId: string; promise: Promise<CodexSessionContextDetail> } | null>(null);
  const savingRef = useRef(false);
  const translateRef = useRef(t);
  translateRef.current = t;
  const pendingNavigationRef = useRef<(() => void) | null>(null);
  const discardDialogRef = useDialogFocusTrap<HTMLElement>({
    active: discardConfirmOpen,
    onEscape: () => setDiscardConfirmOpen(false),
  });

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const copyToClipboard = useCallback(async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      if (!mountedRef.current) return;
      setCopiedKey(key);
      setTimeout(() => {
        if (mountedRef.current) setCopiedKey((curr) => (curr === key ? null : curr));
      }, 2000);
    } catch {
      if (mountedRef.current) setNotice({ kind: 'error', message: translateRef.current('common.copyFailed') });
    }
  }, []);

  const loadContext = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    if (!silent) setNotice(null);
    const pending = pendingReadRef.current?.sessionId === sessionId
      ? pendingReadRef.current
      : { sessionId, promise: invoke<CodexSessionContextDetail>('get_codex_session_context', { request: { sessionId } }) };
    pendingReadRef.current = pending;
    try {
      const data = await pending.promise;
      if (!mountedRef.current || pendingReadRef.current !== pending) return;
      setDetail(data);
      setTitle(data.title || '');
      setCwd(data.cwd || '');
      setModel(data.model || '');
      setModelProvider(data.modelProvider || '');
      setArchived(Boolean(data.archived));

      const initialEdits: Record<number, MessageEditState> = {};
      data.messages.forEach((msg) => {
        initialEdits[msg.lineNumber] = {
          content: msg.content,
          isEditing: false,
          isDeleted: false,
        };
      });
      setMessageEdits(initialEdits);
      setNewMessages([]);
      setNewContent('');
      setAddingMessage(false);
      return true;
    } catch (error) {
      if (!mountedRef.current) return;
      const msg = error instanceof Error ? error.message : String(error);
      setNotice({ kind: 'error', message: translateRef.current('agents.sessions.context.loadFailed', { error: msg }) });
      return false;
    } finally {
      if (pendingReadRef.current === pending) pendingReadRef.current = null;
      if (!silent && mountedRef.current) setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    void loadContext();
  }, [loadContext]);

  const metadataDirty = useMemo(() => {
    if (!detail) return false;
    return title !== (detail.title || '')
      || cwd !== (detail.cwd || '')
      || model !== (detail.model || '')
      || modelProvider !== (detail.modelProvider || '')
      || archived !== Boolean(detail.archived);
  }, [detail, title, cwd, model, modelProvider, archived]);

  const messagesByLine = useMemo(() => new Map(detail?.messages.map((message) => [message.lineNumber, message])), [detail]);

  const structuredDirty = useMemo(() => {
    if (!detail) return false;
    if (newMessages.length > 0 || (addingMessage && newContent.trim())) return true;
    return Object.entries(messageEdits).some(([lineStr, edit]) => {
      const lineNum = Number(lineStr);
      const original = messagesByLine.get(lineNum);
      if (!original) return true;
      if (edit.isDeleted) return true;
      if (edit.content !== original.content) return true;
      return false;
    });
  }, [detail, newMessages, messageEdits, messagesByLine, addingMessage, newContent]);

  const isDirty = metadataDirty || structuredDirty;

  useEffect(() => {
    const guardNavigation = (event: Event) => {
      if (!isDirty && !savingRef.current) return;
      event.preventDefault();
      if (savingRef.current) return;
      pendingNavigationRef.current = (event as AppNavigationRequest).detail;
      setDiscardConfirmOpen(true);
    };
    const guardUnload = (event: BeforeUnloadEvent) => {
      if (!isDirty && !savingRef.current) return;
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener(APP_NAVIGATION_REQUEST_EVENT, guardNavigation);
    window.addEventListener('beforeunload', guardUnload);
    return () => {
      window.removeEventListener(APP_NAVIGATION_REQUEST_EVENT, guardNavigation);
      window.removeEventListener('beforeunload', guardUnload);
    };
  }, [isDirty]);

  const handleBackRequest = () => {
    if (savingRef.current) return;
    if (isDirty) {
      pendingNavigationRef.current = onBack;
      setDiscardConfirmOpen(true);
    } else {
      onBack();
    }
  };

  const handleSave = async () => {
    if (!detail || !isDirty || loading || savingRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setNotice(null);

    try {
      const payload: SaveCodexSessionContextRequest = {
        sessionId,
        expectedRolloutSha256: detail.rolloutSha256 || undefined,
      };
      if (title !== (detail.title || '')) payload.title = title;
      if (cwd !== (detail.cwd || '')) payload.cwd = cwd;
      if (model !== (detail.model || '')) payload.model = model;
      if (modelProvider !== (detail.modelProvider || '')) payload.modelProvider = modelProvider;
      if (archived !== Boolean(detail.archived)) payload.archived = archived;

      if (structuredDirty) {
        const messageUpdates = Object.entries(messageEdits)
          .map(([lineStr, edit]) => {
            const lineNum = Number(lineStr);
            const original = messagesByLine.get(lineNum);
            if (!original) return null;
            if (edit.isDeleted) {
              return { lineNumber: lineNum, deleted: true };
            }
            const update: { lineNumber: number; content?: string } = {
              lineNumber: lineNum,
            };
            if (edit.content !== original.content) update.content = edit.content;
            return update.content !== undefined ? update : null;
          })
          .filter((item): item is NonNullable<typeof item> => item !== null);

        payload.messageUpdates = messageUpdates;
        const messagesToAppend = [...newMessages];
        if (addingMessage && newContent.trim()) messagesToAppend.push({ role: newRole, content: newContent });
        if (messagesToAppend.length > 0) {
          payload.newMessages = messagesToAppend;
        }
      }

      const result = await invoke<SaveCodexSessionContextResult>('save_codex_session_context', {
        request: payload,
      });

      if (!mountedRef.current) return;
      let successMsg = t('agents.sessions.context.saveSuccess');
      if (result.backupPath) {
        successMsg += ' ' + t('agents.sessions.backupAt', { path: result.backupPath });
      }
      onSessionUpdated?.();
      if (await loadContext(true)) {
        setNotice({ kind: 'success', message: successMsg });
      }
    } catch (error) {
      if (!mountedRef.current) return;
      const msg = error instanceof Error ? error.message : String(error);
      setNotice({ kind: 'error', message: t('agents.sessions.context.saveFailed', { error: msg }) });
    } finally {
      savingRef.current = false;
      if (mountedRef.current) setSaving(false);
    }
  };

  const handleSaveRef = useRef(handleSave);
  useEffect(() => {
    handleSaveRef.current = handleSave;
  }, [handleSave]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        void handleSaveRef.current();
        return;
      }
      if (event.key === 'Escape') {
        if (discardConfirmOpen) {
          event.preventDefault();
          setDiscardConfirmOpen(false);
        } else if (addingMessage) {
          event.preventDefault();
          setAddingMessage(false);
          setNewContent('');
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [discardConfirmOpen, addingMessage]);

  const handleExportJsonl = () => {
    if (!detail) return;
    const textToExport = detail.rawJsonl || '';
    if (!textToExport) return;
    const blob = new Blob([textToExport], { type: 'application/x-jsonlines;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const downloadLink = document.createElement('a');
    downloadLink.href = url;
    downloadLink.download = `session-${sessionId}-${new Date().toISOString().slice(0, 10)}.jsonl`;
    document.body.appendChild(downloadLink);
    downloadLink.click();
    document.body.removeChild(downloadLink);
    URL.revokeObjectURL(url);
  };

  const handleAddNewMessage = () => {
    if (!newContent.trim()) return;
    setNewMessages((prev) => [...prev, { role: newRole, content: newContent }]);
    setNewContent('');
    setAddingMessage(false);
  };

  const filteredMessages = useMemo(() => {
    if (!detail) return [];
    return detail.messages.filter((msg) => {
      const edit = messageEdits[msg.lineNumber];
      const currentContent = edit ? edit.content : msg.content;
      const currentRole = msg.role;

      if (roleFilter !== 'all' && currentRole !== roleFilter) {
        return false;
      }
      if (searchQuery.trim()) {
        const query = searchQuery.toLowerCase();
        return currentContent.toLowerCase().includes(query) || currentRole.toLowerCase().includes(query);
      }
      return true;
    });
  }, [detail, messageEdits, roleFilter, searchQuery]);

  return (
    <div className="codex-session-context-editor">
      <header className="context-editor-header">
        <div className="context-editor-nav">
          <button
            type="button"
            className="secondary-button compact-button"
            onClick={handleBackRequest}
            disabled={saving}
            title={t('agents.sessions.context.backToList')}
          >
            <ArrowLeft size={15} />
            <span>{t('agents.sessions.context.backToList')}</span>
          </button>
          <div className="context-editor-title-group">
            <h1>{title || t('agents.sessions.untitled')}</h1>
          </div>
        </div>

        <div className="context-editor-actions">
          <button
            type="button"
            className="secondary-button compact-button"
            disabled={loading || saving || isDirty}
            onClick={() => void loadContext()}
            title={t('agents.sessions.context.reload')}
          >
            <RefreshCw size={14} className={loading ? 'spin' : ''} />
            <span>{t('agents.sessions.context.reload')}</span>
          </button>

          <button
            type="button"
            className={`primary-button compact-button ${isDirty ? 'highlight-save' : ''}`}
            disabled={loading || saving || !isDirty}
            onClick={() => void handleSave()}
          >
            {saving ? <LoaderCircle size={14} className="spin" /> : <Save size={14} />}
            <span>{saving ? t('agents.sessions.context.saving') : t('agents.sessions.context.saveChanges')}</span>
          </button>
        </div>
      </header>

      <p className="context-edit-safety" role="note">{t('agents.sessions.context.editSafety')}</p>

      {notice ? (
        <div className={`codex-session-notice ${notice.kind}`}>
          {notice.kind === 'error' ? <AlertCircle size={16} /> : <CheckCircle2 size={16} />}
          <span>{notice.message}</span>
          <button type="button" onClick={() => setNotice(null)} aria-label={t('common.close')}>
            <X size={14} />
          </button>
        </div>
      ) : null}

      <section className="context-meta-card">
        <button type="button" className="context-meta-head" onClick={() => setMetaExpanded((prev) => !prev)}
          aria-expanded={metaExpanded} aria-controls="context-metadata" aria-label={t('agents.sessions.context.toggleMetadata')}>
          <span className="context-meta-head-title">
            <Folder size={16} />
            <strong>{t('agents.sessions.context.metadata')}</strong>
          </span>
          {metaExpanded ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
        </button>

        {metaExpanded ? (
          <div className="context-meta-body" id="context-metadata">
            <div className="context-editor-badges">
              <button type="button" className="context-badge context-id-badge" title={sessionId}
                aria-label={`${t('common.copy')}: ${sessionId}`} onClick={() => copyToClipboard(sessionId, 'id')}>
                <code>{sessionId}</code>
                {copiedKey === 'id' ? <Check size={12} className="success-icon" /> : <Copy size={12} />}
              </button>
              <span className={`context-badge ${archived ? 'archived' : 'active'}`}>
                {archived ? t('agents.sessions.context.isArchived') : t('agents.sessions.context.isNotArchived')}
              </span>
              <small>{detail?.updatedAtMs ? displayDate(detail.updatedAtMs) : t('agents.sessions.noTime')}</small>
              {detail ? <small>{t('agents.sessions.context.fileSize')}: {formatBytes(detail.stats.fileSizeBytes)}</small> : null}
            </div>
            <div className="context-form-grid">
              {([
                { label: 'sessionTitle', value: title, set: setTitle, placeholder: 'agents.sessions.untitled' },
                { label: 'workspace', value: cwd, set: setCwd, placeholder: 'agents.sessions.context.workspacePlaceholder' },
                { label: 'model', value: model, set: setModel, placeholder: 'agents.sessions.context.modelPlaceholder' },
                { label: 'provider', value: modelProvider, set: setModelProvider, placeholder: 'agents.sessions.context.providerPlaceholder' },
              ] as const).map((field) => (
                <label key={field.label} className="context-form-field">
                  <span>{t(`agents.sessions.context.${field.label}`)}</span>
                  <input type="text" value={field.value} disabled={loading || saving || !detail}
                    placeholder={t(field.placeholder)} onChange={(event) => field.set(event.target.value)} />
                </label>
              ))}
            </div>

            <div className="context-meta-paths">
              <div className="context-path-item">
                <span><FileText size={13} /> {t('agents.sessions.context.rolloutPath')}:</span>
                <code>{detail?.rolloutPath || '—'}</code>
                {detail?.rolloutPath ? (
                  <button
                    type="button"
                    className="icon-button"
                    onClick={() => copyToClipboard(detail.rolloutPath!, 'rollout')}
                    title={t('common.copy')}
                  >
                    {copiedKey === 'rollout' ? <Check size={13} className="success-icon" /> : <Copy size={13} />}
                  </button>
                ) : null}
              </div>

              <div className="context-path-item">
                <span><Cpu size={13} /> {t('agents.sessions.context.dbPath')}:</span>
                <code>{detail?.databasePath || '—'}</code>
              </div>

              <label className="context-archived-checkbox">
                <input
                  type="checkbox"
                  checked={archived}
                  disabled={loading || saving || !detail?.rolloutPath}
                  onChange={(event) => setArchived(event.target.checked)}
                />
                <span>{t('agents.sessions.context.archived')}</span>
              </label>
            </div>
            <div className="context-editor-actions">
              <button type="button" className="secondary-button compact-button"
                disabled={loading || saving || isDirty || !detail?.rawJsonlAvailable || !detail.rawJsonl}
                onClick={handleExportJsonl} title={t('agents.sessions.context.exportJsonl')}>
                <Download size={14} />
                <span>{t('agents.sessions.context.exportJsonl')}</span>
              </button>
              <button type="button" className="secondary-button compact-button"
                disabled={loading || saving || opening || isDirty || !detail?.rolloutPath}
                onClick={async () => {
                  setOpening(true);
                  try {
                    await invoke('open_codex_session_rollout', { request: { sessionId } });
                  } catch (error) {
                    setNotice({ kind: 'error', message: t('agents.sessions.context.openFailed', { error: String(error) }) });
                  } finally {
                    setOpening(false);
                  }
                }}>
                <ExternalLink size={15} />
                <span>{t('agents.sessions.context.openRaw')}</span>
              </button>
            </div>
            <p className="context-edit-safety">{t('agents.sessions.context.externalEditNotice')}</p>
          </div>
        ) : null}
      </section>

        <section className="context-messages-section">
          <div className="context-filter-toolbar">
            <div className="context-role-filters">
              {([
                { role: 'all', label: 'filterAll', Icon: null, count: null },
                { role: 'user', label: 'userRole', Icon: User, count: detail?.stats.userMessageCount ?? 0 },
                { role: 'assistant', label: 'assistantRole', Icon: Bot, count: detail?.stats.assistantMessageCount ?? 0 },
                { role: 'reasoning', label: 'reasoningRole', Icon: Brain, count: detail?.stats.reasoningCount ?? 0 },
                { role: 'tool', label: 'toolRole', Icon: Wrench, count: detail?.stats.toolCount ?? 0 },
              ] as const).map(({ role, label, Icon, count }) => (
                <button key={role} type="button" className={`filter-chip ${role} ${roleFilter === role ? 'active' : ''}`}
                  onClick={() => setRoleFilter(role)}>
                  {Icon ? <Icon size={13} /> : null}
                  <span>{t(`agents.sessions.context.${label}`)}{count !== null ? ` (${count})` : ''}</span>
                </button>
              ))}
            </div>

            <div className="context-search-box">
              <Search size={14} />
              <input
                type="text"
                placeholder={t('agents.sessions.context.searchPlaceholder')}
                value={searchQuery}
                onChange={(event) => setSearchQuery(event.target.value)}
              />
              {searchQuery ? (
                <button type="button" onClick={() => setSearchQuery('')} aria-label={t('common.clear')}>
                  <X size={13} />
                </button>
              ) : null}
            </div>
          </div>

          {loading ? (
            <div className="codex-session-empty">
              <LoaderCircle size={22} className="spin" />
              <span>{t('agents.sessions.loading')}</span>
            </div>
          ) : (
            <div className="context-message-list">
              {filteredMessages.length === 0 && newMessages.length === 0 ? (
                <div className="codex-session-empty">
                  <FileText size={22} />
                  <span>{t('agents.sessions.context.emptyMessages')}</span>
                </div>
              ) : null}
              {filteredMessages.map((msg) => {
                const edit = messageEdits[msg.lineNumber] || {
                  content: msg.content,
                  isEditing: false,
                  isDeleted: false,
                        };
                const isModified = edit.content !== msg.content;

                return (
                  <article
                    key={msg.id}
                    className={`context-message-card ${msg.role} ${edit.isDeleted ? 'deleted' : ''} ${isModified ? 'modified' : ''}`}
                  >
                    <div className="message-card-header">
                      <div className="message-role-badge">
                        {msg.role === 'user' ? <User size={13} /> : msg.role === 'assistant' ? <Bot size={13} /> : msg.role === 'reasoning' ? <Brain size={13} /> : <Wrench size={13} />}
                        <strong>{msg.role.toUpperCase()}</strong>
                        <span className="line-num">{t('agents.sessions.context.lineNumber', { line: msg.lineNumber + 1 })}</span>
                      </div>

                      <div className="message-header-meta">
                        {msg.timestamp ? (
                          <time title={msg.timestamp}>
                            <Clock size={12} />
                            {displayDate(msg.timestamp)}
                          </time>
                        ) : null}
                        {msg.model ? <span className="message-model-tag">{msg.model}</span> : null}
                        {msg.signatureStripped ? (
                          <span className="message-signature-tag" title={t('agents.sessions.context.signatureStrippedNotice')}>
                            <ShieldAlert size={11} />
                            <span>{t('agents.sessions.context.signatureStrippedBadge')}</span>
                          </span>
                        ) : null}

                        <div className="message-item-actions">
                          <button
                            type="button"
                            className="icon-button"
                            onClick={() => copyToClipboard(edit.content, `msg-${msg.lineNumber}`)}
                            title={t('common.copy')}
                          >
                            {copiedKey === `msg-${msg.lineNumber}` ? <Check size={13} className="success-icon" /> : <Copy size={13} />}
                          </button>

                          {msg.role !== 'tool' ? (
                            <button
                              type="button"
                              className={`secondary-button compact-button ${edit.isEditing ? 'active' : ''}`}
                              disabled={saving || edit.isDeleted}
                              onClick={() => {
                                setMessageEdits((prev) => ({
                                  ...prev,
                                  [msg.lineNumber]: {
                                    ...edit,
                                    isEditing: !edit.isEditing,
                                  },
                                }));
                              }}
                            >
                              {t('agents.sessions.context.editMessage')}
                            </button>
                          ) : null}

                          <button
                            type="button"
                            className={`compact-button ${edit.isDeleted ? 'secondary-button' : 'danger-button'}`}
                            disabled={saving}
                            onClick={() => {
                              setMessageEdits((prev) => {
                                const next = { ...prev };
                                for (const message of detail?.messages ?? []) {
                                  if (message.lineNumber === msg.lineNumber || (msg.role === 'tool' && msg.callId && message.role === 'tool' && message.callId === msg.callId)) {
                                    next[message.lineNumber] = { ...prev[message.lineNumber], isDeleted: !edit.isDeleted };
                                  }
                                }
                                return next;
                              });
                            }}
                          >
                            {edit.isDeleted ? (
                              <>
                                <Undo size={13} />
                                <span>{t('agents.sessions.context.restoreMessage')}</span>
                              </>
                            ) : (
                              <>
                                <Trash2 size={13} />
                                <span>{t('agents.sessions.context.deleteMessage')}</span>
                              </>
                            )}
                          </button>
                        </div>
                      </div>
                    </div>

                    <div className="message-card-body">
                      {edit.isDeleted ? (
                        <div className="message-deleted-placeholder">
                          <span>{t('agents.sessions.context.pendingDeletion')}</span>
                        </div>
                      ) : edit.isEditing && msg.role !== 'tool' ? (
                        <div className="message-editor-wrapper">
                          {msg.role === 'reasoning' ? (
                            <div className="reasoning-edit-warning">
                              <ShieldAlert size={13} />
                              <span>{t('agents.sessions.context.reasoningEditNotice')}</span>
                            </div>
                          ) : null}
                          <textarea
                            className="message-textarea"
                            aria-label={t('agents.sessions.context.addMessageContent')}
                            disabled={saving}
                            value={edit.content}
                            rows={Math.min(15, Math.max(3, edit.content.split('\n').length))}
                            onChange={(event) => {
                              const val = event.target.value;
                              setMessageEdits((prev) => ({
                                ...prev,
                                [msg.lineNumber]: {
                                  ...edit,
                                  content: val,
                                },
                              }));
                            }}
                          />
                          <div className="message-editor-footer">
                            {isModified ? (
                              <button
                                type="button"
                                className="text-button"
                                disabled={saving}
                                onClick={() => {
                                  setMessageEdits((prev) => ({
                                    ...prev,
                                    [msg.lineNumber]: {
                                      ...edit,
                                      content: msg.content,
                                    },
                                  }));
                                }}
                              >
                                {t('agents.sessions.context.restoreOriginal')}
                              </button>
                            ) : null}
                          </div>
                        </div>
                      ) : (
                        <div className="message-text-display">
                          <pre>{edit.content}</pre>
                        </div>
                      )}
                    </div>
                  </article>
                );
              })}

              {newMessages.map((newMsg, index) => (
                <article key={`new-${index}`} className={`context-message-card ${newMsg.role} new-item`}>
                  <div className="message-card-header">
                    <div className="message-role-badge">
                      {newMsg.role === 'user' ? <User size={13} /> : <Bot size={13} />}
                      <strong>{newMsg.role.toUpperCase()}</strong>
                      <span className="line-num">{t('agents.sessions.context.newMessageBadge')}</span>
                    </div>
                    <div className="message-item-actions">
                      <button
                        type="button"
                        className="danger-button compact-button"
                        disabled={saving}
                        onClick={() => setNewMessages((prev) => prev.filter((_, messageIndex) => messageIndex !== index))}
                      >
                        <Trash2 size={13} />
                        <span>{t('common.delete')}</span>
                      </button>
                    </div>
                  </div>
                  <div className="message-card-body">
                    <pre>{newMsg.content}</pre>
                  </div>
                </article>
              ))}

              {addingMessage ? (
                <div className="context-add-message-card">
                  <div className="add-message-head">
                    <strong>{t('agents.sessions.context.addMessage')}</strong>
                    <div className="add-message-role-select">
                      <button
                        type="button"
                        className={`role-select-chip ${newRole === 'user' ? 'active' : ''}`}
                        disabled={saving}
                        onClick={() => setNewRole('user')}
                      >
                        <User size={12} />
                        <span>{t('agents.sessions.context.userRole')}</span>
                      </button>
                      <button
                        type="button"
                        className={`role-select-chip ${newRole === 'assistant' ? 'active' : ''}`}
                        disabled={saving}
                        onClick={() => setNewRole('assistant')}
                      >
                        <Bot size={12} />
                        <span>{t('agents.sessions.context.assistantRole')}</span>
                      </button>
                    </div>
                  </div>
                  <textarea
                    className="add-message-textarea"
                    aria-label={t('agents.sessions.context.addMessageContent')}
                    disabled={saving}
                    placeholder={t('agents.sessions.context.addMessageContent')}
                    value={newContent}
                    rows={4}
                    onChange={(event) => setNewContent(event.target.value)}
                  />
                  <div className="add-message-actions">
                    <button
                      type="button"
                      className="secondary-button compact-button"
                      disabled={saving}
                      onClick={() => {
                        setAddingMessage(false);
                        setNewContent('');
                      }}
                    >
                      {t('common.cancel')}
                    </button>
                    <button
                      type="button"
                      className="primary-button compact-button"
                      disabled={saving || !newContent.trim()}
                      onClick={handleAddNewMessage}
                    >
                      <Plus size={14} />
                      <span>{t('agents.sessions.context.addMessage')}</span>
                    </button>
                  </div>
                </div>
              ) : (
                <button
                  type="button"
                  className="context-add-button"
                  disabled={saving || !detail?.rolloutPath}
                  onClick={() => setAddingMessage(true)}
                >
                  <Plus size={15} />
                  <span>{t('agents.sessions.context.addMessage')}</span>
                </button>
              )}
            </div>
          )}
        </section>

      {discardConfirmOpen ? (
        <div className="config-dialog-backdrop">
          <section ref={discardDialogRef} className="config-dialog codex-session-dialog" role="alertdialog" aria-modal="true" aria-labelledby="codex-session-context-discard-title">
            <div className="config-dialog-heading">
              <div>
                <AlertCircle size={19} />
                <h2 id="codex-session-context-discard-title">{t('agents.sessions.context.discardTitle')}</h2>
              </div>
            </div>
            <p>{t('agents.sessions.context.confirmDiscard')}</p>
            <div className="config-dialog-actions two-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => setDiscardConfirmOpen(false)}
              >
                {t('common.cancel')}
              </button>
              <button
                type="button"
                className="danger-button"
                onClick={() => {
                  setDiscardConfirmOpen(false);
                  (pendingNavigationRef.current ?? onBack)();
                }}
              >
                {t('agents.sessions.context.discard')}
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </div>
  );
}
