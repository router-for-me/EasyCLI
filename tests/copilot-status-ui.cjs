const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const base = process.env.AGENT_CLIENT_TEST_URL || 'http://127.0.0.1:1421';

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    for (const embedded of [false, true]) {
      for (const pluginInstalled of [false, true]) {
        const page = await browser.newPage();
        await page.addInitScript(installed => {
          window.fixtureClientStatusesOverride = { 'vscode-copilot': {
            installed: true, version: '1.141.0', appVersion: '1.141.0', cliVersion: null,
            pluginInstalled: installed, pluginVersion: installed ? '0.40.2' : null,
          } };
        }, pluginInstalled);
        await page.goto(`${base}/tests/fixtures/agent-backups.html?client=vscode-copilot${embedded ? '&embedded' : ''}`);
        await page.getByText('Copilot 配置时需要一个临时窗口，完成后会自动关闭。', { exact: true }).waitFor();
        assert.equal(await page.getByText('一次性配置：操作说明与排查指令', { exact: true }).count(), 0);
        const plugin = page.getByTestId('copilot-plugin-status');
        await plugin.waitFor();
        assert.equal(await plugin.locator('strong').textContent(), pluginInstalled ? '0.40.2' : '未安装');
        assert.ok((await page.locator('body').innerText()).includes('1.141.0'));
        assert.ok((await page.locator('body').innerText()).includes('已检测到客户端'));
        assert.equal(await page.getByRole('button', { name: '启动 vscode-copilot', exact: true }).isEnabled(), true);
        await page.getByRole('button', { name: '启动 vscode-copilot', exact: true }).click();
        const dialog = page.getByRole('dialog');
        await dialog.waitFor();
        const confirm = dialog.getByRole('button', { name: '启动 VSCode Copilot', exact: true });
        assert.equal(await confirm.isDisabled(), true);
        assert.equal(await page.evaluate(() => window.fixtureCalls.filter(call => call.cmd === 'launch_agent').length), 0);
        const directory = 'E:\\项目 files\\Copilot workspace';
        await page.evaluate(value => { window.fixtureSelectedProgram = value; }, directory);
        await dialog.locator('.agent-launch-directory-picker').click();
        await confirm.click();
        await dialog.waitFor({ state: 'hidden' });
        const launch = await page.evaluate(() => window.fixtureCalls.find(call => call.cmd === 'launch_agent'));
        assert.equal(launch.args.client, 'vscode-copilot');
        assert.equal(launch.args.target, 'app');
        assert.equal(launch.args.workingDirectory, directory);
        await page.getByRole('button', { name: '启动 vscode-copilot', exact: true }).click();
        await dialog.waitFor();
        assert.equal(await dialog.locator('.agent-launch-directory-picker strong').textContent(), directory);
        assert.equal(await dialog.locator('.agent-launch-directory-history button').count(), 1);
        await dialog.getByRole('button', { name: '取消', exact: true }).click();
        assert.equal(await page.evaluate(() => window.fixtureCalls.filter(call => call.cmd === 'launch_agent').length), 1);
        if (!embedded && pluginInstalled) await page.screenshot({ path: '.codex/vscode-copilot-launch.png', fullPage: true });
        await page.setViewportSize({ width: 390, height: 844 });
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
        if (!embedded && pluginInstalled) await page.screenshot({ path: '.codex/vscode-copilot-setup-mobile.png', fullPage: true });
        await page.close();
      }
    }
    console.log('PASS: VS Code status, startup directory, remembered history and cancellation in both layouts');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exit(1); });
