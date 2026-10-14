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
    const open = query => page.goto(`http://127.0.0.1:1422/tests/fixtures/theme.html?${query}`);
    const theme = expected => page.waitForFunction(expected => document.documentElement.dataset.theme === expected, expected);
    const trigger = () => page.locator('.theme-selector-trigger:visible');
    const menu = () => page.locator('.theme-selector-menu');
    const mode = id => page.locator(`.theme-selector-mode[data-mode="${id}"]`);
    const card = id => page.locator(`.theme-selector-card[data-theme-id="${id}"]`);
    const openMenu = async () => {
      if (await trigger().getAttribute('aria-expanded') !== 'true') await trigger().click();
      await menu().waitFor();
    };
    const select = async id => {
      await openMenu();
      if (id === 'system') await mode('system').click();
      else {
        await card(id).click();
        const appearance = id.includes('dark') ? 'dark' : 'light';
        if (await mode(appearance).getAttribute('aria-pressed') !== 'true') await mode(appearance).click();
      }
      assert.equal(await trigger().getAttribute('aria-expanded'), 'true', 'selection keeps the theme picker open');
      await page.keyboard.press('Escape');
    };
    const assign = async id => {
      await openMenu();
      assert.equal(await mode('system').getAttribute('aria-pressed'), 'true', 'assignment happens while following the system');
      await card(id).click();
      assert.equal(await card(id).getAttribute('aria-pressed'), 'true');
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
      await openMenu(); await card('warm-sand-light').click();
      assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'warm-sand-light', 'picking a theme of the other appearance switches to it');
      assert.equal(await mode('light').getAttribute('aria-pressed'), 'true');
      await page.keyboard.press('Escape');
      await select('system'); await saved('system');
      await assign('sakura-light'); await assign('midnight-dark');
      await system('light'); await theme('light');
      assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'sakura-light');
      await open(`platform=${platform}&system=dark`);
      await nativeReady(); await theme('dark');
      assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'midnight-dark');
      assert.deepEqual(selection, { version: 1, selectedId: 'system', lastLightId: 'sakura-light', lastDarkId: 'midnight-dark' });
      await openMenu();
      assert.equal(await mode('system').getAttribute('aria-pressed'), 'true');
      await page.keyboard.press('Escape');
      await page.locator('.sidebar-easy-entry').click();
      await select('light'); await theme('light'); await saved('light');
      await page.locator('.simple-mode-exit-btn').click();
      await openMenu();
      assert.equal(await mode('light').getAttribute('aria-pressed'), 'true');
      assert.equal(await card('light').getAttribute('aria-pressed'), 'true');
      await page.keyboard.press('Escape');
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
    assert.ok(await isFocused(mode('dark')), 'opening the picker focuses the active mode');
    await page.keyboard.press('Escape');
    assert.ok(await isFocused(trigger()));
    assert.equal(await trigger().getAttribute('aria-expanded'), 'false');
    await page.keyboard.press('Enter');
    await menu().waitFor();
    await retry().focus();
    const saveAttempts = await page.evaluate(() => window.themeFixture.calls.filter(call => call.cmd === 'save_theme_selection').length);
    await page.keyboard.press('Enter');
    await page.waitForFunction(count => window.themeFixture.calls.filter(call => call.cmd === 'save_theme_selection').length > count, saveAttempts);
    assert.ok(await isFocused(trigger()), 'retry must return focus to a stable control');
    assert.equal(await trigger().getAttribute('aria-expanded'), 'true');
    assert.equal(await retry().count(), 1, 'a failed retry remains available');
    await page.evaluate(() => window.themeFixture.setSaveError(false));
    await retry().focus(); await page.keyboard.press(' ');
    await page.waitForFunction(() => !document.querySelector('.theme-selector-warning'));
    assert.ok(await isFocused(trigger()), 'removing a successful retry must preserve focus');
    assert.equal(await retry().count(), 0);
    assert.equal(selection.selectedId, 'dark');
    await page.keyboard.press('Escape');
    assert.equal(await trigger().getAttribute('aria-expanded'), 'false', 'Escape closes the picker after a retry moved focus to the trigger');

    selection = { version: 1, selectedId: 'light' };
    await open('platform=macos'); await nativeReady(); await theme('light');
    await openMenu();
    const check = await card('light').locator('.theme-selector-check').evaluate(element => getComputedStyle(element).color);
    const contrast = await page.evaluate(() => {
      const probe = document.createElement('span'); probe.style.color = 'var(--primary-contrast)'; document.body.append(probe);
      const value = getComputedStyle(probe).color; probe.remove(); return value;
    });
    assert.equal(check, contrast, 'the selected check uses the accent contrast color in the default theme');
    await page.keyboard.press('Escape');

    selection = { version: 1, selectedId: 'warm-sand-light' };
    await open('platform=macos'); await nativeReady(); await theme('light');
    await openMenu();
    assert.equal(await card('warm-sand-light').getAttribute('aria-pressed'), 'true');
    await mode('dark').click(); await theme('dark');
    assert.equal(await card('warm-sand-light').getAttribute('aria-pressed'), 'true', 'switching mode keeps the fixed theme in its group');
    await mode('light').click(); await theme('light');
    assert.equal(await page.locator('html').getAttribute('data-theme-id'), 'warm-sand-light');
    await page.keyboard.press('Escape');

    selection = null;
    for (const locale of ['zh-CN', 'zh-TW', 'en', 'ja']) for (const width of [1440, 1100, 900, 640]) {
      await page.setViewportSize({ width, height: 760 });
      await open(`platform=windows&locale=${locale}`); await nativeReady();
      await trigger().focus(); await page.keyboard.press('Enter');
      await menu().waitFor();
      assert.ok(await mode('light').evaluate(element => element === document.activeElement), 'keyboard opening focuses the active mode');
      assert.equal(await page.locator('.theme-selector-group[data-appearance="light"] .theme-selector-card').count(), 3);
      assert.equal(await page.locator('.theme-selector-group[data-appearance="dark"] .theme-selector-card').count(), 3);
      assert.ok((await card('warm-sand-light').getAttribute('title')).includes('\n'), 'the tooltip contains the name and description');
      const menuBox = await menu().boundingBox();
      assert.ok(menuBox.x >= 0 && menuBox.y >= 0 && menuBox.x + menuBox.width <= width && menuBox.y + menuBox.height <= 760, `${locale} at ${width}: menu outside viewport`);
      await card('sakura-light').focus(); await page.keyboard.press(' ');
      await saved('sakura-light');
      assert.equal(await trigger().getAttribute('aria-expanded'), 'true');
      await page.keyboard.press('Escape');
      assert.ok(await trigger().evaluate(element => element === document.activeElement));
      await trigger().click();
      await page.locator('.sidebar-privacy-toggle').click();
      assert.equal(await trigger().getAttribute('aria-expanded'), 'false');
      assert.ok(await page.locator('.sidebar-privacy-toggle').evaluate(element => element === document.activeElement));
      await trigger().click(); await page.locator('.sidebar-privacy-toggle').focus();
      assert.equal(await trigger().getAttribute('aria-expanded'), 'false', 'focus leaving the picker closes it');
      const controls = await page.locator('.sidebar-language-trigger, .sidebar-privacy-toggle, .theme-selector').evaluateAll(elements => elements.map(element => {
        const { x, right, width } = element.getBoundingClientRect(); return { x, right, width };
      }));
      assert.ok(controls[0].width >= 48 && controls[1].width >= 28 && controls[2].width >= 36 && controls[2].width <= 48);
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
    await trigger().click(); await menu().waitFor();
    const menuState = () => page.evaluate(() => {
      const element = document.querySelector('.theme-selector-menu');
      const { top, bottom } = element.getBoundingClientRect();
      return { scroll: element.scrollTop, pageScroll: window.scrollY, top, bottom, overflow: element.scrollHeight > element.clientHeight };
    });
    const beforeHover = await menuState();
    assert.ok(beforeHover.overflow, 'the picker scrolls in a short window');
    assert.ok(beforeHover.top >= 0 && beforeHover.bottom <= 320, 'the picker stays inside a short window');
    const menuBounds = await menu().boundingBox();
    await page.mouse.move(menuBounds.x + menuBounds.width / 2, menuBounds.y + menuBounds.height - 4);
    await page.mouse.move(menuBounds.x + menuBounds.width / 2, menuBounds.y + menuBounds.height / 2);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.deepEqual(await menuState(), beforeHover, 'hover must not scroll or reposition the picker');
    await page.mouse.wheel(0, 160);
    await page.waitForFunction(() => document.querySelector('.theme-selector-menu').scrollTop > 0);
    const afterWheel = await menuState();
    assert.deepEqual({ ...afterWheel, scroll: 0 }, beforeHover, 'only the picker may scroll');
    await card('midnight-dark').click(); await saved('midnight-dark');
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
    console.log('Theme UI passed: desktop IPC, persistence, errors, mode and theme picker, focus, 4 locales at 4 widths.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
