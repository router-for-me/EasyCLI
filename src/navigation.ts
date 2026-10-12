const alwaysAvailablePages = new Set(['easy', 'home', 'versions', 'config', 'usage-records', 'agents']);

export function isAlwaysAvailablePage(pageId: string) {
  return alwaysAvailablePages.has(pageId);
}

export function canOpenAppPage(pageId: string, coreReady: boolean) {
  return coreReady || isAlwaysAvailablePage(pageId);
}

export const APP_NAVIGATION_REQUEST_EVENT = 'cpa-gui-navigation-request';

export type AppNavigationRequest = CustomEvent<() => void>;

export function requestAppNavigation(navigate: () => void) {
  const event: AppNavigationRequest = new CustomEvent(APP_NAVIGATION_REQUEST_EVENT, {
    cancelable: true,
    detail: navigate,
  });
  if (window.dispatchEvent(event)) navigate();
}
