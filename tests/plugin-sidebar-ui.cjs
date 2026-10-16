const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const root = path.resolve(__dirname, '..');
  const server = await createServer({ configFile: false, root, plugins: [react()], logLevel: 'error', server: { host: '127.0.0.1', port: 0, strictPort: false } });
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
    const parent = (page) => section(page).locator('.nav-plugin-parent > button:not(.nav-plugin-toggle, .nav-plugin-more)');
    const toggle = (page) => section(page).locator('button.nav-plugin-toggle');
    const children = (page) => section(page).locator('#nav-plugin-children button.nav-plugin-entry');
    const settle = (page, value) => page.waitForFunction((expected) =>
      document.querySelector('.nav-section')?.getAttribute('data-plugin-layout') === expected, value);
    const HINT = '请先在首页启动内核服务';
    const pageKeyOf = (sub) => `[data-plugin-nav-key="page:/v0/resource/plugins/request-inspector/${sub}"]`;
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
    const order = await section(page).locator('button:not(.nav-plugin-toggle, .nav-plugin-more)').evaluateAll((buttons) => buttons.map((button) => button.textContent.trim()));
    assert.deepEqual(order.slice(-4), ['版本管理', '插件', 'Inspector status', 'Sampled requests'], order.join('|'));

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

    const arranged = await open('mock=running&mockPluginPages=2', { width: 1280, height: 900 });
    const nav = section(arranged);
    const pageKey = (sub) => `[data-plugin-nav-key="page:/v0/resource/plugins/request-inspector/${sub}"]`;
    const pageRow = (sub) => nav.locator(pageKey(sub));
    const pluginsRow = nav.locator('[data-plugin-nav-key="plugins"]');
    const menu = arranged.getByRole('menu');
    const labels = () => nav.locator('button:not(.nav-plugin-toggle, .nav-plugin-more)').evaluateAll((buttons) => buttons.map((button) => button.textContent.trim()));
    const tail = async () => (await labels()).slice((await labels()).indexOf('版本管理') + 1);
    const savedLayout = () => arranged.evaluate(() => window.__TAURI_INTERNALS__.invoke('get_plugin_nav_layout'));
    await children(arranged).first().waitFor();

    await pageRow('status').locator('.nav-plugin-entry').click();
    const inspectorPlaceholder = arranged.locator('[data-mock-plugin-webview]');
    await inspectorPlaceholder.waitFor();
    await pluginsRow.locator('.nav-plugin-more').click();
    await menu.waitFor();
    await inspectorPlaceholder.waitFor({ state: 'hidden' });
    assert.equal(await arranged.evaluate(() => document.activeElement.textContent), '新建文件夹');
    await arranged.keyboard.press('ArrowDown');
    assert.equal(await arranged.evaluate(() => document.activeElement.textContent), '整理侧栏…');
    await arranged.keyboard.press('ArrowDown');
    assert.equal(await arranged.evaluate(() => document.activeElement.textContent), '恢复默认布局');
    await arranged.keyboard.press('Escape');
    await menu.waitFor({ state: 'detached' });
    await arranged.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === '「插件」的更多操作');
    await inspectorPlaceholder.waitFor();

    await arranged.keyboard.press('Enter');
    await arranged.keyboard.press('Enter');
    const renameInput = nav.locator('input.nav-plugin-rename');
    assert.equal(await renameInput.inputValue(), '新建文件夹');
    await renameInput.fill('Tools');
    await renameInput.press('Enter');
    const folder = nav.locator('.nav-plugin-folder');
    await folder.getByRole('button', { name: 'Tools', exact: true }).waitFor();
    await nav.getByText('空文件夹').waitFor();

    await pageRow('samples').click({ button: 'right' });
    await menu.getByRole('menuitem', { name: '移入文件夹' }).focus();
    await arranged.keyboard.press('ArrowRight');
    assert.equal(await arranged.evaluate(() => document.activeElement.textContent), 'Tools');
    await arranged.keyboard.press('Enter');
    await pageRow('extra-1').click({ button: 'right' });
    await menu.getByRole('menuitem', { name: '移到一级菜单' }).click();
    assert.deepEqual(await tail(), ['Tools', 'Sampled requests', 'Extra page 1', '插件', 'Inspector status', 'Extra page 2']);
    assert.equal(await nav.locator('#nav-plugin-folder-0 [data-plugin-route]').count(), 1);

    await pageRow('status').hover();
    await pageRow('status').locator('.nav-plugin-more').click();
    await menu.getByRole('menuitem', { name: '重命名' }).click();
    await renameInput.fill('Inspector');
    await renameInput.press('Enter');
    const status = pageRow('status').locator('.nav-plugin-entry');
    assert.equal(await status.textContent(), 'Inspector');
    assert.equal(await status.getAttribute('title'), 'Inspector status · Request Inspector');
    assert.equal(await arranged.locator('section.plugin-resource-page h1').textContent(), 'Inspector');
    await pageRow('status').click({ button: 'right' });
    await menu.getByRole('menuitem', { name: '重命名' }).click();
    await renameInput.fill('discarded');
    await renameInput.press('Escape');
    assert.equal(await status.textContent(), 'Inspector');
    await arranged.waitForFunction(() => window.__TAURI_INTERNALS__.invoke('get_plugin_nav_layout').then((layout) => layout?.aliases && Object.keys(layout.aliases).length === 1));
    assert.deepEqual(await savedLayout(), {
      version: 1,
      aliases: { '/v0/resource/plugins/request-inspector/status': 'Inspector' },
      custom: [
        { kind: 'folder', id: (await savedLayout()).custom[0].id, name: 'Tools', children: ['/v0/resource/plugins/request-inspector/samples'] },
        { kind: 'page', path: '/v0/resource/plugins/request-inspector/extra-1' },
      ],
    });

    await arranged.evaluate(() => window.dispatchEvent(new Event('plugin-resources-refresh')));
    await arranged.waitForTimeout(300);
    assert.deepEqual(await tail(), ['Tools', 'Sampled requests', 'Extra page 1', '插件', 'Inspector', 'Extra page 2']);

    await pageRow('samples').locator('.nav-plugin-entry').click();
    await folder.getByRole('button', { name: 'Tools', exact: true }).click();
    assert.equal(await pageRow('samples').count(), 0);
    assert.equal(await folder.getByRole('button', { name: 'Tools', exact: true }).evaluate((element) => element.classList.contains('active')), true);
    await folder.getByRole('button', { name: 'Tools', exact: true }).click();

    await arranged.evaluate(() => {
      const internals = window.__TAURI_INTERNALS__;
      const original = internals.invoke;
      internals.invoke = (command, args, options) => command === 'save_plugin_nav_layout'
        ? Promise.reject(new Error('disk full'))
        : original(command, args, options);
    });
    await pageRow('extra-2').click({ button: 'right' });
    await menu.getByRole('menuitem', { name: '移到一级菜单' }).click();
    await arranged.getByText('侧栏布局保存失败，已恢复到上次保存的状态').waitFor();
    assert.deepEqual(await tail(), ['Tools', 'Sampled requests', 'Extra page 1', '插件', 'Inspector', 'Extra page 2']);
    await arranged.context().close();

    const reset = await open('mock=running', { width: 1280, height: 900 });
    await children(reset).first().waitFor();
    await section(reset).locator(pageKeyOf('samples')).click({ button: 'right' });
    await reset.getByRole('menu').getByRole('menuitem', { name: '移入文件夹' }).click();
    await reset.getByRole('menuitem', { name: '新建文件夹…' }).click();
    await section(reset).locator('input.nav-plugin-rename').press('Enter');
    assert.equal(await section(reset).locator('#nav-plugin-folder-0 [data-plugin-route]').count(), 1);
    await section(reset).locator('.nav-plugin-folder').click({ button: 'right' });
    await reset.getByRole('menuitem', { name: '删除文件夹' }).click();
    assert.equal(await section(reset).locator('.nav-plugin-folder').count(), 0);
    assert.equal(await children(reset).count(), 2);
    await section(reset).locator(pageKeyOf('status')).click({ button: 'right' });
    await reset.getByRole('menuitem', { name: '移到一级菜单' }).click();
    await section(reset).locator('[data-plugin-nav-key="plugins"]').click({ button: 'right' });
    await reset.getByRole('menuitem', { name: '恢复默认布局' }).click();
    assert.equal(await children(reset).count(), 2);
    await reset.context().close();

    const unreadable = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: 'zh-CN' });
    await unreadable.addInitScript(() => {
      let inner;
      const internals = {};
      Object.defineProperty(internals, 'invoke', {
        configurable: true,
        get: () => (command, args, options) => command === 'get_plugin_nav_layout' ? Promise.resolve({ version: 99 }) : inner(command, args, options),
        set: (value) => { inner = value; },
      });
      window.__TAURI_INTERNALS__ = internals;
    });
    const broken = await unreadable.newPage();
    broken.on('pageerror', (error) => errors.push(error.message));
    await broken.goto(`${base}?mock=running`, { waitUntil: 'commit' });
    await children(broken).first().waitFor();
    assert.equal(await section(broken).locator('.nav-plugin-entry .nav-plugin-more, [data-plugin-nav-key^="page:"] .nav-plugin-more').count(), 0);
    await section(broken).locator('[data-plugin-nav-key="plugins"] .nav-plugin-more').click();
    const unreadableItem = broken.getByRole('menuitem', { name: '侧栏布局读取失败，暂时无法整理' });
    assert.equal(await unreadableItem.isDisabled(), true);
    await unreadable.close();

    const dragTo = async (page, source, target, ratio) => {
      const from = await source.boundingBox();
      await page.mouse.move(from.x + 24, from.y + from.height / 2);
      await page.mouse.down();
      await page.mouse.move(from.x + 24, from.y + from.height / 2 + 8, { steps: 3 });
      const to = await target.boundingBox();
      await page.mouse.move(to.x + 24, to.y + to.height * ratio, { steps: 12 });
      await page.waitForTimeout(50);
      await page.mouse.up();
      await page.waitForTimeout(80);
    };
    const organize = await open('mock=running&mockPluginPages=3', { width: 1280, height: 900 });
    const onav = section(organize);
    const rowOf = (sub) => onav.locator(pageKeyOf(sub));
    const entryOf = (sub) => rowOf(sub).locator('.nav-plugin-entry');
    const pluginsHead = onav.locator('[data-plugin-nav-key="plugins"]');
    const folderHead = onav.locator('.nav-plugin-folder');
    const organizing = () => onav.evaluate((element) => element.hasAttribute('data-organizing'));
    const otail = async () => {
      const all = await onav.locator('button:not(.nav-plugin-toggle, .nav-plugin-more)').evaluateAll((buttons) => buttons.map((button) => button.textContent.trim()));
      return all.slice(all.indexOf('版本管理') + 1);
    };
    const pages = ['Inspector status', 'Sampled requests', 'Extra page 1', 'Extra page 2', 'Extra page 3'];
    const bar = ['新建文件夹', '完成'];
    await children(organize).first().waitFor();
    await pluginsHead.locator('.nav-plugin-more').click();
    await organize.getByRole('menuitem', { name: '整理侧栏…' }).click();
    await organize.waitForFunction(() => document.querySelector('.nav-section')?.hasAttribute('data-organizing'));
    await organize.waitForFunction(() => document.activeElement?.getAttribute('data-plugin-route') === '/plugin-pages/request-inspector/0');
    assert.equal(await organize.evaluate(() => document.activeElement.getAttribute('aria-roledescription')), '可拖动');
    assert.equal(await onav.getByRole('button', { name: '首页' }).isDisabled(), true);
    assert.equal(await onav.getByRole('button', { name: '版本管理' }).isDisabled(), true);
    assert.equal(await onav.locator('.nav-plugin-more, .nav-plugin-toggle').count(), 0);
    assert.equal(await onav.locator('.nav-plugin-grip').count(), 5);
    assert.deepEqual(await otail(), ['插件', ...pages, ...bar]);
    await entryOf('samples').click();
    assert.equal(await organize.locator('section.plugin-resource-page').count(), 0, 'rows do not navigate while organizing');

    await onav.locator('.nav-plugin-organize-new').click();
    await onav.locator('input.nav-plugin-rename').fill('Tools');
    await onav.locator('input.nav-plugin-rename').press('Enter');
    const emptyRow = onav.locator('[data-plugin-nav-key^="empty:"]');
    assert.equal(await emptyRow.textContent(), '拖入页面');
    await dragTo(organize, entryOf('samples'), emptyRow, 0.5);
    assert.deepEqual(await otail(), ['Tools', 'Sampled requests', '插件', 'Inspector status', 'Extra page 1', 'Extra page 2', 'Extra page 3', ...bar]);
    await dragTo(organize, entryOf('extra-1'), folderHead, 0.1);
    assert.deepEqual(await otail(), ['Extra page 1', 'Tools', 'Sampled requests', '插件', 'Inspector status', 'Extra page 2', 'Extra page 3', ...bar]);
    await dragTo(organize, entryOf('status'), rowOf('samples'), 0.8);
    assert.deepEqual(await otail(), ['Extra page 1', 'Tools', 'Sampled requests', 'Inspector status', '插件', 'Extra page 2', 'Extra page 3', ...bar]);

    const head = folderHead.locator('button').first();
    const from = await head.boundingBox();
    await organize.mouse.move(from.x + 24, from.y + from.height / 2);
    await organize.mouse.down();
    await organize.mouse.move(from.x + 24, from.y + from.height / 2 - 8, { steps: 3 });
    assert.equal(await onav.locator('[id^="nav-plugin-folder-"]').count(), 0, 'folders collapse while a folder is dragged');
    const extraBox = await rowOf('extra-1').boundingBox();
    await organize.mouse.move(from.x + 24, extraBox.y + extraBox.height * 0.2, { steps: 12 });
    assert.equal(await rowOf('extra-1').getAttribute('data-drop'), 'before');
    await organize.mouse.up();
    await organize.waitForTimeout(80);
    assert.deepEqual(await otail(), ['Tools', 'Sampled requests', 'Inspector status', 'Extra page 1', '插件', 'Extra page 2', 'Extra page 3', ...bar]);
    await dragTo(organize, folderHead.locator('button').first(), rowOf('extra-2'), 0.5);
    assert.deepEqual(await otail(), ['Tools', 'Sampled requests', 'Inspector status', 'Extra page 1', '插件', 'Extra page 2', 'Extra page 3', ...bar], 'a folder cannot enter the plugin group');
    await dragTo(organize, entryOf('samples'), pluginsHead, 0.8);
    assert.deepEqual(await otail(), ['Tools', 'Inspector status', 'Extra page 1', '插件', 'Sampled requests', 'Extra page 2', 'Extra page 3', ...bar]);

    const press = async (key) => {
      await organize.keyboard.press(key);
      await organize.waitForTimeout(50);
    };
    const pickUp = () => press('Space');
    await entryOf('extra-3').focus();
    await pickUp();
    for (let step = 0; step < 3; step += 1) await press('ArrowUp');
    assert.equal(await pluginsHead.getAttribute('data-drop'), 'before');
    await press('Space');
    assert.deepEqual(await otail(), ['Tools', 'Inspector status', 'Extra page 1', 'Extra page 3', '插件', 'Sampled requests', 'Extra page 2', ...bar]);
    await organize.waitForFunction(() => document.activeElement?.getAttribute('data-plugin-route') === '/plugin-pages/request-inspector/4');
    await pickUp();
    await press('ArrowUp');
    await press('ArrowUp');
    assert.equal(await rowOf('status').getAttribute('data-drop'), 'before');
    await press('Escape');
    assert.equal(await onav.locator('[data-drop]').count(), 0);
    assert.deepEqual(await otail(), ['Tools', 'Inspector status', 'Extra page 1', 'Extra page 3', '插件', 'Sampled requests', 'Extra page 2', ...bar], 'Escape cancels a keyboard move');
    const organized = await organize.evaluate(() => window.__TAURI_INTERNALS__.invoke('get_plugin_nav_layout'));
    assert.deepEqual(organized.custom.map((node) => node.kind === 'folder' ? `${node.name}:${node.children.map((child) => child.split('/').pop())}` : node.path.split('/').pop()),
      ['Tools:status', 'extra-1', 'extra-3']);

    await organize.waitForTimeout(80);
    await onav.locator('.nav-plugin-organize-done').click();
    assert.equal(await organizing(), false);
    await organize.waitForFunction(() => document.activeElement?.textContent === '插件');
    assert.equal(await onav.getByRole('button', { name: '首页' }).isDisabled(), false);
    assert.equal(await onav.locator('.nav-plugin-organize-bar').count(), 0);
    await entryOf('samples').click();
    await organize.locator('section.plugin-resource-page').waitFor();

    await pluginsHead.locator('button').first().click();
    await organize.getByRole('button', { name: '插件设置' }).click();
    const settingsEntry = organize.locator('.plugin-nav-organize');
    assert.equal(await settingsEntry.locator('h2').textContent(), '侧栏中的插件页面');
    await settingsEntry.getByRole('button', { name: '整理侧栏…' }).click();
    await organize.waitForFunction(() => document.querySelector('.nav-section')?.hasAttribute('data-organizing'));
    await onav.locator('.nav-plugin-organize-done').click();
    assert.equal(await organizing(), false);
    await organize.context().close();

    const autoContext = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: 'zh-CN' });
    await autoContext.addInitScript(() => {
      let inner;
      let layout = {
        version: 1,
        aliases: {},
        custom: [{ kind: 'folder', id: 'tools', name: 'Tools', children: ['samples', 'extra-1'].map((sub) => `/v0/resource/plugins/request-inspector/${sub}`) }],
      };
      const internals = {};
      Object.defineProperty(internals, 'invoke', {
        configurable: true,
        get: () => (command, args, options) => command === 'get_plugin_nav_layout' ? Promise.resolve(layout)
          : command === 'save_plugin_nav_layout' ? Promise.resolve(void (layout = args.layout))
            : inner(command, args, options),
        set: (value) => { inner = value; },
      });
      window.__TAURI_INTERNALS__ = internals;
    });
    const auto = await autoContext.newPage();
    auto.on('pageerror', (error) => errors.push(error.message));
    await auto.goto(`${base}?mock=running&mockPluginPages=3`, { waitUntil: 'commit' });
    const anav = section(auto);
    const folderToggle = anav.locator('.nav-plugin-folder button.nav-plugin-toggle');
    const folderOpen = () => folderToggle.getAttribute('aria-expanded');
    await children(auto).first().waitFor();
    await anav.locator(pageKeyOf('status')).locator('.nav-plugin-entry').click();
    await folderToggle.waitFor();
    assert.equal(await folderOpen(), 'true');
    let height = 900;
    while (await folderOpen() === 'true' && height > 300) {
      height -= 10;
      await resize(auto, { width: 1280, height });
    }
    assert.equal(await folderOpen(), 'false', 'the folder without the current page collapses first');
    assert.equal(await anav.getAttribute('data-plugin-layout'), 'expanded', 'the plugin group with the current page stays open');
    while (await anav.getAttribute('data-plugin-layout') === 'expanded' && height > 300) {
      height -= 10;
      await resize(auto, { width: 1280, height });
    }
    assert.equal(await anav.getAttribute('data-plugin-layout'), 'collapsed');
    await folderToggle.click();
    assert.equal(await folderOpen(), 'true');
    await resize(auto, { width: 1280, height: 900 });
    await settle(auto, 'expanded');
    await resize(auto, { width: 1280, height });
    await settle(auto, 'collapsed');
    assert.equal(await folderOpen(), 'true', 'a folder the user expanded is not collapsed automatically');
    assert.deepEqual(await auto.evaluate(() => JSON.parse(window.localStorage.getItem('cpa-gui.plugin-nav-collapsed-folders'))), { tools: 'expanded' });
    await autoContext.close();

    const seeded = async (custom, size) => {
      const context = await browser.newContext({ viewport: size, locale: 'zh-CN' });
      await context.addInitScript((initial) => {
        let inner;
        let layout = { version: 1, aliases: {}, custom: initial };
        const internals = {};
        Object.defineProperty(internals, 'invoke', {
          configurable: true,
          get: () => (command, args, options) => command === 'get_plugin_nav_layout' ? Promise.resolve(layout)
            : command === 'save_plugin_nav_layout' ? Promise.resolve(void (layout = args.layout))
              : inner(command, args, options),
          set: (value) => { inner = value; },
        });
        window.__TAURI_INTERNALS__ = internals;
      }, custom);
      const page = await context.newPage();
      page.on('pageerror', (error) => errors.push(error.message));
      await page.goto(`${base}?mock=running`, { waitUntil: 'commit' });
      await page.locator('.app-shell').waitFor();
      return page;
    };

    const folders = Array.from({ length: 25 }, (_, index) => ({ kind: 'folder', id: `f${index}`, name: `Folder ${index}`, children: [] }));
    const crowded = await seeded(folders, { width: 1280, height: 600 });
    await settle(crowded, 'collapsed');
    await section(crowded).locator('[data-plugin-nav-key="plugins"] .nav-plugin-toggle').click();
    await settle(crowded, 'expanded');
    await section(crowded).locator(pageKeyOf('status')).click({ button: 'right' });
    await crowded.getByRole('menuitem', { name: '移入文件夹' }).click();
    const submenu = crowded.getByRole('menu').nth(1);
    await submenu.getByRole('menuitem', { name: 'Folder 0' }).waitFor();
    const box = await submenu.evaluate((element) => ({ bottom: element.getBoundingClientRect().bottom, scrolls: element.scrollHeight > element.clientHeight }));
    assert.ok(box.bottom <= 600, `folder submenu stays inside the window (${box.bottom})`);
    assert.equal(box.scrolls, true, 'folder submenu scrolls');
    await crowded.keyboard.press('End');
    assert.equal(await crowded.evaluate(() => document.activeElement.textContent), '新建文件夹…');
    await crowded.context().close();

    const duplicated = await seeded([
      { kind: 'folder', id: 'tools', name: 'Tools', children: ['/v0/resource/plugins/request-inspector/samples'] },
      { kind: 'folder', id: 'tools', name: 'Tools copy', children: [] },
      { kind: 'page', path: '/v0/resource/plugins/request-inspector/samples' },
    ], { width: 1280, height: 900 });
    await section(duplicated).locator(pageKeyOf('samples')).click({ button: 'right' });
    await duplicated.getByRole('menuitem', { name: '移到一级菜单' }).waitFor();
    await duplicated.getByRole('menuitem', { name: '移入文件夹' }).click();
    assert.deepEqual(await duplicated.getByRole('menu').nth(1).getByRole('menuitem').allTextContents(), ['新建文件夹…'], 'the menu follows the displayed layout, not duplicate records');
    await duplicated.context().close();

    const ime = await seeded([{ kind: 'folder', id: 'tools', name: 'Tools', children: [] }], { width: 1280, height: 900 });
    const inav = section(ime);
    await inav.locator('.nav-plugin-folder').click({ button: 'right' });
    await ime.getByRole('menuitem', { name: '重命名' }).click();
    const imeInput = inav.locator('input.nav-plugin-rename');
    await imeInput.fill('工具');
    await imeInput.evaluate((element) => {
      element.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
      element.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true, isComposing: true }));
      element.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true, isComposing: true }));
      element.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '具' }));
    });
    assert.equal(await imeInput.count(), 1, 'Enter and Escape during IME composition keep the rename input open');
    await imeInput.fill('工具箱');
    await imeInput.press('Enter');
    await inav.locator('.nav-plugin-folder').getByRole('button', { name: '工具箱', exact: true }).waitFor();
    await ime.context().close();

    const disabled = await seeded([{ kind: 'folder', id: 'tools', name: 'Tools', children: ['/v0/resource/plugins/request-inspector/samples'] }], { width: 1280, height: 900 });
    const dnav = section(disabled);
    await dnav.locator('.nav-plugin-folder').waitFor();
    await dnav.getByRole('button', { name: '插件', exact: true }).click();
    await disabled.getByRole('button', { name: '插件设置' }).click();
    await disabled.locator('.plugin-global-toggle input').uncheck();
    await disabled.waitForFunction(() => !document.querySelector('.nav-section [data-plugin-route]'));
    await dnav.locator('.nav-plugin-folder').getByRole('button', { name: 'Tools', exact: true }).waitFor();
    await dnav.getByText('空文件夹').waitFor();
    const organizeButton = disabled.locator('.plugin-nav-organize').getByRole('button', { name: '整理侧栏…' });
    assert.equal(await organizeButton.isEnabled(), true, 'saved folders keep the settings organize entry');
    await dnav.locator('[data-plugin-nav-key="plugins"] .nav-plugin-more').click();
    await disabled.getByRole('menuitem', { name: '恢复默认布局' }).waitFor();
    await disabled.keyboard.press('Escape');

    await resize(disabled, { width: 800, height: 900 });
    assert.equal(await organizeButton.isDisabled(), true, 'the settings organize entry is disabled in compact mode');
    await disabled.locator('.plugin-nav-organize').getByText('窗口过窄，加宽窗口后才能整理侧栏').waitFor();
    await disabled.evaluate(() => window.dispatchEvent(new Event('plugin-nav-organize')));
    await resize(disabled, { width: 1280, height: 900 });
    await disabled.waitForTimeout(200);
    assert.equal(await dnav.evaluate((element) => element.hasAttribute('data-organizing')), false, 'a request from a narrow window does not start organizing later');
    await disabled.context().close();

    const compactArranged = await open('mock=running', { width: 800, height: 400 });
    await children(compactArranged).first().waitFor();
    assert.equal(await section(compactArranged).locator('.nav-plugin-more').count(), 0);
    await compactArranged.context().close();

    assert.deepEqual(errors, []);
    console.log('PASS: plugin submenu auto and manual expansion, live resize, navigation, locked state, compact bar, page rename, folders, promotion, save rollback, unreadable layouts, organize mode with pointer and keyboard drag, multi-group auto collapse, scrollable folder submenus, IME-safe rename, empty folders without plugin pages and compact organize gating.');
  } finally {
    if (browser) await browser.close();
    await server.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
