import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ArrowLeft,
  CalendarCheck,
  Check,
  Coins,
  Download,
  LoaderCircle,
  Play,
  RefreshCw,
  Sparkles,
  TrendingUp,
} from 'lucide-react';
import workbuddyIcon from '../assets/icons/workbuddy.png';
import { FloatingNotice, useAppNotice } from '../appNotice';
import './WorkBuddyTasksPage.css';

export type WorkBuddyAccountCredits = {
  remain?: number;
  used?: number;
  size?: number;
  used_percent?: string;
  remain_percent?: string;
  is_paid_user?: boolean;
  checkin?: {
    today_checked_in?: boolean;
    streak_days?: number;
    daily_credit?: number;
    today_credit?: number;
    active?: boolean;
  } | null;
  earliest_expiring?: {
    name?: string;
    remain?: number;
    days_left?: number;
  } | null;
};

export type WorkBuddyAccount = {
  uid?: string;
  nickname?: string;
  email?: string;
  account?: string;
  realm?: string;
  enabled?: boolean;
  credits?: WorkBuddyAccountCredits | null;
};

type TaskStep = {
  task: string;
  label: string;
  result: Record<string, unknown>;
};

type TaskDef = {
  task: string;
  label: string;
  realm: 'cn' | 'intl' | 'both';
  desc: string;
  command: string;
};

const TASK_DEFS: TaskDef[] = [
  {
    task: 'checkin',
    label: '每日签到 / 活跃打卡',
    realm: 'both',
    desc: '每日领取积分：国内版账号执行每日签到，国际版账号执行每日活跃打卡。',
    command: 'run_workbuddy_checkin',
  },
  {
    task: 'daily-activity',
    label: '国际版网页打卡',
    realm: 'intl',
    desc: '国际版每日网页会话打卡，走完整网页会话以计入官方活跃奖励。',
    command: 'run_workbuddy_daily_activity',
  },
  {
    task: 'growth',
    label: '一键成长任务',
    realm: 'cn',
    desc: '批量执行成长任务包，自动领取成长与积分奖励。',
    command: 'run_workbuddy_growth_tasks',
  },
  {
    task: 'streak-bonus',
    label: '连登管家与抽奖',
    realm: 'cn',
    desc: '维持连续登录天数并完成抽奖，累积连登奖励。',
    command: 'run_workbuddy_streak_bonus',
  },
  {
    task: 'travel',
    label: '猫猫旅行',
    realm: 'cn',
    desc: '派出猫猫旅行，带回积分与随机道具。',
    command: 'run_workbuddy_travel',
  },
];

const accountDisplay = (account: WorkBuddyAccount) => {
  const nick = (account.nickname || '').trim();
  const email = (account.email || account.account || '').trim();
  const uid = (account.uid || '').trim();

  let name = '';
  if (nick && email && nick !== email) {
    name = `${nick} (${email})`;
  } else if (email) {
    name = email;
  } else if (nick) {
    name = `${nick} (${uid.slice(0, 8)})`;
  } else {
    name = uid ? `账号 ${uid.slice(0, 10)}` : '未命名账号';
  }

  const realmLabel = account.realm === 'cn' ? '国内版' : '国际版';
  return `${name} · ${realmLabel}${account.enabled === false ? ' · 已停用' : ''}`;
};

const summarize = (result: Record<string, unknown>): string => {
  if (typeof result.msg === 'string' && result.msg.trim()) return result.msg;
  if (typeof result.error === 'string' && result.error.trim()) return result.error;
  if (Array.isArray(result.results)) {
    return (result.results as Record<string, unknown>[])
      .map((entry) => {
        const ok = entry.ok === true ? '✓' : '✗';
        const msg = (entry.msg as string) || (entry.error as string) || '';
        return `${ok} ${msg}`;
      })
      .join('\n');
  }
  if (typeof result.credit_added === 'number') {
    return `累计新增积分 +${result.credit_added}`;
  }
  if (Array.isArray(result.logs)) return (result.logs as string[]).join('\n');
  return JSON.stringify(result, null, 2);
};

const isTaskOk = (result: Record<string, unknown> | undefined): boolean | null => {
  if (!result) return null;
  if (result.ok === true) return true;
  if (result.ok === false) return false;
  if (Array.isArray(result.results) && result.results.length > 0) {
    return (result.results as Record<string, unknown>[]).every(
      (entry) => entry.ok !== false && !entry.error,
    );
  }
  return null;
};

