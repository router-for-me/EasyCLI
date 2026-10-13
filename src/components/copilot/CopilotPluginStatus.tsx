import { useI18n } from '../../i18n';

export function CopilotPluginStatus({ installed, version, standalone = true }: { installed: boolean; version: string | null; standalone?: boolean }) {
  const { t } = useI18n();
  const card = <div data-testid="copilot-plugin-status">
      <span>GitHub Copilot Chat · {t('agents.pluginVersion')}</span>
      <strong>{installed ? version ?? t('agents.notFetched') : t('agents.status.notInstalled')}</strong>
  </div>;
  return standalone ? <div className="agent-status-grid">{card}</div> : card;
}
