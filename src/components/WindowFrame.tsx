import { useEffect, useState, type ReactNode } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X } from 'lucide-react';
import { useI18n } from '../i18n';
import './WindowFrame.css';

// The browser preview is explicitly opt-in and never affects production builds.
const preview = import.meta.env.DEV
  && new URLSearchParams(window.location.search).get('windowPreview') === 'windows';

/**
 * Must agree with the `#[cfg(target_os = "windows")]` branch in `main.rs` that removes the native
 * decorations. The check is synchronous and cannot fail, so the window never ends up without any
 * controls, and the titlebar is present on the first render (no layout shift). WebView2 always
 * reports "Windows NT" in its user agent.
 */
function isNativeWindowsShell() {
  return isTauri() && !document.body.dataset.browserMock && /\bWindows NT\b/.test(navigator.userAgent);
}

export function WindowFrame({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const [enabled] = useState(() => preview || isNativeWindowsShell());
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);
  const [error, setError] = useState(false);

  useEffect(() => {
    if (!enabled || preview) return;
    const appWindow = getCurrentWindow();
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const register = (promise: Promise<() => void>) => {
      void promise.then(unlisten => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      }).catch(() => { if (!disposed) setError(true); });
    };
    const refresh = async () => {
      try {
        const value = await appWindow.isMaximized();
        if (!disposed) setMaximized(value);
      } catch { if (!disposed) setError(true); }
    };
    // Resize events fire continuously while dragging the border; query at most once per frame.
    let frame = 0;
    register(appWindow.onResized(() => {
      if (frame) return;
      frame = requestAnimationFrame(() => { frame = 0; void refresh(); });
    }));
    register(appWindow.onFocusChanged(({ payload }) => setFocused(payload)));
    void refresh();
    void appWindow.isFocused().then(value => {
      if (!disposed) setFocused(value);
    }).catch(() => { if (!disposed) setError(true); });
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      unlisteners.forEach(unlisten => unlisten());
    };
  }, [enabled]);

  const act = async (action: 'minimize' | 'toggleMaximize' | 'close') => {
    setError(false);
    if (preview) {
      if (action === 'toggleMaximize') setMaximized(value => !value);
      // A web preview must not close or minimize the browser itself.
      return;
    }
    try {
      const appWindow = getCurrentWindow();
      await appWindow[action]();
      if (action === 'toggleMaximize') setMaximized(await appWindow.isMaximized());
    } catch { setError(true); }
  };

  if (!enabled) return <>{children}</>;
  const maximizeLabel = t(maximized ? 'app.window.restore' : 'app.window.maximize');
  return (
    <div className="immersive-window" data-focused={focused}>
      <header className="window-titlebar">
        <div className="window-drag-region" data-tauri-drag-region={!preview ? true : undefined}
          onDoubleClick={preview ? () => void act('toggleMaximize') : undefined} />
        {error ? <span className="window-error" role="alert">{t('app.window.error')}</span> : null}
        <div className="window-controls">
          <button type="button" aria-label={t('app.window.minimize')} title={t('app.window.minimize')}
            onClick={() => void act('minimize')}><Minus aria-hidden="true" size={14} /></button>
          <button type="button" aria-label={maximizeLabel} title={maximizeLabel}
            onClick={() => void act('toggleMaximize')}>
            {maximized ? <Copy aria-hidden="true" size={12} /> : <Square aria-hidden="true" size={12} />}
          </button>
          <button type="button" className="window-close" aria-label={t('app.window.close')} title={t('app.window.close')}
            onClick={() => void act('close')}><X aria-hidden="true" size={16} /></button>
        </div>
      </header>
      <div className="window-content">{children}</div>
    </div>
  );
}
