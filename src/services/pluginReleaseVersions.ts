import { buildRepositoryURL, getPluginRepositorySlug } from './pluginResources';
import { invoke } from '@tauri-apps/api/core';
import type { PluginStoreResponse } from './plugins';

export const supportsPluginVersionSelection = (installType: string): boolean =>
  installType.trim().toLowerCase() === 'github-release';

export const isValidManualReleaseTag = (value: string): boolean =>
  /^[A-Za-z0-9][A-Za-z0-9._+-]{0,127}$/.test(value.trim());

export function getGitHubRepositorySlug(repository: string): string {
  const normalized = repository.trim().replace(/^github\.com\//i, '');
  const value = buildRepositoryURL(normalized);
  if (!value) return '';
  const url = new URL(value);
  if (!['github.com', 'www.github.com'].includes(url.hostname.toLowerCase())) return '';
  return getPluginRepositorySlug(normalized);
}

export function buildGitHubReleasesPageURL(repository: string): string {
  const slug = getGitHubRepositorySlug(repository);
  return slug ? `https://github.com/${slug}/releases` : '';
}

export function isNewerPluginRelease(installed: string, latest: string): boolean {
  const current = installed.trim().replace(/^[vV]/, '');
  const next = latest.trim().replace(/^[vV]/, '');
  if (!current || !next || current === next) return false;
  if (!/^\d+(\.\d+)*$/.test(current) || !/^\d+(\.\d+)*$/.test(next)) return true;
  const a = current.split('.').map(Number);
  const b = next.split('.').map(Number);
  for (let index = 0; index < Math.max(a.length, b.length); index++) {
    if ((a[index] ?? 0) !== (b[index] ?? 0)) return (a[index] ?? 0) < (b[index] ?? 0);
  }
  return false;
}

export async function refreshPluginReleaseVersions(store: PluginStoreResponse): Promise<PluginStoreResponse> {
  const repositories = [...new Set(store.plugins.filter(entry =>
    supportsPluginVersionSelection(entry.installType) && !entry.authRequired,
  ).map(entry => getGitHubRepositorySlug(entry.repository)).filter(Boolean))];
  if (!repositories.length) return store;
  const releases = await invoke<{ repository: string; version?: string | null; error?: string | null }[]>('get_plugin_latest_releases', { repositories });
  const versions = new Map(releases.flatMap(release => release.version ? [[release.repository, release.version] as const] : []));
  const sourceErrors = [...store.sourceErrors, ...releases.flatMap(release => {
    if (!release.error) return [];
    const entry = store.plugins.find(entry => getGitHubRepositorySlug(entry.repository) === release.repository);
    return [{ sourceId: entry?.sourceId ?? '', sourceName: entry?.name || entry?.id || release.repository, sourceUrl: entry?.sourceUrl ?? '', message: release.error }];
  })];
  const plugins = store.plugins.map(entry => {
    if (!supportsPluginVersionSelection(entry.installType) || entry.authRequired) return entry;
    const version = versions.get(getGitHubRepositorySlug(entry.repository));
    return version ? { ...entry, version, updateAvailable: entry.installed
      && ['matched', 'assumed'].includes(entry.installSourceStatus)
      && isNewerPluginRelease(entry.installedVersion, version) } : entry;
  });
  return { ...store, plugins, sourceErrors };
}
