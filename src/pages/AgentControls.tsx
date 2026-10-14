import { MessageNotice } from '../appNotice';
import { AppWindow, FolderOpen, LoaderCircle, Play, RefreshCw, Square, Terminal, Trash2 } from 'lucide-react';
import { useI18n } from '../i18n';

type LaunchTarget = { id: 'app' | 'cli'; label: string; detail: string };

export function AgentRunControls({
  name, dualTargets, desktop, cliService, cliServiceRestarting, targets, enabled, busyAction, harness, onLaunch, onRestart, onRestartCli, onStop, onRestartWeb, error, onErrorDismiss,
}: {
  name: string;
  dualTargets: boolean;
  desktop: boolean;
  cliService: boolean;
  cliServiceRestarting: boolean;
  targets: LaunchTarget[];
  enabled: boolean;
  busyAction: string | null;
  harness: { running: boolean; pid: number | null; mode: string | null } | null;
  onLaunch: (target: LaunchTarget | null) => void;
  onRestart: () => void;
  onRestartCli: () => void;
  onStop: () => void;
  onRestartWeb: () => void;
  error: string;
  onErrorDismiss?: () => void;
}) {
  const { t } = useI18n();
  const busy = busyAction !== null;
  const cli = targets.find((target) => target.id === 'cli') ?? null;
  const app = targets.find((target) => target.id === 'app') ?? null;
  const launchButton = (target: LaunchTarget | null, kind?: 'cli' | 'app') => {
    const action = dualTargets ? `launch-${kind}` : 'launch';
    const starting = busyAction === action;
    return <button type="button" className="secondary-button agent-launch-button"
      disabled={busy || !enabled || !target || (cliServiceRestarting && target.id === 'cli')} onClick={() => onLaunch(target)}
      title={target?.detail ?? t('agents.launch.unavailable')}>
      {starting ? <LoaderCircle size={16} className="spin" />
        : target?.id === 'cli' ? <Terminal size={16} />
        : target?.id === 'app' ? <AppWindow size={16} /> : <Play size={16} />}
      {starting ? t('agents.launch.starting') : kind ? t(kind === 'cli' ? 'agents.launch.startCli' : 'agents.launch.startApp')
        : t('agents.launch.start', { target: target?.label ?? name })}
    </button>;
  };
  return <section className="agent-run-controls" aria-label={t('agents.run.title')}>
    <div className="agent-section-heading"><div><strong>{t('agents.run.title')}</strong></div></div>
    <div className="agent-launch-actions">
      {cliService ? <button type="button" className="secondary-button agent-launch-button"
        onClick={onRestartCli} disabled={busy || cliServiceRestarting || !enabled || !cli}
        title={cliServiceRestarting ? t('agents.launch.restartingCliDetail') : cli?.detail ?? t('agents.launch.unavailable')}>
        {cliServiceRestarting ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />}
        {t(cliServiceRestarting ? 'agents.launch.restartingCli' : 'agents.launch.restartCli')}
      </button> : null}
      {harness?.running ? <button type="button" className="danger-button agent-launch-button"
          disabled={busy} onClick={onStop} title={t('agents.deepseekLaunch.runningDetail', { pid: harness.pid ?? '—', mode: harness.mode ?? '—' })}>
          {busyAction === 'stop-deepseek' ? <LoaderCircle size={16} className="spin" /> : <Square size={16} />}
          {t(busyAction === 'stop-deepseek' ? 'agents.deepseekLaunch.stopping' : 'agents.deepseekLaunch.stop')}
        </button> : dualTargets ? launchButton(cli, 'cli') : launchButton(targets[0] ?? null)}
      {dualTargets ? launchButton(app, 'app') : null}
      {desktop ? <button type="button" className="secondary-button agent-launch-button"
        onClick={onRestart} disabled={busy || !enabled || !app} title={app?.detail ?? t('agents.launch.unavailable')}>
        {busyAction === 'restart-app' ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />}
        {t(busyAction === 'restart-app' ? 'agents.launch.restartingApp' : 'agents.launch.restartApp')}
      </button> : null}
      {harness?.running && harness.mode === 'web' ? <button type="button" className="secondary-button agent-launch-button"
        onClick={onRestartWeb} disabled={busy}>
        {busyAction === 'restart-deepseek' ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />}
        {t(busyAction === 'restart-deepseek' ? 'agents.deepseekLaunch.restarting' : 'agents.deepseekLaunch.restart')}
      </button> : null}
    </div>
    {error ? <MessageNotice message={error} onDismiss={onErrorDismiss} /> : null}
  </section>;
}

export function AgentConfigurationFeedback({ pending, description, status = '' }: {
  pending: boolean; description: string; status?: string;
}) {
  const { t } = useI18n();
  const state = pending ? t('agents.modify.pending') : status;
  if (!state && !description) return null;
  return <div className="agent-save-feedback agent-shared-feedback" aria-live="polite">
    {state ? <span className="agent-write-state">{state}</span> : null}
    {description ? <small>{description}</small> : null}
  </div>;
}

