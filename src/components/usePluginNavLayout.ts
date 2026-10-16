import { useLayoutEffect, useState } from 'react';
import { pluginNavLayout, type PluginNavLayout } from '../services/pluginNavigation';

const COMPACT_QUERY = '(max-width: 900px)';

const px = (value: string) => Number.parseFloat(value) || 0;

export function usePluginNavLayout(
  nav: HTMLElement | null,
  staticCount: number,
  pluginCount: number,
): { layout: PluginNavLayout; compact: boolean } {
  const [state, setState] = useState<{ layout: PluginNavLayout; compact: boolean }>({ layout: 'expanded', compact: false });

  useLayoutEffect(() => {
    const sidebar = nav?.closest<HTMLElement>('.sidebar');
    if (!nav || !sidebar || pluginCount === 0) {
      setState((current) => (current.layout === 'expanded' && !current.compact ? current : { layout: 'expanded', compact: false }));
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
      const layout = pluginNavLayout({
        available,
        rowHeight: row?.offsetHeight ?? 0,
        gap: px(navStyle.rowGap),
        padding: px(navStyle.paddingTop) + px(navStyle.paddingBottom),
        staticCount,
        pluginCount,
        compact: media.matches,
      });
      setState((current) => (
        current.layout === layout && current.compact === media.matches
          ? current
          : { layout, compact: media.matches }
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
  }, [nav, staticCount, pluginCount]);

  return state;
}
