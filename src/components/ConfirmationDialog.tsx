import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, X } from 'lucide-react';
import { useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';

export type ConfirmationOptions = {
  title: string;
  message: string;
  confirmText?: string;
  warning?: string;
  details?: { label: string; value: string }[];
  variant?: 'primary' | 'danger';
};

type ConfirmationDialogProps = ConfirmationOptions & { onDecision: (confirmed: boolean) => void };

export function ConfirmationDialog({ title, message, confirmText, warning, details, variant = 'primary', onDecision }: ConfirmationDialogProps) {
  const { t } = useI18n();
  const privacy = usePrivacyMask();
  const titleId = useId();
  const descriptionId = useId();
  const dialogRef = useRef<HTMLElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const previousFocus = document.activeElement;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    cancelRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        onDecision(false);
      } else if (event.key === 'Tab') {
        const controls = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled), [tabindex="0"]') ?? []);
        const first = controls[0];
        const last = controls[controls.length - 1];
        if (event.shiftKey && (document.activeElement === first || !dialogRef.current?.contains(document.activeElement))) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && (document.activeElement === last || !dialogRef.current?.contains(document.activeElement))) {
          event.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener('keydown', onKeyDown, true);
    return () => {
      document.removeEventListener('keydown', onKeyDown, true);
      document.body.style.overflow = previousOverflow;
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, [onDecision]);

  const content = (
    <div className="config-dialog-backdrop app-confirm-backdrop" onMouseDown={(event) => {
      if (event.currentTarget !== event.target) return;
      event.preventDefault();
      onDecision(false);
    }}>
      <section ref={dialogRef} className="config-dialog app-confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId}>
        <div className="config-dialog-heading">
          <div><AlertTriangle size={20} aria-hidden="true" /><h2 id={titleId}>{title}</h2></div>
          <button type="button" className="icon-button quiet" onClick={() => onDecision(false)} title={t('common.close')} aria-label={t('common.close')}><X size={18} aria-hidden="true" /></button>
        </div>
        <p id={descriptionId}>{privacy.text(message)}</p>
        {details?.length ? <dl className="app-confirm-details">{details.map((detail) => <div key={detail.label}><dt>{detail.label}</dt><dd>{privacy.text(detail.value)}</dd></div>)}</dl> : null}
        {warning ? <p className="app-confirm-warning">{privacy.text(warning)}</p> : null}
        <div className="config-dialog-actions two-actions">
          <button ref={cancelRef} type="button" className="secondary-button" onClick={() => onDecision(false)}>{t('common.cancel')}</button>
          <button type="button" className={variant === 'danger' ? 'danger-button' : 'primary-button'} onClick={() => onDecision(true)}>{confirmText ?? t('common.confirm')}</button>
        </div>
      </section>
    </div>
  );
  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}

type PendingConfirmation = { options: ConfirmationOptions; resolve: (confirmed: boolean) => void };

export function useConfirmation() {
  const pendingRef = useRef<PendingConfirmation | null>(null);
  const mountedRef = useRef(true);
  const [pending, setPending] = useState<PendingConfirmation | null>(null);
  const askConfirmation = useCallback((options: ConfirmationOptions): Promise<boolean> => {
    if (!mountedRef.current) return Promise.resolve(false);
    pendingRef.current?.resolve(false);
    return new Promise((resolve) => {
      const request = { options, resolve };
      pendingRef.current = request;
      setPending(request);
    });
  }, []);
  const decide = useCallback((confirmed: boolean) => {
    if (!pending || pendingRef.current !== pending) return;
    pendingRef.current = null;
    setPending(null);
    pending.resolve(confirmed);
  }, [pending]);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      pendingRef.current?.resolve(false);
      pendingRef.current = null;
    };
  }, []);
  return {
    askConfirmation,
    confirmationDialog: pending ? <ConfirmationDialog {...pending.options} onDecision={decide} /> : null,
  };
}
