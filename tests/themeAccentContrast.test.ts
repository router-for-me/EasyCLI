import { expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { themeRegistry } from '../src/themes/registry';

const source = fileURLToPath(new URL('../src/', import.meta.url));
const accentFill = /background(?:-color)?:\s*var\(--(?:ui-accent|ui-accent-strong|primary-color|primary-hover|primary-active)[,)]/;

const luminance = (hex: string) => {
  const [r, g, b] = [1, 3, 5].map((index) => {
    const channel = parseInt(hex.slice(index, index + 2), 16) / 255;
    return channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a: string, b: string) => {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
};

test('text on accent fills uses the accent contrast color', async () => {
  const offenders: string[] = [];
  for await (const file of new Bun.Glob('**/*.css').scan(source)) {
    const css = await readFile(`${source}${file}`, 'utf8');
    for (const [, selector, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      if (!accentFill.test(body)) continue;
      for (const [, value] of body.matchAll(/(?<![-\w])color:\s*([^;]+)/g)) {
        if (!value.includes('var(')) offenders.push(`${file}|${selector.trim()}|${value.trim()}`);
      }
    }
  }
  expect(offenders).toEqual([]);
});

test('default accents keep readable contrast text', async () => {
  const css = await readFile(`${source}styles.css`, 'utf8');
  const value = (block: string, name: string) => block.match(new RegExp(`--${name}:\\s*(#[\\da-f]{6})`, 'i'))![1];
  const light = css.slice(css.indexOf(':root {'));
  const dark = css.slice(css.indexOf(":root[data-theme='dark'] {"));
  for (const block of [light, dark]) {
    expect(contrast(value(block, 'primary-color'), value(block, 'primary-contrast'))).toBeGreaterThanOrEqual(4.5);
  }
  for (const theme of themeRegistry.themes) {
    const { accent, accentContrast } = theme.tokens;
    if (accent && accentContrast) expect(contrast(accent.slice(0, 7), accentContrast.slice(0, 7))).toBeGreaterThanOrEqual(4.5);
  }
});
