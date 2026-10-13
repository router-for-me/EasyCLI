import { useI18n } from '../../i18n';
import type { MessageKey } from '../../i18n/resources';
import './CopilotSetupGuide.css';

const setupErrors: Record<string, MessageKey> = {
  COPILOT_SETUP_NOT_INSTALLED: 'agents.copilotSetup.notInstalled',
  COPILOT_SETUP_LAUNCH_FAILED: 'agents.copilotSetup.launchFailed',
  COPILOT_SETUP_DISABLED: 'agents.copilotSetup.disabled',
  COPILOT_SETUP_DIRTY: 'agents.copilotSetup.dirty',
  COPILOT_SETUP_UNSUPPORTED: 'agents.copilotSetup.unsupported',
  COPILOT_SETUP_PROFILE: 'agents.copilotSetup.profile',
  COPILOT_SETUP_FAILED: 'agents.copilotSetup.failed',
  COPILOT_SETUP_TIMEOUT: 'agents.copilotSetup.timeout',
};

export function copilotSetupErrorKey(error: string): MessageKey | undefined {
  const code = error.match(/\bCOPILOT_SETUP_[A-Z_]+\b/)?.[0];
  return code ? setupErrors[code] : undefined;
}

export function CopilotSetupGuide({ pending }: { pending: boolean }) {
  const { t } = useI18n();
  return pending ? <p className="copilot-setup-guide" role="status">{t('agents.copilotSetup.pending')}</p> : null;
}
