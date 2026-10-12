const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const { mkdirSync } = require('node:fs');

const screenshots = path.resolve(__dirname, '..', 'output', 'playwright');
mkdirSync(screenshots, { recursive: true });

(async () => {
  const { createServer } = await import('vite');
  const react = (await import('@vitejs/plugin-react')).default;
  const server = await createServer({ configFile: false, root: path.resolve(__dirname, '..'), plugins: [react()],
    logLevel: 'error', server: { host: '127.0.0.1', port: 0, watch: null } });
  let browser;
  try {
    await server.listen();
    const base = `http://127.0.0.1:${server.httpServer.address().port}`;
    browser = await chromium.launch({ channel: process.platform === 'win32' ? 'msedge' : undefined, headless: true, args: ['--no-proxy-server'] });
    const page = await browser.newPage({ viewport: { width: 1600, height: 1100 } });
    page.setDefaultTimeout(15000);
    const errors = [];
    const externalRequests = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.route('**/*', route => {
      if (route.request().url().startsWith(`${base}/`)) return route.continue();
      externalRequests.push(route.request().url());
      return route.abort();
    });
    await page.addInitScript(() => {
      localStorage.setItem('easy-cli-proxy-api.locale', 'en');
      localStorage.setItem('cpa-gui.agent-selected-client.v1', 'codex');
    });
    await page.goto(`${base}/?mock=running&mockDelay=5`);
    await page.locator('.app-shell').waitFor();
    await page.evaluate(() => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      window.contextReadRequests = [];
      window.__TAURI_INTERNALS__.invoke = (command, args, options) => {
        if (command === 'get_codex_session_context') window.contextReadRequests.push(args.request.sessionId);
        return invoke(command, args, options);
      };
    });
    await page.locator('.nav-section button').nth(4).click();
    await page.locator('#agent-subpage-tab-sessions').click();
    await page.locator('.codex-sessions-page').waitFor();

    const entry = page.locator('.codex-session-context-button').first();
    for (const [language, title] of [
      ['English', 'Edit Context (Experimental)'],
      ['简体中文', '编辑上下文 (实验性)'],
      ['日本語', 'コンテキスト編集 (実験的)'],
    ]) {
      await page.locator('.sidebar-language-trigger').click();
      await page.getByRole('option', { name: language, exact: true }).click();
      assert.equal(await entry.getAttribute('title'), title);
      assert.equal((await entry.textContent()).trim(), title);
    }
    await page.locator('.sidebar-language-trigger').click();
    await page.getByRole('option', { name: 'English', exact: true }).click();

    assert.deepEqual(await page.evaluate(() => window.contextReadRequests), []);
    await page.locator('.codex-sessions-page').getByRole('button', { name: 'Refresh', exact: true }).click();
    await entry.waitFor();
    assert.deepEqual(await page.evaluate(() => window.contextReadRequests), []);
    await page.locator('.codex-session-context-button').first().click();
    await page.locator('.codex-session-context-editor').waitFor();
    await page.locator('.context-message-card').first().waitFor();
    assert.equal((await page.evaluate(() => window.contextReadRequests)).length, 1);
    assert.equal(await page.locator('.context-metrics-grid').count(), 0);
    assert.equal(await page.getByRole('button', { name: 'Toggle session metadata', exact: true }).getAttribute('aria-expanded'), 'false');
    assert.equal(await page.getByLabel('Session Title', { exact: true }).count(), 0);
    assert.equal(await page.locator('.context-role-filters .filter-chip').count(), 5);
    await page.screenshot({ path: path.join(screenshots, 'codex-context-collapsed.png'), fullPage: true, animations: 'disabled' });

    const userCard = page.locator('.context-message-card.user').first();
    const toolCard = page.locator('.context-message-card.tool').first();
    await userCard.waitFor();
    assert.equal(await toolCard.getByRole('button', { name: 'Edit', exact: true }).count(), 0);
    assert.equal(await userCard.getByRole('button', { name: 'Edit', exact: true }).count(), 1);

    await userCard.getByRole('button', { name: 'Edit', exact: true }).click();
    await userCard.locator('.message-textarea').fill('Mock persistence check');
    for (const navigationButton of [
      page.locator('.nav-section button').first(),
      page.locator('#agent-subpage-tab-core'),
      page.locator('.agent-list-items button:not(.active)').first(),
    ]) {
      await navigationButton.click();
      await page.getByRole('alertdialog').waitFor();
      await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel', exact: true }).click();
      assert.equal(await userCard.locator('.message-textarea').inputValue(), 'Mock persistence check');
    }
    await page.locator('.sidebar-language-trigger').click();
    await page.getByRole('option', { name: '简体中文', exact: true }).click();
    assert.equal(await userCard.locator('.message-textarea').inputValue(), 'Mock persistence check');
    await page.locator('.sidebar-language-trigger').click();
    await page.getByRole('option', { name: 'English', exact: true }).click();
    assert.equal(await userCard.locator('.message-textarea').inputValue(), 'Mock persistence check');
    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    await page.locator('.codex-session-notice.success').waitFor();

    await page.getByRole('button', { name: 'Back to Sessions', exact: true }).click();
    await page.locator('.codex-sessions-page').waitFor();
    await page.locator('.codex-session-context-button').first().click();
    await page.locator('.context-message-card.user', { hasText: 'Mock persistence check' }).waitFor();

    assert.equal(await page.locator('.raw-jsonl-textarea').count(), 0);
    await page.getByRole('button', { name: 'Toggle session metadata', exact: true }).click();
    const sessionId = await page.locator('.context-id-badge').getAttribute('title');
    assert.ok((await page.evaluate(() => window.contextReadRequests)).every(id => id === sessionId));
    await page.evaluate(() => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      window.rolloutOpenRequests = [];
      window.__TAURI_INTERNALS__.invoke = (command, args, options) => {
        if (command === 'open_codex_session_rollout') {
          window.rolloutOpenRequests.push(args);
          if (window.failRolloutOpen) return Promise.reject('No default application is configured');
        }
        return invoke(command, args, options);
      };
    });
    const openRaw = page.getByRole('button', { name: 'Edit with local app', exact: true });
    await openRaw.click();
    await page.waitForFunction(() => window.rolloutOpenRequests.length === 1);
    assert.deepEqual(await page.evaluate(() => window.rolloutOpenRequests), [{ request: { sessionId } }]);
    await page.evaluate(() => { window.failRolloutOpen = true; });
    await openRaw.click();
    await page.locator('.codex-session-notice.error', { hasText: 'No default application is configured' }).waitFor();
    assert.equal(await openRaw.isEnabled(), true);
    await page.evaluate(() => { window.failRolloutOpen = false; });
    assert.equal(await page.getByRole('button', { name: 'Save Changes', exact: true }).isEnabled(), false);
    await page.getByLabel('Session Title', { exact: true }).fill('Structured metadata title');
    assert.equal(await openRaw.isEnabled(), false);
    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    await page.locator('.codex-session-notice.success').waitFor();
    assert.equal(await openRaw.isEnabled(), true);
    await page.locator('.context-message-card.tool').first().getByRole('button', { name: 'Remove Message', exact: true }).click();
    assert.equal(await page.locator('.context-message-card.tool.deleted').count(), 1);
    await page.keyboard.press('Control+s');
    await page.locator('.codex-session-notice.success').waitFor();
    assert.equal(await page.locator('.context-message-card.tool').count(), 0);

    await page.getByRole('button', { name: 'Add Message', exact: true }).click();
    await page.locator('.add-message-textarea').fill('Unstaged draft can be saved');
    await page.getByRole('button', { name: 'Back to Sessions', exact: true }).click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.keyboard.press('Control+s');
    await page.locator('.codex-session-notice.success').waitFor();
    await page.locator('.context-message-card.user', { hasText: 'Unstaged draft can be saved' }).waitFor();

    for (const message of await page.locator('.context-message-card').all()) {
      await message.getByRole('button', { name: 'Remove Message', exact: true }).click();
    }
    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    await page.locator('.codex-session-notice.success').waitFor();
    assert.equal(await page.locator('.context-message-card').count(), 0);
    await page.getByRole('button', { name: 'Add Message', exact: true }).click();
    await page.locator('.add-message-textarea').fill('Add to an empty session');
    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    await page.locator('.codex-session-notice.success').waitFor();
    await page.locator('.context-message-card.user', { hasText: 'Add to an empty session' }).waitFor();

    for (const theme of ['dark', 'light']) {
      await page.locator('.sidebar-theme-selector button').nth(theme === 'dark' ? 1 : 0).click();
      await page.waitForFunction(expected => document.documentElement.dataset.theme === expected, theme);
      await page.locator('.sidebar-theme-selector button[aria-pressed="true"]').waitFor();
      const background = await page.locator('.context-meta-card').evaluate(card => getComputedStyle(card).backgroundColor);
      assert.equal(background, theme === 'dark' ? 'rgb(20, 23, 32)' : 'rgb(255, 255, 255)');
      await page.screenshot({ path: path.join(screenshots, `codex-session-context-${theme}.png`), fullPage: true, animations: 'disabled' });
    }

    for (const width of [900, 640]) {
      await page.setViewportSize({ width, height: 1100 });
      const fits = await page.locator('.codex-session-context-editor').evaluate(editor => editor.scrollWidth <= editor.clientWidth);
      assert.ok(fits, `Context editor overflows at ${width}px`);
    }
    await page.setViewportSize({ width: 1600, height: 1100 });

    await page.getByLabel('Session Title', { exact: true }).fill('Discarded title');
    await page.locator('.nav-section button').first().click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Discard', exact: true }).click();
    await page.locator('.codex-session-context-editor').waitFor({ state: 'hidden' });

    await page.evaluate(() => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      window.contextMessageCount = 43;
      window.__TAURI_INTERNALS__.invoke = async (command, args, options) => {
        const result = await invoke(command, args, options);
        if (command === 'get_codex_session_context') {
          result.messages = Array.from({ length: window.contextMessageCount }, (_, index) => ({
            id: `layout-message-${index}`, lineNumber: index,
            role: ['user', 'assistant', 'reasoning', 'tool'][index % 4],
            content: index % 5 ? `Message ${index}` : 'Long message\n'.repeat(40),
            rawType: 'response_item', timestamp: '2026-10-07T12:00:00Z', model: 'mock-model',
          }));
          for (const [role, key] of [['user', 'userMessageCount'], ['assistant', 'assistantMessageCount'],
            ['reasoning', 'reasoningCount'], ['tool', 'toolCount']]) {
            result.stats[key] = result.messages.filter(message => message.role === role).length;
          }
          result.stats.messageCount = result.messages.length - result.stats.toolCount;
        }
        return result;
      };
    });
    await page.locator('.nav-section button').nth(4).click();
    await page.locator('#agent-subpage-tab-sessions').click();
    await page.locator('.codex-session-context-button').first().click();
    for (const count of [43, 1000]) {
      await page.evaluate(count => { window.contextMessageCount = count; }, count);
      await page.getByRole('button', { name: 'Reload', exact: true }).click();
      await page.waitForFunction(count => document.querySelectorAll('.context-message-card').length === count, count);
      for (const width of [1600, 640]) {
        await page.setViewportSize({ width, height: 800 });
        for (const role of ['all', 'user', 'assistant', 'reasoning', 'tool']) {
          await page.locator(`.context-role-filters .filter-chip.${role}`).click();
          const clipped = await page.locator('.context-message-card').evaluateAll(cards => cards.filter(card => {
            const bottom = card.getBoundingClientRect().bottom;
            return ['.message-card-header', '.message-card-body'].some(selector =>
              card.querySelector(selector).getBoundingClientRect().bottom > bottom - 8);
          }).length);
          assert.equal(clipped, 0, `Clipped ${role} cards with ${count} messages at ${width}px`);
        }
      }
      await page.locator('.context-role-filters .filter-chip.all').click();
      await page.locator('.context-message-list').evaluate(list => { list.scrollTop = list.scrollHeight; });
      const lastCardFits = await page.locator('.context-message-list').evaluate(list => {
        const bounds = list.getBoundingClientRect();
        const last = list.querySelector('.context-message-card:last-of-type').getBoundingClientRect();
        return last.top >= bounds.top && last.bottom <= bounds.bottom;
      });
      assert.ok(lastCardFits, 'The last message must be reachable by scrolling');
    }
    await page.setViewportSize({ width: 1600, height: 1100 });
    await page.locator('.sidebar-theme-selector button').nth(1).click();
    await page.locator('.sidebar-language-trigger').click();
    await page.getByRole('option', { name: '简体中文', exact: true }).click();
    await page.locator('.context-role-filters .filter-chip.assistant').click();
    await page.locator('.context-message-list').evaluate(list => { list.scrollTop = 0; });
    await page.locator('.context-message-card').first().scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(screenshots, 'codex-context-many-messages-fixed.png'), animations: 'disabled' });

    assert.deepEqual(errors, []);
    assert.deepEqual(externalRequests, []);
    console.log('PASS: No context preloading; one target read on entry; collapsed metadata; saves, draft guards, themes and unclipped 43/1000-message lists.');
  } finally { await browser?.close(); await server.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
