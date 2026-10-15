const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const server = await createServer({
    configFile: false, root: path.resolve(__dirname, '..'), plugins: [react()], logLevel: 'error',
    cacheDir: 'node_modules/.vite-plugin-pins-ui',
    optimizeDeps: { entries: ['index.html'], include: ['react', 'react-dom/client', 'react/jsx-runtime', 'lucide-react'] },
    server: { host: '127.0.0.1', port: 1457, strictPort: false },
  });
  let browser;
  try {
    await server.listen();
    const base = `http://127.0.0.1:${server.httpServer.address().port}/?mock=running`;
    browser = await chromium.launch({ headless: true,
      channel: process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined),
      args: ['--no-proxy-server'],
    });
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const capture = async name => {
      if (!process.env.PLUGIN_SCREENSHOT_DIR) return;
      fs.mkdirSync(process.env.PLUGIN_SCREENSHOT_DIR, { recursive: true });
      await page.screenshot({ path: path.join(process.env.PLUGIN_SCREENSHOT_DIR, name), fullPage: true });
    };
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => { if (window === window.top) localStorage.setItem('easy-cli-proxy-api.locale', 'en'); });
    const nav = () => page.locator('.nav-section');
    const shortcut = () => nav().getByRole('button', { name: 'Inspector status', exact: true });
    const pin = () => page.getByRole('checkbox', { name: 'Pin to sidebar: Inspector status' });
    const card = () => page.locator('.plugin-card').filter({ has: page.getByRole('heading', { name: 'Request Inspector', exact: true }) });
    await page.goto(base);
    await nav().getByRole('button', { name: 'Plugins', exact: true }).click();
    await pin().waitFor();
    assert.equal(await pin().isChecked(), false);
    assert.equal(await shortcut().count(), 0);
    await capture('pin-off.png');
    await pin().check();
    await shortcut().waitFor();
    await capture('pin-on.png');
    await shortcut().click();
    await page.frameLocator('iframe').getByRole('heading', { name: 'Request Inspector' }).waitFor();
    assert.equal(await shortcut().getAttribute('aria-current'), 'page');
    await capture('pinned-page.png');
    assert.equal(await nav().getByRole('button', { name: 'Plugins', exact: true }).getAttribute('aria-current'), null);
    assert.equal(await page.locator('iframe').getAttribute('sandbox'), 'allow-scripts allow-same-origin allow-forms allow-popups');
    await page.getByRole('button', { name: 'Open in browser', exact: true }).click();
    await page.reload();
    await shortcut().waitFor();
    await nav().getByRole('button', { name: 'Plugins', exact: true }).click();
    await pin().waitFor();
    assert.equal(await pin().isChecked(), true);
    await card().getByRole('button', { name: 'Disable', exact: true }).click();
    await shortcut().waitFor({ state: 'detached' });
    assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('cpa-gui-plugin-pins-v1')).length), 1);
    await card().getByRole('button', { name: 'Enable', exact: true }).click();
    await shortcut().waitFor();
    await page.getByRole('checkbox', { name: 'Enable plugins', exact: true }).uncheck();
    await shortcut().waitFor({ state: 'detached' });
    await page.getByRole('checkbox', { name: 'Enable plugins', exact: true }).check();
    await shortcut().waitFor();
    await pin().uncheck();
    await shortcut().waitFor({ state: 'detached' });
    await pin().check();
    await shortcut().waitFor();
    for (const width of [1280, 820]) {
      await page.setViewportSize({ width, height: 800 });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await shortcut().focus();
      await page.keyboard.press('Enter');
      await page.frameLocator('iframe').getByRole('heading', { name: 'Request Inspector' }).waitFor();
      await nav().getByRole('button', { name: 'Plugins', exact: true }).click();
    }
    await card().getByRole('button', { name: 'Uninstall Request Inspector', exact: true }).click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Uninstall', exact: true }).click();
    await shortcut().waitFor({ state: 'detached' });
    assert.deepEqual(await page.evaluate(() => JSON.parse(localStorage.getItem('cpa-gui-plugin-pins-v1'))), []);
    assert.deepEqual(errors, []);
    console.log('PASS: default-off pins, sidebar navigation, iframe restrictions, external open, reload persistence, disable/re-enable, global toggle, unpin, keyboard navigation, responsive layout, uninstall cleanup');
  } finally {
    await browser?.close();
    await server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
