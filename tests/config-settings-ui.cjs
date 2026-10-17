const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');

const categories = ['general', 'aliases', 'routing', 'requests', 'oauth', 'diagnostics', 'software'];
const viewports = [{ width: 640, height: 600 }, { width: 964, height: 700 }, { width: 1280, height: 800 }, { width: 1600, height: 1000 }];

(async () => {
  let server;
  let browser;
  let base = 'http://127.0.0.1:1420/';
  const runtimeErrors = [];
  try {
    try {
      const response = await fetch(base, { signal: AbortSignal.timeout(3000) });
      if (!response.ok) throw new Error(String(response.status));
    } catch {
      const { createServer } = await import('vite');
      const react = (await import('@vitejs/plugin-react')).default;
      server = await createServer({
        configFile: false,
        root: path.resolve(__dirname, '..'),
        plugins: [react()],
        optimizeDeps: { entries: ['index.html'] },
        logLevel: 'error',
        server: { host: '127.0.0.1', port: 1421, strictPort: false },
      });
      await server.listen();
      base = `http://127.0.0.1:${server.httpServer.address().port}/`;
    }

    const channel = process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined);
    browser = await chromium.launch({ channel, headless: true, args: ['--no-proxy-server'] });

    for (const locale of ['zh-CN', 'en']) {
      for (const viewport of viewports) {
        const label = `${locale} ${viewport.width}x${viewport.height}`;
        const page = await browser.newPage({ viewport });
        page.setDefaultTimeout(10000);
        page.setDefaultNavigationTimeout(60000);
        page.on('pageerror', error => runtimeErrors.push(`${label}: ${error}`));
        await page.route('**/*', route => route.request().url().startsWith(base) ? route.continue() : route.abort());
        await page.addInitScript(language => localStorage.setItem('easy-cli-proxy-api.locale', language), locale);
        await page.goto(`${base}?mock=running`, { waitUntil: 'domcontentloaded' });
        await page.locator('.app-shell').waitFor();
        await page.addStyleTag({ content: '#browser-mock-toolbar { display: none; }' });
        await page.locator('.nav-section').getByRole('button', {
          name: locale === 'en' ? 'Advanced Features' : '高级功能', exact: true,
        }).click();
        await page.locator('#config-subpage-panel').waitFor();

        const settle = () => page.evaluate(async () => {
          await document.fonts.ready;
          await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        });
        const tab = async id => {
          await page.locator(`#config-subpage-tab-${id}`).click();
          await page.locator(`#config-subpage-panel[aria-labelledby="config-subpage-tab-${id}"]`).waitFor();
          assert.equal(await page.locator(`#config-subpage-tab-${id}`).getAttribute('aria-selected'), 'true');
          await settle();
        };
        const templateVisible = async (id, visible = true) => {
          assert.equal(await page.locator(`#template-group-${id}`).isVisible(), visible, `${label}: ${id} visibility`);
        };
        const noOverflow = async context => {
          await settle();
          const overflowing = await page.evaluate(() => [
            document.documentElement, document.body, document.querySelector('.content'),
            document.querySelector('.config-page'), document.querySelector('#config-subpage-panel'),
            document.querySelector('.config-search-results'),
          ].filter(Boolean).filter(element => element.scrollWidth > element.clientWidth + 1)
            .map(element => ({ element: element.className || element.tagName, width: element.clientWidth, scroll: element.scrollWidth })));
          assert.deepEqual(overflowing, [], `${label} ${context}: settings must fit without horizontal overflow`);
          const narrowLists = await page.evaluate(() => {
            const width = document.querySelector('.config-settings-content').getBoundingClientRect().width;
            return [...document.querySelectorAll('#config-native-keys, #config-native-aliases, #config-native-sensitive-words, .template-config-card:has([data-field-type="custom"], [data-field-type="json"], [data-field-type="string-list"])')]
              .filter(card => card.id !== 'config-group-extensions-plugins' && card.getBoundingClientRect().width > 0 && Math.abs(card.getBoundingClientRect().width - width) > 1)
              .map(card => card.id);
          });
          assert.deepEqual(narrowLists, [], `${label} ${context}: expandable lists and rules occupy independent full rows`);
          if (viewport.width >= 1240) {
            const incompleteRows = await page.evaluate(() => {
              const content = document.querySelector('.config-settings-content').getBoundingClientRect();
              const cards = [...document.querySelectorAll('.config-settings-cards > section, .template-config-card, #config-native-aliases, #config-native-sensitive-words')]
                .map(card => ({ id: card.id, box: card.getBoundingClientRect() })).filter(card => card.box.width > 0);
              return cards.filter(card => {
                if (['config-native-software', 'config-group-extensions-plugins'].includes(card.id)) {
                  return Math.abs(card.box.left + card.box.width / 2 - content.left - content.width / 2) > 1;
                }
                const peers = cards.filter(peer => Math.abs(peer.box.top - card.box.top) < 2);
                return Math.abs(Math.min(...peers.map(peer => peer.box.left)) - content.left) > 1
                  || Math.abs(Math.max(...peers.map(peer => peer.box.right)) - content.right) > 1;
              }).map(card => card.id);
            });
            assert.deepEqual(incompleteRows, [], `${label} ${context}: card rows must not leave an orphan half-width card`);
          }
        };
        const reachable = async (locator, context) => {
          await locator.scrollIntoViewIfNeeded();
          await locator.evaluate(element => element.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'instant' }));
          await settle();
          const metrics = await locator.evaluate(element => {
            const bounds = element.getBoundingClientRect();
            const hit = document.elementFromPoint(bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
            return {
              reachable: bounds.width > 0 && bounds.height > 0 && bounds.left >= -1 && bounds.right <= innerWidth + 1
                && bounds.top >= -1 && bounds.bottom <= innerHeight + 1 && Boolean(hit && element.contains(hit)),
              bounds: { left: bounds.left, right: bounds.right, top: bounds.top, bottom: bounds.bottom },
              hit: hit?.className || hit?.tagName,
            };
          });
          if (!metrics.reachable && process.env.SETTINGS_SCREENSHOT_DIR) {
            const fs = require('node:fs');
            fs.mkdirSync(path.resolve(process.env.SETTINGS_SCREENSHOT_DIR), { recursive: true });
            await page.screenshot({ path: path.join(path.resolve(process.env.SETTINGS_SCREENSHOT_DIR), 'settings-reachability-failure.png') });
          }
          assert.equal(metrics.reachable, true, `${label} ${context}: control must remain reachable: ${JSON.stringify(metrics)}`);
        };
        const hasDirtyMarker = async id => page.locator(`#config-subpage-tab-${id}`).evaluate(element =>
          Boolean(element.querySelector('.config-nav-dirty, .config-settings-dirty, [data-dirty="true"]'))
          || /未保存|unsaved/i.test([element.getAttribute('aria-label'), element.getAttribute('title'), element.textContent].join(' ')));
        const search = page.getByLabel(locale === 'en' ? 'Search settings' : '搜索设置', { exact: true });
        const retrySection = page.locator('#config-retry-section-title').locator('xpath=ancestor::section[1]');
        const exerciseHelp = async (trigger, context) => {
          await reachable(trigger, `${context} help trigger`);
          const controlledId = await trigger.getAttribute('aria-controls');
          assert.ok(controlledId, `${label}: help triggers must identify their description`);
          const popover = page.locator(`[id="${controlledId}"]`);
          assert.equal(await trigger.getAttribute('aria-expanded'), 'false', `${label}: help starts collapsed`);
          assert.equal(await popover.isVisible(), false, `${label}: descriptions stay hidden until requested`);
          await trigger.click();
          await popover.waitFor({ state: 'visible' });
          assert.equal(await trigger.getAttribute('aria-expanded'), 'true');
          assert.equal(await popover.getAttribute('role'), 'tooltip');
          assert.ok((await popover.innerText()).trim().length > 5, `${label}: expanded help must contain useful field guidance`);
          await noOverflow(`${context} help open`);
          await trigger.focus();
          await page.keyboard.press('Escape');
          assert.equal(await trigger.getAttribute('aria-expanded'), 'false', `${label}: Escape closes requested help`);
          await page.keyboard.press('Enter');
          assert.equal(await trigger.getAttribute('aria-expanded'), 'true', `${label}: Enter opens field help`);
          await page.keyboard.press('Space');
          assert.equal(await trigger.getAttribute('aria-expanded'), 'false', `${label}: Space closes field help`);
          await trigger.click();
          await search.click();
          assert.equal(await trigger.getAttribute('aria-expanded'), 'false', `${label}: clicking outside closes field help`);
        };

        assert.equal(await page.getByRole('heading', { level: 1, name: locale === 'en' ? 'Advanced settings' : '高级设置', exact: true }).count(), 0);
        assert.equal(await page.locator('.config-page [role="tabpanel"]').count(), 1);
        assert.equal(await page.locator('.config-settings-header p').count(), 0,
          `${label}: settings header must not repeat layout explanations`);
        assert.equal(await page.locator('.config-nav-save-hint, .config-category-heading, .config-section-index').count(), 0,
          `${label}: categories should lead directly to their settings without redundant descriptions or jump navigation`);
        assert.deepEqual(await page.locator('.config-subpage-tabs [role="tab"]').evaluateAll(elements =>
          elements.map(element => element.id.replace('config-subpage-tab-', ''))), categories);
        assert.equal(await page.locator('.config-subpage-tabs [role="tab"]').evaluateAll(elements =>
          elements.every(element => element.getAttribute('aria-controls') === 'config-subpage-panel')), true);
        const navigation = page.locator('.config-subpage-tabs');
        assert.equal(await page.locator('.config-settings-topnav .config-settings-search').count(), 1);
        if (viewport.width > 720) {
          const navBox = await navigation.boundingBox();
          const searchBox = await page.locator('.config-settings-search').boundingBox();
          assert.ok(searchBox.x >= navBox.x + navBox.width, `${label}: search sits to the right of tabs`);
          assert.ok(Math.abs(searchBox.y + searchBox.height / 2 - navBox.y - navBox.height / 2) < 2, `${label}: tabs and search share a row`);
        }
        assert.equal(await navigation.getAttribute('aria-orientation'), 'horizontal');
        assert.equal(await page.locator('.config-settings-sidebar, .config-page aside').count(), 0,
          `${label}: settings must not add another vertical sidebar`);
        await settle();
        const navigationLayout = await navigation.evaluate(element => {
          const toolbar = document.querySelector('.config-settings-topnav').getBoundingClientRect();
          const panel = document.querySelector('#config-subpage-panel').getBoundingClientRect();
          const bounds = element.getBoundingClientRect();
          const elements = [...element.querySelectorAll('[role="tab"]')];
          const buttons = elements.map(button => button.getBoundingClientRect());
          return {
            aboveContent: bounds.top >= toolbar.top - 1 && toolbar.bottom <= panel.top + 1,
            singleRow: Math.max(...buttons.map(button => button.top)) - Math.min(...buttons.map(button => button.top)) <= 1,
            labelsFit: elements.every((button, index) => {
              const range = document.createRange();
              range.selectNodeContents(button);
              const text = range.getBoundingClientRect();
              return text.left >= buttons[index].left - 1 && text.right <= buttons[index].right + 1
                && text.top >= buttons[index].top - 1 && text.bottom <= buttons[index].bottom + 1;
            }),
            overflowing: element.scrollWidth > element.clientWidth + 1,
            scrollable: ['auto', 'scroll'].includes(getComputedStyle(element).overflowX),
          };
        });
        assert.equal(navigationLayout.aboveContent, true, `${label}: category navigation belongs above settings content`);
        assert.equal(navigationLayout.singleRow, true, `${label}: all seven categories must stay in a single horizontal row`);
        assert.equal(navigationLayout.labelsFit, true, `${label}: complete category labels must fit their own buttons`);
        if (viewport.width === 640) {
          assert.equal(navigationLayout.overflowing, true, `${label}: narrow navigation keeps readable labels through scrolling`);
          assert.equal(navigationLayout.scrollable, true, `${label}: narrow category navigation exposes horizontal scrolling`);
          assert.equal(await navigation.evaluate(element => {
            element.scrollLeft = element.scrollWidth;
            return element.scrollLeft > 0;
          }), true, `${label}: users can scroll to categories at the right edge`);
        }
        const finalCategory = page.locator('#config-subpage-tab-software');
        await reachable(finalCategory, 'rightmost horizontal category');
        await tab('software');
        assert.equal(await page.locator('.config-software-panel').isVisible(), true,
          `${label}: rightmost category remains activatable`);
        await reachable(page.locator('#config-subpage-tab-general'), 'first horizontal category');

        await tab('general');
        assert.equal(await page.locator('.config-keys-panel').isVisible(), true);
        assert.equal(await page.locator('#config-native-network').isVisible(), true, 'connection settings share the service page');
        assert.equal(await page.locator('#config-native-tls').isVisible(), true, 'TLS settings share the service page');
        assert.equal(await page.locator('.config-settings-views').count(), 0, 'secondary pagination is removed');
        const sectionStyle = await page.locator('.config-keys-panel').evaluate(node => {
          const style = getComputedStyle(node);
          return { radius: style.borderRadius, shadow: style.boxShadow, background: style.backgroundColor };
        });
        assert.equal(sectionStyle.radius, '12px');
        assert.notEqual(sectionStyle.background, 'rgba(0, 0, 0, 0)', 'settings groups have their own surface');
        if (viewport.width >= 1240) {
          const keys = await page.locator('#config-native-keys').boundingBox();
          const management = await page.locator('#config-native-management').boundingBox();
          const content = await page.locator('.config-settings-content').boundingBox();
          assert.ok(Math.abs(keys.width - content.width) <= 1, 'the expandable key list occupies its own full row');
          assert.ok(management.y >= keys.y + keys.height, 'fixed settings follow the independent key list');
          const network = await page.locator('#config-native-network').boundingBox();
          const tls = await page.locator('#config-native-tls').boundingBox();
          assert.ok(Math.abs(network.y - tls.y) <= 1 && Math.abs(network.height - tls.height) <= 1, 'paired native cards align at both edges');
        }
        await templateVisible('management');
        {
          const row = await page.locator('#template-field-management-0').boundingBox();
          const toggle = await page.locator('#template-field-management-0 .switch-control').boundingBox();
          const caption = await page.locator('#template-field-management-0 .template-config-label').boundingBox();
          assert.ok(Math.abs(row.x + row.width - toggle.x - toggle.width) <= 1, `${label}: switches align with the end of their setting row`);
          assert.ok(Math.abs(caption.y + caption.height / 2 - toggle.y - toggle.height / 2) <= 1, `${label}: switch and label stay on the same line`);
        }
        assert.equal(await page.locator('#config-retry-section-title').isVisible(), false);
        await templateVisible('diagnostics', false);
        await noOverflow('service access');
        await exerciseHelp(page.locator('#template-field-management-0 .settings-help-trigger').first(), 'management template');
        assert.equal(await page.locator('#config-network-section-title').isVisible(), true);
        assert.equal(await page.locator('#config-tls-section-title').isVisible(), true);
        assert.equal(await page.locator('#config-retry-section-title').isVisible(), false);
        await noOverflow('service connection');
        const nativePort = page.locator('#config-native-network .config-network-port-field input');
        assert.ok((await nativePort.boundingBox()).width <= 160, `${label}: port input stays compact`);
        if (viewport.width >= 964) {
          const secret = await page.locator('#config-native-management .config-secret-input').boundingBox();
          assert.ok(secret.width <= 420, `${label}: secret input must not stretch across the page`);
        }
        assert.equal(await page.getByLabel(locale === 'en' ? 'Port' : '端口', { exact: true }).count(), 1,
          `${label}: field help must not alter the input's accessible label`);
        const initialPort = await nativePort.inputValue();
        await exerciseHelp(page.locator('#config-native-network .settings-help-trigger').first(), 'network field');
        assert.equal(await nativePort.inputValue(), initialPort, `${label}: requesting help must not edit the field`);

        await tab('routing');
        assert.equal(await page.locator('#config-routing-section-title').isVisible(), true);
        assert.equal(await page.locator('#config-retry-section-title').isVisible(), false);
        await templateVisible('routing-advanced');
        await templateVisible('extensions-concurrency');
        if (viewport.width >= 964) {
          const toggle = await page.locator('#config-native-routing .switch-control').boundingBox();
          const ttl = await page.locator('#config-input-config-network-sessionTtl').boundingBox();
          assert.ok(toggle.width > 0 && ttl.width > 0, `${label}: controls remain visible inside their card`);
          assert.ok(ttl.width <= 200, `${label}: duration input stays compact`);
        }
        await templateVisible('extensions-plugins', false);
        await noOverflow('routing and stability');
        await tab('requests');
        assert.equal(await retrySection.isVisible(), true);
        if (viewport.width >= 964) {
          const value = await page.locator('#config-input-config-network-requestRetry').boundingBox();
          assert.ok(value.width <= 160, `${label}: retry count stays compact`);
          const caption = await page.locator('label[for="config-input-config-network-requestRetry"]').boundingBox();
          assert.ok(Math.abs(value.y + value.height / 2 - caption.y - caption.height / 2) <= 2,
            `${label}: compact numeric settings keep labels next to their values`);
          if (viewport.width === 1280) {
            assert.ok((await retrySection.boundingBox()).height < 460,
              `${label}: paired routing cards should not create excessive vertical whitespace`);
          }
        }
        await tab('routing');
        await page.locator('.template-config-card:visible').last().scrollIntoViewIfNeeded();
        await settle();
        const pinnedCategory = await page.locator('#config-subpage-tab-routing').evaluate(element => {
          const bounds = element.getBoundingClientRect();
          const hit = document.elementFromPoint(bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
          return bounds.top >= 0 && bounds.bottom <= innerHeight && Boolean(hit && element.contains(hit));
        });
        assert.equal(pinnedCategory, true, `${label}: categories remain visible and clickable while scrolling long forms`);
        await tab('general');
        assert.equal(await page.locator('.config-settings-views').count(), 0, `${label}: secondary pagination is removed`);
        await tab('routing');
        const routingOptions = await page.locator('.routing-segmented').evaluate(group => {
          const container = group.getBoundingClientRect();
          return [...group.querySelectorAll('button')].map(button => {
            const bounds = button.getBoundingClientRect();
            const range = document.createRange();
            range.selectNodeContents(button);
            const text = range.getBoundingClientRect();
            return {
              label: button.textContent.trim(),
              contained: bounds.left >= container.left - 1 && bounds.right <= container.right + 1
                && bounds.top >= container.top - 1 && bounds.bottom <= container.bottom + 1,
              readable: text.left >= bounds.left - 1 && text.right <= bounds.right + 1
                && text.top >= bounds.top - 1 && text.bottom <= bounds.bottom + 1,
              height: bounds.height,
            };
          });
        });
        assert.equal(routingOptions.length, 3);
        assert.ok(routingOptions.every(option => option.contained && option.readable && option.height >= 32),
          `${label}: all routing strategy labels need complete text and usable button bounds: ${JSON.stringify(routingOptions)}`);

        await tab('requests');
        for (const id of ['request-behavior', 'codex-client', 'multimedia', 'request-payload']) await templateVisible(id);
        if (viewport.width >= 1240) {
          const left = await page.locator('#config-group-codex-client').boundingBox();
          const right = await page.locator('#config-group-multimedia').boundingBox();
          assert.ok(Math.abs(left.y - right.y) <= 1 && Math.abs(left.height - right.height) <= 1, 'paired template cards align at both edges');
          const leftSave = await page.locator('#config-group-codex-client .template-config-actions button').last().boundingBox();
          const rightSave = await page.locator('#config-group-multimedia .template-config-actions button').last().boundingBox();
          assert.ok(Math.abs(leftSave.y + leftSave.height - rightSave.y - rightSave.height) <= 1, 'paired save buttons share a bottom baseline');
        }
        await noOverflow('model and request behavior');
        assert.equal(await page.locator('#config-native-aliases').isVisible(), false, 'aliases are separate from model and request settings');
        await tab('aliases');
        await templateVisible('request-behavior', false);
        assert.equal(await page.locator('#config-native-sensitive-words').isVisible(), false);
        assert.equal(await page.locator('.thinking-alias-page').isVisible(), true);
        await reachable(page.locator('.thinking-alias-page').getByRole('button', { name: locale === 'en' ? 'Create Alias' : '创建别名', exact: true }), 'model alias creation');
        await noOverflow('model aliases');
        // Exercise the editor inside the settings page, where ancestor CSS applies.
        for (const theme of ['light', 'dark']) {
          await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
          await page.locator('.thinking-alias-page > .management-header button').click();
          const dialog = page.getByRole('dialog');
          await dialog.waitFor();
          await page.waitForFunction(() => document.activeElement?.id === 'thinking-model-search');
          await page.keyboard.press('Escape');
          assert.equal(await dialog.isVisible(), true, 'Escape closes the model picker before the editor');
          assert.equal(await dialog.getByRole('listbox').count(), 0);
          const surface = await dialog.evaluate(node => {
            const style = getComputedStyle(node);
            const bounds = node.getBoundingClientRect();
            return {
              background: style.backgroundColor, radius: parseFloat(style.borderRadius),
              padding: style.padding, withinViewport: bounds.left >= 0 && bounds.top >= 0
                && bounds.right <= innerWidth && bounds.bottom <= innerHeight,
            };
          });
          assert.ok(!/rgba\([^)]*,\s*0\)$/.test(surface.background) && surface.background !== 'transparent', `${label} ${theme}: editor needs an opaque surface`);
          assert.ok(surface.radius > 0, `${label} ${theme}: editor retains its own rounded container`);
          assert.equal(surface.padding, '0px', `${label} ${theme}: page spacing must not override editor layout`);
          assert.equal(surface.withinViewport, true, `${label} ${theme}: editor fits the window`);
          await reachable(dialog.getByRole('button', { name: locale === 'en' ? 'Cancel' : '取消', exact: true }), 'alias dialog cancel');
          if (process.env.SETTINGS_SCREENSHOT_DIR) {
            const fs = require('node:fs');
            const directory = path.resolve(process.env.SETTINGS_SCREENSHOT_DIR);
            fs.mkdirSync(directory, { recursive: true });
            await page.screenshot({ path: path.join(directory, `alias-dialog-${locale}-${viewport.width}-${theme}.png`) });
          }
          await dialog.getByRole('button', { name: locale === 'en' ? 'Cancel' : '取消', exact: true }).click();
          await dialog.waitFor({ state: 'detached' });
        }
        await page.evaluate(() => { document.documentElement.dataset.theme = 'light'; });
        await tab('requests');
        await page.locator('.config-sensitive-words-row input').first().waitFor();
        await noOverflow('sensitive words');

        await tab('oauth');
        for (const id of ['oauth-common', 'oauth-models', 'oauth-codex', 'oauth-media', 'oauth-claude', 'oauth-others']) await templateVisible(id);
        await noOverflow('upstream credentials');
        await tab('requests');
        assert.equal(await page.locator('.config-settings-views').count(), 0, `${label}: secondary pagination is removed`);
        await tab('diagnostics');
        assert.equal(await page.locator('.config-diagnostics-panel').isVisible(), true);
        await templateVisible('diagnostics');
        await templateVisible('extensions-inflight');
        await noOverflow('diagnostics');
        assert.equal(await page.locator('#config-subpage-tab-extensions').count(), 0);
        await templateVisible('extensions-plugins', false);
        await tab('software');
        assert.equal(await page.locator('.config-software-panel').isVisible(), true);
        await noOverflow('application preferences');

        // Search remains available across categories, and result activation must
        // select the category before locating the integrated section.
        await search.fill(locale === 'en' ? 'Retries & cooldowns' : '重试与冷却');
        const results = page.locator('.config-search-results');
        await results.getByRole('button').first().waitFor();
        await noOverflow('search results');
        await reachable(results.getByRole('button').first(), 'search result');
        await results.getByRole('button', { name: locale === 'en' ? /Retries & cooldowns/ : /重试与冷却/ }).first().click();
        assert.equal(await page.locator('#config-subpage-tab-requests').getAttribute('aria-selected'), 'true');
        await reachable(page.locator('#config-retry-section-title'), 'search target');
        await search.fill(locale === 'en' ? 'Sensitive Words' : '敏感词');
        await results.getByRole('button').first().waitFor();
        await results.getByRole('button').first().click();
        assert.equal(await page.locator('#config-subpage-tab-requests').getAttribute('aria-selected'), 'true');
        assert.equal(await page.locator('.config-sensitive-words-row input').first().isVisible(), true);
        await page.waitForFunction(() => document.querySelector('#config-native-sensitive-words')?.contains(document.activeElement));
        await search.fill(locale === 'en' ? 'thinking aliases' : '思考');
        await results.getByRole('button').first().waitFor();
        await results.getByRole('button').first().click();
        await page.waitForFunction(() => document.querySelector('#config-native-aliases')?.contains(document.activeElement));
        assert.equal(await page.locator('#config-subpage-tab-aliases').getAttribute('aria-selected'), 'true');
        await search.fill(locale === 'en' ? 'Default values' : '补充缺失参数');
        await results.getByRole('button').first().waitFor();
        await results.getByRole('button').first().click();
        assert.equal(await page.locator('#config-subpage-tab-requests').getAttribute('aria-selected'), 'true');
        const payloadField = page.locator('#template-field-request-payload-0');
        assert.equal(await payloadField.locator('details.template-config-custom').evaluate(element => element.open), true,
          `${label}: search expands structured controls before locating a field`);
        await page.waitForFunction(() => document.querySelector('#template-field-request-payload-0')?.contains(document.activeElement));
        if (viewport.width === 1280) {
          await payloadField.locator('button.structured-add').first().click();
          await payloadField.locator('input').first().waitFor();
          await search.fill(locale === 'en' ? 'Default values' : '补充缺失参数');
          await results.getByRole('button').first().click();
          await page.waitForFunction(() => document.querySelector('#template-field-request-payload-0')?.contains(document.activeElement)
            && document.activeElement?.matches('input, textarea, select'));
          await page.locator('#config-group-request-payload').getByRole('button', {
            name: locale === 'en' ? 'Discard changes' : '放弃本组修改', exact: true,
          }).click();
        }
        await search.fill('');
        await search.fill('settings-that-do-not-exist-2026');
        assert.equal(await results.getByRole('button').count(), 0);
        await noOverflow('empty search');
        await search.fill('');

        if (process.env.SETTINGS_SCREENSHOT_DIR && viewport.width === 640) {
          const fs = require('node:fs');
          const directory = path.resolve(process.env.SETTINGS_SCREENSHOT_DIR);
          fs.mkdirSync(directory, { recursive: true });
          await page.setViewportSize({ width: 640, height: 900 });
          await tab('general');
          await page.locator('.config-settings-topnav').scrollIntoViewIfNeeded();
          await settle();
          await page.screenshot({ path: path.join(directory, locale === 'en' ? 'settings-access-640-en.png' : 'settings-access-640.png') });
          await tab('routing');
          await page.locator('.config-settings-topnav').scrollIntoViewIfNeeded();
          await settle();
          await page.screenshot({ path: path.join(directory, locale === 'en' ? 'settings-routing-640-en.png' : 'settings-routing-640.png') });
          await page.setViewportSize(viewport);
        }

        if (process.env.SETTINGS_SCREENSHOT_DIR && viewport.width === 1280 && locale === 'zh-CN') {
          const fs = require('node:fs');
          const directory = path.resolve(process.env.SETTINGS_SCREENSHOT_DIR);
          fs.mkdirSync(directory, { recursive: true });
          await page.setViewportSize({ width: 1280, height: 900 });
          await tab('general');
          await page.locator('.config-settings-topnav').scrollIntoViewIfNeeded();
          await settle();
          await page.screenshot({ path: path.join(directory, 'settings-access.png') });
          await tab('routing');
          await page.locator('.config-settings-topnav').scrollIntoViewIfNeeded();
          await settle();
          await page.screenshot({ path: path.join(directory, 'settings-routing.png') });
          await page.locator('.theme-selector-trigger').click();
          await page.locator('.theme-selector-card[data-theme-id="dark"]').click();
          await page.keyboard.press('Escape');
          await settle();
          await page.screenshot({ path: path.join(directory, 'settings-routing-dark.png') });
          await tab('general');
          await page.locator('.config-settings-topnav').scrollIntoViewIfNeeded();
          await settle();
          await page.screenshot({ path: path.join(directory, 'settings-access-dark.png') });
          await page.locator('.theme-selector-trigger').click();
          await page.locator('.theme-selector-card[data-theme-id="light"]').click();
          await page.keyboard.press('Escape');
          await page.setViewportSize(viewport);
        }

        // Exercise drafts and save feedback once per locale; the responsive cases
        // above use fresh browser contexts so preferences never leak between them.
        if (viewport.width === 1280) {
          await tab('general');
          const remoteManagement = page.locator('#template-management-0');
          await remoteManagement.check();
          assert.equal(await hasDirtyMarker('general'), true, `${label}: template drafts mark their category`);
          await tab('requests');
          const retryInput = retrySection.locator('input[type="text"]').first();
          await retryInput.fill('5');
          const cooling = page.getByRole('switch', { name: locale === 'en' ? 'Enable cooldowns' : '启用冷却', exact: true });
          assert.equal(await cooling.isChecked(), true, 'existing false disable-cooling means enabled');
          await cooling.uncheck();
          assert.equal(await hasDirtyMarker('requests'), true, `${label}: core drafts mark their category`);
          await tab('software');
          assert.equal(await hasDirtyMarker('requests'), true, `${label}: dirty category remains marked while inactive`);
          await tab('requests');
          assert.equal(await retryInput.inputValue(), '5', `${label}: changing categories preserves core drafts`);
          const retrySave = retrySection.getByRole('button', { name: locale === 'en' ? /Save/ : /保存/ });
          await retrySave.click();
          await page.waitForFunction(button => {
            return button?.disabled && !/正在保存|保存中|saving/i.test(button.textContent);
          }, await retrySave.elementHandle());
          assert.equal(await hasDirtyMarker('requests'), false, `${label}: saving clears the category marker`);
          await tab('software');
          await tab('requests');
          assert.equal(await cooling.isChecked(), false, 'saved disable-cooling true means switch off');
          await cooling.check();
          await retrySave.click();
          await page.waitForFunction(button => button?.disabled && !/正在保存|保存中|saving/i.test(button.textContent), await retrySave.elementHandle());
          await tab('software');
          await tab('requests');
          assert.equal(await cooling.isChecked(), true, 'manually enabled cooling survives saving');
          await tab('general');
          assert.equal(await remoteManagement.isChecked(), true, `${label}: saving another category preserves template drafts`);
          assert.equal(await hasDirtyMarker('general'), true);
          const management = page.locator('#template-group-management').locator('xpath=ancestor::section[1]');
          await management.getByRole('button', { name: locale === 'en' ? 'Discard changes' : '放弃本组修改', exact: true }).click();
          assert.equal(await remoteManagement.isChecked(), false);
          assert.equal(await hasDirtyMarker('general'), false, `${label}: discarding clears the category marker`);

          await tab('requests');
          const sensitiveWord = page.locator('.config-sensitive-words-row input').first();
          await sensitiveWord.fill('settings-navigation-draft');
          await tab('software');
          assert.equal(await hasDirtyMarker('requests'), true, `${label}: sensitive-word drafts mark their category`);
          await tab('requests');
          assert.equal(await sensitiveWord.inputValue(), 'settings-navigation-draft', `${label}: sensitive-word drafts survive category changes`);
        }
        await page.close();
      }
    }
    assert.deepEqual(runtimeErrors, [], 'Settings navigation must not produce runtime errors');
    console.log('PASS: seven settings categories including independent model aliases, adaptive cards, search, retained drafts, save feedback, and Chinese/English responsive layouts.');
  } finally {
    if (browser) await browser.close();
    if (server) await server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
