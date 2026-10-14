const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs/promises');

const USER_AGENTS = {
  windows: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0',
  macos: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)',
  linux: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)',
};

// Records what #root first rendered, to prove the titlebar does not appear in a later render.
const recordFirstRender = () => {
  new MutationObserver((_, observer) => {
    const first = document.getElementById('root')?.firstElementChild;
    if (!first) return;
    window.firstRenderedClass = first.className;
    observer.disconnect();
  }).observe(document, { childList: true, subtree: true });
};

const coversCenterOf = (locator) => locator.evaluate(el => {
  const rect = el.getBoundingClientRect();
  return el.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
});

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const server = await createServer({ configFile: false, root: path.resolve(__dirname, '..'), plugins: [react()],
    logLevel: 'error', server: { host: '127.0.0.1', port: 0, watch: null } });
  let browser;
  try {
    await server.listen();
    const base = `http://127.0.0.1:${server.httpServer.address().port}`;
    browser = await chromium.launch({ headless: true });
    const errors = [];
    const newPage = async (options = {}) => {
      const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: 'reduce', ...options });
      const page = await context.newPage();
      page.on('pageerror', error => errors.push(String(error)));
      await page.addInitScript(() => localStorage.setItem('easy-cli-proxy-api.locale', 'en'));
      await page.addInitScript(recordFirstRender);
      return page;
    };

    // --- Browser preview of the full app shell ---
    const page = await newPage();
    await page.goto(`${base}/?mock=running&mockDelay=0`);
    await page.locator('.app-shell').waitFor();
    assert.equal(await page.locator('.window-titlebar').count(), 0, 'ordinary browser is unchanged');
    await page.goto(`${base}/?mock=running&mockDelay=0&windowPreview=windows`);
    await page.locator('.window-titlebar').waitFor();
    assert.equal(await page.locator('.window-caption, .window-preview-label').count(), 0);
    const sidebarBounds = await page.locator('.sidebar').boundingBox();
    const titlebarBounds = await page.locator('.window-titlebar').boundingBox();
    const workspaceBounds = await page.locator('.workspace').boundingBox();
    assert.equal(sidebarBounds.y, 0, 'sidebar starts at the top edge');
    assert.equal(sidebarBounds.height, 800, 'sidebar fills the entire window height');
    assert.equal(titlebarBounds.x, sidebarBounds.width, 'titlebar only occupies the workspace');
    assert.equal(workspaceBounds.y, titlebarBounds.height, 'workspace scrollport starts below the titlebar');
    assert.equal(workspaceBounds.y + workspaceBounds.height, 800, 'workspace reaches the bottom edge');
    await page.getByRole('button', { name: 'Maximize', exact: true }).click();
    await page.getByRole('button', { name: 'Restore window', exact: true }).waitFor();
    await page.locator('.window-drag-region').dblclick();
    await page.getByRole('button', { name: 'Maximize', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Minimize', exact: true }).click();
    await page.locator('.window-close').click();
    assert.equal(await page.locator('.app-shell').count(), 1, 'preview controls do not close the browser');
    await page.getByRole('button', { name: 'Maximize', exact: true }).focus();
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('.window-close').evaluate(el => el.matches(':focus-visible')), true);
    await fs.mkdir('bin-work', { recursive: true });
    await page.locator('.theme-selector-trigger[data-appearance="light"]').click();
    await page.locator('.theme-selector-option[data-theme-id="light"]').click();
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'light');
    await page.screenshot({ path: 'bin-work/windows-immersive-light.png' });
    await page.locator('.theme-selector-trigger[data-appearance="dark"]').click();
    await page.locator('.theme-selector-option[data-theme-id="dark"]').click();
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    await page.screenshot({ path: 'bin-work/windows-immersive-dark.png' });

    await page.locator('.sidebar-easy-entry').click();
    await page.locator('.app-shell-easy-mode').waitFor();
    assert.equal(await page.locator('.window-titlebar').count(), 1, 'easy mode keeps controls');
    const easyBounds = await page.locator('.app-shell-easy-mode .workspace').boundingBox();
    assert.equal(easyBounds.y + easyBounds.height, 800, 'easy mode is not clipped below the titlebar');
    await page.setViewportSize({ width: 640, height: 600 });
    assert.equal(await page.locator('.window-close').isVisible(), true);
    const bounds = await page.locator('.window-close').boundingBox();
    assert.ok(bounds.x + bounds.width <= 640 && bounds.y === 0);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    await page.screenshot({ path: 'bin-work/windows-immersive-compact.png' });

    // --- Native window API through the Tauri IPC mock (Windows WebView2 user agent) ---
    const native = await newPage({ userAgent: USER_AGENTS.windows });
    await native.goto(`${base}/tests/fixtures/windows-titlebar.html`);
    await native.locator('.window-titlebar').waitFor();
    assert.equal(await native.evaluate(() => window.firstRenderedClass), 'immersive-window',
      'titlebar is present on the first render, without waiting for IPC');
    assert.equal(await native.locator('[data-tauri-drag-region]').count(), 1);
    await native.getByRole('button', { name: 'Maximize', exact: true }).click();
    await native.getByRole('button', { name: 'Restore window', exact: true }).waitFor();
    await native.getByRole('button', { name: 'Minimize', exact: true }).click();
    await native.locator('.window-close').click();
    const calls = await native.evaluate(() => window.titlebarTest.calls);
    for (const command of ['toggle_maximize', 'minimize', 'close']) {
      assert.ok(calls.includes(`plugin:window|${command}`), `${command} uses native API`);
    }
    assert.ok(!calls.includes('detect_core_platform'), 'platform detection does not depend on IPC');
    await native.evaluate(() => {
      window.titlebarTest.maximized = false;
      for (let i = 0; i < 20; i += 1) window.titlebarTest.emit('tauri://resize', { width: 640, height: 600 });
      window.titlebarTest.emit('tauri://blur', null);
    });
    await native.getByRole('button', { name: 'Maximize', exact: true }).waitFor();
    await native.waitForFunction(() => document.querySelector('.immersive-window').dataset.focused === 'false');
    const isMaximizedCalls = await native.evaluate(() =>
      window.titlebarTest.calls.filter(command => command === 'plugin:window|is_maximized').length);
    assert.ok(isMaximizedCalls <= 4, `resize bursts are coalesced (got ${isMaximizedCalls} is_maximized calls)`);
    await native.evaluate(() => { window.titlebarTest.fail = true; });
    await native.getByRole('button', { name: 'Maximize', exact: true }).click();
    await native.getByRole('alert').waitFor();

    // Modal backdrops (lowest app backdrop is .install-dialog-backdrop) must cover the titlebar so a
    // dialog's own close button can never sit underneath the window close button.
    assert.equal(await coversCenterOf(native.locator('.window-close')), true);
    await native.evaluate(() => {
      const backdrop = document.createElement('div');
      backdrop.className = 'install-dialog-backdrop';
      document.body.append(backdrop);
    });
    assert.equal(await coversCenterOf(native.locator('.window-close')), false,
      'modal backdrops sit above the titlebar');

    for (const os of ['macos', 'linux']) {
      const other = await newPage({ userAgent: USER_AGENTS[os] });
      await other.goto(`${base}/tests/fixtures/windows-titlebar.html`);
      await other.getByText('Window content', { exact: true }).waitFor();
      assert.equal(await other.locator('.window-titlebar').count(), 0, `${os} keeps native decorations`);
    }
    assert.deepEqual(errors, []);
    console.log('PASS: preview, layout, first render, native IPC, state events, error feedback, modal layering, non-Windows exclusion, themes and compact layout.');
  } finally { await browser?.close(); await server.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
