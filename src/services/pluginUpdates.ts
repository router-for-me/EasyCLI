import type { PluginStoreResponse } from './plugins';

/** Only report current for an unambiguous, known installation source/version. */
export function getPluginUpdateStatus(store: PluginStoreResponse, id: string) {
  if (store.sourceErrors.length) return 'updateIncomplete';
  const listings = store.plugins.filter(entry => entry.id === id);
  if (!listings.length) return 'updateMissing';
  const installed = listings.filter(entry => entry.installed && entry.installedSourceId
    && entry.sourceId === entry.installedSourceId);
  if (installed.length !== 1) return 'updateReview';
  const entry = installed[0];
  if (!entry.version || !entry.installedVersion) return 'updateReview';
  if (entry.updateAvailable) return 'updateFound';
  // A missing update flag can also mean that versions could not be compared.
  if (entry.version.replace(/^v/, '') !== entry.installedVersion.replace(/^v/, '')) return 'updateReview';
  return 'updateCurrent';
}
