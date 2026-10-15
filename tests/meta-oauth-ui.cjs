const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const base = process.env.META_OAUTH_TEST_BASE_URL || process.env.DEVIN_TEST_BASE_URL || 'http://127.0.0.1:1421';

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--no-proxy-server'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(10000);
    const errors = [];
    page.on('pageerror', (error) => errors.push(String(error)));
    await page.route('**/*', route => route.request().url().startsWith(base + '/') ? route.continue() : route.abort());

    const open = async (query = '') => {
      await page.goto(base + '/tests/fixtures/meta-oauth.html?' + query, { waitUntil: 'domcontentloaded' });
      await page.locator('.oauth-card, .simple-mode-provider-card').first().waitFor();
    };
    const managementCard = () => page.locator('.oauth-card').filter({ hasText: /Muse \(Meta\) OAuth|Muse OAuth|Meta OAuth/ });
    const easyCard = () => page.locator('.simple-mode-provider-card').filter({ hasText: /Muse \(Meta\) OAuth|Muse OAuth|Meta OAuth/ });
    const calls = () => page.evaluate(() => window.metaFixture.calls);
    const fixture = () => page.evaluate(() => window.metaFixture);
    const installClipboard = () => page.evaluate(() => {
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: { writeText: async (value) => window.metaFixture.copied.push(String(value)) },
      });
    });
    const waitForStatusCall = () => page.waitForFunction(() => window.metaFixture.statusCalls > 0);
    const startManagement = async () => {
      const card = managementCard();
      await card.getByRole('button', { name: /^(Sign In|登录)$/ }).click();
      await card.getByText('META-CODE-1', { exact: true }).waitFor();
    };

    // Management view: Meta is a seventh OAuth provider, with device code and no callback input.
    // `.oauth-card` counts built-in provider cards only; the Vertex import card carries
    // `.vertex-login-card` and plugin cards carry `.plugin-oauth-card`.
    await open('locale=en');
    const card = managementCard();
    assert.equal(await page.locator('.oauth-card').count(), 7, 'Meta should add a seventh OAuth card');
    assert.equal(await card.count(), 1, 'the Muse (Meta) card should be unique');
    assert.equal(await card.getByRole('textbox').count(), 0, 'Meta must not render a callback textbox');
    await installClipboard();
    await startManagement();
    assert.ok((await calls()).some(call => call.cmd === 'start_oauth_login' && call.args.provider === 'meta' && call.args.browser === 'none'));
    assert.equal(await card.getByText('Device Code', { exact: true }).count(), 1);
    assert.equal(await card.getByRole('button', { name: 'Copy Device Code', exact: true }).count(), 1);
    await card.getByRole('button', { name: 'Copy Device Code', exact: true }).click();
    await page.waitForFunction(() => window.metaFixture.copied.includes('META-CODE-1'));
    await card.getByRole('button', { name: 'Open Link', exact: true }).click();
    assert.ok((await calls()).some(call => call.cmd === 'open_oauth_url' && String(call.args.url).includes('meta-attempt-1')));

    // Complete the flow and confirm the newly written credential is assigned priority zero.
    await waitForStatusCall();
    await page.evaluate(() => { window.metaFixture.status = 'ok'; });
    await card.getByText('Authorization successful. Credentials saved to Authentication Files.', { exact: true }).waitFor();
    assert.ok((await fixture()).priorityPatches.some(patch => patch.name === 'meta-oauth-1.json' && patch.priority === 0));
    assert.equal(await card.getByText('META-CODE-1', { exact: true }).count(), 0, 'success clears the device code');

    // A refresh cancels the old session and stale completion cannot win over attempt two.
    await open('locale=en');
    const refreshed = managementCard();
    await page.evaluate(() => { window.metaFixture.holdStatus = true; });
    await refreshed.getByRole('button', { name: 'Sign In', exact: true }).click();
    await refreshed.getByText('META-CODE-1', { exact: true }).waitFor();
    await page.waitForFunction(() => window.metaFixture.releaseStatus !== null);
    await refreshed.getByRole('button', { name: 'Refresh Link', exact: true }).click();
    await page.waitForFunction(() => window.metaFixture.attempts === 2);
    await refreshed.getByText('META-CODE-2', { exact: true }).waitFor();
    const refreshedFixture = await fixture();
    assert.ok(refreshedFixture.cancelledStates.includes('meta-attempt-1'));
    assert.equal(await refreshed.getByText('META-CODE-1', { exact: true }).count(), 0);
    await page.evaluate(() => { if (window.metaFixture.releaseStatus) window.metaFixture.releaseStatus(); });
    await page.waitForTimeout(100);
    assert.equal(await refreshed.getByText('Authorization successful. Credentials saved to Authentication Files.', { exact: true }).count(), 0, 'stale attempt must not complete refreshed login');
    assert.ok(await refreshed.getByText('META-CODE-2', { exact: true }).isVisible());

    // Error state can be retried and a later success still clears its device code.
    await open('locale=en');
    const retry = managementCard();
    await retry.getByRole('button', { name: 'Sign In', exact: true }).click();
    await retry.getByText('META-CODE-1', { exact: true }).waitFor();
    await waitForStatusCall();
    await page.evaluate(() => { window.metaFixture.status = 'error'; });
    await page.getByText(/sign-in failed/i).waitFor();
    await retry.getByRole('button', { name: 'Sign In', exact: true }).click();
    await page.waitForFunction(() => window.metaFixture.attempts === 2);
    await retry.getByText('META-CODE-2', { exact: true }).waitFor();
    await page.waitForFunction(() => window.metaFixture.statusCalls > 1);
    await page.evaluate(() => { window.metaFixture.status = 'ok'; });
    await retry.getByText('Authorization successful. Credentials saved to Authentication Files.', { exact: true }).waitFor();

    // Easy mode uses the same Meta flow and keeps the browser selection explicit.
    await open('view=easy&locale=en&openError=1');
    const easy = easyCard();
    assert.equal(await page.locator('.simple-mode-provider-card').count(), 7, 'Easy mode should expose Meta OAuth');
    await installClipboard();
    await easy.getByRole('button', { name: 'Start Sign-In', exact: true }).click();
    await easy.getByText('META-CODE-1', { exact: true }).waitFor();
    assert.equal(await easy.getByText('Device Code', { exact: true }).count(), 1);
    assert.equal(await easy.getByRole('textbox').count(), 0);
    await easy.getByRole('button', { name: 'Copy Device Code', exact: true }).click();
    await page.waitForFunction(() => window.metaFixture.copied.includes('META-CODE-1'));
    await easy.getByRole('button', { name: 'Copy Link', exact: true }).click();
    await page.waitForFunction(() => window.metaFixture.copied.includes('https://auth.meta.ai/device?state=meta-attempt-1'));
    await easy.getByRole('button', { name: 'Open Link', exact: true }).click();
    assert.ok((await calls()).some(call => call.cmd === 'open_oauth_url'
      && call.args.url === 'https://auth.meta.ai/device?state=meta-attempt-1'
      && call.args.browser === 'default'), 'Easy mode retains a usable fallback link when automatic browser launch fails');
    await page.waitForFunction(() => window.metaFixture.calls.some(call => call.cmd === 'start_oauth_login' && call.args.provider === 'meta'));
    const easyStart = (await calls()).find(call => call.cmd === 'start_oauth_login' && call.args.provider === 'meta');
    assert.equal(easyStart.args.browser, 'default');
    await waitForStatusCall();
    await page.evaluate(() => { window.metaFixture.status = 'ok'; });
    await easy.locator('.state-pill.success').waitFor();
    assert.equal(await easy.getByText('META-CODE-1', { exact: true }).count(), 0, 'Easy mode success clears the device code');
    assert.equal(await easy.locator('.simple-mode-oauth-authorization').count(), 0, 'Easy mode success clears its authorization link');

    // Meta's 900-second expiry outlives the old ten-minute fallback, then clears stale controls.
    await open('view=easy&locale=en');
    const expiring = easyCard();
    await expiring.getByRole('button', { name: 'Start Sign-In', exact: true }).click();
    await expiring.getByText('META-CODE-1', { exact: true }).waitFor();
    const pollsBeforeClockAdvance = await page.evaluate(() => {
      window.metaFixture.clockOffsetMs = 601_000;
      return window.metaFixture.statusCalls;
    });
    await page.waitForFunction((previous) => window.metaFixture.statusCalls > previous, pollsBeforeClockAdvance);
    assert.ok(await expiring.getByText('META-CODE-1', { exact: true }).isVisible(), 'device code is still valid after 601 seconds');
    assert.ok(await expiring.getByRole('button', { name: 'Waiting for browser authorization...', exact: true }).isDisabled());
    await page.evaluate(() => { window.metaFixture.clockOffsetMs = 901_000; });
    await page.getByText('Authorization timed out. Please initiate login again.', { exact: true }).waitFor();
    assert.equal(await expiring.getByText('META-CODE-1', { exact: true }).count(), 0, 'expiry clears the device code');
    assert.equal(await expiring.locator('.simple-mode-oauth-authorization').count(), 0, 'expiry clears the authorization link');
    assert.ok(await expiring.getByRole('button', { name: 'Start Sign-In', exact: true }).isEnabled());

    // Existing kimi-ai credentials continue to map to Kimi in Easy mode.
    await open('view=easy&locale=en&kimiAi=1');
    const kimi = page.locator('.simple-mode-provider-card').filter({ hasText: 'Kimi OAuth' });
    await kimi.locator('.state-pill.success').waitFor();
    assert.ok((await kimi.getAttribute('class')).split(' ').includes('connected'));
    assert.equal((await calls()).filter(call => call.cmd === 'start_oauth_login').length, 0);

    // Check the rendered labels and width constraints in both themes and viewport classes.
    fs.mkdirSync('misc', { recursive: true });
    for (const [theme, width] of [['light', 1280], ['dark', 390]]) {
      await page.setViewportSize({ width, height: 900 });
      await open(`locale=zh-CN&theme=${theme}`);
      const zhCard = managementCard();
      await zhCard.getByRole('button', { name: '登录', exact: true }).click();
      await zhCard.getByText('META-CODE-1', { exact: true }).waitFor();
      assert.equal(await zhCard.getByText('设备码', { exact: true }).count(), 1);
      assert.equal(await zhCard.getByRole('button', { name: '复制设备码', exact: true }).count(), 1);
      assert.equal(await zhCard.getByRole('textbox').count(), 0);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme} viewport overflows horizontally`);
      await page.screenshot({ path: `misc/meta-oauth-${theme}.png`, fullPage: true });

      await open(`view=easy&locale=zh-CN&theme=${theme}`);
      const easyResponsive = easyCard();
      await easyResponsive.getByRole('button', { name: /开始登录/ }).click();
      await easyResponsive.getByText('META-CODE-1', { exact: true }).waitFor();
      assert.equal(await easyResponsive.getByText('设备码', { exact: true }).count(), 1);
      assert.equal(await easyResponsive.getByRole('button', { name: '复制设备码', exact: true }).count(), 1);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Easy mode ${theme} viewport overflows horizontally`);
      await easyResponsive.screenshot({ path: `misc/meta-oauth-easy-${theme}.png` });
    }

    assert.deepEqual(errors, []);
    console.log('Meta OAuth browser checks passed: provider card, device code copy/open, refresh cancellation, stale poll isolation, success priority, retry, Easy mode expiry, Kimi credential compatibility and responsive labels.');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
