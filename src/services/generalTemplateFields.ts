import type { TemplateConfigField, TemplateConfigGroup } from './templateConfig';

const text = (zh: string, en: string, ja: string) => ({ zh, en, ja });
const field = (path: string, type: TemplateConfigField['type'], zh: string, en: string, ja: string, defaultValue: unknown, description?: ReturnType<typeof text>, extra: Partial<TemplateConfigField> = {}): TemplateConfigField => ({ path: path.split('.'), type, label: text(zh, en, ja), defaultValue, description, ...extra });

export const generalTemplateGroups: readonly TemplateConfigGroup[] = [
  {
    id: 'management', title: text('管理接口与面板', 'Management API & panel', '管理 API とパネル'),
    fields: [
      field('management.allow-remote', 'boolean', '允许远程管理', 'Allow remote management', 'リモート管理を許可', false, text('允许非本机访问管理接口，仍需管理密钥。', 'Allow non-local management requests; a management key is still required.', '他の端末からの管理 API アクセスを許可します。管理キーは引き続き必要です。')),
      field('management.disable-control-panel', 'boolean', '禁用内置管理面板', 'Disable bundled control panel', '内蔵管理パネルを無効化', false),
      field('management.disable-auto-update-panel', 'boolean', '禁用面板自动更新', 'Disable automatic panel updates', 'パネルの自動更新を無効化', false, text('缺少面板时仍会在首次访问时下载。', 'A missing panel is still downloaded on first access.', 'パネルが存在しない場合は、初回アクセス時にダウンロードします。')),
      field('management.panel-github-repository', 'string', '管理面板仓库', 'Panel GitHub repository', '管理パネルの GitHub リポジトリ', 'https://github.com/router-for-me/Cli-Proxy-API-Management-Center', text('支持 GitHub 仓库 URL 或 releases API URL。', 'Accepts a GitHub repository URL or releases API URL.', 'GitHub リポジトリ URL または releases API URL を指定できます。')),
      field('server.github-token', 'string', 'GitHub 访问令牌', 'GitHub access token', 'GitHub アクセストークン', '', text('插件商店和面板更新查询 GitHub 时使用，避免匿名请求被限流（每小时 60 次）。令牌只发送给 api.github.com；留空时使用 GITHUB_TOKEN 环境变量。', 'Used by the plugin store and panel updates when querying GitHub, avoiding the anonymous rate limit of 60 requests per hour. The token is only sent to api.github.com; leave blank to use the GITHUB_TOKEN environment variable.', 'プラグインストアとパネル更新が GitHub に問い合わせる際に使用し、匿名リクエストの制限（1 時間 60 回）を回避します。トークンは api.github.com にのみ送信されます。空欄では GITHUB_TOKEN 環境変数を使用します。'), { sensitive: true, placeholder: 'ghp_…', link: { url: 'https://github.com/settings/tokens/new?description=EasyCLIProxyAPI', label: text('创建令牌', 'Create token', 'トークンを作成'), hint: text('无需任何权限：在打开的页面中选择有效期，权限（Select scopes）全部保持不勾选，点击底部 Generate token，再将 ghp_ 开头的令牌粘贴到上方。', 'No permissions needed: on the page that opens, choose an expiration, leave every scope unchecked, click Generate token at the bottom, then paste the ghp_ token above.', '権限は不要です。開いたページで有効期限を選び、スコープ（Select scopes）はすべて未選択のまま下部の Generate token をクリックし、ghp_ で始まるトークンを上に貼り付けてください。') } }),
      field('management.base-url', 'string', 'TUI 管理接口地址', 'TUI management base URL', 'TUI 管理 API のベース URL', '', text('仅用于内核 TUI 客户端模式，不更改桌面应用的连接地址。留空使用本机端口。', 'Used only by the core TUI client. Does not change the desktop connection. Leave blank to use the local port.', 'コアの TUI クライアントモード専用です。デスクトップアプリの接続先は変わりません。空欄ではローカルポートを使用します。')),
    ],
  },
  {
    id: 'diagnostics', title: text('请求日志与诊断', 'Request logging & diagnostics', 'リクエストログと診断'),
    description: text('应用日志、请求报文日志和用量统计独立配置。', 'Application logs, request/response logs and usage aggregation are configured separately.', 'アプリログ、リクエストと応答のログ、使用量集計は個別に設定します。'),
    fields: [
      field('observability.logs.request-log', 'boolean', '记录请求与响应', 'Log requests and responses', 'リクエストと応答を記録', false, text('请求日志可能包含请求与响应内容。', 'Request logs may include request and response content.', 'ログにはリクエストや応答の内容が含まれる場合があります。')),
      field('observability.pprof.enable', 'boolean', '启用性能诊断服务', 'Enable pprof server', 'pprof サーバーを有効化', false),
      field('observability.pprof.addr', 'string', '性能诊断监听地址', 'pprof listen address', 'pprof の待ち受けアドレス', '127.0.0.1:8316'),
    ],
  },
];

