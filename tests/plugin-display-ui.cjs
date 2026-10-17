const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const server = await createServer({ configFile: false, root: path.resolve(__dirname, '..'), plugins: [react()], logLevel: 'error', cacheDir: 'node_modules/.vite-plugin-display', server: { host: '127.0.0.1', port: 0, watch: null } });
  let browser;
  try {
    await server.listen();
    browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--no-proxy-server'] });
    const page = await browser.newPage();
    const base = `http://127.0.0.1:${server.httpServer.address().port}`;
    const out = process.env.PLUGIN_SCREENSHOT_DIR;
    if (out) fs.mkdirSync(out, { recursive: true });
    const capture = async name => {
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), true, name + ' horizontal overflow');
      const dialog = page.getByRole('dialog');
      if (await dialog.count()) {
        assert.equal(await dialog.evaluate(el => { const b = el.getBoundingClientRect(); return b.top >= 0 && b.bottom <= innerHeight + 1 && el.scrollWidth <= el.clientWidth + 1; }), true, name + ' dialog overflow');
      }
      if (out) await page.screenshot({ path: path.join(out, name + '.png'), fullPage: true });
    };
    for (const [width, theme, locale] of [[1280, 'light', 'en'], [375, 'dark', 'zh-CN']]) {
      await page.setViewportSize({ width, height: 800 });
      const query = `theme=${theme}&locale=${locale}`;
      const suffix = `${width}-${theme}`;
      await page.goto(`${base}/tests/fixtures/plugins.html?${query}&external-repository`);
      await page.locator('.plugin-card').first().waitFor();
      await capture('local-' + suffix);
      await page.locator('.plugin-tabs button').nth(1).click();
      await page.locator('.plugin-card').filter({ hasText: 'Community analytics' }).waitFor();
      await capture('store-' + suffix);
      await page.locator('.plugin-card').filter({ hasText: 'Sample provider' }).getByRole('button').click();
      await page.getByRole('dialog').waitFor();
      await capture('install-' + suffix);
      await page.keyboard.press('Escape');
      await page.locator('.plugin-tabs button').nth(2).click();
      await page.locator('.plugin-settings textarea').first().waitFor();
      await capture('settings-' + suffix);
      for (const [fixture, button] of [['plugin-config', 'Open configuration'], ['plugin-oauth', 'Open plugin login']]) {
        await page.goto(`${base}/tests/fixtures/${fixture}.html?${query}`);
        await page.getByRole('button', { name: button }).click();
        await page.getByRole('dialog').locator('input, textarea').first().waitFor();
        await capture(fixture + '-' + suffix);
      }
      await page.goto(`${base}/tests/fixtures/plugin-oauth-providers.html?${query}`);
      await page.getByText('ZCode OAuth', { exact: true }).waitFor();
      await capture('providers-' + suffix);
    }
    console.log('Plugin display checks passed: all management pages, config/install/OAuth dialogs and OAuth provider cards, desktop light and narrow dark layouts.');
  } finally { await browser?.close(); await server.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
