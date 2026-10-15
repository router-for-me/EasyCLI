import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { isTauri } from '@tauri-apps/api/core';
import { AppErrorBoundary } from './AppErrorBoundary';
import App from './App';
import { I18nProvider } from './i18n';
import { initializeTheme, initializeThemePreferences } from './theme';
import { initializeUsagePreferences } from './services/usagePreferences';
import { initializePrivacyMode } from './privacy';
import './styles/index.css';

async function bootstrap() {
  try {
    await Promise.all([initializeUsagePreferences(), initializeThemePreferences(), initializePrivacyMode()]);
    if (import.meta.env.DEV && !isTauri()) {
      const { installBrowserMock } = await import('./mocks/browserMock');
      installBrowserMock();
    }

    initializeTheme();

    createRoot(document.getElementById('root')!).render(
      <StrictMode>
        <I18nProvider>
          <AppErrorBoundary>
            <App />
          </AppErrorBoundary>
        </I18nProvider>
      </StrictMode>,
    );
  } catch (err: unknown) {
    console.error('Fatal bootstrap error:', err);
    const root = document.getElementById('root') || document.body;
    const message = err instanceof Error ? (err.stack || err.message) : String(err);
    root.innerHTML = `<div style="padding:24px;color:#dc2626;background:#ffffff;font-family:monospace;font-size:14px;position:fixed;top:0;left:0;right:0;bottom:0;overflow:auto;z-index:999999;">
      <h2>[Fatal Bootstrap Error]</h2>
      <pre>${message}</pre>
    </div>`;
  }
}

void bootstrap();
