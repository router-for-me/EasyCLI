import { KeyRound } from 'lucide-react';
import { useI18n } from '../i18n';
import { pluginText } from '../i18n/plugins';
import { openSettingField } from '../services/settingsNavigation';

export const isGitHubRateLimit = (message: string) => /plugin_store_rate_limited|GitHub API rate limited/i.test(message);

export function GitHubTokenHint() {
  const { locale } = useI18n();
  return <span className="plugin-rate-limit-hint">{pluginText('rateLimitHint', locale)}
    <button type="button" className="secondary-button compact-button" onClick={() => openSettingField(['server', 'github-token'])}><KeyRound size={14} aria-hidden="true" />{pluginText('configureToken', locale)}</button>
  </span>;
}
