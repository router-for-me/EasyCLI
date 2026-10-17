const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const root = path.resolve(__dirname, '..');
  const server = await createServer({ configFile: false, root, cacheDir: 'node_modules/.vite-plugin-webview-ui', optimizeDeps: { noDiscovery: true, include: ['react', 'react-dom/client', 'react/jsx-runtime', 'lucide-react', '@dnd-kit/core', '@dnd-kit/sortable', '@dnd-kit/utilities', 'react-markdown'] }, plugins: [react()], logLevel: 'error', server: { host: '127.0.0.1', port: 0, strictPort: false, watch: null } });
  await server.listen();
  const address = server.httpServer.address();
  const base = `http://127.0.0.1:${typeof address === 'object' && address ? address.port : 1421}/`;
  let browser;
  try {
    const channel = process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined);
    browser = await chromium.launch(channel ? { channel, headless: true, args: ['--no-proxy-server'] } : { headless: true, args: ['--no-proxy-server'] });
    const errors = [];
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: 'zh-CN' });
    const page = await context.newPage();
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto(`${base}?mock=running`, { waitUntil: 'commit' });
    await page.locator('.app-shell').waitFor();

    const hole = page.locator('[data-mock-plugin-webview]');
    const pluginPage = page.locator('section.plugin-resource-page');
    const counter = (name) => hole.evaluate((element, key) => Number(element.dataset[key]), name);
    const aligned = () => page.waitForFunction(() => {
      const placeholder = document.querySelector('[data-mock-plugin-webview]');
      const host = document.querySelector('.plugin-resource-host');
      if (!placeholder || !host || placeholder.hidden) return false;
      const a = placeholder.getBoundingClientRect();
      const b = host.getBoundingClientRect();
      return [a.left - b.left, a.top - b.top, a.width - b.width, a.height - b.height].every((delta) => Math.abs(delta) <= 1);
    });
    const hidden = () => page.waitForFunction(() => document.querySelector('[data-mock-plugin-webview]')?.hidden === true);
    const resize = async (size) => {
      await page.setViewportSize(size);
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    };
    const entry = (index) => page.locator('#nav-plugin-children button.nav-plugin-entry').nth(index);

    await entry(0).click();
    await pluginPage.waitFor();
    await aligned();
    const content = await page.locator('main.content').boundingBox();
    const placed = await hole.boundingBox();
    assert.deepEqual([placed.x, placed.y, placed.width, placed.height].map(Math.round), [content.x, content.y, content.width, content.height].map(Math.round), 'the native view covers the whole content area');
    assert.equal(await pluginPage.locator('.plugin-resource-bar').count(), 0, 'no HTML toolbar is rendered');
    assert.deepEqual(JSON.parse(await hole.getAttribute('data-mock-labels')), { back: '后退', reload: '重新加载', openExternal: '在浏览器打开', move: '移动按钮' });
    assert.match(await hole.innerText(), /injected into the page/);
    assert.equal(await hole.getAttribute('data-mock-url'), 'http://127.0.0.1:8317/v0/resource/plugins/request-inspector/status');
    assert.match(await hole.innerText(), /browser preview/);

    for (const [label, open, close] of [
      ['language menu', () => page.locator('.sidebar-language-trigger').click(), () => page.keyboard.press('Escape')],
      ['theme menu', () => page.locator('.theme-selector-trigger').click(), () => page.keyboard.press('Escape')],
      ['confirmation dialog',
        () => page.evaluate(() => { const dialog = document.createElement('div'); dialog.setAttribute('role', 'alertdialog'); dialog.id = 'fixture-dialog'; dialog.textContent = 'Confirm'; document.body.append(dialog); }),
        () => page.evaluate(() => document.getElementById('fixture-dialog').remove())],
    ]) {
      await open();
      await hidden();
      await close();
      await aligned();
      console.log(`overlay ok: ${label}`);
    }

    await resize({ width: 1280, height: 520 });
    await aligned();
    await resize({ width: 1280, height: 900 });
    await aligned();
    await resize({ width: 1000, height: 800 });
    await aligned();
    const wide = await hole.boundingBox();
    await resize({ width: 800, height: 700 });
    await aligned();
    const narrow = await hole.boundingBox();
    assert.equal(narrow.x, 0, 'the top-bar layout uses the full width');
    assert.equal(narrow.y > wide.y, true, 'the top bar pushes the native view down');
    const sidebar = await page.locator('aside.sidebar').boundingBox();
    assert.equal(Math.abs(narrow.y - (sidebar.y + sidebar.height)) <= 2, true, 'the host starts right below the top bar');
    assert.equal(Math.abs(narrow.y + narrow.height - 700) <= 2, true, 'the host fills the rest of the window');
    await page.locator('.nav-section').getByRole('button', { name: '首页' }).click();
    await hidden();
    const homeSidebar = await page.locator('aside.sidebar').boundingBox();
    assert.equal(Math.abs(sidebar.height - homeSidebar.height) <= 2, true, `top bar height on a plugin page (${sidebar.height}) matches Home (${homeSidebar.height})`);
    await entry(0).click();
    await aligned();
    await resize({ width: 1280, height: 900 });
    await aligned();

    await entry(0).click();
    await page.waitForFunction(() => document.querySelector('[data-mock-plugin-webview]')?.dataset.mockReloads === '1');
    await aligned();
    assert.equal(await counter('mockReloads'), 1);

    await page.evaluate(() => {
      const internals = window.__TAURI_INTERNALS__;
      const invoke = internals.invoke;
      window.__showGate = null;
      window.__failNextShow = false;
      internals.invoke = (command, args, options) => {
        if (command !== 'show_plugin_webview') return invoke(command, args, options);
        if (window.__failNextShow) {
          window.__failNextShow = false;
          return invoke(command, { ...args, menuIndex: 99 }, options);
        }
        const gate = window.__showGate;
        const result = invoke(command, args, options);
        return gate ? result.then((value) => gate.then(() => value)) : result;
      };
    });
    await page.evaluate(() => { window.__showGate = new Promise((resolve) => { window.__openShowGate = resolve; }); });
    await entry(0).click();
    await page.locator('.plugin-resource-status').waitFor();
    await page.locator('.theme-selector-trigger').click();
    await hidden();
    await page.evaluate(() => { window.__showGate = null; window.__openShowGate(); });
    await page.locator('.plugin-resource-status').waitFor({ state: 'detached' });
    assert.equal(await hole.evaluate((element) => element.hidden), true, 'a slow reload that finishes under an overlay stays hidden');
    await page.keyboard.press('Escape');
    await aligned();
    console.log('overlay ok: slow reload');

    await page.evaluate(() => { window.__failNextShow = true; });
    await entry(0).click();
    await pluginPage.locator('.plugin-error').waitFor();
    await hidden();
    await pluginPage.getByRole('button', { name: '重试' }).click();
    await aligned();
    console.log('failed reload hides the previous page');

    await entry(1).click();
    await page.waitForFunction(() => document.querySelector('[data-mock-plugin-webview]')?.dataset.mockUrl?.endsWith('/samples'));
    await aligned();

    await page.locator('.nav-plugin-parent > button:not(.nav-plugin-toggle)').click();
    await page.getByRole('heading', { level: 1, name: '插件' }).waitFor();
    await hidden();
    await page.locator('.plugin-menus button', { hasText: 'Inspector status' }).first().click();
    await pluginPage.waitFor();
    assert.equal(await pluginPage.locator('h1').textContent(), 'Inspector status');
    await aligned();

    await page.locator('.nav-section').getByRole('button', { name: '首页' }).click();
    await page.locator('.home-runtime-stop').click();
    await page.locator('.nav-section button[title="请先在首页启动内核服务"]').first().waitFor();
    await hidden();

    assert.deepEqual(errors, []);
    console.log('PASS: plugin webview placeholder follows layout, hides under overlays, reloads and navigates from plugin cards.');
    await context.close();
  } finally {
    if (browser) await browser.close();
    await server.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
