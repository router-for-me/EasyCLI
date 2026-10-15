import { describe, expect, test } from 'bun:test';
import { EMPTY_BOUNDS, effectiveBounds, hasArea, newWebviewToken, roundBounds, sameBounds } from '../src/services/pluginWebview';

describe('plugin webview bounds', () => {
  test('rounds measured rectangles and clamps negatives', () => {
    expect(roundBounds({ left: 10.4, top: 20.6, width: 300.5, height: 200.2 }, 767.6)).toEqual({ x: 10, y: 21, width: 301, height: 200, viewportHeight: 768 });
    expect(roundBounds({ left: -3, top: -0.4, width: -5, height: 10 }, 600)).toEqual({ x: 0, y: 0, width: 0, height: 10, viewportHeight: 600 });
  });

  test('requires a positive area to be visible', () => {
    expect(hasArea({ x: 0, y: 0, width: 1, height: 1, viewportHeight: 10 })).toBe(true);
    expect(hasArea({ x: 5, y: 5, width: 0, height: 10, viewportHeight: 10 })).toBe(false);
    expect(hasArea(EMPTY_BOUNDS)).toBe(false);
    expect(sameBounds({ x: 1, y: 2, width: 3, height: 4, viewportHeight: 9 }, { x: 1, y: 2, width: 3, height: 4, viewportHeight: 9 })).toBe(true);
    expect(sameBounds({ x: 1, y: 2, width: 3, height: 4, viewportHeight: 9 }, { x: 1, y: 2, width: 3, height: 5, viewportHeight: 9 })).toBe(false);
    expect(sameBounds({ x: 1, y: 2, width: 3, height: 4, viewportHeight: 9 }, { x: 1, y: 2, width: 3, height: 4, viewportHeight: 8 })).toBe(false);
  });

  test('shows the native view only when active, unobstructed and non-empty', () => {
    const bounds = { x: 10, y: 20, width: 300, height: 200, viewportHeight: 768 };
    expect(effectiveBounds({ active: true, overlay: false, bounds })).toEqual(bounds);
    expect(effectiveBounds({ active: false, overlay: false, bounds })).toEqual(EMPTY_BOUNDS);
    expect(effectiveBounds({ active: true, overlay: true, bounds })).toEqual(EMPTY_BOUNDS);
    expect(effectiveBounds({ active: true, overlay: false, bounds: { ...bounds, height: 0 } })).toEqual(EMPTY_BOUNDS);
  });

  test('issues strictly increasing tokens that stay ahead of earlier sessions', () => {
    const tokens = Array.from({ length: 5 }, newWebviewToken);
    tokens.slice(1).forEach((token, index) => expect(token).toBeGreaterThan(tokens[index]));
    expect(tokens.every(Number.isSafeInteger)).toBe(true);
    expect(tokens[0]).toBeGreaterThanOrEqual(Date.now() * 1000 - 1_000_000);
  });
});
