import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, CalendarCheck, Check, Download, LoaderCircle, Play, RefreshCw, Sparkles } from 'lucide-react';
import workbuddyIcon from '../assets/icons/workbuddy.png';
import { FloatingNotice, useAppNotice } from '../appNotice';
import './WorkBuddyTasksPage.css';

type WorkBuddyAccount = {
  uid?: string;
  nickname?: string;
  realm?: string;
  enabled?: boolean;
};

type TaskStep = {
  task: string;
  label: string;
  result: Record<string, unknown>;
};

const TASK_DEFS = [
  {
    task: 'checkin',
    label: '每日签到',
    desc: '国内版每日签到，领取基础签到积分（企业版账号不支持）。',
    command: 'run_workbuddy_checkin',
  },
  {
    task: 'daily-activity',
    label: '每日活跃打卡',
    desc: '国际版每日活跃打卡，走完整网页会话以计入官方活跃奖励。',
    command: 'run_workbuddy_daily_activity',
  },
  {
    task: 'growth',
    label: '一键成长任务',
    desc: '批量执行成长任务包，自动领取成长与积分奖励。',
    command: 'run_workbuddy_growth_tasks',
  },
  {
    task: 'streak-bonus',
    label: '连登管家与抽奖',
    desc: '维持连续登录天数并完成抽奖，累积连登奖励。',
    command: 'run_workbuddy_streak_bonus',
  },
  {
    task: 'travel',
    label: '猫猫旅行',
    desc: '派出猫猫旅行，带回积分与随机道具。',
    command: 'run_workbuddy_travel',
  },
] as const;

