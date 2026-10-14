import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Check,
  Copy,
  ExternalLink,
  Layers,
  LoaderCircle,
  LogIn,
  Play,
  RefreshCw,
  RotateCw,
  Square,
  Zap,
} from 'lucide-react';
import workbuddyIcon from '../assets/icons/workbuddy.png';
import { useI18n } from '../i18n';
import { useAppNotice, FloatingNotice } from '../appNotice';
import './WorkBuddyProxyPage.css';

interface WorkBuddyProxyStatus {
  running: boolean;
  ready: boolean;
  host: string;
  port: number;
  pid: number | null;
  pythonPath: string | null;
  accountsCount: number;
  baseUrl: string;
  accountsDir: string;
  message: string;
  error: string | null;
}

interface DesktopAccount {
  fileName: string;
  filePath: string;
  uid: string | null;
  nickname: string | null;
  realm: string;
  domain: string | null;
  encrypted: boolean;
  lastModified: string | null;
}

const WORKBUDDY_MODELS = [
  { id: 'default-model', name: 'Auto (通用编码)', desc: '腾讯自研/精选编码模型，适合日常通用任务' },
  { id: 'fast-model', name: 'Fast (极速响应)', desc: '极速推理通道，适合代码补全与轻量问答' },
  { id: 'balanced-model', name: 'Balanced (平衡精选)', desc: '兼顾速度与质量，平衡主力模型' },
  { id: 'primary-model', name: 'Primary (旗舰高阶)', desc: '高复杂工程挑战模型，高质量长文本分析' },
  { id: 'deep-model', name: 'Deep (深度推理)', desc: '深度思考分析链，针对数学与疑难算法' },
  { id: 'deepseek-v4.1-flash', name: 'DeepSeek Flash', desc: '深度求索超快闪电模型' },
  { id: 'claude-3-7-sonnet', name: 'Claude 3.7 Sonnet', desc: 'Anthropic 前沿混合思考模型' },
  { id: 'claude-3-5-sonnet-20241022', name: 'Claude 3.5 Sonnet', desc: '标杆级全能编码推理模型' },
  { id: 'gpt-4o', name: 'GPT-4o', desc: 'OpenAI 多模态旗舰' },
  { id: 'deepseek-chat', name: 'DeepSeek V3 (Chat)', desc: '深度求索通用对话模型' },
  { id: 'deepseek-reasoner', name: 'DeepSeek R1 (Reasoner)', desc: '深度求索推理模型' },
];

