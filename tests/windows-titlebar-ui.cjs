const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs/promises');

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
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.addInitScript(() => localStorage.setItem('easy-cli-proxy-api.locale', 'en'));
    await page.goto(`${base}/?mock=running&mockDelay=0`);
    await page.locator('.app-shell').waitFor();
    assert.equal(await page.locator('.window-titlebar').count(), 0, 'ordinary browser is unchanged');
    await page.goto(`${base}/?mock=running&mockDelay=0&windowPreview=windows`);
    await page.locator('.window-titlebar').waitFor();
    assert.equal(await page.locator('.window-caption, .window-preview-label').count(), 0);
    const sidebarBounds = await page.locator('.sidebar').boundingBox();
    const titlebarBounds = await page.locator('.window-titlebar').boundingBox();
    assert.equal(sidebarBounds.y, 0, 'sidebar starts at the top edge');
    assert.equal(sidebarBounds.height, 800, 'sidebar fills the entire window height');
    assert.equal(titlebarBounds.x, sidebarBounds.width, 'titlebar only occupies the workspace');
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
    await page.locator('.sidebar-theme-selector button').nth(0).click();
    await page.waitForTimeout(600);
    await fs.mkdir('bin-work', { recursive: true });
    await page.screenshot({ path: 'bin-work/windows-immersive-light.png' });
    await page.locator('.sidebar-theme-selector button').nth(1).click();
    await page.waitForTimeout(300);
    await page.screenshot({ path: 'bin-work/windows-immersive-dark.png' });
    await page.locator('.sidebar-easy-entry').click();
    assert.equal(await page.locator('.window-titlebar').count(), 1, 'easy mode keeps controls');
    await page.setViewportSize({ width: 640, height: 600 });
    assert.equal(await page.locator('.window-close').isVisible(), true);
    const bounds = await page.locator('.window-close').boundingBox();
    assert.ok(bounds.x + bounds.width <= 640 && bounds.y === 0);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    await page.screenshot({ path: 'bin-work/windows-immersive-compact.png' });
    await page.goto(`${base}/tests/fixtures/windows-titlebar.html`);
    await page.locator('.window-titlebar').waitFor();
    assert.equal(await page.locator('[data-tauri-drag-region]').count(), 1);
    await page.getByRole('button', { name: 'Maximize', exact: true }).click();
    await page.getByRole('button', { name: 'Restore window', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Minimize', exact: true }).click();
    await page.locator('.window-close').click();
    const calls = await page.evaluate(() => window.titlebarTest.calls);
    for (const command of ['toggle_maximize', 'minimize', 'close']) {
      assert.ok(calls.includes(`plugin:window|${command}`), `${command} uses native API`);
    }
    await page.evaluate(() => {
      window.titlebarTest.maximized = false;
      window.titlebarTest.emit('tauri://resize', { width: 640, height: 600 });
      window.titlebarTest.emit('tauri://blur', null);
    });
    await page.getByRole('button', { name: 'Maximize', exact: true }).waitFor();
    await page.waitForFunction(() => document.querySelector('.immersive-window').dataset.focused === 'false');
    await page.evaluate(() => { window.titlebarTest.fail = true; });
    await page.getByRole('button', { name: 'Maximize', exact: true }).click();
    await page.getByRole('alert').waitFor();
    await page.evaluate(() => {
      const backdrop = document.createElement('div');
      backdrop.style.cssText = 'position:fixed;inset:0;z-index:10000;background:#0008';
      document.body.append(backdrop);
    });
    assert.equal(await page.locator('.window-close').evaluate(el => {
      const rect = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
    }), true, 'window controls remain usable above modal backdrops');
    await page.locator('.window-close').click();
    for (const os of ['macos', 'linux']) {
      await page.goto(`${base}/tests/fixtures/windows-titlebar.html?os=${os}`);
      await page.getByText('Window content', { exact: true }).waitFor();
      await page.waitForTimeout(100);
      assert.equal(await page.locator('.window-titlebar').count(), 0, `${os} keeps native decorations`);
    }
    assert.deepEqual(errors, []);
    console.log('PASS: preview, native IPC, state events, error feedback, non-Windows exclusion, themes and compact layout.');
  } finally { await browser?.close(); await server.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