const accountLabel = (account: WorkBuddyAccount, index: number) => {
  const nick = (account.nickname || '').trim();
  const uid = (account.uid || '').trim();
  const realm = account.realm === 'cn' ? '国内版' : '国际版';
  const base = nick || (uid ? uid.slice(0, 12) : `账号 ${index + 1}`);
  return `${base} · ${realm}${account.enabled === false ? ' · 已停用' : ''}`;
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

export function WorkBuddyTasksPage({ onBack }: { onBack?: () => void }) {
  const notice = useAppNotice();
  const [accounts, setAccounts] = useState<WorkBuddyAccount[]>([]);
  const [selectedUid, setSelectedUid] = useState('');
  const [loadingAccounts, setLoadingAccounts] = useState(true);
  const [busyTask, setBusyTask] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, Record<string, unknown>>>({});
  const [runLog, setRunLog] = useState<string[]>([]);

  const loadAccounts = useCallback(async () => {
    setLoadingAccounts(true);
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
      // The gateway may not be running yet; keep the page usable and explain why.
      setAccounts([]);
      notice.showNotice(`读取 WorkBuddy 账号失败：${error}`, 'error');
    } finally {
      setLoadingAccounts(false);
    }
  }, [notice]);

  useEffect(() => {
    void loadAccounts();
  }, [loadAccounts]);

  const importDesktopAccounts = async () => {
    setBusyTask('__import__');
    try {
      await invoke('import_workbuddy_desktop_account', { targetPath: null });
      notice.showNotice('已从本地 WorkBuddy 桌面客户端导入账号', 'success');
      await loadAccounts();
    } catch (error) {
      notice.showNotice(`导入桌面端凭证失败：${error}`, 'error');
    } finally {
      setBusyTask(null);
    }
  };

  const runTask = async (task: string, command: string, label: string) => {
    setBusyTask(task);
    setRunLog((current) => [...current, `▶ 开始执行：${label}`]);
    try {
      const payload = await invoke<Record<string, unknown>>(command, {
        uid: selectedUid || null,
      });
      setResults((current) => ({ ...current, [task]: payload }));
      const summary = summarize(payload);
      setRunLog((current) => [...current, `  ${summary.replace(/\n/g, '\n  ')}`, `✓ ${label} 执行完成`]);
      notice.showNotice(`${label} 已执行`, 'success');
    } catch (error) {
      const message = String(error);
      setResults((current) => ({ ...current, [task]: { ok: false, error: message } }));
      setRunLog((current) => [...current, `✗ ${label} 失败：${message}`]);
      notice.showNotice(`${label} 失败：${message}`, 'error');
    } finally {
      setBusyTask(null);
    }
  };

  const runAll = async () => {
    setBusyTask('__all__');
    setRunLog((current) => [...current, '▶ 开始执行全部任务（一键完成）']);
    try {
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
      setRunLog((current) => [...current, '✓ 全部任务执行完成']);
      notice.showNotice('WorkBuddy 全部任务已执行', 'success');
    } catch (error) {
      const message = String(error);
      setRunLog((current) => [...current, `✗ 全部任务执行失败：${message}`]);
      notice.showNotice(`全部任务执行失败：${message}`, 'error');
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
            <h1>WorkBuddy 每日任务</h1>
            <p>为已登录的 WorkBuddy 账号执行每日签到、活跃打卡与一键任务，完成后自动发放积分</p>
          </div>
        </div>
        <div className="workbuddy-tasks-actions">
          {onBack ? (
            <button type="button" className="secondary-button" onClick={onBack}>
              <ArrowLeft size={15} aria-hidden="true" />
              返回 OAuth
            </button>
          ) : null}
          <button type="button" className="secondary-button" disabled={loadingAccounts} onClick={() => void loadAccounts()}>
            {loadingAccounts ? <LoaderCircle size={15} className="spin" aria-hidden="true" /> : <RefreshCw size={15} aria-hidden="true" />}
            刷新账号
          </button>
          <button type="button" className="secondary-button" disabled={busyTask !== null} onClick={() => void importDesktopAccounts()}>
            {busyTask === '__import__' ? <LoaderCircle size={15} className="spin" aria-hidden="true" /> : <Download size={15} aria-hidden="true" />}
            导入桌面端账号
          </button>
          <button type="button" className="primary-button" disabled={busyTask !== null || accounts.length === 0} onClick={() => void runAll()}>
            {busyTask === '__all__' ? <LoaderCircle size={15} className="spin" aria-hidden="true" /> : <Sparkles size={15} aria-hidden="true" />}
            一键完成全部任务
          </button>
        </div>
      </header>

      <section className="workbuddy-tasks-card">
        <h2>目标账号</h2>
        {accounts.length > 0 ? (
          <label className="workbuddy-account-select">
            <span>执行账号</span>
            <select value={selectedUid} onChange={(event) => setSelectedUid(event.currentTarget.value)}>
              {accounts.map((account, index) => (
                <option key={account.uid || index} value={account.uid ?? ''}>
                  {accountLabel(account, index)}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <p className="workbuddy-task-hint">
            {loadingAccounts
              ? '正在读取账号…'
              : '尚未读取到 WorkBuddy 账号。请先在 OAuth 页面完成 WorkBuddy 登录，或在「WorkBuddy 反代」页面导入桌面端凭证。'}
          </p>
        )}
      </section>

      <section className="workbuddy-tasks-card">
        <h2>任务列表</h2>
        <div className="workbuddy-task-grid">
          {TASK_DEFS.map((def) => {
            const result = results[def.task];
            const ok = result ? result.ok === true : null;
            return (
              <div className="workbuddy-task-item" key={def.task}>
                <div className="workbuddy-task-item-title">
                  <CalendarCheck size={15} aria-hidden="true" />
                  {def.label}
                </div>
                <p className="workbuddy-task-item-desc">{def.desc}</p>
                {result ? (
                  <div className={`workbuddy-task-item-result ${ok ? 'success' : 'error'}`}>
                    {summarize(result)}
                  </div>
                ) : null}
                <button
                  type="button"
                  className="secondary-button compact-button"
                  disabled={busyTask !== null}
                  onClick={() => void runTask(def.task, def.command, def.label)}
                >
                  {busyTask === def.task ? <LoaderCircle size={14} className="spin" aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}
                  {result ? '再次执行' : '执行'}
                </button>
              </div>
            );
          })}
        </div>
      </section>

      {runLog.length > 0 ? (
        <section className="workbuddy-tasks-card">
          <h2>
            <Check size={16} aria-hidden="true" /> 执行日志
          </h2>
          <div className="workbuddy-task-log">{runLog.join('\n')}</div>
        </section>
      ) : null}
    </section>
  );
}