export function WorkBuddyProxyPage() {
  const { t } = useI18n();
  const notice = useAppNotice();

  const [status, setStatus] = useState<WorkBuddyProxyStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [actionBusy, setActionBusy] = useState(false);
  const [portInput, setPortInput] = useState<number>(8788);
  const [hostInput, setHostInput] = useState<string>('127.0.0.1');
  const [apiKeyInput, setApiKeyInput] = useState<string>('');
  const [desktopAccounts, setDesktopAccounts] = useState<DesktopAccount[]>([]);
  const [scanningDesktop, setScanningDesktop] = useState(false);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  // Load Status
  const refreshStatus = useCallback(async () => {
    try {
      const current = await invoke<WorkBuddyProxyStatus>('get_workbuddy_proxy_status');
      setStatus(current);
      if (current.port) setPortInput(current.port);
      if (current.host) setHostInput(current.host);
    } catch (err) {
      console.error('Failed to get WorkBuddy proxy status:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  // Scan Desktop Accounts
  const scanDesktop = useCallback(async () => {
    setScanningDesktop(true);
    try {
      const accounts = await invoke<DesktopAccount[]>('detect_workbuddy_desktop_accounts');
      setDesktopAccounts(accounts);
    } catch (err) {
      console.warn('Failed to detect desktop accounts:', err);
    } finally {
      setScanningDesktop(false);
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
    void scanDesktop();
    const interval = setInterval(refreshStatus, 4000);
    return () => clearInterval(interval);
  }, [refreshStatus, scanDesktop]);

  const handleStart = async () => {
    setActionBusy(true);
    try {
      const result = await invoke<WorkBuddyProxyStatus>('start_workbuddy_proxy', {
        port: portInput,
        host: hostInput,
        apiKey: apiKeyInput ? apiKeyInput.trim() : null,
      });
      setStatus(result);
      notice.showNotice('WorkBuddy 反代服务已启动', 'success');
    } catch (err) {
      notice.showNotice(`启动失败: ${err}`, 'error');
    } finally {
      setActionBusy(false);
      void refreshStatus();
    }
  };

  const handleStop = async () => {
    setActionBusy(true);
    try {
      const result = await invoke<WorkBuddyProxyStatus>('stop_workbuddy_proxy');
      setStatus(result);
      notice.showNotice('WorkBuddy 反代服务已停止', 'info');
    } catch (err) {
      notice.showNotice(`停止失败: ${err}`, 'error');
    } finally {
      setActionBusy(false);
      void refreshStatus();
    }
  };

  const handleRestart = async () => {
    setActionBusy(true);
    try {
      const result = await invoke<WorkBuddyProxyStatus>('restart_workbuddy_proxy', {
        port: portInput,
        host: hostInput,
        apiKey: apiKeyInput ? apiKeyInput.trim() : null,
      });
      setStatus(result);
      notice.showNotice('WorkBuddy 反代服务已重启', 'success');
    } catch (err) {
      notice.showNotice(`重启失败: ${err}`, 'error');
    } finally {
      setActionBusy(false);
      void refreshStatus();
    }
  };

  const handleOpenDashboard = () => {
    const port = status?.port ?? portInput;
    const url = `http://127.0.0.1:${port}`;
    invoke('open_external_url', { url }).catch(() => {
      window.open(url, '_blank');
    });
  };

  const handleImportDesktop = async (filePath?: string) => {
    setActionBusy(true);
    try {
      await invoke('import_workbuddy_desktop_account', { targetPath: filePath });
      notice.showNotice('桌面端账号已成功导入到反代池', 'success');
      void refreshStatus();
    } catch (err) {
      notice.showNotice(`导入桌面凭证失败: ${err}`, 'error');
    } finally {
      setActionBusy(false);
    }
  };

  const handleSyncToCpa = async () => {
    setActionBusy(true);
    try {
      const msg = await invoke<string>('sync_workbuddy_provider_to_cpa', {
        port: status?.port ?? portInput,
        apiKey: apiKeyInput ? apiKeyInput.trim() : null,
      });
      notice.showNotice(msg, 'success');
    } catch (err) {
      notice.showNotice(`同步到 API 接入失败: ${err}`, 'error');
    } finally {
      setActionBusy(false);
    }
  };

  const handleCopy = (text: string, key: string) => {
    void navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 2000);
    notice.showNotice('已复制到剪贴板', 'success');
  };

  const currentPort = status?.port ?? portInput;
  const currentHost = status?.host ?? hostInput;
  const baseUrl = `http://${currentHost}:${currentPort}/v1`;

  const isRunning = Boolean(status?.running);
  const isReady = Boolean(status?.ready);

  return (
    <div className="workbuddy-page">
      <FloatingNotice notice={notice.notice} onDismiss={notice.clearNotice} />

      {/* Header */}
      <header className="workbuddy-header">
        <div className="workbuddy-title-group">
          <img src={workbuddyIcon} alt="WorkBuddy Logo" className="workbuddy-logo" />
          <div>
            <h1>WorkBuddy 反向代理</h1>
            <p>腾讯 WorkBuddy &amp; CodeBuddy 国际与国内双区反代中枢，支持多账号池与 OpenAI / Claude / Codex 协议转换</p>
          </div>
        </div>

        <div className="workbuddy-status-pill-container">
          <span className={`workbuddy-status-pill ${isRunning && isReady ? 'running' : isRunning ? 'starting' : 'stopped'}`}>
            <span className="workbuddy-dot" />
            {isRunning && isReady ? '服务运行中' : isRunning ? '服务启动中' : '服务已停止'}
          </span>
        </div>
      </header>

      {/* Main Grid */}
      <div className="workbuddy-grid">
        <div className="workbuddy-notice-box">
          <strong>💡 已与 EasyCLIProxyAPI 合二为一（零配置伴生引擎）</strong>：
          WorkBuddy 反代已深度接入内核生命周期。当你在首页启动内核时，WorkBuddy 伴生引擎会自动在后台静默就绪，并自动将所有模型挂载到 EasyCLIProxyAPI 中。你的所有客户端（Claude Code、Codex、Cursor、NextChat 等）只需连接 EasyCLIProxyAPI 的统一地址即可直接调用 WorkBuddy 的所有模型。
        </div>

        {/* Service Control Card */}
        <section className="workbuddy-card">
          <div className="workbuddy-card-title">
            <span>
              <Zap size={18} />
              网关服务控制与状态
            </span>
            <div className="workbuddy-controls">
              {!isRunning ? (
                <button
                  type="button"
                  className="primary-button"
                  disabled={actionBusy || loading}
                  onClick={handleStart}
                >
                  {actionBusy ? <LoaderCircle size={15} className="spinner" /> : <Play size={15} />}
                  启动服务
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={actionBusy}
                    onClick={handleRestart}
                  >
                    <RotateCw size={15} />
                    重启服务
                  </button>
                  <button
                    type="button"
                    className="danger-button"
                    disabled={actionBusy}
                    onClick={handleStop}
                  >
                    <Square size={15} />
                    停止服务
                  </button>
                </>
              )}
              <button
                type="button"
                className="secondary-button"
                disabled={!isRunning}
                onClick={handleOpenDashboard}
              >
                <ExternalLink size={15} />
                打开 Web 看板
              </button>
              <button
                type="button"
                className="secondary-button"
                onClick={handleSyncToCpa}
                title="自动将此 WorkBuddy 端点添加为 EasyCLIProxyAPI 的 Provider"
              >
                <Layers size={15} />
                一键同步到 API 接入
              </button>
            </div>
          </div>

          {/* Metrics */}
          <div className="workbuddy-metrics">
            <div className="workbuddy-metric">
              <span className="workbuddy-metric-label">监听地址与端口</span>
              <span className="workbuddy-metric-value">{currentHost}:{currentPort}</span>
            </div>
            <div className="workbuddy-metric">
              <span className="workbuddy-metric-label">进程 PID</span>
              <span className="workbuddy-metric-value">{status?.pid ? status.pid : '无'}</span>
            </div>
            <div className="workbuddy-metric">
              <span className="workbuddy-metric-label">账号池数量</span>
              <span className="workbuddy-metric-value">{status?.accountsCount ?? 0} 个账号</span>
            </div>
            <div className="workbuddy-metric">
              <span className="workbuddy-metric-label">Python 环境</span>
              <span className="workbuddy-metric-value" title={status?.pythonPath ?? ''}>
                {status?.pythonPath ? status.pythonPath.split(/[\\/]/).pop() : '未检测到'}
              </span>
            </div>
          </div>

          {/* Quick Endpoints */}
          <div className="workbuddy-endpoints">
            <div className="workbuddy-endpoint-row">
              <div className="workbuddy-endpoint-info">
                <span className="workbuddy-endpoint-name">OpenAI 兼容 Base URL</span>
                <span className="workbuddy-endpoint-url">{baseUrl}</span>
              </div>
              <button
                type="button"
                className={`icon-button quiet ${copiedKey === 'base' ? 'copied' : ''}`}
                onClick={() => handleCopy(baseUrl, 'base')}
                title="复制 Base URL"
              >
                {copiedKey === 'base' ? <Check size={16} /> : <Copy size={16} />}
              </button>
            </div>

            <div className="workbuddy-endpoint-row">
              <div className="workbuddy-endpoint-info">
                <span className="workbuddy-endpoint-name">Chat Completions (对话补全)</span>
                <span className="workbuddy-endpoint-url">{baseUrl}/chat/completions</span>
              </div>
              <button
                type="button"
                className={`icon-button quiet ${copiedKey === 'chat' ? 'copied' : ''}`}
                onClick={() => handleCopy(`${baseUrl}/chat/completions`, 'chat')}
                title="复制 Chat Completions URL"
              >
                {copiedKey === 'chat' ? <Check size={16} /> : <Copy size={16} />}
              </button>
            </div>

            <div className="workbuddy-endpoint-row">
              <div className="workbuddy-endpoint-info">
                <span className="workbuddy-endpoint-name">Anthropic Messages (Claude Code 协议)</span>
                <span className="workbuddy-endpoint-url">{baseUrl}/messages</span>
              </div>
              <button
                type="button"
                className={`icon-button quiet ${copiedKey === 'msg' ? 'copied' : ''}`}
                onClick={() => handleCopy(`${baseUrl}/messages`, 'msg')}
                title="复制 Messages URL"
              >
                {copiedKey === 'msg' ? <Check size={16} /> : <Copy size={16} />}
              </button>
            </div>
          </div>

          {/* Configuration Form */}
          <div className="workbuddy-config-form">
            <div className="workbuddy-form-group">
              <label>服务端口 (Port)</label>
              <input
                type="number"
                disabled={isRunning}
                value={portInput}
                onChange={(e) => setPortInput(Number(e.target.value))}
                min={1024}
                max={65535}
              />
            </div>
            <div className="workbuddy-form-group">
              <label>网络监听 (Host)</label>
              <select
                disabled={isRunning}
                value={hostInput}
                onChange={(e) => setHostInput(e.target.value)}
              >
                <option value="127.0.0.1">仅本地访问 (127.0.0.1)</option>
                <option value="0.0.0.0">允许局域网设备访问 (0.0.0.0)</option>
              </select>
            </div>
            <div className="workbuddy-form-group">
              <label>访问密钥 (可选 API Key)</label>
              <input
                type="text"
                disabled={isRunning}
                placeholder="留空即无需 Bearer Token"
                value={apiKeyInput}
                onChange={(e) => setApiKeyInput(e.target.value)}
              />
            </div>
          </div>
        </section>

        {/* Local Desktop Accounts Detection Card */}
        <section className="workbuddy-card">
          <div className="workbuddy-card-title">
            <span>
              <LogIn size={18} />
              本地 WorkBuddy 桌面端账号集成
            </span>
            <div className="workbuddy-controls">
              <button
                type="button"
                className="secondary-button"
                disabled={scanningDesktop}
                onClick={scanDesktop}
              >
                {scanningDesktop ? <LoaderCircle size={15} className="spinner" /> : <RefreshCw size={15} />}
                扫描桌面客户端
              </button>
              {desktopAccounts.length > 0 && isRunning && (
                <button
                  type="button"
                  className="primary-button"
                  disabled={actionBusy}
                  onClick={() => handleImportDesktop()}
                >
                  一键导入全部账号
                </button>
              )}
            </div>
          </div>

          <p className="workbuddy-hint">
            自动读取本地 WorkBuddy / CodeBuddy 客户端存储的 <code>.info</code> 登录凭据（支持 2026 最新 At-Rest AES-GCM 密文就地解密）：
          </p>

          {desktopAccounts.length > 0 ? (
            <div style={{ overflowX: 'auto' }}>
              <table className="workbuddy-accounts-table">
                <thead>
                  <tr>
                    <th>凭据文件</th>
                    <th>账号 UID</th>
                    <th>区域 (Realm)</th>
                    <th>加密状态</th>
                    <th>最近活跃时间</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {desktopAccounts.map((acc) => (
                    <tr key={acc.filePath}>
                      <td style={{ fontFamily: 'monospace' }}>{acc.fileName}</td>
                      <td style={{ fontFamily: 'monospace' }}>
                        {acc.uid ? `${acc.uid.slice(0, 12)}...` : '未识别 UID'}
                      </td>
                      <td>
                        <span className={`workbuddy-realm-badge ${acc.realm}`}>
                          {acc.realm === 'cn' ? '国内版 (Tencent)' : '国际版 (WorkBuddy)'}
                        </span>
                      </td>
                      <td>{acc.encrypted ? '已加密 (AES-GCM)' : '明文'}</td>
                      <td>{acc.lastModified ?? '未知'}</td>
                      <td>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={actionBusy || !isRunning}
                          onClick={() => handleImportDesktop(acc.filePath)}
                        >
                          导入到账号池
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <div className="workbuddy-notice-box">
              未扫描到本地 WorkBuddy 桌面客户端登录文件。如需接入，请确保已安装并登录 WorkBuddy 桌面客户端，或通过上方“打开 Web 看板”添加网页 OAuth 账号。
            </div>
          )}
        </section>

        {/* Supported Models Overview */}
        <section className="workbuddy-card">
          <div className="workbuddy-card-title">
            <span>
              <Layers size={18} />
              支持的上游模型目录
            </span>
          </div>
          <p className="workbuddy-hint">
            反代网关会将下列模型自动映射为标准 OpenAI 格式，同步至 EasyCLIProxyAPI 后可供所有客户端直接使用：
          </p>
          <div className="workbuddy-models-grid">
            {WORKBUDDY_MODELS.map((model) => (
              <div key={model.id} className="workbuddy-model-item">
                <span className="workbuddy-model-name">{model.name}</span>
                <span style={{ fontSize: '0.75rem', fontFamily: 'monospace', color: '#64748b' }}>{model.id}</span>
                <span className="workbuddy-model-desc">{model.desc}</span>
              </div>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}
