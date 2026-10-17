# -*- coding: utf-8 -*-
"""wb_prompt.py — 网关自有系统提示词模式（panel prompt.go / degrade.go 语义）。

客户端（Codex / Claude Code 等）在 system prompt 里注入固定模板句时，上游内容
审核可能按逐字匹配误杀合法流量。本模组让网关在出站前改写 system/developer
讯息，从源头消灭 system 来源的指纹误报；user/assistant/tool 讯息不动（既有
sanitize 指纹清洗照旧，两层互不替代）。

模式（settings.json 的 prompt.mode，预设 passthrough）：
  - passthrough：透传客户端 system/developer（hub 原本的行为）；
  - custom：删除所有 system/developer，换成一条网关提示词；
  - append：在开头连续 system/developer 区块之后插入一条网关提示词，
    客户端既有讯息逐字保留。

降级（passthrough / append）：内容审核拦截（403）多半是 system 来源的指纹
误杀。此时切到最小中性提示词，直到次日 00:00 CST 重置，并把同一个请求用
中性提示词重试一次（panel degrade.go）。custom 模式不进降级路径。
"""

import threading
import time

# 内置默认提示词：用于 custom / append 模式（可用 prompt.file 覆盖）。
DEFAULT_PROMPT = """你是一名工程助手，幫助用戶完成軟件工程任務。

- 先理解代碼與上下文再動手，遵循既有模式與約定。
- 最小改動：只改必要部分，不做無關重構。
- 改動後要驗證：跑測試或構建確認結果，不假設「應該沒問題」。
- 對不確定的事保持誠實：說明不確定並給出驗證路徑，不編造。
- 跟隨用戶語言；簡潔直接，技術術語精確。
- 如實報告失敗與邊界，不掩蓋、不粉飾。
"""

# 降级提示词：刻意极简中性，只用于绕开 system 来源的审核误报。
DEGRADED_PROMPT = ("You are a helpful assistant. Respond in the user's language, "
                   "follow the user's instructions, and be direct and concise.")

VALID_MODES = ("passthrough", "custom", "append")


def normalize_mode(value):
    """未知模式回退 passthrough（保守默认，不意外改写客户端提示词）。"""
    mode = str(value or "").strip().lower()
    return mode if mode in VALID_MODES else "passthrough"


def load_prompt(mode, file_path=""):
    """读取网关提示词：file 非空则读档（失败抛出，调用方 fail open），否则内置默认。"""
    path = str(file_path or "").strip()
    if not path:
        return DEFAULT_PROMPT
    with open(path, encoding="utf-8") as fh:
        return fh.read()


def rewrite(messages, system_prompt):
    """删除所有 system/developer，头部插入一条网关 system 讯息。"""
    if not system_prompt:
        return messages
    kept = [m for m in (messages or [])
            if not (isinstance(m, dict)
                    and m.get("role") in ("system", "developer"))]
    return [{"role": "system", "content": system_prompt}] + kept


def append(messages, system_prompt):
    """在开头连续 system/developer 区块之后插入网关提示词，其余讯息不动。"""
    if not system_prompt:
        return messages
    msgs = list(messages or [])
    insert_at = 0
    for m in msgs:
        if not isinstance(m, dict) or m.get("role") not in ("system", "developer"):
            break
        insert_at += 1
    return (msgs[:insert_at]
            + [{"role": "system", "content": system_prompt}]
            + msgs[insert_at:])


def apply_mode(messages, mode, text, degraded=False):
    """按模式路由一次请求的 messages；degraded 只影响 passthrough/append。"""
    mode = normalize_mode(mode)
    if mode == "custom":
        return rewrite(messages, text)
    if mode == "append":
        if degraded:
            return rewrite(messages, DEGRADED_PROMPT)
        return append(messages, text)
    if degraded:
        return rewrite(messages, DEGRADED_PROMPT)
    return messages


def next_midnight_cst(now=None):
    """now 之后最近的 Asia/Shanghai 00:00（epoch 秒）。

    用固定 +08:00 计算，不依赖宿主机时区（容器/宿主时区不确定）。
    00:00 整点 → 次日 00:00；23:59 → 几秒后的次日 00:00。
    """
    now = time.time() if now is None else float(now)
    offset = 8 * 3600
    day = int((now + offset) // 86400)
    return float((day + 1) * 86400 - offset)


class DegradeGate(object):
    """进程内存降级窗口：触发后到次日 00:00 CST 为止（不续期）。"""

    def __init__(self):
        self._lock = threading.Lock()
        self._until = 0.0

    def active(self, now=None):
        now = time.time() if now is None else float(now)
        with self._lock:
            return now < self._until

    def trigger(self, now=None):
        now = time.time() if now is None else float(now)
        with self._lock:
            if now >= self._until:
                self._until = next_midnight_cst(now)

    def reset(self):
        with self._lock:
            self._until = 0.0