export const routingTemplateGroups: readonly TemplateConfigGroup[] = [
  {
    id: 'routing-advanced', title: text('会话与冷却策略', 'Session & cooldown behavior', 'セッションとクールダウンの動作'),
    fields: [
      field('routing.session-affinity-subagents', 'boolean', '子代理继承父会话凭据', 'Bind subagents to parent credentials', '子エージェントで親セッションの認証情報を使用', true, text('仅在启用会话粘性时生效。关闭后子代理按回退选择器分配。', 'Effective only with session affinity. When disabled, subagents use the fallback selector.', 'セッション固定が有効な場合のみ適用します。無効にすると子エージェントは代替選択方式を使用します。')),
      field('routing.force-model-prefix', 'boolean', '强制模型前缀匹配', 'Enforce model prefix matching', 'モデル接頭辞の一致を必須にする', false, text('无前缀模型请求只使用无前缀凭据，前缀等于模型名的情况除外。', 'Unprefixed requests only use credentials without a prefix, except when prefix equals model name.', '接頭辞のないモデル要求には接頭辞のない認証情報のみを使用します。接頭辞とモデル名が一致する場合を除きます。')),
      field('routing.cooldown.save-cooldown-status', 'boolean', '持久保存冷却状态', 'Persist cooldown status', 'クールダウン状態を永続化', false, text('将凭据冷却状态保存到认证文件旁的 .cds 文件。', 'Save per-credential cooldown state in .cds files next to auth files.', '認証情報ごとのクールダウン状態を認証ファイルと同じ場所の .cds ファイルに保存します。')),
      field('routing.cooldown.transient-error-cooldown-seconds', 'number', '瞬时错误冷却秒数', 'Transient error cooldown (seconds)', '一時的なエラーのクールダウン秒数', 0, text('0 沿用 60 秒；-1 禁用瞬时错误冷却。', 'Zero uses the legacy 60 seconds; -1 disables transient error cooldown.', '0 は従来の 60 秒を使用し、-1 は一時的なエラーのクールダウンを無効化します。'), { min: -1, max: 2147483647 }),
    ],
  },
];

