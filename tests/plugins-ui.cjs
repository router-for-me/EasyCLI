const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const server = await createServer({
    configFile: false, root: path.resolve(__dirname, '..'), plugins: [react()], logLevel: 'error',
    cacheDir: 'node_modules/.vite-plugins-ui',
    optimizeDeps: { noDiscovery: true, include: ['react', 'react-dom/client', 'react/jsx-runtime', 'lucide-react'] },
    server: { host: '127.0.0.1', port: 1434, strictPort: false, watch: null },
  });
  let browser;
  try {
    await server.listen();
    const base = `http://127.0.0.1:${server.httpServer.address().port}`;
    const channel = process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined);
    browser = await chromium.launch({ channel, headless: true, args: ['--no-proxy-server'] });
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12000);
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.route('**/*', route => route.request().url().startsWith(`${base}/`) ? route.continue() : route.abort());
    const open = async (query = '') => {
      await page.goto(`${base}/tests/fixtures/plugins.html?${query}`, { waitUntil: 'domcontentloaded' });
      await page.locator('.plugins-heading h1').waitFor();
    };
    const card = title => page.locator('.plugin-card').filter({ has: page.getByRole('heading', { name: title, exact: true }) });
    const tab = name => page.locator('.plugin-tabs').getByRole('button', { name, exact: true });
    const dialog = () => page.getByRole('dialog');
    const installs = () => page.evaluate(() => window.pluginsFixture.installs);
    const noOverflow = async () => assert.equal(await page.evaluate(() =>
      document.documentElement.scrollWidth <= innerWidth + 1), true, 'Plugin page must fit the viewport');
    await open();
    await card('Sample provider').getByRole('button', { name: 'Configure', exact: true }).waitFor();
    assert.equal(await page.locator('.plugin-card').count(), 4);
    await tab('Local plugins').waitFor();
    assert.match(await page.locator('.plugin-summary').innerText(), /Installed\s+3/);
    await card('kiro').getByText('Plugin file not found', { exact: true }).waitFor();
    await card('Unloaded provider').getByText('Not loaded', { exact: true }).waitFor();
    assert.equal(await page.getByText('Waiting to load', { exact: true }).count(), 0);
    assert.equal(await card('kiro').getByRole('button', { name: 'Sign in', exact: true }).count(), 0);
    assert.equal(await card('kiro').getByRole('button', { name: 'Uninstall kiro', exact: true }).count(), 0);
    assert.equal(await card('kiro').getByRole('button', { name: 'Disable', exact: true }).count(), 0);
    assert.equal(await card('Unloaded provider').getByRole('button', { name: 'Find in store', exact: true }).count(), 1);
    await card('Unloaded provider').getByRole('button', { name: 'Open logs', exact: true }).click();
    assert.equal(await page.evaluate(() => window.pluginsFixture.commands.some(call => call.command === 'open_core_logs_directory')), true);
    await noOverflow();
    await card('Sample provider').getByRole('button', { name: 'Check for updates', exact: true }).click();
    await page.getByText('sample..plugin: Some sources could not be checked.', { exact: false }).waitFor();
    assert.equal(await tab('Plugin store').getAttribute('aria-current'), 'page');
    assert.equal(await page.getByRole('searchbox').inputValue(), 'sample..plugin');
    assert.equal((await installs()).length, 0, 'checking never installs');
    assert.equal(await page.evaluate(() => window.pluginsFixture.commands.some(call => call.command === 'get_plugin_latest_releases')), true, 'Check for updates refreshes releases');
    await tab('Local plugins').click();
    await card('Sample provider').getByRole('button', { name: 'Find in store', exact: true }).click();
    await card('Sample provider').getByRole('button', { name: 'Update', exact: true }).waitFor();
    assert.equal(await page.getByText('sample..plugin: Some sources could not be checked.', { exact: false }).count(), 0);
    await tab('Local plugins').click();


    const searchInput = page.getByRole('searchbox');
    await searchInput.fill('qoder');
    const focusedSearch = await searchInput.evaluate(input => ({
      outline: getComputedStyle(input).outlineStyle,
      shadow: getComputedStyle(input).boxShadow,
      containerShadow: getComputedStyle(input.parentElement).boxShadow,
    }));
    assert.equal(focusedSearch.outline, 'none', 'the input must not draw a second focus ring inside the search field');
    assert.equal(focusedSearch.shadow, 'none');
    assert.notEqual(focusedSearch.containerShadow, 'none', 'the outer search field retains a visible focus indicator');
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('.plugin-search-clear').evaluate(button => button === document.activeElement), true);
    await page.keyboard.press('Enter');
    assert.equal(await searchInput.inputValue(), '');
    assert.equal(await searchInput.evaluate(input => input === document.activeElement), true, 'clearing returns focus to search');
    assert.equal(await page.locator('.plugin-card').count(), 4, 'clearing restores the unfiltered list');

    await card('kiro').getByRole('button', { name: 'Find in store', exact: true }).click();
    await card('Kiro').getByRole('button', { name: 'Install', exact: true }).waitFor();
    assert.equal(await tab('Plugin store').getAttribute('aria-current'), 'page');
    assert.equal(await page.getByRole('searchbox').inputValue(), 'kiro');
    assert.equal(await page.locator('.plugin-card').count(), 1);
    await tab('Local plugins').click();
    await card('kiro').getByRole('button', { name: 'Remove configuration kiro', exact: true }).click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Remove configuration', exact: true }).click();
    await card('kiro').waitFor({ state: 'detached' });
    assert.equal(await page.evaluate(() => Object.hasOwn(window.pluginsFixture.settings.configs, 'kiro')), false);
    assert.equal(await page.evaluate(() => window.pluginsFixture.requests.some(request =>
      request.method === 'DELETE' && request.path === '/plugins/kiro')), true);
    assert.match(await page.locator('.plugin-summary').innerText(), /Installed\s+3/);
    assert.equal(await page.getByText('Restart the core to fully apply this change.', { exact: true }).count(), 0);

    assert.equal(await card('Dormant provider').getByRole('button', { name: 'Sign in', exact: true }).count(), 0);
    await card('Sample provider').getByRole('button', { name: 'Sign in', exact: true }).click();
    await dialog().getByLabel('Sign-in URL', { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.pluginsFixture.commands.some(call => call.command === 'start_oauth_login' && call.args.provider === 'sample-sso' && call.args.pluginProvider === true)), true);
    await page.keyboard.press('Escape');
    await dialog().waitFor({ state: 'detached' });

    await card('Sample provider').getByRole('button', { name: 'Usage dashboard' }).click();
    assert.deepEqual(await page.evaluate(() => window.pluginsFixture.navigations), ['/plugin-pages/sample..plugin/2']);
    assert.equal(await page.evaluate(() => window.pluginsFixture.commands.some(call => call.command.includes('webview') || call.command.includes('resource_url'))), false);

    const globalSwitch = page.getByRole('checkbox', { name: 'Enable plugins', exact: true });
    await globalSwitch.uncheck();
    await page.getByText('Plugins are globally disabled.', { exact: false }).waitFor();
    await card('Sample provider').getByText('Globally disabled', { exact: true }).waitFor();
    assert.equal(await card('Sample provider').getByRole('button', { name: 'Usage dashboard' }).count(), 0);
    assert.equal(await card('Sample provider').getByRole('button', { name: 'Sign in', exact: true }).count(), 0);
    assert.equal(await page.evaluate(() => window.pluginsFixture.settings.configs['sample..plugin'].untouched.retained), true);
    await globalSwitch.check();
    await card('Sample provider').getByRole('button', { name: 'Disable', exact: true }).click();
    await card('Sample provider').getByRole('button', { name: 'Enable', exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.pluginsFixture.requests.some(request =>
      request.method === 'PUT' && request.path === '/config/plugins/configs/sample%2E%2Eplugin/enabled' && request.body === false)), true);
    await card('Sample provider').getByRole('button', { name: 'Enable', exact: true }).click();
    const search = page.getByRole('searchbox');
    await search.fill('does-not-exist');
    await page.getByRole('heading', { name: 'No matching plugins' }).waitFor();
    await search.fill('');

    await page.evaluate(() => { window.pluginsFixture.commands.length = 0; });
    await tab('Plugin store').click();
    await card('Community analytics').getByRole('button', { name: 'Install', exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.pluginsFixture.commands.some(call => call.command === 'get_plugin_latest_releases')), false);
    await page.locator('.plugins-heading').getByRole('button', { name: 'Refresh', exact: true }).click();
    await card('Sample provider').getByText('1.0.0 → 3.0.0', { exact: false }).waitFor();
    await page.evaluate(() => { window.pluginsFixture.latestReleaseVersion = '4.0.0'; });
    await page.locator('.plugins-heading').getByRole('button', { name: 'Refresh', exact: true }).click();
    await card('Sample provider').getByText('1.0.0 → 4.0.0', { exact: false }).waitFor();
    assert.equal(await page.evaluate(() => window.pluginsFixture.commands.some(call => call.command === 'get_plugin_latest_releases' && call.args.repositories.includes('router-for-me/direct'))), false);
    await page.getByText('Offline source: Fixture registry unavailable').waitFor();
    assert.equal(await card('Private provider').getByRole('button', { name: 'Store authentication required' }).isDisabled(), true);
    await page.getByRole('combobox', { name: 'Plugin store' }).selectOption('updates');
    assert.equal(await page.locator('.plugin-card').count(), 1);
    await page.getByRole('combobox', { name: 'Plugin store' }).selectOption('all');
    await card('Community analytics').getByRole('button', { name: 'Install', exact: true }).click();
    await dialog().getByLabel('Version tag (blank for latest)').fill('../unsafe');
    assert.equal(await dialog().getByRole('button', { name: 'Install', exact: true }).isDisabled(), true);
    await dialog().getByLabel('Version tag (blank for latest)').fill('v1.2.3');
    await page.evaluate(() => { window.pluginsFixture.holdInstall = true; });
    await dialog().getByRole('button', { name: 'Install', exact: true }).click();
    await page.waitForFunction(() => window.pluginsFixture.installs.length === 1);
    await page.keyboard.press('Escape');
    assert.equal(await dialog().count(), 1, 'In-flight installation keeps the dialog open');
    assert.equal(await dialog().getByRole('button', { name: 'Close', exact: true }).isDisabled(), true);
    assert.deepEqual((await installs())[0].query, { source: 'community', version: 'v1.2.3' });
    await page.evaluate(() => { window.pluginsFixture.holdInstall = false; window.pluginsFixture.releaseInstall(); });
    await dialog().waitFor({ state: 'detached' });
    await page.getByText('Restart the core to fully apply this change.', { exact: true }).waitFor();

    await card('Sample provider').getByRole('button', { name: 'Update', exact: true }).click();
    await dialog().getByLabel('Version tag (blank for latest)').fill('v2.0.0');
    await page.evaluate(() => { window.pluginsFixture.failNextInstall = true; });
    await dialog().getByRole('button', { name: 'Update', exact: true }).click();
    await dialog().getByRole('alert').filter({ hasText: 'Fixture installation failed' }).waitFor();
    await dialog().getByRole('button', { name: 'Update', exact: true }).click();
    await dialog().waitFor({ state: 'detached' });
    assert.equal((await installs()).length, 3, 'Official install retries without a third-party gate');
    await card('Direct artifact').getByRole('button', { name: 'Install', exact: true }).click();
    assert.equal(await dialog().getByLabel('Version tag (blank for latest)').count(), 0, 'Direct artifacts do not accept GitHub tags');
    await dialog().getByRole('button', { name: 'Cancel', exact: true }).click();

    await tab('Plugin settings').click();
    const directory = page.getByLabel('Plugin directory', { exact: true });
    const sources = page.getByLabel(/Additional store sources/);
    const auth = page.getByLabel(/Store authentication rules/);
    await directory.fill('custom-plugins');
    await sources.fill('javascript:alert(1)');
    await page.locator('.plugin-settings').getByRole('button', { name: 'Save', exact: true }).click();
    await page.locator('.plugin-settings').getByRole('alert').waitFor();
    assert.equal(await page.evaluate(() => window.pluginsFixture.settings.dir), 'plugins');
    await sources.fill('https://another.example.test/registry.json');
    await auth.fill('[{"host":"another.example.test","token-env":"PLUGIN_TEST_TOKEN"}]');
    await page.locator('.plugin-settings').getByRole('button', { name: 'Save', exact: true }).click();
    await page.waitForFunction(() => window.pluginsFixture.settings.dir === 'custom-plugins');
    assert.equal(await page.evaluate(() => window.pluginsFixture.settings['auth-revision']), 9);
    assert.equal(await page.evaluate(() => window.pluginsFixture.settings.configs['sample..plugin'].untouched.retained), true);

    await tab('Local plugins').click();
    await card('Dormant provider').getByRole('button', { name: 'Uninstall Dormant provider' }).click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Uninstall', exact: true }).click();
    await card('Dormant provider').waitFor({ state: 'detached' });
    assert.equal(await page.evaluate(() => Object.hasOwn(window.pluginsFixture.settings.configs, 'dormant')), false);
    await page.evaluate(() => { window.pluginsFixture.failNextList = true; });
    await page.getByRole('button', { name: 'Refresh', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Fixture list temporarily unavailable' }).waitFor();
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Fixture list temporarily unavailable' }).waitFor({ state: 'detached' });

    for (const locale of ['en', 'zh-CN']) {
      for (const width of [375, 640, 1070, 1368]) {
        await page.setViewportSize({ width, height: 800 });
        await open(`locale=${locale}&theme=${locale === 'en' ? 'light' : 'dark'}`);
        await page.locator('.plugin-card').first().waitFor();
        await tab(locale === 'en' ? 'Local plugins' : '本地插件').waitFor();
        await card('kiro').getByText(locale === 'en' ? 'Plugin file not found' : '未找到插件文件', { exact: true }).waitFor();
        await card('Unloaded provider').getByText(locale === 'en' ? 'Not loaded' : '未加载', { exact: true }).waitFor();
        assert.match(await page.locator('.plugin-summary').innerText(), locale === 'en' ? /Installed\s+3/ : /已安装\s+3/);
        await noOverflow();
        if (process.env.PLUGIN_SCREENSHOT_DIR) {
          fs.mkdirSync(process.env.PLUGIN_SCREENSHOT_DIR, { recursive: true });
          await page.screenshot({ path: path.join(process.env.PLUGIN_SCREENSHOT_DIR, `plugins-local-${locale}-${width}.png`), fullPage: true });
        }
        await page.locator('.plugin-tabs button').nth(1).click();
        await card('Community analytics').waitFor();
        await noOverflow();
        if (process.env.PLUGIN_SCREENSHOT_DIR) {
          fs.mkdirSync(process.env.PLUGIN_SCREENSHOT_DIR, { recursive: true });
          await page.screenshot({ path: path.join(process.env.PLUGIN_SCREENSHOT_DIR, `plugins-${locale}-${width}.png`), fullPage: true });
          await page.getByRole('searchbox').fill('qoder');
          await page.screenshot({ path: path.join(process.env.PLUGIN_SCREENSHOT_DIR, `plugins-search-${locale}-${width}.png`), fullPage: true });
          await page.locator('.plugin-search-clear').click();
        }
        await card('Community analytics').getByRole('button', { name: locale === 'en' ? 'Install' : '安装', exact: true }).click();
        await dialog().waitFor();
        // Exercise wrapping and scrolling with metadata longer than a typical registry entry.
        await dialog().locator('h2').evaluate(el => { el.textContent += ' · model-fallback-router-compatibility'; });
        await dialog().locator('.plugin-install-origin small').evaluate(el => { el.textContent = 'https://raw.githubusercontent.com/example/plugins-store/main/' + 'long-registry-path-'.repeat(10) + 'registry.json'; });
        await page.setViewportSize({ width, height: 480 });
        const layout = await dialog().evaluate(el => {
          const heading = el.querySelector('.config-dialog-heading').getBoundingClientRect();
          const body = el.querySelector('.plugin-install-body').getBoundingClientRect();
          const footer = el.querySelector('.plugin-dialog-actions').getBoundingClientRect();
          const bounds = el.getBoundingClientRect();
          return { ordered: heading.bottom <= body.top + 1 && body.bottom <= footer.top + 1,
            fits: bounds.top >= 0 && bounds.bottom <= innerHeight && el.scrollWidth <= el.clientWidth + 1 };
        });
        assert.deepEqual(layout, { ordered: true, fits: true }, 'Dialog sections must not overlap or overflow');
        await noOverflow();
        if (process.env.PLUGIN_SCREENSHOT_DIR) {
          await page.screenshot({ path: path.join(process.env.PLUGIN_SCREENSHOT_DIR, `plugins-install-${locale}-${width}.png`) });
        }
        await page.keyboard.press('Escape');
        await dialog().waitFor({ state: 'detached' });
      }
    }
    await open('unsupported');
    await page.getByText('This core does not support plugins.', { exact: false }).waitFor();
    assert.equal(await page.locator('.plugin-tabs').count(), 0);
    assert.equal(await page.evaluate(() => window.pluginsFixture.requests.length), 0, 'Unsupported cores do not receive plugin mutations or list requests');
    assert.deepEqual(errors, []);
    console.log('Plugin UI passed: configured-only and unloaded states, installed counts, store lookup, configuration removal, resources, global/instance toggle, store filters, single-step installation, versions, retry, settings, uninstall, unsupported cores, responsive English/Chinese.');
  } finally {
    await browser?.close();
    await server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
