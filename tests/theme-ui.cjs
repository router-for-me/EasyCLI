const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');

(async () => {
  const browser = await chromium.launch({ channel: process.env.THEME_BROWSER || 'msedge', headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, colorScheme: 'light' });
    const errors = [];
    let selection = null;
    page.on('pageerror', error => errors.push(String(error)));
    page.setDefaultTimeout(10000);
    await page.route('**/__theme_fixture_selection', async route => {
      if (route.request().method() === 'POST') selection = JSON.parse(route.request().postData());
      await route.fulfill({ json: selection });
    });
    let appearance = 'light';
    const open = query => { appearance = 'light'; return page.goto(`http://127.0.0.1:1422/tests/fixtures/theme.html?${query}`); };
    const theme = expected => page.waitForFunction(expected => document.documentElement.dataset.theme === expected, expected);
    const trigger = (mode = appearance) => { appearance = mode; return page.locator(`.theme-selector-trigger[data-appearance="${mode}"]`); };
    const select = async id => {
      if (id === 'system') { await page.locator('.theme-selector-system-toggle').click(); return; }
      await trigger(id.includes('dark') ? 'dark' : 'light').click();
      await page.locator(`.theme-selector-option[data-theme-id="${id}"]`).click();
      assert.equal(await trigger().getAttribute('aria-expanded'), 'true', 'selection keeps the theme list open');
      await page.keyboard.press('Escape');
    };
    const assign = async id => {
      await trigger(id.includes('dark') ? 'dark' : 'light').click();
      await page.locator(`.theme-selector-option[data-theme-id="${id}"]`).click({ button: 'right' });
      assert.equal(await page.getByRole('menu').count(), 0, 'right-click assigns directly');
      await page.keyboard.press('Escape');
    };
    const system = value => page.evaluate(value => window.themeFixture.setSystem(value), value);
    const nativeReady = () => page.waitForFunction(() => window.themeFixture?.calls.some(call => call.cmd.includes('background_color')));
    const saved = id => page.waitForFunction(id => window.themeFixture.calls.some(call => call.cmd === 'save_theme_selection' && call.args.selection.selectedId === id), id);

    for (const platform of ['windows', 'macos', 'linux']) {
      console.log(`Checking desktop IPC: ${platform}`);
      selection = null;
      await open(`platform=${platform}`);
      await nativeReady(); await theme('light');
      assert.equal(selection, null);
      await system('dark'); await theme('light');
      for (const [id, appearance] of [['warm-sand-light', 'light'], ['graphite-dark', 'dark'], ['sakura-light', 'light'], ['midnight-dark', 'dark']]) {
        await select(id); await saved(id);
        await page.waitForFunction(id => document.documentElement.dataset.themeId === id, id);
        await system(appearance === 'dark' ? 'light' : 'dark');
        assert.equal(await page.locator('html').getAttribute('data-theme-id'), id);
        await open(`platform=${platform}&system=${appearance === 'dark' ? 'light' : 'dark'}`); await nativeReady();
        assert.equal(await page.locator('html').getAttribute('data-theme-id'), id);
        await theme(appearance);
        await page.waitForFunction(value => window.themeFixture.appearance === value, appearance);
      }
      await theme('dark');
      await page.waitForFunction(() => window.themeFixture.appearance === 'dark');
      await assign('sakura-light'); await assign('midnight-dark');
      assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'midnight-dark', 'assignment preserves manual selection');
      await select('system'); await saved('system');
      await system('light'); await theme('light');
      assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'sakura-light');
      await open(`platform=${platform}&system=dark`);
      await nativeReady(); await theme('dark');
      assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'midnight-dark');
      assert.deepEqual(selection, { version: 1, selectedId: 'system', lastLightId: 'sakura-light', lastDarkId: 'midnight-dark' });
      assert.equal(await page.locator('.theme-selector-system-toggle').getAttribute('aria-pressed'), 'true');
      await page.locator('.sidebar-easy-entry').click();
      await select('light'); await theme('light'); await saved('light');
      await page.locator('.simple-mode-exit-btn').click();
      assert.equal(await trigger('light').getAttribute('data-selected'), 'true');
      const writes = await page.evaluate(() => window.themeFixture.calls.filter(call => call.cmd === 'plugin:window|set_theme').length);
      assert.ok(writes < 30, `${platform}: native theme echo loop (${writes} writes)`);
    }

    selection = { version: 1, selectedId: 'removed-dark' };
    await open('platform=macos'); await nativeReady(); await theme('light');
    assert.match(await trigger().getAttribute('aria-label'), /主题不可用/);
    assert.equal(selection.selectedId, 'removed-dark');
    await select('light'); await saved('light');

    await open('platform=macos&read-error'); await nativeReady(); await theme('light');
    await trigger().click();
    assert.match(await page.locator('.theme-selector-notice').innerText(), /无法读取/);
    await page.keyboard.press('Escape');
    await open('platform=macos&save-error'); await nativeReady();
    await select('dark');
    await page.waitForFunction(() => document.querySelector('.theme-selector-warning'));
    await trigger().click();
    assert.match(await page.locator('.theme-selector-notice').innerText(), /未能保存/);
    const retry = () => page.getByRole('button', { name: '重试保存', exact: true });
    const isFocused = locator => locator.evaluate(element => element === document.activeElement);
    await page.keyboard.press('Tab');
    assert.ok(await isFocused(retry()), 'Tab must reach save retry without closing the menu');
    assert.equal(await trigger().getAttribute('aria-expanded'), 'true');
    await page.keyboard.press('Shift+Tab');
    assert.ok(await isFocused(trigger()));
    assert.equal(await trigger().getAttribute('aria-expanded'), 'true');
    await page.keyboard.press('Tab');
    await page.keyboard.press('Escape');
    assert.ok(await isFocused(trigger()));
    assert.equal(await trigger().getAttribute('aria-expanded'), 'false');
    await page.keyboard.press('Enter');
    await page.keyboard.press('Tab');
    await page.keyboard.press('Tab');
    assert.equal(await trigger().getAttribute('aria-expanded'), 'false');

    await trigger().focus(); await page.keyboard.press('Enter'); await page.keyboard.press('Tab');
    const saveAttempts = await page.evaluate(() => window.themeFixture.calls.filter(call => call.cmd === 'save_theme_selection').length);
    await page.keyboard.press('Enter');
    await page.waitForFunction(count => window.themeFixture.calls.filter(call => call.cmd === 'save_theme_selection').length > count, saveAttempts);
    assert.ok(await isFocused(trigger()), 'retry must return focus to a stable control');
    assert.equal(await retry().count(), 1, 'a failed retry remains available');
    await page.evaluate(() => window.themeFixture.setSaveError(false));
    await page.keyboard.press('Tab'); await page.keyboard.press(' ');
    await page.waitForFunction(() => !document.querySelector('.theme-selector-warning'));
    assert.ok(await isFocused(trigger()), 'removing a successful retry must preserve focus');
    assert.equal(await retry().count(), 0);
    assert.equal(selection.selectedId, 'dark');
    await page.keyboard.press('Tab');
    assert.equal(await trigger().getAttribute('aria-expanded'), 'false');

    selection = null;
    for (const locale of ['zh-CN', 'zh-TW', 'en', 'ja']) for (const width of [1440, 1100, 900, 640]) {
      await page.setViewportSize({ width, height: 760 });
      await open(`platform=windows&locale=${locale}`); await nativeReady();
      const systemDescription = await page.locator('.theme-selector-system-toggle').evaluate(element =>
        element.getAttribute('aria-describedby').split(' ').map(id => document.getElementById(id).textContent));
      assert.equal(systemDescription.length, 2);
      assert.ok(systemDescription.every(text => text && !text.includes('，')), `${locale}: system hints must not contain hardcoded punctuation`);
      await trigger().focus(); await page.keyboard.press('Enter');
      assert.equal(await page.getByRole('option').count(), 3);
      const lightRow = await page.locator('.theme-selector-option[data-theme-id="light"]').boundingBox();
      const warmOption = page.locator('.theme-selector-option[data-theme-id="warm-sand-light"]');
      const warmRow = await warmOption.boundingBox();
      assert.equal(warmRow.height, lightRow.height, 'descriptions must not change theme row height');
      assert.ok(warmRow.y - (lightRow.y + lightRow.height) >= 8, 'theme options need a visible gap');
      assert.ok((await warmOption.getAttribute('title')).includes('\n'), 'the tooltip contains the name and description');
      assert.equal(await warmOption.locator('.theme-selector-copy small').count(), 0);
      const menuBox = await page.locator('.theme-selector-menu').boundingBox();
      assert.ok(menuBox.x >= 0 && menuBox.y >= 0 && menuBox.x + menuBox.width <= width && menuBox.y + menuBox.height <= 760, `${locale} at ${width}: menu outside viewport`);
      await page.keyboard.press('End'); await page.keyboard.press(' ');
      await saved('sakura-light');
      assert.equal(await trigger().getAttribute('aria-expanded'), 'true');
      assert.ok(await trigger().evaluate(element => element === document.activeElement));
      await page.keyboard.press('Escape');
      assert.ok(await trigger().evaluate(element => element === document.activeElement));
      await trigger().click();
      await page.locator('.sidebar-privacy-toggle').click();
      assert.equal(await trigger().getAttribute('aria-expanded'), 'false');
      assert.ok(await page.locator('.sidebar-privacy-toggle').evaluate(element => element === document.activeElement));
      await trigger().click(); await page.keyboard.press('Tab');
      assert.equal(await trigger().getAttribute('aria-expanded'), 'false');
      const controls = await page.locator('.sidebar-language-trigger, .sidebar-privacy-toggle, .theme-selector').evaluateAll(elements => elements.map(element => {
        const { x, right, width } = element.getBoundingClientRect(); return { x, right, width };
      }));
      assert.ok(controls[0].width >= 48 && controls[1].width >= 28 && controls[2].width === 92);
      assert.ok(controls[0].right < controls[1].x && controls[1].right < controls[2].x, `${locale} at ${width}: controls overlap`);
      selection = null;
    }
    selection = null;
    await page.setViewportSize({ width: 1280, height: 800 });
    await open('platform=macos&extensions'); await nativeReady();
    const chooseId = async id => {
      await select(id);
      await page.waitForFunction(id => document.documentElement.dataset.themeId === id, id);
    };
    const color = selector => page.locator(selector).evaluate(element => {
      const style = getComputedStyle(element); return { background: style.backgroundColor, color: style.color, border: style.borderTopColor };
    });
    const base = await color('.config-dialog');
    const legacySurfaces = ['.version-metric-tile', '.app-release-notes-footer', '.agent-core-setting-section', '.model-config-card'];
    const defaultSurfaces = await Promise.all(legacySurfaces.map(color));
    await chooseId('graphite-dark'); await saved('graphite-dark');
    for (const selector of legacySurfaces) {
      assert.equal((await color(selector)).background, 'rgb(40, 44, 49)', `${selector} must use graphite's secondary surface`);
    }
    await chooseId('dark'); await saved('dark');
    for (const selector of legacySurfaces) {
      assert.equal((await color(selector)).background, 'rgb(11, 13, 17)', `${selector} must restore the default dark palette`);
    }
    await chooseId('light'); await saved('light');
    assert.deepEqual(await Promise.all(legacySurfaces.map(color)), defaultSurfaces);
    await chooseId('test-light'); await saved('test-light');
    assert.equal((await color('.workspace')).background, 'rgb(233, 239, 255)');
    assert.equal((await color('.panel')).background, 'rgb(220, 232, 250)');
    assert.equal((await color('.config-dialog')).background, 'rgb(220, 232, 250)');
    assert.equal((await color('.usage-analysis-card')).background, 'rgb(220, 232, 250)');
    assert.equal((await color('.panel-heading h2')).color, 'rgb(18, 37, 68)');
    assert.equal((await color('.panel-heading p')).color, 'rgb(35, 54, 85)');
    await page.waitForFunction(() => getComputedStyle(document.querySelector('.panel')).borderTopColor === 'rgb(69, 103, 137)');
    assert.equal(await trigger().innerText(), '', 'localized names must not widen the icon control');
    await chooseId('test-dark'); await saved('test-dark');
    assert.equal((await color('.config-dialog')).background, 'rgb(32, 48, 64)');
    assert.equal((await color('.usage-analysis-card')).background, 'rgb(32, 48, 64)');
    await system('light');
    assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'test-dark');
    await chooseId('test-dark-2'); await saved('test-dark-2');
    assert.equal((await color('.panel')).background, 'rgb(37, 68, 51)');
    await page.reload(); await nativeReady();
    assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'test-dark-2');
    await chooseId('light'); await saved('light');
    assert.deepEqual(await color('.config-dialog'), base);
    assert.equal(await page.locator('html').evaluate(element => element.style.getPropertyValue('--theme-surface')), '');
    await chooseId('midnight-dark'); await saved('midnight-dark');
    await page.setViewportSize({ width: 1280, height: 320 });
    await trigger().click();
    const list = page.locator('.theme-selector-themes');
    const scrolls = await list.evaluate(element => element.scrollHeight > element.clientHeight);
    assert.ok(scrolls, 'long theme list should scroll in a short window');
    const activeVisible = () => page.waitForFunction(() => {
      const bounds = document.querySelector('.theme-selector-themes').getBoundingClientRect();
      const id = document.querySelector('.theme-selector-trigger[aria-expanded="true"]').getAttribute('aria-activedescendant');
      const row = document.getElementById(id).getBoundingClientRect();
      return row.top >= bounds.top - 1 && row.bottom <= bounds.bottom + 1;
    });
    await activeVisible();
    await page.keyboard.press('Home'); await activeVisible();
    const menuState = () => page.evaluate(() => {
      const menu = document.querySelector('.theme-selector-menu');
      const list = document.querySelector('.theme-selector-themes');
      const bounds = menu.getBoundingClientRect();
      const heading = menu.querySelector('.theme-selector-heading').getBoundingClientRect();
      const system = menu.querySelector('.theme-selector-footer').getBoundingClientRect();
      return { listScroll: list.scrollTop, menuScroll: menu.scrollTop, pageScroll: window.scrollY,
        top: bounds.top, bottom: bounds.bottom, headingTop: heading.top, systemBottom: system.bottom };
    });
    const beforeHover = await menuState();
    assert.ok(beforeHover.headingTop >= beforeHover.top && beforeHover.systemBottom <= beforeHover.bottom);
    const listBounds = await list.boundingBox();
    await page.mouse.move(listBounds.x + listBounds.width / 2, listBounds.y + listBounds.height - 2);
    await page.locator('.theme-selector-footer').hover();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.deepEqual(await menuState(), beforeHover, 'hover must not scroll or reposition the menu');
    await page.mouse.move(listBounds.x + listBounds.width / 2, listBounds.y + listBounds.height / 2);
    await page.mouse.wheel(0, 160);
    await page.waitForFunction(() => document.querySelector('.theme-selector-themes').scrollTop > 0);
    const afterWheel = await menuState();
    assert.deepEqual({ ...afterWheel, listScroll: 0 }, beforeHover, 'only the theme list may scroll');
    await page.keyboard.press('End');
    await page.keyboard.press('ArrowUp'); await activeVisible();
    await page.keyboard.press('End');
    await page.keyboard.press('Enter'); await saved('midnight-dark');
    assert.equal(await trigger().getAttribute('aria-expanded'), 'true');
    await page.keyboard.press('Escape');
    selection = null;

    if (process.env.THEME_SCREENSHOT_DIR) {
      await page.setViewportSize({ width: 1440, height: 960 });
      await open('platform=macos'); await nativeReady(); await trigger().click();
      await page.screenshot({ path: `${process.env.THEME_SCREENSHOT_DIR}/theme-picker-light.png`, animations: 'disabled' });
      await page.keyboard.press('Escape'); await select('dark'); await theme('dark'); await trigger().click();
      await page.screenshot({ path: `${process.env.THEME_SCREENSHOT_DIR}/theme-picker-dark.png`, animations: 'disabled' });
    }
    assert.deepEqual(errors, []);
    console.log('Theme UI passed: desktop IPC, persistence, errors, both selectors, focus, 4 locales at 4 widths.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
