import type { TemplateText } from './templateConfig';
import { generalTemplateGroups, requestTemplateGroups, routingTemplateGroups } from './generalTemplateFields';
import { oauthTemplateGroups } from './oauthTemplateFields';
import { extensionTemplateGroups } from './extensionTemplateFields';
import { payloadTemplateGroups } from './payloadTemplateFields';

const text = (zh: string, en: string, ja: string): TemplateText => ({ zh, en, ja });
export const settingsCategories = [
  { id: 'general', title: text('服务与访问', 'Service & access', 'サービスとアクセス') },
  { id: 'aliases', title: text('模型别名', 'Model aliases', 'モデル別名') },
  { id: 'routing', title: text('路由与稳定性', 'Routing & reliability', 'ルーティングと安定性') },
  { id: 'requests', title: text('模型与请求', 'Models & requests', 'モデルとリクエスト') },
  { id: 'oauth', title: text('上游与凭据', 'Upstreams & credentials', '上流と認証情報') },
  { id: 'diagnostics', title: text('日志与诊断', 'Logs & diagnostics', 'ログと診断') },
  { id: 'software', title: text('应用偏好', 'App preferences', 'アプリの設定') },
] as const;
export type SettingsCategory = typeof settingsCategories[number]['id'];
export const settingsTemplateGroups = {
  general: generalTemplateGroups.filter(group => group.id === 'management'),
  aliases: [],
  routing: [...routingTemplateGroups, ...extensionTemplateGroups.filter(group => group.id === 'extensions-concurrency')],
  requests: [...requestTemplateGroups, ...payloadTemplateGroups],
  oauth: ['oauth-common', 'oauth-codex', 'oauth-others', 'oauth-claude', 'oauth-models', 'oauth-media']
    .map(id => oauthTemplateGroups.find(group => group.id === id)!),
  diagnostics: [...generalTemplateGroups.filter(group => group.id === 'diagnostics'), ...extensionTemplateGroups.filter(group => group.id === 'extensions-inflight')],
  software: [],
};
export const allSettingsTemplateGroups = Object.values(settingsTemplateGroups).flat();
export const settingsMessages = {
  title: text('高级设置', 'Advanced settings', '詳細設定'),
  search: text('搜索设置', 'Search settings', '設定を検索'),
  searchHint: text('搜索设置…', 'Search settings…', '設定を検索…'),
  clearSearch: text('清空搜索', 'Clear search', '検索をクリア'),
  results: text('搜索结果', 'Search results', '検索結果'),
  noResults: text('没有找到匹配的设置，试试其他关键词。', 'No matching settings. Try another keyword.', '一致する設定がありません。別のキーワードをお試しください。'),
  restart: text('保存后重启内核', 'Core restarts after saving', '保存後にコアを再起動'),
  automatic: text('自动保存', 'Auto-save', '自動保存'),
  dirty: text('有未保存修改', 'Unsaved changes', '未保存の変更'),
  nativeNetwork: text('监听与上游代理', 'Listening & upstream proxy', '待ち受けと上流プロキシ'),
  nativeRouting: text('凭据选择与会话', 'Credential selection & sessions', '認証情報の選択とセッション'),
  nativeRetry: text('重试与冷却', 'Retries & cooldowns', '再試行とクールダウン'),
} satisfies Record<string, TemplateText>;

// The settings page is mounted only while active, so another page hands over the
// field to open once instead of dispatching an event nobody is listening to yet.
let requestedSettingPath: string | null = null;
export const openSettingField = (path: readonly string[]) => {
  requestedSettingPath = path.join('.');
  window.dispatchEvent(new CustomEvent('app:navigate', { detail: 'config' }));
};
export const takeRequestedSettingField = () => {
  const path = requestedSettingPath;
  requestedSettingPath = null;
  for (const category of settingsCategories) for (const group of settingsTemplateGroups[category.id]) {
    const index = group.fields.findIndex(field => field.path.join('.') === path);
    if (index >= 0) return { category: category.id, target: `config-group-${group.id}`, field: `template-field-${group.id}-${index}` };
  }
  return null;
};
