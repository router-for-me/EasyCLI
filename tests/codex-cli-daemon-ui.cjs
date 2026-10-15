const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const server = await createServer({ configFile: false, root: path.resolve(__dirname, '..'),
    plugins: [react()], logLevel: 'error', server: { host: '127.0.0.1', port: 0, watch: null } });
  let browser;
  try {
    await server.listen();
    const base = `http://127.0.0.1:${server.httpServer.address().port}`;
    const channel = process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined);
    browser = await chromium.launch(channel ? { channel, headless: true } : { headless: true });
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    const restart = () => page.getByRole('button', { name: 'Restart CLI Service', exact: true });
    const calls = command => page.evaluate(command => window.fixtureCalls.filter(call => call.cmd === command).length, command);
    const open = async query => {
      await page.goto(`${base}/tests/fixtures/agent-backups.html?locale=en&client=codex&reset-selections&${query}`);
      await page.waitForFunction(() => window.fixtureCalls.some(call => call.cmd === 'get_agent_models'));
      await page.getByRole('button', { name: 'Start CLI', exact: true }).waitFor();
    };
    await open('');
    await restart().click();
    const dialog = page.getByRole('alertdialog');
    await dialog.waitFor();
    assert.equal(await calls('restart_codex_cli_daemon'), 0);
    await page.evaluate(() => { window.fixtureDeferredCommands = ['restart_codex_cli_daemon']; });
    await dialog.getByRole('button', { name: 'Restart service', exact: true }).click();
    const waiting = page.getByRole('button', { name: 'Waiting for tasks', exact: true });
    await waiting.waitFor();
    assert.equal(await waiting.isDisabled(), true);
    assert.equal(await page.getByRole('button', { name: 'Start CLI', exact: true }).isDisabled(), true);
    assert.equal(await page.getByRole('button', { name: 'Start App', exact: true }).isDisabled(), false);
    await page.evaluate(() => window.fixtureRemount());
    await waiting.waitFor();
    assert.equal(await waiting.isDisabled(), true);
    assert.equal(await page.getByRole('button', { name: 'Start CLI', exact: true }).isDisabled(), true);
    await page.evaluate(() => {
      localStorage.setItem('cpa-gui.agent-selected-client.v1', 'opencode');
      window.fixtureRemount();
    });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.equal(await calls('restart_codex_cli_daemon'), 1);
    await page.evaluate(() => { window.fixtureDeferredCommands = []; window.fixturePendingCommands.splice(0).forEach(entry => entry.resolve()); });
    const restarted = page.getByText('Codex CLI background service restarted. Start Codex CLI again to use the latest configuration.', { exact: true });
    assert.equal(await restarted.count(), 0);
    await page.getByText('Codex', { exact: true }).click();
    await restarted.waitFor();
    assert.equal(await calls('restart_codex_cli_daemon'), 1);
    await page.evaluate(() => {
      window.fixtureDeferredCommands = ['restart_codex_cli_daemon'];
      window.fixtureFailedCommands = ['restart_codex_cli_daemon'];
    });
    await restart().click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Restart service', exact: true }).click();
    await page.getByRole('button', { name: 'Waiting for tasks', exact: true }).waitFor();
    await page.evaluate(() => {
      localStorage.setItem('cpa-gui.agent-selected-client.v1', 'opencode');
      window.fixtureRemount();
    });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.evaluate(() => {
      window.fixtureDeferredCommands = [];
      window.fixturePendingCommands.splice(0).forEach(entry => entry.resolve());
    });
    assert.equal(await page.getByText(/模拟后台刷新失败/).count(), 0);
    await page.getByText('Codex', { exact: true }).click();
    await page.getByText(/模拟后台刷新失败/).waitFor();
    await open('daemon-stopped');
    await restart().click();
    await page.getByText('The Codex CLI background service is not running. The latest configuration loads the next time Codex CLI starts.', { exact: true }).waitFor();
    assert.equal(await calls('restart_codex_cli_daemon'), 0);
    await open('daemon-error');
    await restart().click();
    await page.getByText(/Codex daemon probe failed/).waitFor();
    assert.equal(await page.getByRole('alertdialog').count(), 0);
    assert.equal(await calls('restart_codex_cli_daemon'), 0);
    assert.deepEqual(errors, []);
    console.log('PASS: shared daemon confirmation, non-blocking restart that survives remounts, stopped status, and probe errors.');
  } finally {
    await browser?.close();
    await server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
