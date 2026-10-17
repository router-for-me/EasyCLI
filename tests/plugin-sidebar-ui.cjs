const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const root = path.resolve(__dirname, '..');
  const server = await createServer({ configFile: false, root, cacheDir: 'node_modules/.vite-plugin-sidebar-ui', optimizeDeps: { noDiscovery: true, include: ['react', 'react-dom/client', 'react/jsx-runtime', 'lucide-react', '@dnd-kit/core', '@dnd-kit/sortable', '@dnd-kit/utilities', 'react-markdown'] }, plugins: [react()], logLevel: 'error', server: { host: '127.0.0.1', port: 0, strictPort: false, watch: null } });
  await server.listen();
  const address = server.httpServer.address();
  const base = `http://127.0.0.1:${typeof address === 'object' && address ? address.port : 1421}/`;
  let browser;
  try {
    const channel = process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined);
    browser = await chromium.launch(channel ? { channel, headless: true, args: ['--no-proxy-server'] } : { headless: true, args: ['--no-proxy-server'] });
    const errors = [];
    const open = async (query, size) => {
      const context = await browser.newContext({ viewport: size, locale: 'zh-CN' });
      const page = await context.newPage();
      page.on('pageerror', (error) => errors.push(error.message));
      await page.goto(`${base}?${query}`, { waitUntil: 'commit' });
      await page.locator('.app-shell').waitFor();
      return page;
    };
    const section = (page) => page.locator('.nav-section');
    const parent = (page) => section(page).locator('.nav-plugin-parent > button:not(.nav-plugin-toggle, .nav-plugin-drag)');
    const toggle = (page) => section(page).locator('button.nav-plugin-toggle');
    const children = (page) => section(page).locator('#nav-plugin-children button.nav-plugin-entry');
    const settle = (page, value) => page.waitForFunction((expected) =>
      document.querySelector('.nav-section')?.getAttribute('data-plugin-layout') === expected, value);
    const HINT = '请先在首页启动内核服务';
    const resize = async (page, size) => {
      await page.setViewportSize(size);
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    };

    const page = await open('mock=running', { width: 1280, height: 900 });
    await children(page).first().waitFor();
    await settle(page, 'expanded');
    assert.equal(await children(page).count(), 2);
    assert.equal(await toggle(page).getAttribute('aria-expanded'), 'true');
    assert.equal(await toggle(page).getAttribute('aria-controls'), 'nav-plugin-children');
    assert.equal(await toggle(page).getAttribute('aria-label'), '插件页面');
    assert.equal(await page.locator('div#nav-plugin-children.nav-plugin-children').getAttribute('role'), 'group');
    assert.equal(await children(page).nth(0).getAttribute('data-plugin-route'), '/plugin-pages/request-inspector/0');
    assert.equal(await children(page).nth(1).getAttribute('data-plugin-route'), '/plugin-pages/request-inspector/1');
    const order = await section(page).locator('button:not(.nav-plugin-toggle, .nav-plugin-drag)').evaluateAll((buttons) => buttons.map((button) => button.textContent.trim()));
    assert.deepEqual(order.slice(-4), ['版本管理', '插件', 'Inspector status', 'Sampled requests'], order.join('|'));

    const originalRoutes = await children(page).evaluateAll(rows => rows.map(row => row.dataset.pluginRoute));
    const handle = page.locator('.nav-plugin-drag').first();
    const start = await handle.boundingBox();
    const target = await page.locator('.nav-plugin-drag').nth(1).boundingBox();
    await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
    const scrollBefore = await section(page).evaluate(el => ({ width: el.scrollWidth, height: el.scrollHeight }));
    await page.mouse.down();
    await page.mouse.move(start.x + 100, start.y + 140, { steps: 8 });
    assert.deepEqual(await section(page).evaluate(el => ({ width: el.scrollWidth, height: el.scrollHeight })), scrollBefore, 'Dragging outside the list must not create scrollbars');

    await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 12 });
    await page.mouse.up();
    await page.waitForFunction(route => document.querySelector('.nav-plugin-entry')?.getAttribute('data-plugin-route') === route, originalRoutes[1]);
    assert.equal(await page.locator('section.plugin-resource-page').count(), 0, 'Reordering must not open a page');
    await page.reload();
    await children(page).first().waitFor();
    assert.equal(await children(page).first().getAttribute('data-plugin-route'), originalRoutes[1], 'Order survives reload and backend refresh');
    await page.locator('.nav-plugin-drag').first().focus();
    await page.keyboard.press('Space');
    await page.locator('.nav-plugin-row.dragging').waitFor();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.keyboard.press('ArrowDown');
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.keyboard.press('Space');
    await page.waitForFunction(route => document.querySelector('.nav-plugin-entry')?.getAttribute('data-plugin-route') === route, originalRoutes[0]);
    await children(page).first().click();
    const pluginPage = page.locator('section.plugin-resource-page');
    await pluginPage.waitFor();
    assert.equal(await pluginPage.locator('h1').textContent(), 'Inspector status');
    const placeholder = page.locator('[data-mock-plugin-webview]');
    await placeholder.waitFor();
    assert.equal(await placeholder.getAttribute('data-mock-url'), 'http://127.0.0.1:8317/v0/resource/plugins/request-inspector/status');
    assert.equal(await page.locator('.plugin-resource-host').evaluate((element) => Math.round(element.getBoundingClientRect().bottom)), 900, 'plugin page fills the content height');

    await resize(page, { width: 1280, height: 520 });
    await settle(page, 'collapsed');
    assert.equal(await children(page).count(), 0);
    assert.equal(await toggle(page).getAttribute('aria-expanded'), 'false');
    assert.equal(await parent(page).evaluate((element) => element.classList.contains('active')), true, 'collapsed parent shows the active child page');
    assert.equal(await pluginPage.locator('h1').textContent(), 'Inspector status');

    await toggle(page).click();
    await settle(page, 'expanded');
    assert.equal(await children(page).count(), 2);
    await resize(page, { width: 1280, height: 540 });
    await page.waitForTimeout(200);
    assert.equal(await children(page).count(), 2, 'manual expand holds while auto result is unchanged');
    await resize(page, { width: 1280, height: 900 });
    await settle(page, 'expanded');
    await resize(page, { width: 1280, height: 520 });
    await settle(page, 'collapsed');
    assert.equal(await children(page).count(), 0, 'crossing the threshold reverts to auto');

    await resize(page, { width: 1280, height: 900 });
    await settle(page, 'expanded');
    await toggle(page).click();
    await settle(page, 'collapsed');
    assert.equal(await children(page).count(), 0);
    await resize(page, { width: 1280, height: 880 });
    await page.waitForTimeout(200);
    assert.equal(await children(page).count(), 0, 'manual collapse holds while auto result is unchanged');
    await resize(page, { width: 1280, height: 520 });
    await settle(page, 'collapsed');
    await resize(page, { width: 1280, height: 900 });
    await settle(page, 'expanded');
    assert.equal(await children(page).count(), 2);

    await page.evaluate(() => {
      const internals = window.__TAURI_INTERNALS__;
      const original = internals.invoke;
      window.failPluginResource = true;
      internals.invoke = (command, args, options) => command === 'show_plugin_webview' && window.failPluginResource
        ? Promise.reject(new Error('Plugin page is unavailable'))
        : original(command, args, options);
    });
    await children(page).nth(1).click();
    const failure = pluginPage.getByRole('alert');
    await failure.getByText('Plugin page is unavailable').waitFor();
    assert.equal(await pluginPage.getByRole('status').count(), 0, 'a failed load does not keep showing the loading state');
    await page.evaluate(() => { window.failPluginResource = false; });
    await failure.getByRole('button', { name: '重试' }).click();
    await placeholder.waitFor();
    assert.equal(await pluginPage.getByRole('alert').count(), 0);
    assert.equal(await pluginPage.locator('h1').textContent(), 'Sampled requests');
    await parent(page).click();
    await page.getByRole('heading', { level: 1, name: '插件' }).waitFor();

    await section(page).getByRole('button', { name: '首页' }).click();
    await page.locator('.home-runtime-stop').click();
    await page.waitForFunction((hint) => document.querySelector(`.nav-section button[title="${hint}"]`), HINT);
    const quota = section(page).getByRole('button', { name: '额度查询' });
    assert.equal(await quota.isDisabled(), true);
    assert.equal(await parent(page).isDisabled(), true);
    assert.equal(await parent(page).evaluate((element) => element.classList.contains('locked')), true);
    assert.equal(await parent(page).getAttribute('title'), HINT);
    assert.equal(await toggle(page).isEnabled(), true);
    assert.equal(await children(page).count(), 2);
    for (const entry of await children(page).all()) {
      assert.equal(await entry.isDisabled(), true);
      assert.equal(await entry.evaluate((element) => element.classList.contains('locked')), true);
      assert.equal(await entry.locator('.nav-lock-icon').count(), 1);
      assert.equal(await entry.getAttribute('title'), HINT);
    }
    await toggle(page).click();
    await settle(page, 'collapsed');
    assert.equal(await children(page).count(), 0);
    await page.context().close();

    const many = await open('mock=running&mockPluginPages=12', { width: 1280, height: 900 });
    await toggle(many).waitFor();
    await settle(many, 'collapsed');
    assert.equal(await children(many).count(), 0);
    await toggle(many).click();
    await settle(many, 'expanded');
    assert.equal(await children(many).count(), 14);
    await many.context().close();

    const remount = await open('mock=running', { width: 1280, height: 520 });
    await toggle(remount).waitFor();
    await settle(remount, 'collapsed');
    await remount.locator('.sidebar-easy-entry').click();
    await remount.locator('.simple-mode-exit-btn').click();
    await toggle(remount).waitFor();
    await settle(remount, 'collapsed');
    await resize(remount, { width: 1280, height: 900 });
    await settle(remount, 'expanded');
    await resize(remount, { width: 1280, height: 520 });
    await settle(remount, 'collapsed');
    await remount.context().close();

    const compact = await open('mock=running', { width: 800, height: 400 });
    await children(compact).first().waitFor();
    assert.equal(await children(compact).count(), 2);
    assert.equal(await toggle(compact).isVisible(), false);
    await compact.context().close();

    assert.deepEqual(errors, []);
    console.log('PASS: plugin submenu auto and manual expansion, live resize, navigation, locked state and compact bar.');
  } finally {
    if (browser) await browser.close();
    await server.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
