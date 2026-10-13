const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const base = process.env.USAGE_TEST_URL || 'http://127.0.0.1:1427';
(async () => {
 const browser = await chromium.launch({ channel: 'msedge', headless: true });
 try {
  const page = await browser.newPage({ viewport: { width: 1342, height: 762 } });
  const errors = []; page.on('pageerror', error => errors.push(String(error)));
  await page.goto(`${base}/tests/fixtures/usage-layout.html?tab=events&locale=zh-CN&theme=light`);
  const filters = page.locator('.usage-filter-item');
  assert.equal(await filters.count(), 5);
  assert.equal(await page.locator('.usage-th-provider').count(), 0);
  assert.equal(await page.locator('.usage-th-source').count(), 1);
  const upstream = filters.nth(2).locator('button').first();
  assert.equal(await upstream.getAttribute('aria-label'), '来源');
  assert.equal(await filters.nth(3).locator('button').first().getAttribute('aria-label'), '密钥');
  await upstream.click();
  assert.deepEqual(await page.locator('.select-menu-group-label').allTextContents(), []);
  await page.getByRole('option', { name: 'first@example.com', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.usage-pagination-summary').textContent.includes('200'));
  assert.ok((await page.locator('.usage-event-source').allTextContents()).every(text => text === 'first@example.com'));
  await upstream.click(); await page.getByRole('option', { name: '全部来源', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.usage-pagination-summary').textContent.includes('400'));
  await upstream.click();
  await page.keyboard.press('Escape');
  for (const width of [640, 1024]) {
   await page.setViewportSize({ width, height: 850 });
   assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  }
  assert.deepEqual(errors, []);
  console.log('PASS: five filters, plain source options, exact filtering, keyboard clear, default columns, narrow layout.');
 } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
