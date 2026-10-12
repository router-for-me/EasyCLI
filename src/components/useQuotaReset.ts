import { useCallback } from 'react';
import { useI18n } from '../i18n';
import { usePrivacyMask } from '../privacy';
import { hasPendingClaudeReset, resetQuotaWithConfirmation } from '../services/quotaActions';
import { formatQuotaTimestamp, quotaResetExpiries, providerForFile, type AuthFile, type QuotaState } from '../services/quotaService';
import type { ConfirmationOptions } from './ConfirmationDialog';

export function useQuotaReset(
  askConfirmation: (options: ConfirmationOptions) => Promise<boolean>,
  setError: (message: string) => void,
) {
  const { locale, t } = useI18n();
  const { credentialName } = usePrivacyMask();
  return useCallback(async (file: AuthFile, quota: QuotaState) => {
    setError('');
    try {
      await resetQuotaWithConfirmation(file, () => askConfirmation({
        title: t('quota.reset'),
        message: t(providerForFile(file) === 'claude' ? 'quota.claude.confirm' : 'quota.confirm.title', { name: credentialName(file) }),
        confirmText: t(hasPendingClaudeReset(file) ? 'quota.claude.retry' : 'quota.confirm.button'),
        details: [
          { label: t('quota.resetCredits'), value: String(quota.resetCredits ?? '—') },
          ...quotaResetExpiries(quota).map((expiry, index) => ({
            label: t('quota.resetCreditExpiry', { index: index + 1 }),
            value: formatQuotaTimestamp(expiry, locale),
          })),
        ],
        warning: t(providerForFile(file) === 'claude' ? hasPendingClaudeReset(file) ? 'quota.claude.unknown' : 'quota.claude.warning' : 'quota.confirm.warning'),
      }));
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : String(requestError));
    }
  }, [askConfirmation, credentialName, locale, setError, t]);
}