export function AgentConfigManagementPanel({
  pi, codex, busyAction, canTemplate, canUpdatePi, canUninstallPi, pluginInstalled, pluginVersion, updateLabel,
  executablePath, onChooseExecutablePath, onClearExecutablePath, onBackup, onRestore, onTemplate, onClear, onUpdatePi, onUninstallPi, canClearIntegration, onClearIntegration,
  dualTargets, desktopExecutablePath, onChooseDesktopExecutablePath, onClearDesktopExecutablePath,
}: {
  pi: boolean; codex: boolean; busyAction: string | null; canTemplate: boolean;
  canUpdatePi: boolean; canUninstallPi: boolean; pluginInstalled: boolean; pluginVersion: string | null; updateLabel: string;
  executablePath: string; onChooseExecutablePath: () => void; onClearExecutablePath: () => void;
  dualTargets: boolean; desktopExecutablePath: string;
  onChooseDesktopExecutablePath: () => void; onClearDesktopExecutablePath: () => void;
  onBackup: () => void; onRestore: () => void; onTemplate: () => void; onClear: () => void;
  onUpdatePi: () => void; onUninstallPi: () => void;
  canClearIntegration: boolean; onClearIntegration: () => void;
}) {
  const { t } = useI18n();
  const busy = busyAction !== null;
  return <div className="agent-management-sections">
    <section className="agent-management-row">
      <div><h3>{t(dualTargets ? 'agents.executablePath.cliTitle' : 'agents.executablePath.title')}</h3><p>{t('agents.executablePath.description')}</p>
        {executablePath ? <small className="agent-path-value" title={executablePath}>{executablePath}</small> : null}</div>
      <div className="agent-management-actions">
        <button type="button" className="secondary-button" onClick={onChooseExecutablePath} disabled={busy}>
          <FolderOpen size={16} />{t('agents.executablePath.choose')}
        </button>
        {executablePath ? <button type="button" className="quiet-button" onClick={onClearExecutablePath} disabled={busy}>{t('agents.executablePath.clear')}</button> : null}
      </div>
    </section>
    {dualTargets ? <section className="agent-management-row">
      <div><h3>{t('agents.executablePath.desktopTitle')}</h3><p>{t('agents.executablePath.description')}</p>
        {desktopExecutablePath ? <small className="agent-path-value" title={desktopExecutablePath}>{desktopExecutablePath}</small> : null}</div>
      <div className="agent-management-actions">
        <button type="button" className="secondary-button" onClick={onChooseDesktopExecutablePath} disabled={busy}>
          <FolderOpen size={16} />{t('agents.executablePath.choose')}
        </button>
        {desktopExecutablePath ? <button type="button" className="quiet-button" onClick={onClearDesktopExecutablePath} disabled={busy}>{t('agents.executablePath.clear')}</button> : null}
      </div>
    </section> : null}
    {!pi ? <section className="agent-management-row">
      <div><h3>{t('agents.management.backups')}</h3><p>{t('agents.management.backupsDescription')}</p></div>
      <div className="agent-management-actions">
        <button type="button" className="secondary-button" onClick={onBackup} disabled={busy}>
          {busyAction === 'backup' ? <LoaderCircle size={16} className="spin" /> : null}{t('agents.backup.create')}
        </button>
        <button type="button" className="secondary-button" onClick={onRestore} disabled={busy}>{t('agents.backup.button')}</button>
      </div>
    </section> : null}
    {!pi ? <section className="agent-management-row agent-management-template">
      <div><h3>{t('agents.management.template')}</h3><p>{t('agents.management.templateDescription')}</p></div>
      <div className="agent-management-actions">
        <button type="button" className="secondary-button" onClick={onTemplate} disabled={busy || !canTemplate}>{t('agents.modify.default')}</button>
      </div>
    </section> : null}
    {codex ? <section className="agent-management-row agent-management-clear-integration">
      <div><h3>{t('agents.nativeOAuth.restoreTitle')}</h3><p>{t('agents.nativeOAuth.restoreHint')}</p></div>
      <div className="agent-management-actions">
        <button type="button" id="agent-clear-integration" className="secondary-button agent-clear-integration"
          onClick={onClearIntegration} disabled={busy || !canClearIntegration}>
          {busyAction === 'native-oauth' ? <LoaderCircle size={16} className="spin" /> : null}
          {t('agents.nativeOAuth.restore')}
        </button>
      </div>
    </section> : null}
    {!pi ? <section className="agent-management-row">
      <div><h3>{t(codex ? 'agents.modify.clear' : 'agents.clearIntegration.button')}</h3><p>{t(codex ? 'agents.management.clearDescription' : 'agents.clearIntegration.description')}</p></div>
      <div className="agent-management-actions">
        <button type="button" className="danger-button" onClick={onClear} disabled={busy || (!codex && !canClearIntegration)}><Trash2 size={16} />{t(codex ? 'agents.modify.clear' : 'agents.clearIntegration.button')}</button>
      </div>
    </section> : null}
    {pi ? <section className="agent-management-row">
      <div><h3>{t('agents.management.plugin')}</h3><p>{pluginInstalled
        ? t('agents.management.pluginVersion', { version: pluginVersion ?? t('agents.notFetched') })
        : t('agents.management.pluginMissing')}</p>{updateLabel ? <small className="agent-inline-message">{updateLabel}</small> : null}</div>
      {pluginInstalled ? <div className="agent-management-actions">
        <button type="button" className="secondary-button" onClick={onUpdatePi} disabled={busy || !canUpdatePi}>
          {busyAction === 'update-pi' ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />}
          {t(busyAction === 'update-pi' ? 'agents.pi.updating' : 'agents.pi.update')}
        </button>
        <button type="button" className="danger-button" onClick={onUninstallPi} disabled={busy || !canUninstallPi}>
          {busyAction === 'uninstall-pi' ? <LoaderCircle size={16} className="spin" /> : <Trash2 size={16} />}
          {t(busyAction === 'uninstall-pi' ? 'agents.pi.uninstalling' : 'agents.pi.uninstall')}
        </button>
      </div> : null}
    </section> : null}
  </div>;
}
