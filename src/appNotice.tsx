import { useCallback, useEffect, useId, useReducer, useRef, useState } from 'react';
import { NoticePortal } from './components/NoticePortal';
import { AlertCircle, CheckCircle2, Info, X } from 'lucide-react';
import { useI18n } from './i18n';
import { usePrivacyMask } from './privacy';
import type { MessageKey } from './i18n/resources';
import {
  appNoticeReducer,
  initialAppNoticeState,
  type AppNotice,
  type AppNoticeAction,
  type AppNoticeState,
  type NoticeTone,
  type NoticeMessage,
} from './services/appNotice';

export type { NoticeTone, NoticeMessage, AppNotice, AppNoticeState, AppNoticeAction };

export interface UseAppNoticeReturn {
  showNotice: (message: NoticeMessage, tone?: NoticeTone) => void;
  clearNotice: () => void;
  notice: AppNotice | null;
  revision: number;
}

export function useAppNotice(source?: MessageKey): UseAppNoticeReturn {
  const [state, dispatch] = useReducer(appNoticeReducer, initialAppNoticeState);
  const owner = useId();

  const showNotice = useCallback((message: NoticeMessage, tone: NoticeTone = 'success') => {
    dispatch({ type: 'show', notice: { owner, source, message, tone } });
  }, [owner, source]);

  const clearNotice = useCallback(() => {
    dispatch({ type: 'dismiss', owner });
  }, [owner]);

  return {
    showNotice,
    clearNotice,
    notice: state.notice,
    revision: state.revision,
  };
}

export interface InlineNoticeProps {
  notice?: AppNotice | null;
  onDismiss?: () => void;
  className?: string;
}

export function InlineNotice({
  notice,
  onDismiss,
  className = '',
}: InlineNoticeProps) {
  const { t } = useI18n();
  const privacy = usePrivacyMask();

  // Notices often echo credential names or backend errors that contain them.
  const message = privacy.text(notice
    ? typeof notice.message === 'string' ? notice.message : t(notice.message.key, notice.message.variables)
    : '');

  if (!notice || !message.trim()) {
    return null;
  }

  const Icon = notice.tone === 'error' ? AlertCircle : notice.tone === 'success' ? CheckCircle2 : Info;
  const isError = notice.tone === 'error';

  return (
    <div
      className={('action-feedback inline-notice ' + notice.tone + (className ? ' ' + className : '')).trim()}
      role={isError ? 'alert' : 'status'}
      aria-live={isError ? 'assertive' : 'polite'}
      aria-atomic="true"
    >
      <div className="action-feedback-main">
        <Icon size={16} className="action-feedback-icon" aria-hidden="true" />
        <div className="action-feedback-text" tabIndex={0}>
          {notice.source ? <strong className="action-feedback-source">{t(notice.source)}: </strong> : null}
          <span className="action-feedback-message">{message}</span>
        </div>
      </div>
      {onDismiss ? (
        <button
          type="button"
          className="action-feedback-dismiss"
          onClick={onDismiss}
          aria-label={t('app.notice.dismiss')}
          title={t('app.notice.dismiss')}
        >
          <X size={14} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}

export function FloatingNotice(props: InlineNoticeProps) {
  const { t } = useI18n();
  const { notice } = props;
  if (!notice) return null;
  const message = typeof notice.message === 'string' ? notice.message : t(notice.message.key, notice.message.variables);
  if (!message.trim()) return null;
  return <FloatingNoticeInstance key={notice.tone + ':' + message} {...props} notice={notice} />;
}

function FloatingNoticeInstance({ notice, onDismiss, className }: InlineNoticeProps & { notice: AppNotice }) {
  const [dismissed, setDismissed] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const entryRef = useRef<HTMLDivElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const dismissRef = useRef(onDismiss);
  useEffect(() => { dismissRef.current = onDismiss; }, [onDismiss]);
  const dismiss = useCallback(() => {
    const entry = entryRef.current;
    if (entry?.contains(document.activeElement)) {
      const dialog = entry.closest('dialog, [role="dialog"], [role="alertdialog"]');
      const previous = returnFocusRef.current;
      const target = previous?.isConnected && !previous.matches(':disabled')
        && !previous.closest('.app-notice-stack') && (!dialog || dialog.contains(previous))
        ? previous
        : Array.from(dialog?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]') ?? [])
          .find(control => !control.closest('.app-notice-stack') && control.getClientRects().length > 0);
      target?.focus();
    }
    setDismissed(true);
    dismissRef.current?.();
  }, []);
  useEffect(() => {
    if (dismissed || hovered || focused || notice.tone !== 'success') return;
    const timer = window.setTimeout(dismiss, 6_000);
    return () => window.clearTimeout(timer);
  }, [dismissed, hovered, focused, notice.tone, dismiss]);
  if (dismissed) return null;
  return <NoticePortal>
    <div ref={entryRef} className="app-notice-entry"
      onPointerEnter={() => setHovered(true)} onPointerLeave={() => setHovered(false)}
      onFocusCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          returnFocusRef.current = event.relatedTarget instanceof HTMLElement ? event.relatedTarget : null;
        }
        setFocused(true);
      }}
      onBlurCapture={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false); }}
      onKeyDown={(event) => { if (event.key !== 'Tab') event.stopPropagation(); }}
      onClick={(event) => event.stopPropagation()} onMouseDown={(event) => event.stopPropagation()}>
      <InlineNotice notice={notice} onDismiss={dismiss} className={className} />
    </div>
  </NoticePortal>;
}

export function MessageNotice({ message, tone = 'error', onDismiss, source }: {
  message?: NoticeMessage | null;
  tone?: NoticeTone;
  onDismiss?: () => void;
  source?: MessageKey;
}) {
  const owner = useId();
  return <FloatingNotice notice={message ? { owner, message, tone, source } : null} onDismiss={onDismiss} />;
}

export const ActionFeedback = FloatingNotice;
