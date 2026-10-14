import type { ThemeDefinition } from './types';

// The default palette stays in the shared base styles.
export const light: ThemeDefinition = {
  id: 'light', nameKey: 'app.theme.light', appearance: 'light', order: 0,
  windowBackground: '#ffffff', tokens: {},
};
