export const oauthSubpages = [
  { id: 'login', labelKey: 'oauth.title' },
  { id: 'workbuddyTasks', labelKey: 'workbuddyTasks.title' },
  { id: 'authFiles', labelKey: 'authFiles.title' },
] as const;

export type OAuthSubpage = (typeof oauthSubpages)[number]['id'];