export const requestTemplateGroups: readonly TemplateConfigGroup[] = [
  {
    id: 'request-behavior', title: text('请求与连接保持', 'Requests & keep-alives', 'リクエストと接続維持'), fields: [
      field('requests.passthrough-headers', 'boolean', '透传上游响应头', 'Pass through upstream response headers', '上流の応答ヘッダーを転送', false, text('转发经过过滤的上游响应头。', 'Forward filtered upstream response headers.', 'フィルター済みの上流応答ヘッダーを転送します。')),
      field('requests.nonstream-keepalive-interval', 'number', '非流式保活间隔（秒）', 'Non-streaming keep-alive interval (seconds)', '非ストリーミングの接続維持間隔（秒）', 0, text('大于 0 时定期发送空行，减少空闲超时。', 'When positive, periodically emit blank lines to prevent idle timeouts.', '正の値では定期的に空行を送信し、アイドルタイムアウトを防ぎます。'), { min: -2147483648, max: 2147483647 }),
      field('requests.streaming.keepalive-seconds', 'number', '流式保活间隔（秒）', 'Streaming keep-alive interval (seconds)', 'ストリーミングの接続維持間隔（秒）', 0, text('0 或负数禁用。', 'Zero or negative disables keep-alives.', '0 または負の値で接続維持を無効化します。'), { min: -2147483648, max: 2147483647 }),
    ],
  },
  {
    id: 'codex-client', title: text('Codex 客户端兼容', 'Codex client compatibility', 'Codex クライアントの互換性'), fields: [
      field('client.codex.enable-apply-patch', 'boolean', '公开 apply_patch 工具能力', 'Advertise apply_patch support', 'apply_patch の対応状況を公開', false, text('在客户端模型目录中，将所有路由执行器均支持的模型标记为支持 freeform；关闭时清空 apply_patch_tool_type。', 'Advertise freeform in client model catalogs only for models supported by every routing executor. When disabled, clear apply_patch_tool_type.', 'すべてのルーティング実行器が対応するモデルのみ、クライアントのモデル一覧で freeform 対応を公開します。無効にすると apply_patch_tool_type を空にします。')),
      field('client.codex.optimize-multi-agent-v2', 'boolean', '启用多代理优化 v2', 'Enable multi-agent optimization v2', 'マルチエージェント最適化 v2 を有効化', false, text('刷新 spawn_agent 模型说明、去除消息参数加密并规范化代理消息；同时适用于 OAuth 和 API 密钥请求。', 'Refresh spawn_agent model descriptions, remove message parameter encryption and normalize agent messages for both OAuth and API-key requests.', 'spawn_agent のモデル説明を更新し、メッセージパラメーターの暗号化を解除して、エージェントのメッセージを正規化します。OAuth と API キーの両方の要求に適用します。')),
    ],
  },
  {
    id: 'multimedia', title: text('图像与视频', 'Images & video', '画像と動画'), fields: [
      field('multimedia.disable-image-generation', 'select', '图像生成策略', 'Image generation policy', '画像生成の動作', false, undefined, { options: [
        { value: false, label: text('启用（默认）', 'Enabled (default)', '有効（既定）') },
        { value: true, label: text('禁用图像生成', 'Disable image generation', '画像生成を無効化') },
        { value: 'chat', label: text('仅保留图像接口', 'Image endpoints only', '画像 API のみ有効') },
        { value: 'passthrough', label: text('透传模式', 'Passthrough mode', 'そのまま転送') },
      ] }),
      field('multimedia.gpt-image-2-base-model', 'string', 'GPT Image 2 基础模型', 'GPT Image 2 base model', 'GPT Image 2 の基本モデル', 'gpt-5.4-mini', text('留空使用 gpt-5.4-mini；自定义模型名称以 gpt- 开头。', 'Leave blank to use gpt-5.4-mini. Custom model names start with gpt-.', '空欄では gpt-5.4-mini を使用します。カスタムモデル名は gpt- で始まります。'), { validate: (value) => typeof value === 'string' && (!value || value.toLowerCase().startsWith('gpt-')) ? null : text('模型名称必须以 gpt- 开头，或留空使用默认值。', 'Model name must start with gpt-, or leave blank for the default.', 'gpt- で始まるモデル名を入力するか、空欄で既定値を使用してください。') }),
      field('multimedia.video-result-auth-cache-ttl', 'string', '视频结果认证缓存时长', 'Video result auth cache TTL', '動画結果の認証キャッシュ有効期間', '3h', text('使用内核时长格式，例如 30m 或 3h。', 'Use a core duration such as 30m or 3h.', '30m や 3h など、コアの時間表記を使用してください。')),
    ],
  },
];
