import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { I18nProvider } from '../../src/i18n';
import { WindowFrame } from '../../src/components/WindowFrame';
import '../../src/styles.css';

const state = {
  calls: [] as string[],
  maximized: false,
  fail: false,
  listeners: new Map<string, number>(),
  emit(event: string, payload: unknown) {
    const id = state.listeners.get(event);
    if (id !== undefined) (window as any).__TAURI_INTERNALS__.runCallback(id, { event, payload });
  },
};
Object.assign(window, { isTauri: true, titlebarTest: state });
mockWindows('main');
mockIPC((command, args) => {
  state.calls.push(command);
  if (command === 'plugin:event|listen') {
    state.listeners.set(args!.event as string, args!.handler as number);
    return state.listeners.size;
  }
  if (command === 'plugin:window|is_maximized') return state.maximized;
  if (command === 'plugin:window|is_focused') return true;
  if (command === 'plugin:window|toggle_maximize') {
    if (state.fail) throw new Error('Test failure');
    state.maximized = !state.maximized;
  }
  return null;
});
createRoot(document.getElementById('root')!).render(
  <React.StrictMode><I18nProvider><WindowFrame><main>Window content</main></WindowFrame></I18nProvider></React.StrictMode>,
);
