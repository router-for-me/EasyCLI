import type { ThemeBackdrop, ThemeGradient, ThemeGradientStop } from './types';

const color = (value: unknown): value is string => typeof value === 'string' && /^#(?:[\da-f]{6}|[\da-f]{8})$/i.test(value);
const numberInRange = (value: unknown, min: number, max: number): value is number =>
  typeof value === 'number' && Number.isFinite(value) && value >= min && value <= max;
const pair = (value: unknown, min: number, max: number): value is [number, number] =>
  Array.isArray(value) && value.length === 2 && value.every((item) => numberInRange(item, min, max));

export function normalizeBackdrop(backdrop: ThemeBackdrop, id: string): ThemeBackdrop {
  const invalid = (): never => { throw new Error(`Invalid theme backdrop: ${id}`); };
  if (!backdrop || !color(backdrop.sidebar) || !color(backdrop.panel)
    || (backdrop.inset !== undefined && !color(backdrop.inset))
    || (backdrop.hover !== undefined && !color(backdrop.hover))
    || !Array.isArray(backdrop.layers) || !numberInRange(backdrop.layers.length, 1, 4)) return invalid();
  const layers = backdrop.layers.map((layer): ThemeGradient => {
    if (!layer || !Array.isArray(layer.stops) || !numberInRange(layer.stops.length, 2, 8)) return invalid();
    const stops = Object.freeze(layer.stops.map((stop: ThemeGradientStop, index: number, all: readonly ThemeGradientStop[]) => {
      if (!stop || !color(stop.color) || !numberInRange(stop.at, 0, 100)
        || (index > 0 && stop.at < all[index - 1].at)) return invalid();
      return Object.freeze({ color: stop.color, at: stop.at });
    }));
    if (layer.kind === 'linear' && numberInRange(layer.angle, 0, 360)) {
      return Object.freeze({ kind: 'linear', angle: layer.angle, stops });
    }
    if (layer.kind === 'radial' && pair(layer.center, 0, 100) && pair(layer.size, 1, 200)) {
      return Object.freeze({ kind: 'radial',
        center: Object.freeze([...layer.center]) as readonly [number, number],
        size: Object.freeze([...layer.size]) as readonly [number, number], stops });
    }
    return invalid();
  });
  return Object.freeze({ layers: Object.freeze(layers), sidebar: backdrop.sidebar, panel: backdrop.panel,
    ...(backdrop.inset === undefined ? {} : { inset: backdrop.inset }),
    ...(backdrop.hover === undefined ? {} : { hover: backdrop.hover }),
  });
}

export function backdropImage(backdrop: ThemeBackdrop): string {
  return backdrop.layers.map((layer) => {
    const stops = layer.stops.map((stop) => `${stop.color} ${stop.at}%`).join(', ');
    return layer.kind === 'linear'
      ? `linear-gradient(${layer.angle}deg, ${stops})`
      : `radial-gradient(ellipse ${layer.size[0]}% ${layer.size[1]}% at ${layer.center[0]}% ${layer.center[1]}%, ${stops})`;
  }).join(', ');
}
