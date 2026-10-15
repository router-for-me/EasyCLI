const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const base = process.env.DEVIN_TEST_BASE_URL || 'http://127.0.0.1:1421';
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--no-proxy-server'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(10000);
    const errors = [];
    page.on('pageerror', (error) => errors.push(String(error)));
    await page.route('**/*', route => route.request().url().startsWith(base + '/') ? route.continue() : route.abort());
    const open = query => page.goto(base + '/tests/fixtures/devin.html?' + (query || ''), { waitUntil: 'domcontentloaded' });
    const card = () => page.locator('.oauth-card').filter({ hasText: 'Devin OAuth' });
    const click = label => card().getByRole('button', { name: label, exact: true }).click();
    const calls = () => page.evaluate(() => window.devinFixture.calls);
    await open();
    await card().waitFor();
    // `.oauth-card` counts built-in OAuth provider cards; the Vertex import card and plugin
    // provider cards use their own classes so they never inflate this number.
    assert.equal(await page.locator('.oauth-card').count(), 7);
    await click('Sign In');
    await card().getByRole('textbox').waitFor();
    assert.ok((await calls()).some(call => call.cmd === 'start_oauth_login' && call.args.provider === 'devin' && call.args.browser === 'none'));
    await card().getByRole('textbox').fill('http://127.0.0.1:8317/callback?state=old&code=code');
    await click('Submit Callback');
    await page.getByText('The callback state does not match', { exact: false }).waitFor();
    assert.equal((await calls()).filter(call => call.cmd === 'submit_oauth_callback').length, 0);
    await card().getByRole('textbox').fill('http://127.0.0.1:8317/callback?state=attempt-1&code=code');
    await click('Submit Callback');
    await card().getByText('Authorization successful. Credentials saved to Authentication Files.', { exact: true }).waitFor();
    assert.equal(await card().getByRole('textbox').count(), 0);

    await open();
    await click('Sign In');
    await card().getByRole('textbox').waitFor();
    await page.evaluate(() => { window.devinFixture.holdStatus = true; });
    await page.waitForFunction(() => window.devinFixture.releaseStatus !== null);
    await click('Refresh Link');
    await page.waitForFunction(() => window.devinFixture.attempts === 2);
    await page.evaluate(() => window.devinFixture.releaseStatus());
    await page.waitForTimeout(150);
    assert.equal(await card().getByText('Authorization successful. Credentials saved to Authentication Files.', { exact: true }).count(), 0);
    assert.ok(await card().getByRole('textbox').isVisible());
    assert.ok((await calls()).some(call => call.cmd === 'management_request' && call.args.request.path === '/oauth/session' && call.args.request.query.state === 'attempt-1'));
    await page.evaluate(() => { window.devinFixture.cancelError = true; });
    await click('Refresh Link');
    await page.getByText('Cancellation failed', { exact: false }).waitFor();
    assert.equal(await page.evaluate(() => window.devinFixture.attempts), 2);

    await open('view=easy');
    const easy = page.locator('.simple-mode-provider-card').filter({ hasText: 'Devin OAuth' });
    await easy.getByRole('button').click();
    await page.waitForFunction(() => window.devinFixture.attempts === 1);
    await page.evaluate(() => { window.devinFixture.status = 'ok'; });
    await page.waitForFunction(() => document.querySelectorAll('.simple-mode-provider-card.connected').length === 1);
    assert.ok((await calls()).some(call => call.cmd === 'start_oauth_login' && call.args.provider === 'devin'));

    await open('view=files');
    await page.getByText('devin-test.json', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'Fetch Quota', exact: true }).click();
    await page.getByText('75%', { exact: false }).first().waitFor();
    assert.equal(await page.locator('.devin-logo').count(), 1);

    await open('view=quota');
    await page.getByRole('button', { name: 'Fetch Quota', exact: true }).click();
    await page.getByText('Daily quota', { exact: true }).waitFor();
    assert.ok((await page.locator('.real-quota-card').innerText()).includes('Pro'));
    assert.equal(await page.locator('.real-quota-track').count(), 2);
    assert.equal(await page.locator('.real-quota-track span').last().evaluate(node => node.style.width), '0%');
    await page.evaluate(() => { window.devinFixture.quotaError = true; });
    await page.getByRole('button', { name: 'Refresh All', exact: true }).click();
    await page.getByText('Session expired', { exact: false }).waitFor();
    assert.equal(await page.locator('.real-quota-track').count(), 0);

    fs.mkdirSync('misc', { recursive: true });
    for (const theme of ['light', 'dark']) {
      await open('theme=' + theme + '&locale=zh-CN');
      await card().waitFor();
      await page.screenshot({ path: 'misc/devin-oauth-' + theme + '.png', fullPage: true });
      const filter = await page.locator('.devin-logo').evaluate(node => getComputedStyle(node).filter);
      assert.equal(filter, theme === 'dark' ? 'invert(1)' : 'none');
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    }
    assert.deepEqual(errors, []);
    console.log('Devin browser checks passed: OAuth, callback validation, session refresh, simplified login, auth files, quota and both themes.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
