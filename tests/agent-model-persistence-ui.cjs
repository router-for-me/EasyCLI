// Run Vite on port 1421, then node tests/agent-model-persistence-ui.cjs.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    for (const client of ['codex', 'antigravity-cli']) {
      for (const embedded of [false, true]) {
        const page = await browser.newPage();
        const errors = [];
        page.on('pageerror', error => errors.push(String(error)));
        const url = `http://127.0.0.1:1421/tests/fixtures/agent-backups.html?client=${client}${embedded ? '&embedded' : ''}`;
        await page.goto(url);
        const picker = page.locator('.agent-model-trigger');
        const missing = page.getByText('当前选择已不在可用模型列表中，请刷新模型后重新选择', { exact: true });
        const update = page.getByRole('button', { name: '更新配置', exact: true });
        const saved = () => page.evaluate(client => JSON.parse(localStorage.getItem('cpa-gui.agent-model-selections.v1'))[client], client);
        const assertSelected = async () => {
          assert.equal(await picker.locator('strong').textContent(), 'gpt-two');
          assert.equal(await saved(), 'gpt-two');
        };
        await page.waitForFunction(() => document.querySelector('.agent-model-trigger strong')?.textContent === 'gpt-one');
        await picker.click();
        await page.getByRole('option', { name: 'gpt-two' }).click();

        // A successful refresh with empty or incomplete results must not erase the choice.
        for (const models of [[], [{ name: 'gpt-one' }]]) {
          await page.evaluate(models => {
            window.fixtureModelsOverride = models;
            window.fixtureRemount();
          }, models);
          await missing.waitFor();
          await assertSelected();
          assert.equal(await update.isDisabled(), true);
        }

        // Full reload discards module caches; the stored choice must still survive.
        await page.addInitScript(() => { window.fixtureModelsOverride = []; });
        await page.reload();
        await missing.waitFor();
        await assertSelected();
        assert.equal(await update.isDisabled(), true);

        await page.evaluate(() => {
          window.fixtureModelsOverride = [{ name: 'gpt-one' }, { name: 'gpt-two' }];
          window.fixtureRemount();
        });
        await missing.waitFor({ state: 'hidden' });
        await assertSelected();
        assert.equal(await update.isEnabled(), true);
        assert.equal(await page.evaluate(() => window.fixtureCalls.some(call =>
          ['update_agent_config', 'set_agent_config_enabled'].includes(call.cmd))), false);
        assert.deepEqual(errors, []);
        await page.close();
      }
    }
    console.log('PASS: Codex and Antigravity preserve selections across empty/partial lists, remount, reload and recovery in both page layouts');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exit(1); });