export function WorkBuddyTasksPage({ onBack }: { onBack?: () => void }) {
  const notice = useAppNotice();
  const noticeRef = useRef(notice);
  noticeRef.current = notice;

  const [accounts, setAccounts] = useState<WorkBuddyAccount[]>([]);
  const [selectedUid, setSelectedUid] = useState('');
  const [loadingAccounts, setLoadingAccounts] = useState(true);
  const [refreshingCredits, setRefreshingCredits] = useState(false);
  const [accountError, setAccountError] = useState('');
  const [busyTask, setBusyTask] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, Record<string, unknown>>>({});
  const [runLog, setRunLog] = useState<string[]>([]);
  const [creditDeltaMessage, setCreditDeltaMessage] = useState<string | null>(null);

  const selectedAccount = accounts.find((a) => a.uid === selectedUid) ?? accounts[0];

  const loadAccounts = useCallback(async (notifyOnFailure = false) => {
    setLoadingAccounts(true);
    setAccountError('');
    try {
      const payload = await invoke<Record<string, unknown>>('get_workbuddy_accounts');
      const list = Array.isArray(payload.accounts)
        ? (payload.accounts as WorkBuddyAccount[])
        : Array.isArray(payload)
          ? (payload as WorkBuddyAccount[])
          : [];
      setAccounts(list);
      setSelectedUid((current) => (list.some((a) => a.uid === current) ? current : (list[0]?.uid ?? '')));
    } catch (error) {
      setAccounts([]);
      const errMsg = `读取 WorkBuddy 账号失败：${error}`;
      setAccountError(errMsg);
      if (notifyOnFailure) {
        noticeRef.current.showNotice(errMsg, 'error');
      }
    } finally {
      setLoadingAccounts(false);
    }
  }, []);

  useEffect(() => {
    void loadAccounts(false);
  }, [loadAccounts]);

  const handleRefreshCredits = async () => {
    setRefreshingCredits(true);
    try {
      const res = await invoke<Record<string, unknown>>('refresh_workbuddy_credits', { uid: selectedUid || null });
      if (Array.isArray(res.accounts) && res.accounts.length > 0) {
        setAccounts(res.accounts as WorkBuddyAccount[]);
      } else {
        await loadAccounts(false);
      }
      noticeRef.current.showNotice('账号积分已刷新', 'success');
    } catch (error) {
      noticeRef.current.showNotice(`刷新积分失败：${error}`, 'error');
    } finally {
      setRefreshingCredits(false);
    }
  };

  const importDesktopAccounts = async () => {
    setBusyTask('__import__');
    try {
      await invoke('import_workbuddy_desktop_account', { targetPath: null });
      noticeRef.current.showNotice('已从本地 WorkBuddy 桌面客户端导入账号', 'success');
      await loadAccounts(true);
    } catch (error) {
      noticeRef.current.showNotice(`导入桌面端凭证失败：${error}`, 'error');
    } finally {
      setBusyTask(null);
    }
  };

  const runTask = async (def: TaskDef) => {
    setBusyTask(def.task);
    setCreditDeltaMessage(null);
    const beforeCredits = selectedAccount?.credits?.remain;
    setRunLog((current) => [
      ...current,
      `▶ [${def.label}] 开始执行 (当前积分: ${beforeCredits ?? '未知'})`,
    ]);

    try {
      const payload = await invoke<Record<string, unknown>>(def.command, {
        uid: selectedUid || null,
      });
      setResults((current) => ({ ...current, [def.task]: payload }));
      const summary = summarize(payload);

      // Refresh credits to calculate point change
      let afterCredits: number | undefined;
      try {
        await invoke('refresh_workbuddy_credits', { uid: selectedUid || null });
        const refreshedPayload = await invoke<Record<string, unknown>>('get_workbuddy_accounts');
        const list = Array.isArray(refreshedPayload.accounts)
          ? (refreshedPayload.accounts as WorkBuddyAccount[])
          : [];
        setAccounts(list);
        afterCredits = list.find((a) => a.uid === selectedUid)?.credits?.remain;
      } catch {
        // Non-critical if refresh fails
      }

      let deltaText = '';
      if (typeof beforeCredits === 'number' && typeof afterCredits === 'number') {
        const delta = afterCredits - beforeCredits;
        if (delta > 0) {
          deltaText = ` · 积分增加: +${delta.toFixed(2)} (余额: ${beforeCredits} ➔ ${afterCredits})`;
          setCreditDeltaMessage(`🎉 ${def.label} 成功！积分增加 +${delta.toFixed(2)} 积分 (现余额: ${afterCredits})`);
        } else {
          deltaText = ` · 积分余额保持 ${afterCredits}`;
        }
      }

      setRunLog((current) => [
        ...current,
        `  ${summary.replace(/\n/g, '\n  ')}`,
        `✓ [${def.label}] 执行成功${deltaText}`,
      ]);
      noticeRef.current.showNotice(`${def.label} 执行成功${deltaText}`, 'success');
    } catch (error) {
      const message = String(error);
      setResults((current) => ({ ...current, [def.task]: { ok: false, error: message } }));
      setRunLog((current) => [...current, `✗ [${def.label}] 失败：${message}`]);
      noticeRef.current.showNotice(`${def.label} 失败：${message}`, 'error');
    } finally {
      setBusyTask(null);
    }
  };

  const runAll = async () => {
    setBusyTask('__all__');
    setCreditDeltaMessage(null);
    const beforeCredits = selectedAccount?.credits?.remain;
    const realm = selectedAccount?.realm || 'intl';
    setRunLog((current) => [
      ...current,
      `▶ 开始为账号 [${selectedAccount ? accountDisplay(selectedAccount) : '全部'}] 一键完成任务`,
      `  当前积分余额: ${beforeCredits ?? '未知'}`,
    ]);

    try {
      // Execute all tasks
      const payload = await invoke<{ steps: TaskStep[] }>('run_workbuddy_all_tasks', {
        uid: selectedUid || null,
      });

      const next: Record<string, Record<string, unknown>> = {};
      (payload.steps ?? []).forEach((step) => {
        next[step.task] = step.result;
        setRunLog((current) => [
          ...current,
          `  [${step.label}]`,
          `    ${summarize(step.result).replace(/\n/g, '\n    ')}`,
        ]);
      });
      setResults(next);

      // Refresh credits
      let afterCredits: number | undefined;
      try {
        await invoke('refresh_workbuddy_credits', { uid: selectedUid || null });
        const refreshedPayload = await invoke<Record<string, unknown>>('get_workbuddy_accounts');
        const list = Array.isArray(refreshedPayload.accounts)
          ? (refreshedPayload.accounts as WorkBuddyAccount[])
          : [];
        setAccounts(list);
        afterCredits = list.find((a) => a.uid === selectedUid)?.credits?.remain;
      } catch {
        // Non-critical if refresh fails
      }

      let deltaText = '';
      if (typeof beforeCredits === 'number' && typeof afterCredits === 'number') {
        const delta = afterCredits - beforeCredits;
        if (delta > 0) {
          deltaText = ` · 积分增加: +${delta.toFixed(2)} (余额: ${beforeCredits} ➔ ${afterCredits})`;
          setCreditDeltaMessage(`🎉 一键任务执行完毕！累计增加 +${delta.toFixed(2)} 积分 (现余额: ${afterCredits})`);
        } else {
          deltaText = ` · 当前积分: ${afterCredits}`;
          setCreditDeltaMessage(`✓ 一键任务执行完毕，当前积分: ${afterCredits} 积分`);
        }
      }

      setRunLog((current) => [
        ...current,
        `✓ 全部任务执行完成 (${realm === 'cn' ? '国内版' : '国际版'})${deltaText}`,
      ]);
      noticeRef.current.showNotice(`WorkBuddy 任务已全部执行${deltaText}`, 'success');
    } catch (error) {
      const message = String(error);
      setRunLog((current) => [...current, `✗ 全部任务执行失败：${message}`]);
      noticeRef.current.showNotice(`全部任务执行失败：${message}`, 'error');
    } finally {
      setBusyTask(null);
    }
  };

  return (
    <section className="page workbuddy-tasks-page">
      <FloatingNotice key={notice.revision} notice={notice.notice} onDismiss={notice.clearNotice} />

      <header className="workbuddy-tasks-header">
        <div className="workbuddy-tasks-title">
          <img src={workbuddyIcon} alt="" />
          <div>
            <h1>WorkBuddy 任务中心</h1>
            <p>为已登录的 WorkBuddy 账号执行每日签到、活跃打卡与成长任务，实时查看积分变动</p>
          </div>
        </div>
        <div className="workbuddy-tasks-actions">
          {onBack ? (
            <button type="button" className="secondary-button" onClick={onBack}>
              <ArrowLeft size={15} aria-hidden="true" />
              返回 OAuth
            </button>
          ) : null}
          <button
            type="button"
            className="secondary-button"
            disabled={loadingAccounts || refreshingCredits}
            onClick={() => void handleRefreshCredits()}
            title="向腾讯服务器同步最新账号积分"
          >
            {refreshingCredits ? (
              <LoaderCircle size={15} className="spin" aria-hidden="true" />
            ) : (
              <RefreshCw size={15} aria-hidden="true" />
            )}
            刷新积分
          </button>
          <button
            type="button"
            className="secondary-button"
            disabled={busyTask !== null}
            onClick={() => void importDesktopAccounts()}
          >
            {busyTask === '__import__' ? (
              <LoaderCircle size={15} className="spin" aria-hidden="true" />
            ) : (
              <Download size={15} aria-hidden="true" />
            )}
            导入桌面端账号
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={busyTask !== null || accounts.length === 0}
            onClick={() => void runAll()}
          >
            {busyTask === '__all__' ? (
              <LoaderCircle size={15} className="spin" aria-hidden="true" />
            ) : (
              <Sparkles size={15} aria-hidden="true" />
            )}
            一键完成全部任务
          </button>
        </div>
      </header>

      {/* Credit Delta Banner */}
      {creditDeltaMessage ? (
        <div className="workbuddy-delta-banner">
          <TrendingUp size={16} aria-hidden="true" />
          <span>{creditDeltaMessage}</span>
        </div>
      ) : null}

      {/* Target Account & Credits Card */}
      <section className="workbuddy-tasks-card">
        <h2>目标账号与积分看板</h2>
        {accountError ? (
          <p className="workbuddy-task-hint" style={{ color: '#ef4444' }}>
            {accountError}
          </p>
        ) : null}
        {accounts.length > 0 ? (
          <>
            <label className="workbuddy-account-select">
              <span>当前账号</span>
              <select value={selectedUid} onChange={(event) => setSelectedUid(event.currentTarget.value)}>
                {accounts.map((account, index) => (
                  <option key={account.uid || index} value={account.uid ?? ''}>
                    {accountDisplay(account)}
                  </option>
                ))}
              </select>
              {selectedAccount?.realm ? (
                <span className={`workbuddy-tag ${selectedAccount.realm}`}>
                  {selectedAccount.realm === 'cn' ? '国内版' : '国际版'}
                </span>
              ) : null}
            </label>

            {/* Credits Dashboard */}
            <div className="workbuddy-credits-dashboard">
              <div className="workbuddy-credit-stat">
                <span className="workbuddy-credit-label">
                  <Coins size={14} aria-hidden="true" />
                  剩余积分 (Remain)
                </span>
                <span className="workbuddy-credit-value highlight">
                  {typeof selectedAccount?.credits?.remain === 'number'
                    ? selectedAccount.credits.remain.toFixed(2)
                    : '未同步'}
                  <span style={{ fontSize: '0.85rem', fontWeight: 500, marginLeft: 4 }}>积分</span>
                </span>
                <span className="workbuddy-credit-sub">
                  已用 {selectedAccount?.credits?.used ?? 0} / 总量 {selectedAccount?.credits?.size ?? 0}
                </span>
              </div>

              <div className="workbuddy-credit-stat">
                <span className="workbuddy-credit-label">
                  <CalendarCheck size={14} aria-hidden="true" />
                  今日签到 / 活跃状态
                </span>
                <span className="workbuddy-credit-value" style={{ fontSize: '1rem' }}>
                  {selectedAccount?.realm === 'cn' ? (
                    selectedAccount?.credits?.checkin?.today_checked_in ? (
                      <span style={{ color: '#16a34a' }}>
                        ✓ 今日已签到 (连续 {selectedAccount.credits.checkin.streak_days ?? 1} 天)
                      </span>
                    ) : (
                      <span style={{ color: '#d97706' }}>待签到</span>
                    )
                  ) : (
                    <span style={{ color: '#0284c7' }}>国际版活跃打卡</span>
                  )}
                </span>
                <span className="workbuddy-credit-sub">
                  {selectedAccount?.realm === 'cn'
                    ? `每日可领 ${selectedAccount?.credits?.checkin?.daily_credit ?? 5} 积分`
                    : '通过每日活跃对话打卡领取奖励'}
                </span>
              </div>

              {selectedAccount?.credits?.earliest_expiring ? (
                <div className="workbuddy-credit-stat">
                  <span className="workbuddy-credit-label">即将到期积分包</span>
                  <span className="workbuddy-credit-value" style={{ fontSize: '1rem', color: '#ea580c' }}>
                    {selectedAccount.credits.earliest_expiring.remain} 积分
                  </span>
                  <span className="workbuddy-credit-sub">
                    {selectedAccount.credits.earliest_expiring.name} · 剩余{' '}
                    {selectedAccount.credits.earliest_expiring.days_left ?? 0} 天
                  </span>
                </div>
              ) : null}
            </div>
          </>
        ) : (
          <p className="workbuddy-task-hint">
            {loadingAccounts
              ? '正在读取账号…'
              : '尚未读取到 WorkBuddy 账号。请先在 OAuth 页面完成 WorkBuddy 登录，或点击右上角「导入桌面端账号」。'}
          </p>
        )}
      </section>

      {/* Task List */}
      <section className="workbuddy-tasks-card">
        <h2>任务列表</h2>
        <div className="workbuddy-task-grid">
          {TASK_DEFS.map((def) => {
            const result = results[def.task];
            const ok = isTaskOk(result);
            const isMatchRealm =
              def.realm === 'both' ||
              (selectedAccount?.realm === 'cn' && def.realm === 'cn') ||
              (selectedAccount?.realm === 'intl' && def.realm === 'intl');

            // For CN checkin, also reflect checkin status from account credits
            const alreadyCheckedIn =
              def.task === 'checkin' &&
              selectedAccount?.realm === 'cn' &&
              selectedAccount?.credits?.checkin?.today_checked_in;

            const isCompleted = ok === true || Boolean(alreadyCheckedIn);

            return (
              <div className="workbuddy-task-item" key={def.task}>
                <div className="workbuddy-task-item-title">
                  <CalendarCheck size={15} aria-hidden="true" />
                  <span>{def.label}</span>
                  <span className={`workbuddy-tag ${def.realm}`}>
                    {def.realm === 'cn' ? '国内版' : def.realm === 'intl' ? '国际版' : '通用'}
                  </span>
                  {isCompleted ? (
                    <span className="workbuddy-task-badge done">✓ 今日已完成</span>
                  ) : null}
                </div>
                <p className="workbuddy-task-item-desc">{def.desc}</p>
                {result ? (
                  <div className={`workbuddy-task-item-result ${ok ? 'success' : 'error'}`}>
                    {summarize(result)}
                  </div>
                ) : null}
                <button
                  type="button"
                  className={isCompleted ? 'secondary-button compact-button' : 'primary-button compact-button'}
                  disabled={busyTask !== null || !isMatchRealm}
                  title={!isMatchRealm ? `此任务仅限${def.realm === 'cn' ? '国内版' : '国际版'}账号使用` : ''}
                  onClick={() => void runTask(def)}
                >
                  {busyTask === def.task ? (
                    <LoaderCircle size={14} className="spin" aria-hidden="true" />
                  ) : (
                    <Play size={14} aria-hidden="true" />
                  )}
                  {!isMatchRealm
                    ? `仅限${def.realm === 'cn' ? '国内版' : '国际版'}`
                    : isCompleted
                      ? '再次执行'
                      : '执行任务'}
                </button>
              </div>
            );
          })}
        </div>
      </section>

      {/* Execution Log */}
      {runLog.length > 0 ? (
        <section className="workbuddy-tasks-card">
          <h2>
            <Check size={16} aria-hidden="true" /> 执行与变动日志
          </h2>
          <div className="workbuddy-task-log">{runLog.join('\n')}</div>
        </section>
      ) : null}
    </section>
  );
}
