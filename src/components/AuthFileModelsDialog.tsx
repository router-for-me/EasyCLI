import { MessageNotice } from '../appNotice';
import { useEffect, useState } from 'react';
import { Check, Copy, LoaderCircle, Search, X } from 'lucide-react';
import { useI18n } from '../i18n';
import { managementApi } from '../services/managementApi';
import { oauthModelsFromPayload, type OAuthModelDefinition } from '../services/oauthModels';
import { useDialogFocusTrap } from './useDialogFocusTrap';

type AuthFileModelsDialogProps = {
  name: string;
  /** Privacy-masked name for display; `name` stays the request key. */
  displayName?: string;
  onClose: () => void;
};

export function AuthFileModelsDialog({ name, displayName = name, onClose }: AuthFileModelsDialogProps) {
  const { t } = useI18n();
  const [models, setModels] = useState<OAuthModelDefinition[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [copied, setCopied] = useState('');
  const dialogRef = useDialogFocusTrap<HTMLElement>({ onEscape: onClose });

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setModels([]);
    setError('');
    void managementApi.get('/credentials/models', { name })
      .then((payload) => { if (!cancelled) setModels(oauthModelsFromPayload(payload)); })
      .catch((requestError: unknown) => { if (!cancelled) setError(String(requestError)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [name]);

  const query = search.trim().toLowerCase();
  const visibleModels = models.filter((model) =>
    [model.id, model.displayName ?? ''].join(' ').toLowerCase().includes(query),
  );
  const copyModel = async (id: string) => {
    try {
      await navigator.clipboard.writeText(id);
      setCopied(id);
    } catch (copyError) {
      setError(String(copyError));
    }
  };

  return (
    <div className="model-discovery-backdrop" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}>
      <section ref={dialogRef} className="model-discovery-dialog auth-model-dialog auth-model-view-dialog" role="dialog" aria-modal="true" aria-labelledby="auth-model-view-title">
        <div className="model-discovery-header">
          <div>
            <h2 id="auth-model-view-title">{t('authFiles.models.viewTitle')}</h2>
            <span className="auth-model-target">{displayName}</span>
            <span>{t('authFiles.models.viewDescription')}</span>
          </div>
          <button type="button" className="icon-button quiet" onClick={onClose} title={t('common.close')} aria-label={t('common.close')}><X size={18} aria-hidden="true" /></button>
        </div>
        <div className="model-discovery-search">
          <Search size={16} aria-hidden="true" />
          <input autoFocus value={search} onChange={(event) => setSearch(event.currentTarget.value)} placeholder={t('authFiles.models.search')} />
        </div>
        <div className="model-discovery-content">
          {loading ? (
            <div className="model-discovery-message"><LoaderCircle size={20} className="spin" />{t('authFiles.models.loading')}</div>
          ) : (
            <div className="model-discovery-results">
              <MessageNotice message={error} onDismiss={() => setError('')} />
              {visibleModels.length === 0 ? (
                <div className="model-discovery-message">{t(models.length ? 'authFiles.models.noMatch' : 'authFiles.models.viewEmpty')}</div>
              ) : (
                <div className="model-discovery-list">
                  {visibleModels.map((model) => (
                    <button type="button" className="model-discovery-row auth-model-view-row" key={model.id} onClick={() => void copyModel(model.id)} title={t('authFiles.models.copyModel')}>
                      <span><strong title={model.id}>{model.id}</strong>{model.displayName ? <small title={model.displayName}>{model.displayName}</small> : null}</span>
                      {copied === model.id ? <Check size={16} /> : <Copy size={16} />}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
        <div className="model-discovery-actions">
          <button type="button" className="secondary-button" onClick={onClose}>{t('common.close')}</button>
        </div>
      </section>
    </div>
  );
}
