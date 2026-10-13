const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const server = await createServer({
    configFile: false,
    root: path.resolve(__dirname, '..'),
    plugins: [react()],
    optimizeDeps: { entries: ['tests/fixtures/provider-manual-models.html'] },
    logLevel: 'error',
    server: { host: '127.0.0.1', port: 1421, strictPort: false, watch: null },
  });
  let browser;
  try {
    await server.listen();
    const base = `http://127.0.0.1:${server.httpServer.address().port}`;
    await (await fetch(base)).text();
    const channel = process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined);
    browser = await chromium.launch({ channel, headless: true, args: ['--no-proxy-server'] });
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    page.setDefaultTimeout(10000);
    const errors = [];
    page.on('pageerror', (error) => errors.push(String(error)));
    await page.route('**/*', (route) => route.request().url().startsWith(`${base}/`) ? route.continue() : route.abort());
    await page.goto(`${base}/tests/fixtures/provider-manual-models.html`, { waitUntil: 'domcontentloaded' });
    await page.locator('.provider-category-panel button').filter({ hasText: 'DeepSeek' }).click();
    await page.getByRole('button', { name: 'Add API Key', exact: true }).click();
    const form = page.getByRole('dialog', { name: 'Add API Key', exact: true });
    await form.getByLabel('API Key', { exact: true }).fill('test-manual-key');
    await form.getByLabel('Base URL', { exact: true }).fill('https://relay.example.test');
    await form.getByRole('button', { name: 'Save', exact: true }).click();
    await form.getByText('Add at least one model manually or select one from the model list', { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.providerFixture.writes.length), 0);
    await form.getByRole('button', { name: 'Add Custom Model', exact: true }).click();
    await form.getByRole('textbox', { name: 'Model name, for example gpt-4o', exact: true }).fill('custom-model');
    await form.getByRole('button', { name: 'Save', exact: true }).click();
    await form.waitFor({ state: 'detached' });
    assert.equal(await page.evaluate(() => window.providerFixture.discoveryCalls), 0);
    assert.equal(await page.evaluate(() => window.providerFixture.records.at(-1).keys[0].models[0].name), 'custom-model');
    await page.locator('.real-provider-row').getByRole('button', { name: 'Edit', exact: true }).click();
    const edit = page.getByRole('dialog', { name: 'Edit API Key', exact: true });
    await edit.getByLabel('Base URL', { exact: true }).fill('https://another-relay.example.test');
    await edit.getByLabel('API Key', { exact: true }).fill('test-updated-key');
    await edit.getByRole('button', { name: 'Fetch Models', exact: true }).click();
    const models = page.getByRole('dialog', { name: 'Select Models', exact: true });
    await models.getByText(/Model listing disabled/).waitFor();
    await page.keyboard.press('Escape');
    await models.waitFor({ state: 'detached' });
    const calls = await page.evaluate(() => window.providerFixture.discoveryCalls);
    assert.ok(calls > 0);
    await edit.getByRole('button', { name: 'Save', exact: true }).click();
    await edit.waitFor({ state: 'detached' });
    assert.equal(await page.evaluate(() => window.providerFixture.discoveryCalls), calls);
    const saved = await page.evaluate(() => window.providerFixture.records.at(-1));
    assert.equal(saved.keys[0].models[0].name, 'custom-model');
    assert.equal(saved['base-url'], 'https://another-relay.example.test');
    assert.deepEqual(errors, []);
    console.log('Manual provider models: create without discovery, empty validation, edit after failed discovery passed');
  } finally {
    await browser?.close();
    await server.close();
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });
