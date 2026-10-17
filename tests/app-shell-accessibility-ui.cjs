const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');

(async () => {
  let server;
  let base = 'http://127.0.0.1:1420/';
  try {
    const response = await fetch(base);
    if (!response.ok) throw new Error(String(response.status));
  } catch {
    const { createServer } = await import('vite');
    const react = (await import('@vitejs/plugin-react')).default;
    const root = path.resolve(__dirname, '..');
    server = await createServer({ configFile: false, root, plugins: [react()], logLevel: 'error', server: { host: '127.0.0.1', port: 1421, strictPort: false } });
    await server.listen();
    const address = server.httpServer.address();
    base = `http://127.0.0.1:${typeof address === 'object' && address ? address.port : 1421}/`;
  }
  let browser;
  try {
    const channel = process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined);
    browser = await chromium.launch(channel ? { channel, headless: true, args: ['--no-proxy-server'] } : { headless: true, args: ['--no-proxy-server'] });
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await page.goto(`${base}?mock=running`, { waitUntil: 'commit' });
    await page.locator('.app-shell').waitFor();

    assert.equal(await page.getByRole('heading', { level: 1, name: '首页' }).count(), 0);

    await page.locator('.nav-section').getByRole('button', { name: '使用记录' }).click();
    const tabs = page.getByRole('tab');
    assert.equal(await tabs.count(), 5);
    assert.deepEqual(await tabs.evaluateAll((elements) => elements
      .map((element) => element.getAttribute('aria-controls'))
      .filter((id) => !id || !document.getElementById(id))), []);
    await tabs.first().focus();
    await page.keyboard.press('ArrowRight');
    assert.equal(await tabs.nth(1).getAttribute('aria-selected'), 'true');
    await page.getByRole('tab', { name: /请求明细/ }).click();
    await page.locator('.usage-events-table tbody tr').first().waitFor();
    const usageSurfaces = await page.locator('.usage-records-page .usage-events-table tbody tr').first().evaluate(row => {
      const cell = row.querySelector('td');
      const page = row.closest('.usage-records-page');
      const status = page?.querySelector('.usage-collector-state');
      return { row: cell ? getComputedStyle(cell).backgroundColor : '', status: status ? getComputedStyle(status).backgroundColor : '' };
    });
    assert.equal(usageSurfaces.row, 'rgb(255, 255, 255)', 'light usage rows stay pure white');
    assert.equal(usageSurfaces.status, 'rgb(255, 255, 255)', 'light usage status stays pure white');
    const shellSurfaces = await page.evaluate(() => {
      const shell = document.querySelector('.app-shell');
      const content = document.querySelector('.content');
      return {
        shell: shell ? getComputedStyle(shell).backgroundColor : '',
        content: content ? getComputedStyle(content).backgroundColor : '',
      };
    });
    assert.equal(shellSurfaces.shell, 'rgb(255, 255, 255)', 'light app shell stays pure white');
    assert.equal(shellSurfaces.content, 'rgb(255, 255, 255)', 'light content stays pure white');
    await page.getByRole('tab', { name: /数据管理/ }).click();
    assert.equal(await page.locator('.usage-filter-panel').count(), 0);

    await page.locator('.nav-section').getByRole('button', { name: '高级功能' }).click();
    assert.equal(await page.locator('.config-page').getAttribute('aria-label'), '高级设置');
    const configTabs = page.locator('.config-subpage-tabs').getByRole('tab');
    assert.equal(await page.locator('.config-subpage-tabs').getAttribute('aria-orientation'), 'horizontal');
    assert.equal(await page.locator('.config-settings-sidebar').count(), 0);
    assert.equal(await page.locator('.config-settings-header p, .config-nav-save-hint, .config-category-heading, .config-section-index').count(), 0);
    assert.deepEqual(await configTabs.evaluateAll(elements => elements.map(element => element.id)), [
      'config-subpage-tab-general', 'config-subpage-tab-aliases', 'config-subpage-tab-routing', 'config-subpage-tab-requests',
      'config-subpage-tab-oauth', 'config-subpage-tab-diagnostics',
      'config-subpage-tab-software',
    ]);
    assert.equal(await page.locator('.config-page [role="tabpanel"]').count(), 1);
    assert.deepEqual(await configTabs.evaluateAll((elements) => elements
      .map((element) => element.getAttribute('aria-controls'))
      .filter((id) => !id || !document.getElementById(id))), []);
    assert.equal(await configTabs.evaluateAll(elements => elements.every(element =>
      element.getAttribute('aria-controls') === 'config-subpage-panel')), true);
    await configTabs.first().focus();
    await page.keyboard.press('ArrowRight');
    assert.equal(await page.locator('#config-subpage-tab-aliases').getAttribute('aria-selected'), 'true');
    await page.keyboard.press('ArrowLeft');
    assert.equal(await configTabs.first().getAttribute('aria-selected'), 'true');
    await page.keyboard.press('End');
    assert.equal(await page.locator('#config-subpage-tab-software').getAttribute('aria-selected'), 'true');
    await page.waitForFunction(() => document.activeElement?.id === 'config-subpage-tab-software');
    await page.keyboard.press('Home');
    assert.equal(await configTabs.first().getAttribute('aria-selected'), 'true');
    await page.waitForFunction(() => document.activeElement?.id === 'config-subpage-tab-general');
    for (const key of ['ArrowDown', 'ArrowUp']) {
      assert.equal(await configTabs.first().evaluate((element, pressedKey) => {
        const event = new KeyboardEvent('keydown', { key: pressedKey, bubbles: true, cancelable: true });
        element.dispatchEvent(event);
        return event.defaultPrevented;
      }, key), false, `${key} must remain available for normal page scrolling`);
      assert.equal(await configTabs.first().getAttribute('aria-selected'), 'true');
    }
    assert.equal(await page.getByLabel('搜索设置', { exact: true }).count(), 1);
    assert.equal(await page.locator('.config-settings-views').count(), 0);
    const fieldHelp = page.locator('#template-field-management-0 .settings-help-trigger').first();
    assert.equal(await fieldHelp.getAttribute('aria-expanded'), 'false');
    assert.ok(await fieldHelp.getAttribute('aria-controls'));
    assert.ok(await fieldHelp.getAttribute('aria-label'), 'Help icons need a meaningful accessible name');
    await fieldHelp.focus();
    await page.keyboard.press('Enter');
    assert.equal(await fieldHelp.getAttribute('aria-expanded'), 'true');
    await page.keyboard.press('Escape');
    assert.equal(await fieldHelp.getAttribute('aria-expanded'), 'false');
    const addKey = page.getByRole('button', { name: /新增鉴权密钥/ }).first();
    await addKey.focus();
    await addKey.click();
    const dialog = page.getByRole('dialog');
    await dialog.waitFor();
    for (let index = 0; index < 8; index += 1) {
      await page.keyboard.press('Tab');
      assert.equal(await page.evaluate(() => Boolean(document.activeElement?.closest('[role="dialog"]'))), true);
    }
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === '新增鉴权密钥');
    assert.equal(await addKey.evaluate((element) => document.activeElement === element), true);

    await page.setViewportSize({ width: 640, height: 600 });
    await page.locator('.nav-section').getByRole('button', { name: '首页' }).click();
    const sidebarHeight = await page.locator('.sidebar').evaluate((element) => element.getBoundingClientRect().height);
    assert.ok(sidebarHeight < 150, `compact sidebar is too tall: ${sidebarHeight}px`);

    await page.locator('.sidebar-easy-entry').click();
    const steps = page.locator('.simple-mode-step-status-item');
    assert.equal(await steps.count(), 2);
    assert.equal(await steps.first().evaluate((element) => element.tagName), 'BUTTON');

    console.log('PASS: page hierarchy, tabs, contextual filters, modal focus and compact shell.');
  } finally {
    if (browser) await browser.close();
    if (server) await server.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
