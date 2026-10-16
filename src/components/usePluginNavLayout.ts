import { useLayoutEffect, useState } from 'react';
import {
  resolvePluginNavCollapse,
  type PluginNavGroup,
  type PluginNavLayout,
  type PluginNavOverride,
} from '../services/pluginNavigation';

const COMPACT_QUERY = '(max-width: 900px)';

const px = (value: string) => Number.parseFloat(value) || 0;

type PluginNavCollapseState = { collapsed: ReadonlySet<string>; pluginsAuto: PluginNavLayout; compact: boolean };
const initialState: PluginNavCollapseState = { collapsed: new Set(), pluginsAuto: 'expanded', compact: false };

export function usePluginNavLayout(
  nav: HTMLElement | null,
  rows: number,
  groups: PluginNavGroup[],
  override: PluginNavOverride | null,
): PluginNavCollapseState {
  const [state, setState] = useState<PluginNavCollapseState>(initialState);
  const signature = JSON.stringify([rows, groups, override]);

  useLayoutEffect(() => {
    const sidebar = nav?.closest<HTMLElement>('.sidebar');
    if (!nav || !sidebar) {
      setState((current) => (current.collapsed.size === 0 && current.pluginsAuto === 'expanded' && !current.compact ? current : initialState));
      return undefined;
    }
    const brand = sidebar.querySelector<HTMLElement>('.sidebar-brand');
    const bottom = sidebar.querySelector<HTMLElement>('.sidebar-bottom');
    const media = window.matchMedia(COMPACT_QUERY);
    const measure = () => {
      const row = nav.querySelector<HTMLElement>('button:not(.nav-plugin-entry, .nav-plugin-toggle)');
      const sidebarStyle = getComputedStyle(sidebar);
      const navStyle = getComputedStyle(nav);
      const available = sidebar.clientHeight
        - px(sidebarStyle.paddingTop) - px(sidebarStyle.paddingBottom)
        - (brand ? brand.offsetHeight + px(getComputedStyle(brand).marginBottom) : 0)
        - (bottom?.offsetHeight ?? 0);
      const { collapsed, pluginsAuto } = resolvePluginNavCollapse({
        available,
        rowHeight: row?.offsetHeight ?? 0,
        gap: px(navStyle.rowGap),
        padding: px(navStyle.paddingTop) + px(navStyle.paddingBottom),
        compact: media.matches,
      }, rows, groups, override);
      setState((current) => (
        current.pluginsAuto === pluginsAuto && current.compact === media.matches
          && current.collapsed.size === collapsed.length && collapsed.every((id) => current.collapsed.has(id))
          ? current
          : { collapsed: new Set(collapsed), pluginsAuto, compact: media.matches }
      ));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(sidebar);
    if (brand) observer.observe(brand);
    if (bottom) observer.observe(bottom);
    window.addEventListener('resize', measure);
    media.addEventListener('change', measure);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', measure);
      media.removeEventListener('change', measure);
    };
  }, [nav, signature]);

  return state;
}
