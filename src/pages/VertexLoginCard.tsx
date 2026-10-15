import { useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, LoaderCircle, Upload, X } from 'lucide-react';
import vertexIcon from '../assets/icons/vertex.svg';
import { useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';
import { importVertexCredential, type VertexImportResult } from '../services/vertexImport';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import './VertexLoginCard.css';

export function VertexLoginCard() {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);

  return (
    <>
      <section className="panel vertex-login-card">
        <div className="provider-title-row">
          <img src={vertexIcon} alt="" className="provider-logo" />
          <h2>{t('oauth.vertex.title')}</h2>
        </div>
        <div className="oauth-card-body vertex-login-card-body">
          <p className="oauth-hint">{t('oauth.vertex.cardHint')}</p>
        </div>
        <div className="button-row management-card-actions">
          <button type="button" className="primary-button" aria-haspopup="dialog" onClick={() => setOpen(true)}>
            <Upload size={16} aria-hidden="true" />{t('oauth.vertex.import')}
          </button>
        </div>
      </section>
      {open ? <VertexImportDialog onClose={() => setOpen(false)} /> : null}
    </>
  );
}

function VertexImportDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const privacy = usePrivacyMask();
  const [file, setFile] = useState<File | null>(null);
  const [location, setLocation] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [result, setResult] = useState<VertexImportResult | null>(null);
  const busy = useRef(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const chooseFile = useRef<HTMLButtonElement>(null);
  const dialogRef = useDialogFocusTrap<HTMLElement>({
    onEscape: loading ? undefined : onClose,
    preventEscape: loading,
    initialFocusRef: chooseFile,
  });

  const importCredential = async () => {
    if (busy.current) return;
    setError('');
    setResult(null);
    if (!file) { setError(t('oauth.vertex.fileRequired')); return; }
    busy.current = true;
    setLoading(true);
    try {
      setResult(await importVertexCredential(file, location));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      busy.current = false;
      setLoading(false);
    }
  };

  return createPortal(
    <div className="config-dialog-backdrop" onMouseDown={event => {
      if (event.currentTarget === event.target && !busy.current) onClose();
    }}>
    <section ref={dialogRef} className="config-dialog vertex-import-dialog" role="dialog" aria-modal="true"
      aria-labelledby="vertex-import-title" aria-describedby="vertex-import-description" aria-busy={loading} tabIndex={-1}>
      <div className="config-dialog-heading">
        <div className="vertex-import-heading">
          <img src={vertexIcon} alt="" className="provider-logo" />
          <h2 id="vertex-import-title">{t('oauth.vertex.import')}</h2>
        </div>
        <button type="button" className="icon-button quiet" disabled={loading} onClick={onClose}
          aria-label={t('common.close')}><X size={18} aria-hidden="true" /></button>
      </div>
      <form className="vertex-import-form" onSubmit={event => { event.preventDefault(); void importCredential(); }}>
      <div className="vertex-import-body">
        <p id="vertex-import-description" className="oauth-hint">{t('oauth.vertex.description')}</p>
        <div className="vertex-login-field">
          <span>{t('oauth.vertex.file')}</span>
          <div className="vertex-file-picker">
            <button ref={chooseFile} type="button" className="secondary-button compact-button" disabled={loading}
              onClick={() => fileInput.current?.click()}><Upload size={16} aria-hidden="true" />{t('oauth.vertex.chooseFile')}</button>
            <span title={file?.name}>{file?.name || t('oauth.vertex.noFile')}</span>
          </div>
          <input ref={fileInput} type="file" accept=".json,application/json" disabled={loading}
            hidden aria-label={t('oauth.vertex.file')}
            onChange={event => { setFile(event.currentTarget.files?.[0] ?? null); setError(''); setResult(null); }} />
        </div>
        <label className="vertex-login-field">
          <span>{t('oauth.vertex.location')}</span>
          <input value={location} disabled={loading} placeholder={t('oauth.vertex.locationPlaceholder')}
            onChange={event => { setLocation(event.currentTarget.value); setError(''); setResult(null); }} />
          <small>{t('oauth.vertex.locationHint')}</small>
        </label>
        {error ? <p role="alert" className="vertex-login-error">{privacy.text(error)}</p> : null}
        {result ? <div role="status" className="vertex-login-result">
          <p><Check size={16} aria-hidden="true" />{t('oauth.vertex.success')}</p>
          <dl>
            <dt>{t('oauth.vertex.project')}</dt><dd>{privacy.identifier(result.projectId)}</dd>
            <dt>{t('oauth.vertex.email')}</dt><dd>{privacy.identifier(result.email)}</dd>
            <dt>{t('oauth.vertex.location')}</dt><dd>{result.location}</dd>
            <dt>{t('oauth.vertex.authFile')}</dt><dd>{privacy.text(result.authFile, [result.projectId, result.email])}</dd>
          </dl>
        </div> : null}
      </div>
      <div className="vertex-import-actions">
        <button type="button" className="secondary-button" disabled={loading} onClick={onClose}>
          {t(result ? 'common.close' : 'common.cancel')}
        </button>
        <button type="submit" className="primary-button" disabled={loading}>
          {loading ? <LoaderCircle size={16} className="spin" aria-hidden="true" /> : <Upload size={16} aria-hidden="true" />}
          {t(loading ? 'oauth.vertex.importing' : 'oauth.vertex.import')}
        </button>
      </div>
      </form>
    </section>
    </div>, document.body,
  );
}
