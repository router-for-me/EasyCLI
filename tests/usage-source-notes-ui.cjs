const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const base = process.env.USAGE_TEST_URL || 'http://127.0.0.1:1427';
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1342, height: 850 } });
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.addInitScript(() => localStorage.setItem('cpa-gui.usage-events-visible-cols.v6', JSON.stringify(['time', 'source', 'key', 'result'])));
    for (const [locale, sourceName, keyName] of [
      ['zh-CN', '来源', '密钥'],
      ['en', 'Source', 'Key'],
      ['ja', 'ソース', 'キー'],
    ]) {
      for (const missing of [false, true]) {
        await page.goto(`${base}/tests/fixtures/usage-layout.html?tab=events&locale=${locale}${missing ? '&noRequestKey' : ''}`);
        await page.locator('.usage-event-source').first().waitFor();
        assert.equal(await page.locator('.usage-th-source .usage-th-content').textContent(), sourceName);
        assert.equal(await page.locator('.usage-th-key .usage-th-content').textContent(), keyName);
        assert.equal(await page.locator('.usage-event-source').first().textContent(), 'first@example.com');
        assert.equal(await page.locator('.usage-td-key strong').first().textContent(), missing ? '—' : 'sk-test');
        if (!missing) assert.equal(await page.locator('.usage-td-key small').first().textContent(), 'Test key, "local"');
        await page.locator('.usage-col-settings-btn').click();
        const dialog = page.getByRole('dialog');
        assert.equal(await dialog.getByRole('checkbox', { name: sourceName, exact: true }).count(), 1);
        assert.equal(await dialog.getByRole('checkbox', { name: keyName, exact: true }).count(), 1);
        await page.keyboard.press('Escape');
        const source = page.locator('.usage-filter-item').nth(2).locator('.select-menu-trigger');
        await source.click();
        await page.getByRole('option', { name: 'first@example.com', exact: true }).click();
        await page.waitForFunction(() => document.querySelector('.usage-pagination-summary').textContent.includes('200'));
        assert.equal(await page.evaluate(() => window.usageKeyFixture.lastEventUpstream), 'auth:auth-primary');
        assert.ok((await page.locator('.usage-event-source').allTextContents()).every(label => label === 'first@example.com'));
      }
    }
    assert.deepEqual(errors, []);
    console.log('PASS: distinct upstream/client key data, independent filtering, column settings and labels in three languages, including missing client keys.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
