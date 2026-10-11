const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const base = process.env.USAGE_TEST_URL || 'http://127.0.0.1:1427';
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage();
    for (const missing of [false, true]) {
      await page.goto(`${base}/tests/fixtures/usage-layout.html?tab=events&locale=zh-CN${missing ? '&noRequestKey' : ''}`);
      await page.locator('.usage-event-source').first().waitFor();
      const labels = await page.locator('.usage-event-source').allTextContents();
      assert.ok(labels.every(label => label === (missing ? '—' : 'Test key, "local"')));
      const source = page.locator('.usage-filter-item').nth(2).locator('.select-menu-trigger');
      await source.click();
      await page.getByRole('option', { name: 'Test source', exact: true }).click();
      await page.waitForFunction(() => document.querySelector('.usage-pagination-summary').textContent.includes('200'));
      assert.ok((await page.locator('.usage-event-source').allTextContents()).every(label => label === (missing ? '—' : 'Test key, "local"')));
    }
    console.log('PASS: source filtering stays independent; request key shows its note or an empty placeholder.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
