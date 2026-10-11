const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const base = process.env.USAGE_TEST_URL || 'http://127.0.0.1:1439';
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    for (const width of [1342, 640]) {
      await page.setViewportSize({ width, height: 762 });
      await page.goto(`${base}/tests/fixtures/select-menu/index.html`, { waitUntil: 'domcontentloaded' });
      for (let i = 0; i < 5; i++) {
        const trigger = page.getByRole('button', { name: `Filter ${i}`, exact: true });
        await trigger.click();
        const menu = page.locator('.select-menu-dropdown');
        const t = await trigger.boundingBox(), m = await menu.boundingBox();
        assert.ok(Math.abs(t.x - m.x) < 2, `filter ${i} aligns at ${width}`);
        assert.ok(m.x >= 12 && m.x + m.width <= width - 11);
        assert.equal(await menu.evaluate(e => getComputedStyle(e).backgroundColor), 'rgb(23, 45, 67)');
        await page.keyboard.press('Escape');
      }
      await page.getByRole('button', { name: 'Filter 4', exact: true }).click();
      const list = page.locator('.select-menu-list');
      await list.hover();
      await page.mouse.wheel(0, 200);
      await page.waitForTimeout(150);
      assert.ok(await list.evaluate(e => e.scrollTop > 0), 'wheel scrolling persists');
      await list.evaluate(e => e.scrollTop = 400);
      await page.waitForTimeout(100);
      assert.equal(await list.evaluate(e => e.scrollTop), 400, 'scroll offset persists');
      await page.setViewportSize({ width, height: 700 });
      assert.equal(await list.evaluate(e => e.scrollTop), 400, 'resize preserves scroll offset');
      const before = await page.locator('.select-menu-dropdown').boundingBox();
      await page.locator('.fixture-scroll').evaluate(e => e.scrollTop = 15);
      await page.waitForTimeout(100);
      const after = await page.locator('.select-menu-dropdown').boundingBox();
      assert.ok(Math.abs(before.y - after.y - 15) < 2, 'menu follows ancestor scroll');
      assert.equal(await list.evaluate(e => e.scrollTop), 400);
      await page.getByRole('option', { name: 'Group 3: Option 79', exact: true }).click();
      assert.equal(await page.locator('output').textContent(), '79');
      assert.equal(await page.locator('.select-menu-dropdown').count(), 0);
      await page.keyboard.press('Enter');
      await page.locator('#outside').focus();
      assert.equal(await page.locator('.select-menu-dropdown').count(), 0, 'outside focus closes portal');
      await page.getByRole('button', { name: 'Filter 0', exact: true }).click();
      await page.locator('#outside').click();
      assert.equal(await page.locator('.select-menu-dropdown').count(), 0);
    }
    assert.deepEqual(errors, []);
    console.log('PASS: five filters, clipped/transformed containers, wheel and long-list selection, resize, ancestor scroll, scoped theme, outside focus/click.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });

