import type { ThemeDefinition } from './types';

// The default palette stays in the shared base styles.
export const dark: ThemeDefinition = {
  id: 'dark', nameKey: 'app.theme.dark', appearance: 'dark', order: 0,
  windowBackground: '#0b0d11', tokens: {},
};
