export type WebviewBounds = { x: number; y: number; width: number; height: number; viewportHeight: number };

export const EMPTY_BOUNDS: WebviewBounds = { x: 0, y: 0, width: 0, height: 0, viewportHeight: 0 };

export const roundBounds = ({ left, top, width, height }: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>, viewportHeight: number): WebviewBounds => ({
  x: Math.max(0, Math.round(left)),
  y: Math.max(0, Math.round(top)),
  width: Math.max(0, Math.round(width)),
  height: Math.max(0, Math.round(height)),
  viewportHeight: Math.max(0, Math.round(viewportHeight)),
});

export const hasArea = (bounds: WebviewBounds): boolean => bounds.width > 0 && bounds.height > 0;

export const sameBounds = (a: WebviewBounds, b: WebviewBounds): boolean =>
  a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height && a.viewportHeight === b.viewportHeight;

export const effectiveBounds = ({ active, overlay, bounds }: { active: boolean; overlay: boolean; bounds: WebviewBounds }): WebviewBounds =>
  active && !overlay && hasArea(bounds) ? bounds : EMPTY_BOUNDS;

let lastToken = 0;

export const newWebviewToken = (): number => {
  lastToken = Math.max(lastToken + 1, Date.now() * 1000);
  return lastToken;
};
