import { getCurrentLocale, translate } from '../i18n';

const normalize = (value: string) => value.trim().toLowerCase().replace(/\s+/g, ' ');
const text = (key: Parameters<typeof translate>[1], variables?: Parameters<typeof translate>[2]) =>
  translate(getCurrentLocale(), key, variables);

export const antigravityWindow = (label: string, window: string): string => {
  const name = normalize(label).replace(/\s*(?:-|:)?\s*remaining$/, '').trim();
  if (/^(5[ -]hour|five[ -]hour) limit$/.test(name)) return '5h';
  if (name === 'weekly limit') return 'weekly';
  if (name === 'daily limit') return 'daily';
  if (name === 'monthly limit') return 'monthly';
  const raw = normalize(window);
  return ({ 'five-hour': '5h', five_hour: '5h', week: 'weekly', day: 'daily', month: 'monthly' } as Record<string, string>)[raw] ?? raw;
};

export const antigravityBucketLabel = (label: string, window: string): string => {
  switch (antigravityWindow(label, window)) {
    case '5h': return text('quota.service.limit.fiveHours');
    case 'weekly': return text('quota.service.limit.week');
    case 'monthly': return text('quota.service.limit.month');
    case 'daily': return text('quota.service.antigravity.daily');
    default: return label || window;
  }
};

export const antigravityGroupLabel = (label: string): string => {
  if (normalize(label) === 'gemini models') return text('quota.service.antigravity.gemini');
  if (normalize(label) === 'claude and gpt models') return text('quota.service.antigravity.claudeGpt');
  return label;
};

export const antigravityDescription = (label: string, description: string): string => {
  const match = description.trim().match(/^models within this group:\s*(.+)$/i);
  if (!match) return description;
  const models = match[1].split(',').map(model => model.trim()).filter(Boolean);
  if (normalize(label) === 'gemini models' && !models.some(model => normalize(model) === 'gemini argon')) {
    models.push('Gemini Argon');
  }
  return text('quota.service.antigravity.models', { models: models.join(', ') });
};
