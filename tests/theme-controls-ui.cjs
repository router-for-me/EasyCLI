// Run Vite on port 1422, then node tests/theme-controls-ui.cjs.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const assert = require('node:assert/strict');

const base = process.env.THEME_TEST_URL || 'http://127.0.0.1:1422';
const luminance = color => {
  const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(color);
  assert.ok(match, `Expected opaque computed color, received ${color}`);
  return match.slice(1).map(Number).map(channel => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  }).reduce((sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index], 0);
};
const contrast = (foreground, background) => {
  const values = [luminance(foreground), luminance(background)].sort((a, b) => a - b);
  return (values[1] + 0.05) / (values[0] + 0.05);
};
const settlePaint = locator => locator.evaluate(async element => {
  await Promise.all(element.getAnimations().map(animation => animation.finished));
});

(async () => {
  const browser = await chromium.launch({ channel: process.env.THEME_BROWSER || 'msedge', headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, colorScheme: 'light' });
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    page.setDefaultTimeout(10000);
    await page.route('**/*', route => route.request().url().startsWith(`${base}/`) ? route.continue() : route.abort());
    await page.addInitScript(() => localStorage.setItem('easy-cli-proxy-api.locale', 'zh-CN'));
    await page.goto(`${base}/?mock=running`, { waitUntil: 'domcontentloaded' });
    await page.locator('.theme-selector-trigger').click();
    await page.locator('.theme-selector-card[data-theme-id="sakura-light"]').click();
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => document.documentElement.dataset.themeId === 'sakura-light');

    const assertButtonContrast = async (button, label) => {
      await button.waitFor();
      await page.mouse.move(0, 0);
      for (const state of ['normal', 'hover']) {
        if (state === 'hover') await button.hover();
        await settlePaint(button);
        const paint = await button.evaluate(element => {
          const style = getComputedStyle(element);
          return { foreground: style.color, background: style.backgroundColor };
        });
        assert.ok(contrast(paint.foreground, paint.background) >= 4.5, `${label} ${state}: ${JSON.stringify(paint)}`);
      }
      await page.mouse.move(0, 0);
    };
    await page.getByRole('button', { name: '额度查询', exact: true }).click();
    await assertButtonContrast(page.locator('.oauth-page-toolbar-actions > .primary-button:not(:disabled)'), 'quota refresh');
    await page.getByRole('button', { name: '首页', exact: true }).click();
    await page.getByRole('button', { name: '模型健康监测', exact: true }).click();
    await assertButtonContrast(page.locator('.core-health-actions .primary-button:not(:disabled)'), 'model health');
    await page.keyboard.press('Escape');

    await page.getByRole('button', { name: '使用记录', exact: true }).click();
    await page.evaluate(async () => {
      const { initializeTheme } = await import('/src/theme.ts');
      const controller = initializeTheme();
      controller.setSystemTheme('warm-sand-light');
      controller.setSystemTheme('graphite-dark');
      controller.setPreference('system');
    });
    await page.waitForFunction(() => document.documentElement.dataset.themeId === 'warm-sand-light');
    await page.getByRole('button', { name: '时间范围', exact: true }).click();
    const menu = page.locator('.select-menu-dropdown');
    const assertMenuTheme = async (scheme, id) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.waitForFunction(id => document.documentElement.dataset.themeId === id, id);
      await page.waitForFunction(() => {
        const option = document.querySelector('.select-menu-option');
        const trigger = document.querySelector('.select-menu-trigger');
        return option && trigger && getComputedStyle(option).color === getComputedStyle(trigger).color;
      });
      assert.equal(await menu.count(), 1, 'system changes keep the menu open');
      const paint = await menu.evaluate(element => ({
        foreground: getComputedStyle(element.querySelector('.select-menu-option')).color,
        background: getComputedStyle(element).backgroundColor,
      }));
      assert.ok(contrast(paint.foreground, paint.background) >= 4.5, `${id} open menu: ${JSON.stringify(paint)}`);
    };
    await assertMenuTheme('dark', 'graphite-dark');
    await page.evaluate(async () => (await import('/src/theme.ts')).setSystemTheme('sakura-light'));
    await assertMenuTheme('light', 'sakura-light');
    await page.waitForFunction(() => document.querySelector('.select-menu-theme-scope').style.getPropertyValue('--theme-primary-button-text') !== '');
    await assertMenuTheme('dark', 'graphite-dark');
    await page.waitForFunction(() => document.querySelector('.select-menu-theme-scope').style.getPropertyValue('--theme-primary-button-text') === '');
    await page.keyboard.press('Escape');

    await page.getByRole('button', { name: '高级功能', exact: true }).click();
    const remote = page.getByRole('checkbox', { name: '允许远程管理', exact: true });
    await remote.waitFor();
    for (const activeThumb of [null, '#6A3554']) {
      await remote.uncheck();
      await page.evaluate(async activeThumb => {
        const { createThemeRegistry, themeRegistry } = await import('/src/themes/registry.ts');
        const { createThemeApplicator } = await import('/src/themes/applyTheme.ts');
        const baseTheme = themeRegistry.get('sakura-light');
        const custom = { ...baseTheme, id: 'test-control-light', tokens: {
          ...baseTheme.tokens, controlThumb: '#843E64', controlActiveBackground: '#F6E3ED',
          ...(activeThumb ? { controlActiveThumb: activeThumb } : {}),
        } };
        const registry = createThemeRegistry([...themeRegistry.themes, custom]);
        createThemeApplicator(document)(registry.get(custom.id));
      }, activeThumb);
      assert.equal(await remote.evaluate(input => getComputedStyle(input.nextElementSibling, '::after').backgroundColor), 'rgb(132, 62, 100)');
      await remote.check();
      await remote.evaluate(async input => { await Promise.all(input.nextElementSibling.getAnimations().map(animation => animation.finished)); });
      const paint = await remote.evaluate(input => ({
        thumb: getComputedStyle(input.nextElementSibling, '::after').backgroundColor,
        background: getComputedStyle(input.nextElementSibling).backgroundColor,
      }));
      assert.equal(paint.thumb, activeThumb ? 'rgb(106, 53, 84)' : 'rgb(132, 62, 100)', 'active state honors theme thumb roles');
      assert.ok(contrast(paint.thumb, paint.background) >= 3, `custom control: ${JSON.stringify(paint)}`);
    }
    for (const [id, expectedThumb] of [['light', 'rgb(255, 255, 255)'], ['dark', 'rgb(11, 13, 17)']]) {
      await page.evaluate(async id => {
        const { themeRegistry } = await import('/src/themes/registry.ts');
        const { createThemeApplicator } = await import('/src/themes/applyTheme.ts');
        createThemeApplicator(document)(themeRegistry.get(id));
      }, id);
      await remote.evaluate(async input => { await Promise.all(input.nextElementSibling.getAnimations().map(animation => animation.finished)); });
      const paint = await remote.evaluate(input => ({
        thumb: getComputedStyle(input.nextElementSibling, '::after').backgroundColor,
        background: getComputedStyle(input.nextElementSibling).backgroundColor,
        activeOverride: document.documentElement.style.getPropertyValue('--theme-control-active-thumb'),
      }));
      assert.equal(paint.activeOverride, '', 'default themes clear configured and derived active thumbs');
      assert.equal(paint.thumb, expectedThumb);
      assert.ok(contrast(paint.thumb, paint.background) >= 3, `${id} control: ${JSON.stringify(paint)}`);
    }
    await remote.uncheck();
    assert.deepEqual(errors, []);
    console.log('PASS: real quota/health button contrast, open menus across system theme changes and token cleanup, configured/active switch thumbs and default fallbacks.');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
