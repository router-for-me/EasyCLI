const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

(async () => {
  const body = fs.readFileSync(path.resolve(__dirname, '../src-tauri/src/plugin_webview_script.js'), 'utf8');
  const script = `(function(){var O="http://127.0.0.1:9",K="key",L={back:"Back",reload:"Reload",open:"Open",move:"Move"};${body}})();`;
  const browser = await chromium.launch({ args: ['--no-proxy-server'] });
  try {
    const context = await browser.newContext({ viewport: { width: 800, height: 600 }, hasTouch: true });
    const page = await context.newPage();
    await page.route('**/*', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><meta http-equiv="Content-Security-Policy" content="style-src \'none\'"><body><script>window.opened=[];window.open=(u)=>window.opened.push(u);</script></body>' }));
    await page.addInitScript(`window.__shadow=null;const a=Element.prototype.attachShadow;Element.prototype.attachShadow=function(i){const r=a.call(this,{...i,mode:'open'});window.__shadow=r;return r}`);
    await page.addInitScript(script);
    await page.goto('http://127.0.0.1:9/a');
    const rect = (label) => page.evaluate((name) => {
      const node = name ? window.__shadow.querySelector(`button[aria-label="${name}"]`) : window.__shadow.firstChild;
      const box = node.getBoundingClientRect();
      return { x: box.x, y: box.y, w: box.width, h: box.height };
    }, label);
    const labels = await page.evaluate(() => [...window.__shadow.querySelectorAll('button')].map((b) => `${b.title}|${b.getAttribute('aria-label')}`));
    assert.deepEqual(labels, ['Move|Move', 'Back|Back', 'Reload|Reload', 'Open|Open']);
    let bar = await rect();
    assert.equal(Math.round(bar.x + bar.w), 792);
    assert.equal(Math.round(bar.y + bar.h), 592, 'defaults to the bottom-right corner');

    const grip = await rect('Move');
    const cx = grip.x + grip.w / 2;
    const cy = grip.y + grip.h / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx - 500, cy - 800, { steps: 8 });
    await page.mouse.up();
    bar = await rect();
    assert.equal(Math.round(bar.x) < 400, true);
    assert.equal(Math.round(bar.y), 0, 'clamped to the viewport top');
    const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('cpa-gui.plugin-controls')));
    assert.equal(stored.h, 'left');
    assert.equal(stored.v, 'top');
    assert.equal(await page.evaluate(() => localStorage.getItem('cli-proxy-auth')), '{"state":{"managementKey":"key"}}', 'the position never touches the auth entry');

    const reloadBox = await rect('Reload');
    await page.mouse.move(reloadBox.x + 12, reloadBox.y + 12);
    await page.mouse.down();
    await page.mouse.move(reloadBox.x + 40, reloadBox.y + 12, { steps: 4 });
    await page.mouse.up();
    assert.equal(await page.evaluate(() => performance.getEntriesByType('navigation').length), 1);
    const openBox = await rect('Open');
    await page.mouse.click(openBox.x + 12, openBox.y + 12);
    assert.equal(await page.evaluate(() => window.opened.length), 1, 'a click without movement still fires');

    await page.setViewportSize({ width: 400, height: 300 });
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    bar = await rect();
    assert.equal(Math.round(bar.y), 0, 'stays anchored to the top edge');
    assert.equal(bar.x >= 0 && bar.x + bar.w <= 400, true);

    await page.reload();
    const restored = await rect();
    assert.equal(Math.round(restored.x), Math.round(bar.x));

    await page.mouse.click(1, 1);
    await page.evaluate(() => window.__shadow.querySelector('button[aria-label="Move"]').focus());
    const before = await rect();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Shift+ArrowDown');
    const after = await rect();
    assert.equal(Math.round(after.x - before.x), 8);
    assert.equal(Math.round(after.y - before.y), 40, 'shift-arrow moves by the larger step');

    await page.evaluate(() => window.__shadow.querySelector('button[aria-label="Move"]').dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    const reset = await rect();
    assert.equal(Math.round(reset.x + reset.w), 392);
    assert.equal(Math.round(reset.y + reset.h), 292, 'reset returns to the bottom-right corner');
    assert.equal(await page.evaluate(() => localStorage.getItem('cpa-gui.plugin-controls')), null);
    console.log('PASS: injected plugin controls drag, clamp, persist, move by keyboard and reset.');
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
