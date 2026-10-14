const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const base = process.env.HOME_DASHBOARD_TEST_BASE_URL || 'http://127.0.0.1:1421';

(async () => {
  let server;
  let browser;
  try {
    try { await fetch(base, { signal: AbortSignal.timeout(5000) }); } catch {
      const { createServer } = await import('vite');
      const react = (await import('@vitejs/plugin-react')).default;
      server = await createServer({ configFile: false, cacheDir: 'node_modules/.vite-home-dashboard', plugins: [react()], logLevel: 'error', optimizeDeps: { noDiscovery: true, include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', '@tauri-apps/api/core', '@tauri-apps/api/mocks', '@tauri-apps/api/event', 'lucide-react', 'react-markdown', '@dnd-kit/core', '@dnd-kit/sortable', '@dnd-kit/utilities'] }, server: { host: '127.0.0.1', port: Number(new URL(base).port), strictPort: true, watch: null } });
      await server.listen();
    }
    browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || 'msedge', headless: true, args: ['--no-proxy-server'] });
    const errors = [];
    const makePage = async (viewport = { width: 1280, height: 1050 }) => {
      const page = await browser.newPage({ viewport });
      page.setDefaultTimeout(10000);
      page.setDefaultNavigationTimeout(60000);
      page.on('pageerror', error => errors.push(String(error)));
      await page.route('**/*', route => route.request().url().startsWith(base + '/') ? route.continue() : route.abort());
      return page;
    };
    const page = await makePage();
    const showHealth = async (target = page, query = '') => {
      await target.locator('.core-health-open').click();
      await target.getByRole('dialog').waitFor();
      if (!query.includes('loading') && !query.includes('modelError') && !query.includes('offline')) {
        await target.locator('.core-health-table tbody tr').first().waitFor();
      }
    };
    const closeHealth = async (target = page) => {
      await target.getByRole('dialog').getByRole('button', { name: 'Close', exact: true }).click();
      await target.getByRole('dialog').waitFor({ state: 'detached' });
      assert.equal(await target.locator('.core-health-open').evaluate(node => node === document.activeElement), true);
    };
    const open = async (query = '', target = page, showDialog = true) => {
      const options = new URLSearchParams(query);
      if (!options.has('locale')) options.set('locale', 'en');
      await target.goto(`${base}/tests/fixtures/home-dashboard.html?${options}`, { waitUntil: 'domcontentloaded' });
      await target.locator('.core-health-entry').waitFor();
      if (showDialog) await showHealth(target, query);
    };
    const probes = target => target.evaluate(() => window.homeFixture.calls.filter(call => call.cmd === 'core_health_probe'));
    const checkAll = target => target.locator('.core-health-actions').getByRole('button', { name: 'Check All', exact: true });
    const awaitIdle = target => target.waitForFunction(() => !document.querySelector('.core-health-progress'));
    const value = id => page.locator(`[data-stat="${id}"] .home-stat-value`);
    const runtimeValue = label => page.locator('.control-panel .panel-detail-row').filter({ has: page.getByText(label, { exact: true }) }).locator('dd');
    const meter = id => page.locator(`[data-stat="${id}"] .home-stat-track[role="meter"]`);

    await open('', page, false);
    await page.locator('.home-overview[aria-busy="false"]').waitFor();
    fs.mkdirSync('misc', { recursive: true });
    await page.screenshot({ path: 'misc/home-dashboard-initial.png', fullPage: true, animations: 'disabled' });
    console.log('Initial homepage rendered: misc/home-dashboard-initial.png');
    assert.equal(await page.locator('.home-stat-card').count(), 4);
    assert.equal(await runtimeValue('Installation Status').innerText(), 'Installed');
    assert.equal(await runtimeValue('Runtime Status').innerText(), 'Running');
    assert.equal(await page.locator('.home-runtime-details .panel-detail-row').count(), 6, 'runtime details retain both statuses, both versions, PID, and port');
    for (const [id, expected] of [['usage', '97.8%'], ['credentials', '11'], ['providerKeys', '2'], ['models', '6']]) {
      assert.equal(await value(id).innerText(), expected, `incorrect ${id} aggregate`);
    }
    assert.match(await page.locator('[data-stat="credentials"] p').innerText(), /5 available · 6 not in rotation/);
    assert.match(await page.locator('[data-stat="usage"] p').innerText(), /503 requests/);
    assert.ok(Math.abs(Number(await meter('usage').getAttribute('aria-valuenow')) - 492 / 503 * 100) < 0.000001, 'success meter must use the measured success rate');
    assert.ok(Math.abs(Number(await meter('credentials').getAttribute('aria-valuenow')) - 5 / 11 * 100) < 0.000001, 'credential meter must use the available share');
    assert.equal(await page.getByRole('meter', { name: 'Available credential share', exact: true }).count(), 1);
    assert.equal(await page.locator('[data-stat="providerKeys"] [role="meter"], [data-stat="models"] [role="meter"]').count(), 0, 'counts without a denominator must not display a ratio');
    const usageQuery = await page.evaluate(() => window.homeFixture.calls.find(call => call.cmd === 'get_usage_overview').args.query);
    assert.equal(Date.parse(usageQuery.end) - Date.parse(usageQuery.start), 86_400_000);
    assert.equal((await probes(page)).length, 0, 'opening the dashboard must not perform inference');
    assert.equal(await page.locator('.core-health-panel, .core-health-table, .core-health-dialog').count(), 0, 'the dashboard must show only the compact entry');
    assert.ok((await page.locator('.core-health-entry').boundingBox()).height < 150, 'the collapsed entry must not take up a full panel');
    assert.equal(await page.locator('.home-overview-heading .core-health-entry').count(), 1, 'health monitoring belongs in the overview header');
    assert.equal(await page.locator('.home-dashboard > .core-health-entry').count(), 0, 'health monitoring must not leave an orphan entry below the overview');
    const healthSummaryBounds = await page.locator('.core-health-entry-status').boundingBox();
    assert.ok(healthSummaryBounds.width > 1 && healthSummaryBounds.height > 1, 'the health summary must stay visible beside its entry');

    const access = page.locator('.home-access-panel');
    assert.equal(await access.locator('.client-api-card').count(), 1, 'only the selected connection details should be shown');
    assert.equal(await access.locator('.home-key-value').innerText(), '••••••••••••••••••••');
    assert.equal(await access.getByText('test-client-access-key-never-real', { exact: true }).count(), 0);
    await access.getByRole('button', { name: 'Show key', exact: true }).click();
    assert.equal(await access.locator('.home-key-value').innerText(), 'test-client-access-key-never-real');
    await access.getByRole('button', { name: 'Hide key', exact: true }).click();
    await access.getByRole('button', { name: 'Copy authentication key', exact: true }).click();
    await page.waitForFunction(() => window.homeFixture.copied.includes('test-client-access-key-never-real'));
    const openaiTab = page.locator('#home-protocol-openai');
    await openaiTab.focus();
    await page.keyboard.press('ArrowRight');
    assert.equal(await page.locator('#home-protocol-claude').getAttribute('aria-selected'), 'true');
    await page.waitForFunction(() => document.activeElement?.id === 'home-protocol-claude');
    assert.equal(await page.locator('#home-protocol-claude').evaluate(node => node === document.activeElement), true);
    assert.equal(await page.getByRole('tabpanel').locator('code').innerText(), 'http://127.0.0.1:8317');
    await page.getByRole('tabpanel').getByRole('button').click();
    await page.waitForFunction(() => window.homeFixture.copied.includes('http://127.0.0.1:8317'));
    await page.locator('#home-protocol-claude').focus();
    await page.keyboard.press('End');
    assert.equal(await page.locator('#home-protocol-gemini').getAttribute('aria-selected'), 'true');
    await page.keyboard.press('Home');
    assert.equal(await openaiTab.getAttribute('aria-selected'), 'true');
    assert.equal(await page.getByRole('tabpanel').locator('code').innerText(), 'http://127.0.0.1:8317/v1');

    await showHealth();
    const dialog = page.getByRole('dialog', { name: 'Model Health', exact: true });
    assert.equal(await dialog.getAttribute('aria-modal'), 'true');
    await page.waitForFunction(() => document.querySelector('[role="dialog"]')?.contains(document.activeElement));
    assert.equal(await dialog.evaluate(node => node.contains(document.activeElement)), true, 'opening the modal must focus a control inside it');
    assert.equal(await dialog.getByRole('checkbox').count(), 0, 'health checks must not offer automatic mode');
    assert.equal(await dialog.getByText(/automatically|automatic check/i).count(), 0);
    assert.equal((await probes(page)).length, 0, 'opening the modal must not perform inference');
    const focusable = dialog.locator('button:not(:disabled), input:not(:disabled), summary, [tabindex="0"]');
    await focusable.last().focus();
    await page.keyboard.press('Tab');
    assert.equal(await focusable.first().evaluate(node => node === document.activeElement), true, 'Tab must wrap inside the modal');
    await page.keyboard.press('Shift+Tab');
    assert.equal(await focusable.last().evaluate(node => node === document.activeElement), true, 'Shift+Tab must wrap inside the modal');
    await page.getByRole('button', { name: 'Check gpt-5.2-codex', exact: true }).click();
    await page.locator('.core-health-status.healthy').waitFor();
    assert.deepEqual((await probes(page)).map(call => call.args), [{ model: 'gpt-5.2-codex', timeoutMs: 15000 }]);
    assert.match(await page.locator('.core-health-table tbody tr').first().locator('.core-health-latency').innerText(), /184 ms.*642 ms/);
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    assert.equal(await page.locator('.core-health-open').evaluate(node => node === document.activeElement), true, 'Escape must restore focus to the entry');
    assert.notEqual(await page.evaluate(() => document.body.style.overflow), 'hidden', 'closing must release the page scroll lock');
    await showHealth();
    assert.equal(await page.locator('.core-health-status.healthy').count(), 1, 'completed results must survive closing and reopening');
    assert.equal((await probes(page)).length, 1, 'reopening must not launch a check');
    await page.locator('.core-health-backdrop').click({ position: { x: 2, y: 2 } });
    await dialog.waitFor({ state: 'detached' });
    assert.equal(await page.locator('.core-health-open').evaluate(node => node === document.activeElement), true, 'backdrop dismissal must restore focus');
    await showHealth();
    await page.locator('.core-health-search input').fill('custom/team');
    assert.equal(await page.locator('.core-health-table tbody tr').count(), 1);
    await page.locator('.core-health-search input').fill('not-a-real-model');
    assert.equal(await page.locator('.core-health-table tbody tr').count(), 0);
    await page.locator('.core-health-search input').clear();
    await page.evaluate(() => {
      window.homeFixture.probeFailures['gemini-3-pro'] = 'HTTP 503: fixture upstream unavailable';
      window.homeFixture.probeFailures['deepseek-chat'] = 'deadline has elapsed';
    });
    await checkAll(page).click();
    await page.waitForFunction(() => document.querySelectorAll('.core-health-status.failed').length === 2);
    assert.equal((await probes(page)).length, 7);
    assert.equal(await page.locator('.core-health-status.healthy').count(), 4);
    const gemini = page.locator('.core-health-table tbody tr').filter({ hasText: 'gemini-3-pro' });
    await gemini.locator('summary').click();
    assert.match(await gemini.locator('details p').innerText(), /HTTP 503/);
    console.log('Home data, connection tabs, clipboard, single/batch/error checks passed.');

    await open();
    await page.evaluate(() => { window.homeFixture.holdProbes = true; });
    await checkAll(page).click();
    await page.waitForFunction(() => window.homeFixture.probeReleases.length === 4);
    assert.equal(await page.locator('.core-health-status.queued').count(), 2);
    await page.getByRole('button', { name: 'Stop Checking', exact: true }).click();
    await page.evaluate(() => window.homeFixture.releaseProbes());
    await awaitIdle(page);
    assert.equal((await probes(page)).length, 4, 'stop must not launch queued model requests');
    assert.equal(await page.locator('.core-health-status.healthy').count(), 0, 'late probe results must not populate stopped checks');

    await open();
    await page.getByRole('button', { name: 'Check gpt-5.2-codex', exact: true }).click();
    await page.locator('.core-health-status.healthy').waitFor();
    await page.evaluate(() => { window.homeFixture.holdProbes = true; });
    await checkAll(page).click();
    await page.waitForFunction(() => window.homeFixture.probeReleases.length === 4);
    assert.equal(await page.locator('.core-health-status.queued').count(), 2);
    await closeHealth();
    await page.evaluate(() => window.homeFixture.releaseProbes());
    await showHealth();
    await awaitIdle(page);
    assert.equal((await probes(page)).length, 5, 'closing must not launch queued model requests');
    assert.equal(await page.locator('.core-health-status.healthy').count(), 1, 'closing must keep completed results and discard late results');

    await open();
    await page.evaluate(() => { window.homeFixture.holdProbes = true; });
    await checkAll(page).click();
    await page.waitForFunction(() => window.homeFixture.probeReleases.length === 4);
    await page.evaluate(() => window.homeFixture.changeConfig());
    await page.waitForFunction(() => document.querySelector('#home-protocol-panel code')?.textContent.includes(':8318/'));
    await page.evaluate(() => window.homeFixture.releaseProbes());
    await awaitIdle(page);
    assert.equal(await page.locator('.core-health-status.healthy').count(), 0, 'config replacement must discard pending results');
    assert.equal((await probes(page)).length, 4);

    await open('loading=1');
    await page.waitForFunction(() => window.homeFixture.modelReleases.length > 0);
    assert.equal(await checkAll(page).isDisabled(), true);
    assert.equal(await page.locator('.core-health-empty .spin').count(), 1);
    await page.evaluate(() => window.homeFixture.releaseModels());
    await page.locator('.core-health-table tbody tr').first().waitFor();
    await open('modelError=1');
    await page.locator('.core-health-error').waitFor();
    assert.equal(await checkAll(page).isDisabled(), true);
    assert.equal(await value('models').innerText(), '—');
    await page.evaluate(() => { window.homeFixture.modelError = ''; });
    await page.getByRole('button', { name: 'Refresh Models', exact: true }).click();
    await page.locator('.core-health-table tbody tr').first().waitFor();
    await open('offline=1');
    await page.getByRole('dialog').getByText('Start the core to check model health', { exact: true }).waitFor();
    await page.locator('.home-overview[aria-busy="false"]').waitFor();
    assert.equal(await value('usage').innerText(), '97.8%');
    assert.equal(await value('models').innerText(), '—');
    assert.equal(await runtimeValue('Installation Status').innerText(), 'Installed');
    assert.equal(await runtimeValue('Runtime Status').innerText(), 'Stopped');
    assert.equal(await meter('usage').count(), 1, 'local usage stays measured while the core is stopped');
    assert.equal(await meter('credentials').count(), 0, 'unavailable credential data must not display a ratio');
    assert.equal(await checkAll(page).isDisabled(), true);
    assert.equal(await page.evaluate(() => window.homeFixture.calls.filter(call => call.cmd === 'get_core_models').length), 0);
    await open('noKey=1', page, false);
    assert.equal(await access.locator('.home-key-value').count(), 0);
    await open('keyError=1', page, false);
    assert.equal(await access.locator('.home-field-error').count(), 1);
    await open('tls=1', page, false);
    assert.equal(await page.getByRole('tabpanel').locator('code').innerText(), 'https://127.0.0.1:8317/v1');
    await open('noRequests=1&unknownCredentials=1', page, false);
    await page.locator('.home-overview[aria-busy="false"]').waitFor();
    assert.equal(await value('usage').innerText(), '—', 'an empty request window has no measured success rate');
    assert.equal(await page.locator('[data-stat="usage"] p').innerText(), 'No completed requests yet');
    assert.equal(await value('credentials').innerText(), '11');
    assert.match(await page.locator('[data-stat="credentials"] p').innerText(), /0 available · 0 not in rotation · 11 with unknown status/);
    for (const id of ['usage', 'credentials']) {
      assert.equal(await meter(id).count(), 0, `unmeasured ${id} must not display a zero-percent meter`);
      assert.equal(await page.locator(`[data-stat="${id}"] .home-stat-track[aria-hidden="true"]`).count(), 1);
    }
    await open('noCredentials=1&usageError=1', page, false);
    await page.locator('.home-overview[aria-busy="false"]').waitFor();
    assert.equal(await value('usage').innerText(), '—');
    assert.equal(await value('credentials').innerText(), '0');
    assert.equal(await meter('usage').count(), 0, 'a failed usage read must not display a ratio');
    assert.equal(await meter('credentials').count(), 0, 'an empty credential list has no denominator');
    console.log('Modal dismissal/focus/retention/cancellation, stop, stale configuration, loading/offline/retry/key/TLS checks passed.');

    // Overview refreshes and elapsed five-minute periods must never trigger inference.
    const timed = await makePage();
    await timed.clock.install({ time: new Date('2026-10-03T04:00:00Z') });
    await timed.clock.pauseAt(new Date('2026-10-03T04:00:01Z'));
    await open('', timed);
    assert.equal(await timed.getByRole('dialog').getByRole('checkbox').count(), 0);
    for (let cycle = 1; cycle <= 2; cycle += 1) {
      await timed.clock.runFor(301000);
      assert.equal((await probes(timed)).length, 0, `the open dialog must stay idle after ${cycle} five-minute periods`);
    }
    assert.ok(await timed.evaluate(() => window.homeFixture.calls.filter(call => call.cmd === 'get_core_models').length) >= 6);
    await checkAll(timed).click();
    await timed.waitForFunction(() => document.querySelectorAll('.core-health-status.healthy').length === 6);
    assert.equal((await probes(timed)).length, 6, 'manual Check All must probe every model once');
    for (let cycle = 1; cycle <= 2; cycle += 1) {
      await timed.clock.runFor(301000);
      assert.equal((await probes(timed)).length, 6, `manual checks must not repeat after ${cycle} five-minute periods`);
    }
    await closeHealth(timed);
    await timed.clock.runFor(301000);
    assert.equal((await probes(timed)).length, 6, 'the closed dialog must not start another check');
    await showHealth(timed);
    assert.equal(await timed.locator('.core-health-status.healthy').count(), 6);
    await timed.clock.runFor(301000);
    assert.equal((await probes(timed)).length, 6, 'reopening must not schedule another check');

    // Also stress loading=true and fresh-but-equal arrays on every refresh.
    await open('view=panel', timed);
    await checkAll(timed).click();
    await timed.waitForFunction(() => document.querySelectorAll('.core-health-status.healthy').length === 6);
    for (let cycle = 1; cycle <= 2; cycle += 1) {
      await timed.evaluate(() => window.homeFixture.refreshPanel());
      await timed.clock.runFor(301000);
      assert.equal((await probes(timed)).length, 6, 'loading model lists must not schedule checks');
      await timed.evaluate(() => window.homeFixture.resolvePanel());
      await timed.clock.runFor(301000);
      assert.equal((await probes(timed)).length, 6, 'refreshed model arrays must not schedule checks');
    }
    await checkAll(timed).click();
    await timed.waitForFunction(() => window.homeFixture.calls.filter(call => call.cmd === 'core_health_probe').length === 12);
    await awaitIdle(timed);
    await timed.close();
    console.log('Fake-clock manual-only checks passed: no inference while idle or across repeated five-minute periods, close/reopen, and model refreshes.');

    fs.mkdirSync('misc', { recursive: true });
    for (const [theme, width] of [['light', 1280], ['dark', 1280], ['light', 640], ['dark', 640], ['light', 390], ['dark', 390]]) {
      const height = width < 640 ? 844 : 1050;
      await page.setViewportSize({ width, height });
      await open(`locale=zh-CN&theme=${theme}`, page, false);
      await page.locator('.home-overview[aria-busy="false"]').waitFor();
      assert.equal(await page.locator('.core-health-table').count(), 0);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme}/${width} collapsed homepage overflows horizontally`);
      await page.screenshot({ path: `misc/home-dashboard-${theme}-${width}.png`, fullPage: true, animations: 'disabled' });
      await showHealth();
      await page.locator('.core-health-actions .primary-button').click();
      await page.waitForFunction(() => document.querySelectorAll('.core-health-status.healthy').length === 6);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme}/${width} overflows horizontally`);
      const bounds = await page.locator('.core-health-dialog').boundingBox();
      assert.ok(bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= width + 1 && bounds.y + bounds.height <= height + 1, `${theme}/${width} modal leaves the viewport`);
      await page.screenshot({ path: `misc/home-health-dialog-${theme}-${width}.png`, animations: 'disabled' });
    }
    for (const [theme, width, height] of [['light', 1280, 900], ['dark', 1280, 900], ['light', 640, 600], ['dark', 640, 600]]) {
      await page.setViewportSize({ width, height });
      await page.evaluate(() => {
        localStorage.setItem('easy-cli-proxy-api.locale', 'zh-CN');
      });
      await page.goto(`${base}/?mock=running`, { waitUntil: 'domcontentloaded' });
      await page.locator(`.theme-selector-trigger[data-appearance="${theme}"]`).click();
      await page.locator(`.theme-selector-option[data-theme-id="${theme}"]`).click();
      await page.keyboard.press('Escape');
      await page.locator('.home-overview[aria-busy="false"]').waitFor();
      const toolbar = page.locator('#browser-mock-toolbar');
      if (await toolbar.count()) await toolbar.evaluate(node => { node.style.display = 'none'; });
      assert.equal(await page.locator('.home-stat-card').count(), 4);
      assert.equal(await page.locator('.home-overview-heading .core-health-open').count(), 1);
      assert.equal(await page.locator('.home-dashboard > .core-health-entry').count(), 0);
      assert.equal(await page.locator('.core-health-table').count(), 0);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme}/${width} app homepage overflows horizontally`);
      await page.screenshot({ path: `misc/home-dashboard-app-${theme}-${width}.png`, animations: 'disabled' });
      if (theme === 'light' && width === 1280) await page.screenshot({ path: 'misc/home-dashboard-app.png', animations: 'disabled' });
      await showHealth();
      const bounds = await page.locator('.core-health-dialog').boundingBox();
      assert.ok(bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= width + 1 && bounds.y + bounds.height <= height + 1, `${theme}/${width} app modal leaves the viewport`);
      await page.screenshot({ path: `misc/home-health-dialog-app-${theme}-${width}.png`, animations: 'disabled' });
      if (theme === 'light' && width === 1280) await page.screenshot({ path: 'misc/home-health-dialog-app.png', animations: 'disabled' });
    }
    // The normal desktop dashboard must fit the usable viewport, including the
    // shorter CSS viewport produced by desktop display scaling.
    const desktopViewports = [
      { width: 1464, height: 845 },
      { width: 1280, height: 800 },
      { width: 1440, height: 900 },
      { width: 1280, height: 720 },
    ];
    for (const locale of ['zh-CN', 'en']) {
      for (const theme of ['light', 'dark']) {
        await page.setViewportSize(desktopViewports[0]);
        await page.evaluate(locale => {
          localStorage.setItem('easy-cli-proxy-api.locale', locale);
        }, locale);
        await page.goto(`${base}/?mock=running`, { waitUntil: 'domcontentloaded' });
        await page.locator(`.theme-selector-trigger[data-appearance="${theme}"]`).click();
        await page.locator(`.theme-selector-option[data-theme-id="${theme}"]`).click();
      await page.keyboard.press('Escape');
        await page.locator('.home-overview[aria-busy="false"]').waitFor();
        const toolbar = page.locator('#browser-mock-toolbar');
        if (await toolbar.count()) await toolbar.evaluate(node => { node.style.display = 'none'; });
        assert.equal(await page.locator('html').getAttribute('lang'), locale);
        assert.equal(await page.locator('html').getAttribute('data-theme'), theme);
        for (const viewport of desktopViewports) {
          await page.setViewportSize(viewport);
          const layout = await page.evaluate(async () => {
            await document.fonts.ready;
            await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
            window.scrollTo(0, 0);
            const root = document.scrollingElement;
            const cards = [...document.querySelectorAll('.home-top-grid > .panel, .home-stat-card')].map(node => {
              const bounds = node.getBoundingClientRect();
              return { name: node.getAttribute('data-stat') || node.className, top: bounds.top, bottom: bounds.bottom, height: bounds.height };
            });
            return { scrollHeight: root.scrollHeight, clientHeight: root.clientHeight, viewportHeight: innerHeight, cards };
          });
          const context = `${locale}/${theme}/${viewport.width}x${viewport.height}`;
          assert.equal(layout.cards.length, 6, `${context} must show both top panels and all four overview cards`);
          assert.ok(layout.scrollHeight <= layout.clientHeight + 1,
            `${context} homepage causes a vertical page scrollbar: ${layout.scrollHeight}px content in ${layout.clientHeight}px viewport`);
          for (const card of layout.cards) {
            assert.ok(card.height > 0 && card.top >= -1 && card.bottom <= layout.viewportHeight + 1,
              `${context} ${card.name} must be fully visible without scrolling: ${JSON.stringify(card)}`);
          }
        }
      }
    }
    console.log('Desktop homepage fits all four viewport sizes in Chinese/English and light/dark themes without vertical page overflow.');
    assert.deepEqual(errors, []);
    console.log('Home dashboard browser checks passed. Screenshots: misc/home-{dashboard,health-dialog}-{light,dark}-{1280,640,390}.png and app.png');
  } finally {
    if (browser) await browser.close();
    if (server) await server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
