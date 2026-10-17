import { useEffect, useRef, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { LoaderCircle, Play, RefreshCw, RotateCw, Square } from 'lucide-react';
import { type CoreStatus, useCoreRuntime } from '../coreRuntime';
import { clientApiProfiles } from '../services/clientAccess';
import { useI18n } from '../i18n';
import { useAppUpdate } from '../appUpdate';
import { FloatingNotice, useAppNotice } from '../appNotice';
import { VersionManagementPage, displayAppVersion } from './VersionManagementPage';
import { HomeAccessPanel } from './HomeAccessPanel';
import { HomeOverviewCards } from './HomeOverviewCards';
import { CoreHealthPanel } from './CoreHealthPanel';
import { useHomeOverview } from './useHomeOverview';
import './HomeDashboard.css';

type CoreProcessCommand = 'start_core_process' | 'stop_core_process' | 'restart_core_process';

type GuiSettings = {
  host: string;
  port: number;
  runOnStartup: boolean;
};

type CoreConfigSummary = {
  apiKeys: Array<{ apiKey: string }>;
};

type CoreTlsSettings = {
  enabled: boolean;
  cert: string;
  key: string;
};

export type KernelView = 'home' | 'versions';

export function KernelPage({ view = 'home' }: { view?: KernelView }) {
  if (view === 'versions') {
    return <VersionManagementPage />;
  }

  const { t } = useI18n();
  const { info: appUpdate } = useAppUpdate();
  const {
    status: coreStatus,
    statusError,
    refreshStatus,
    publishStatus,
  } = useCoreRuntime();

  const [installedAppVersion, setInstalledAppVersion] = useState('');
  const [listenHost, setListenHost] = useState('127.0.0.1');
  const [customPort, setCustomPort] = useState('8317');
  const [processBusy, setProcessBusy] = useState(false);
  const processFeedback = useAppNotice();
  const { showNotice: showProcessNotice, clearNotice: clearProcessNotice } = processFeedback;
  const copyFeedback = useAppNotice();
  const [copiedApiField, setCopiedApiField] = useState('');
  const [homeApiKey, setHomeApiKey] = useState<string | null | undefined>(undefined);
  const [homeApiKeyError, setHomeApiKeyError] = useState(false);
  const [configRevision, setConfigRevision] = useState(0);
  const [tlsEnabled, setTlsEnabled] = useState(false);

  const savedPortRef = useRef(8317);
  const copiedApiTimerRef = useRef<number | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlistenConfig: (() => void) | null = null;

    void listen('config-files-changed', () => {
      if (disposed) return;
      setConfigRevision((value) => value + 1);
      void loadGuiSettings();
      void loadTlsSettings();
      void refreshStatus();
      void loadHomeApiKey();
    }).then((stop) => {
      if (disposed) stop();
      else unlistenConfig = stop;
    });

    loadGuiSettings();
    void loadTlsSettings();
    void getVersion()
      .then((version) => {
        if (!disposed) setInstalledAppVersion(version);
      })
      .catch(() => undefined);
    void loadHomeApiKey();

    return () => {
      disposed = true;
      unlistenConfig?.();
      if (copiedApiTimerRef.current !== null) {
        window.clearTimeout(copiedApiTimerRef.current);
      }
    };
  }, []);

  const runCoreProcessCommand = async (command: CoreProcessCommand) => {
    const actionLabel =
      command === 'start_core_process'
        ? t('kernel.action.start')
        : command === 'stop_core_process'
          ? t('kernel.action.stop')
          : t('kernel.action.restart');
    setProcessBusy(true);
    clearProcessNotice();

    try {
      const result = await invoke<CoreStatus>(command);
      publishStatus(result);
      if (command === 'restart_core_process') {
        showProcessNotice({ key: 'kernel.notice.restarted' }, 'success');
      }
      return true;
    } catch (error) {
      const errorMessage = String(error);
      await refreshStatus();
      showProcessNotice(
        { key: 'kernel.notice.actionFailed', variables: { action: actionLabel, error: errorMessage } },
        'error',
      );
      return false;
    } finally {
      setProcessBusy(false);
    }
  };

  const loadGuiSettings = async () => {
    try {
      const settings = await invoke<GuiSettings>('get_gui_settings');
      setListenHost(settings.host);
      setCustomPort(String(settings.port));
      savedPortRef.current = settings.port;
    } catch {}
  };

  const loadHomeApiKey = async () => {
    try {
      const settings = await invoke<CoreConfigSummary>('get_core_config_settings');
      setHomeApiKey(settings.apiKeys[0]?.apiKey ?? null);
      setHomeApiKeyError(false);
    } catch {
      setHomeApiKey(undefined);
      setHomeApiKeyError(true);
    }
  };

  const loadTlsSettings = async () => {
    try {
      const settings = await invoke<CoreTlsSettings>('get_core_tls_settings');
      setTlsEnabled(settings.enabled);
    } catch {
      setTlsEnabled(false);
    }
  };

  const copyApiValue = async (value: string, field: string) => {
    copyFeedback.clearNotice();
    try {
      await navigator.clipboard.writeText(value);
      setCopiedApiField(field);
      if (copiedApiTimerRef.current !== null) {
        window.clearTimeout(copiedApiTimerRef.current);
      }
      copiedApiTimerRef.current = window.setTimeout(() => {
        setCopiedApiField('');
        copiedApiTimerRef.current = null;
      }, 1800);
    } catch {
      copyFeedback.showNotice({ key: 'kernel.notice.copyFailed' }, 'error');
    }
  };

  const currentVersion = coreStatus?.currentVersion ?? '';
  const coreInstalled = Boolean(coreStatus?.installed);
  const coreRunning = Boolean(coreStatus?.running);
  const coreReady = Boolean(coreStatus?.ready);
  const recovery = coreStatus?.recovery;
  const recovering = recovery?.phase === 'waiting' || recovery?.phase === 'restarting';
  const canStop = coreRunning || recovering;
  const coreProcessBusy = processBusy || Boolean(coreStatus?.starting) || recovery?.phase === 'restarting';

  const statusTone = statusError || recovery?.phase === 'failed' ? 'error' : coreProcessBusy || recovering ? 'pending' : coreRunning ? 'success' : 'neutral';
  const statusLabel = statusError
    ? t('common.detectionFailed')
    : !coreStatus
      ? t('common.detecting')
      : recovering
        ? t('kernel.recovery.recovering')
        : recovery?.phase === 'failed'
          ? t('kernel.recovery.failed')
          : coreProcessBusy
            ? t('common.processing')
            : coreRunning
              ? t('kernel.status.running')
              : t(coreInstalled ? 'kernel.status.stopped' : 'kernel.status.notInstalled');

  const resolvedAppVersion = appUpdate?.currentVersion || installedAppVersion;
  const currentAppVersion = resolvedAppVersion
    ? displayAppVersion(resolvedAppVersion)
    : t('common.detecting');

  const apiPort = Number(customPort);
  const apiProfiles = clientApiProfiles(
    Number.isInteger(apiPort) && apiPort >= 1 && apiPort <= 65535
      ? apiPort
      : savedPortRef.current,
    tlsEnabled,
    listenHost,
  );
  const healthContext = [listenHost, customPort, tlsEnabled, coreStatus?.processId, configRevision].join(':');
  const overview = useHomeOverview(coreReady, healthContext);

  return (
    <section className="page kernel-page home-page home-dashboard" aria-label={t('app.nav.home')}>
      <HomeOverviewCards snapshot={overview.snapshot} loading={overview.loading} coreReady={coreReady} onRefresh={overview.refresh} actions={<CoreHealthPanel compact
        coreReady={coreReady} models={overview.snapshot?.models ?? []}
        modelsLoading={overview.loading}
        modelsError={overview.snapshot?.errors.models ?? ''}
        onRefreshModels={overview.refresh} contextKey={healthContext}
      />} />
      <div className="home-top-grid">
        <div className="panel control-panel">
          <div className="panel-heading home-panel-heading">
            <div>
              <h2>{t('kernel.control.title')}</h2>
              <p>{t(coreReady ? 'home.runtime.ready' : coreInstalled ? 'home.runtime.stopped' : 'home.runtime.notInstalled')}</p>
            </div>
            <button type="button" className="icon-button quiet home-runtime-refresh" disabled={coreProcessBusy} onClick={() => void refreshStatus()} title={t('kernel.control.refresh')} aria-label={t('kernel.control.refresh')}><RefreshCw size={15} aria-hidden="true" /></button>
          </div>
          <dl className="panel-detail-grid home-runtime-details">
            <div className="panel-detail-row" data-runtime="installation">
              <dt>{t('kernel.control.installStatus')}</dt>
              <dd>{coreStatus ? t(coreInstalled ? 'kernel.control.installed' : 'kernel.status.notInstalled') : t(statusError ? 'common.detectionFailed' : 'common.detecting')}</dd>
            </div>
            <div className="panel-detail-row" data-runtime="status">
              <dt>{t('kernel.control.runStatus')}</dt>
              <dd><span className={`home-runtime-state ${statusTone}`} role="status" title={statusError || undefined}>
                {coreProcessBusy ? <LoaderCircle size={12} className="spin" aria-hidden="true" /> : <span className="home-runtime-dot" aria-hidden="true" />}
                {statusLabel}
              </span></dd>
            </div>
            <div className="panel-detail-row">
              <dt>{t('kernel.overview.coreVersion')}</dt>
              <dd>
                <span className="mono-tag">
                  {currentVersion
                    || (coreInstalled ? t('common.unavailable') : t('kernel.status.notInstalled'))}
                </span>
              </dd>
            </div>
            <div className="panel-detail-row">
              <dt>{t('kernel.overview.appVersion')}</dt>
              <dd><span className="mono-tag">{currentAppVersion}</span></dd>
            </div>
            <div className="panel-detail-row">
              <dt>{t('kernel.control.pid')}</dt>
              <dd><span className="mono-tag">{coreStatus?.processId || t('kernel.control.noPid')}</span></dd>
            </div>
            <div className="panel-detail-row">
              <dt>{t('home.runtime.port')}</dt>
              <dd><span className="mono-tag">{customPort}</span></dd>
            </div>
          </dl>
          {statusError && <p className="home-runtime-error" role="alert">{statusError}</p>}
          {recovery && <div role={recovery.phase === 'failed' ? 'alert' : 'status'}>
            <p>{t(recovery.phase === 'failed' ? 'kernel.recovery.failedHint' : recovery.phase === 'waiting' ? 'kernel.recovery.waitingHint' : 'kernel.recovery.restartingHint', { attempt: recovery.attempt, seconds: recovery.delaySeconds })}</p>
            {recovery.detail && <details><summary>{t('kernel.recovery.details')}</summary><p className="home-runtime-error">{recovery.detail}</p></details>}
          </div>}

          <div className="button-row panel-action-row control-action-row">
            <button
              type="button"
              className={canStop ? 'danger-button home-runtime-stop' : 'primary-button'}
              disabled={!coreInstalled || coreProcessBusy}
              onClick={() =>
                void runCoreProcessCommand(
                  canStop ? 'stop_core_process' : 'start_core_process',
                )
              }
            >
              {canStop ? <Square size={13} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}
              {coreProcessBusy ? t('common.processing') : canStop ? t('kernel.action.stop') : t('kernel.action.start')}
            </button>
            <button
              type="button"
              className="secondary-button"
              disabled={!coreInstalled || !coreRunning || coreProcessBusy}
              onClick={() =>
                void runCoreProcessCommand('restart_core_process')
              }
            >
              <RotateCw size={14} aria-hidden="true" />
              {t('kernel.action.restart')}
            </button>
          </div>
          <FloatingNotice key={processFeedback.revision} notice={processFeedback.notice} onDismiss={processFeedback.clearNotice} />
        </div>
        <HomeAccessPanel
          profiles={apiProfiles} apiKey={homeApiKey} keyError={homeApiKeyError}
          ready={coreReady} copiedField={copiedApiField} onCopy={copyApiValue}
        />
      </div>
      <FloatingNotice key={copyFeedback.revision} notice={copyFeedback.notice} onDismiss={copyFeedback.clearNotice} />
    </section>
  );
}
