# -*- coding: utf-8 -*-
"""出口级瞬时失败退避（issue #271）。

账号级熔断（`wb_accounts.BREAKER_*`）把「连续 3 次硬失败」记在**账号**头上：
30 分钟起、翻倍到 6 小时。当失败来自出口/链路的**瞬时抖动**而不是账号本身时，
这个口径会把一次抖动记到每个账号头上——现场实测 4 小时 196 次 `URLError`
陆续关掉 13 个账号（最长窗口约 3 小时），期间池子几乎空掉。

本模块把这类失败改记在**出口**上：同一个出口的账号共用一个计数器，于是一次
抖动只换来一段**秒级**的全局退避，而不是 N 个 30 分钟熔断。出口身份用
`wb_upstream_pool.egress_key()`——与连接池键同一口径，所以「直连」本来就是一个
共享出口（12 个账号的失败会落进同一个计数器）。

尺度是这里的重点：账号级阶梯是**分钟**级（30 分钟起），出口级阶梯是**秒**级
（1 秒起、翻倍、封顶 15 秒、带抖动）。一次几秒钟的链路抖动不该换来半小时不可用。
"""
import random
import threading
import time

#: 第一次抖动的退避秒数；之后每次翻倍，封顶 EGRESS_BACKOFF_MAX。
EGRESS_BACKOFF_BASE = 1.0
EGRESS_BACKOFF_MAX = 15.0
#: 距上次抖动超过这么久，就把连续计数当作新的一轮（而不是接着上一次翻倍）。
EGRESS_STREAK_RESET = 60.0
#: 抖动的比例区间：实际退避在计算值的 0.75~1.25 倍之间，避免所有请求同步重试。
EGRESS_JITTER_LOW = 0.75
EGRESS_JITTER_HIGH = 1.25

_lock = threading.Lock()
#: egress_key -> [streak, last_hit_epoch]
_state = {}


def transient_backoff(streak):
    """连续 `streak` 次抖动后、这一段出口的退避秒数（未加抖动）。"""
    step = max(0, int(streak) - 1)
    return min(EGRESS_BACKOFF_BASE * (2 ** min(step, 20)), EGRESS_BACKOFF_MAX)


def _jitter(wait):
    """给退避加抖动。单独抽出来，测试可以替换成确定值。"""
    return wait * random.uniform(EGRESS_JITTER_LOW, EGRESS_JITTER_HIGH)


def note_transient(key, now=None):
    """记一次出口级瞬时失败，返回这一次应当退避的秒数（带抖动）。

    计数器在出口上、不是账号上，所以同一出口的多个账号一起抖时只会让这一段
    退避变长，而不是各自进熔断。
    """
    now = time.time() if now is None else now
    with _lock:
        state = _state.get(key)
        if state is None or now - state[1] > EGRESS_STREAK_RESET:
            state = [0, now]
        state[0] += 1
        state[1] = now
        _state[key] = state
        return _jitter(transient_backoff(state[0]))


def note_success(key):
    """这一段出口成功了一次：清零它的连续计数。

    与账号级 `note_success()` 同一条规矩——成功即清零。抖动是瞬时的，一旦这一
    段出口又通了，就不该再背着上一次的阶梯。
    """
    with _lock:
        _state.pop(key, None)


def streak(key, now=None):
    """这一段出口当前的连续抖动次数（测试与排障用）。"""
    now = time.time() if now is None else now
    with _lock:
        state = _state.get(key)
        if state is None or now - state[1] > EGRESS_STREAK_RESET:
            return 0
        return state[0]


def reset():
    """清空全部状态（测试用）。"""
    with _lock:
        _state.clear()
