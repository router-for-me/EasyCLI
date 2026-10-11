const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const base = process.env.USAGE_TEST_URL || 'http://127.0.0.1:1427';

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1342, height: 762 } });
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    for (const tab of ['events', 'overview', 'analysis']) {
      await page.goto(`${base}/tests/fixtures/usage-layout.html?tab=${tab}&locale=en`);
      const keyButton = page.locator('.usage-filter-item').nth(3).locator('.select-menu-trigger');
      await keyButton.click();
      await page.getByRole('option', { name: 'Test key', exact: true }).click();
      await page.evaluate(() => {
        window.usageKeyFixture.apiKeys[0].label = 'Renamed key';
        window.usageKeyFixture.apiKeys.push({
          key: 'second-key', label: 'Second key', requests: 1, failures: 0, tokens: 1,
        });
      });
      await page.locator('.usage-refresh-btn').click();
      await page.waitForFunction(() =>
        document.querySelectorAll('.usage-filter-item')[3].textContent.includes('Renamed key'));
      await keyButton.click();
      assert.equal(await page.getByRole('option', { name: 'Test key', exact: true }).count(), 0);
      await page.getByRole('option', { name: 'Second key', exact: true }).waitFor();
      await page.keyboard.press('Escape');
      if (tab === 'events') {
        assert.equal(await page.evaluate(() => window.usageKeyFixture.lastEventKey), 'test-key');
      }
      if (tab === 'analysis') {
        await page.locator('.usage-analysis-context').getByText('Renamed key', { exact: true }).first().waitFor();
      }
      // Fixed time ranges still refresh options after the cache expires.
      await page.evaluate(() => {
        window.usageKeyFixture.apiKeys[0].label = 'Periodic rename';
        const now = Date.now;
        Date.now = () => now() + 31000;
      });
      await page.waitForFunction(() =>
        document.querySelectorAll('.usage-filter-item')[3].textContent.includes('Periodic rename'));
    }
    assert.deepEqual(errors, []);
    console.log('PASS: key rename/addition, manual and periodic refresh across events/overview/analysis, stable hash selection.');
  } finally {
    await browser.close();
  }
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
