import { useState } from 'react';
import { ThemeSelector } from '../../src/components/ThemeSelector';
import { useThemePreference } from '../../src/theme';
import { createThemeRegistry, themeRegistry } from '../../src/themes/registry';

export const extensionRegistry = createThemeRegistry([
  ...themeRegistry.themes,
  { id: 'test-light', nameKey: 'app.theme.unavailableHint', appearance: 'light', order: 1,
    windowBackground: '#e9efff', tokens: {
      workspaceBackground: '#e9efff', surface: '#dce8fa', hoverBackground: '#c0d0ee',
      textPrimary: '#122544', textSecondary: '#233655', textTertiary: '#345766', textMuted: '#456877',
      border: '#456789', borderSubtle: '#6789ab', sidebarBackground: '#d1dbee', accent: '#3344aa',
    } },
  { id: 'test-dark', nameKey: 'app.theme.dark', appearance: 'dark', order: 1,
    windowBackground: '#102030', tokens: {
      workspaceBackground: '#102030', surface: '#203040', hoverBackground: '#304050',
      textPrimary: '#f4eccc', textSecondary: '#e2d8c8', textTertiary: '#c0bdbb', textMuted: '#aaa9a8',
      border: '#556677', borderSubtle: '#334455', sidebarBackground: '#132333', accent: '#abcdef',
    } },
  { id: 'test-dark-2', nameKey: 'app.theme.dark', appearance: 'dark', order: 2,
    windowBackground: '#193020', tokens: { workspaceBackground: '#193020', surface: '#254433', accent: '#aabbcc' } },
]);

export function ThemeExtensionFixture() {
  const [value, onChange] = useThemePreference();
  const [open, onOpenChange] = useState(false);
  return <div className="app-shell">
    <aside className="sidebar"><div className="sidebar-bottom">
      <ThemeSelector value={value} onChange={onChange} open={open} onOpenChange={onOpenChange} registry={extensionRegistry} />
    </div></aside>
    <div className="workspace"><main className="content">
      <section className="panel"><div className="panel-heading"><h2>Surface</h2><p>Secondary text</p></div></section>
      <div className="usage-records-page"><div className="usage-tab-panel" aria-labelledby="usage-tab-events">
        <section className="usage-analysis-card">Usage surface</section>
      </div></div>
      <div className="config-dialog"><div className="config-dialog-heading"><h2>Dialog</h2></div></div>
      <div className="version-metric-tile"><span className="version-metric-label">Current version</span><strong className="version-metric-value">v1.0.0</strong></div>
      <footer className="app-release-notes-footer">Release notes footer</footer>
      <section className="agent-core-setting-section agent-model-section">Model settings</section>
      <section className="model-config-card">Model configuration</section>
    </main></div>
  </div>;
}
