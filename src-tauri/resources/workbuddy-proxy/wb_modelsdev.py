# -*- coding: utf-8 -*-
"""wb_modelsdev.py — 模型 context/output 四级查找（panel context_catalog.go + modelsdev.go）。

查找链（每个栏位独立）：
  1. 上游动态值（remote > 0）权威；
  2. 内建知识表 CONTEXT_FALLBACK（panel 实测 + models.dev 共识值，CN/global 共用）；
  3. 本地 model.json 快取（models.dev 索引的持久化，accounts 目录）；
  4. models.dev 按需拉取（异步、5s 超时、失败静默；拉到后写 model.json）。

未知 context → 1_000_000（宁可高估：低估会让客户端提前截断上下文）；未知
output → None（输出上限没有「高估安全侧」，不编造、省略字段）。
纯标准库，Python 3.9 兼容。
"""

import json
import os
import threading
import time
import urllib.request

MODELSDEV_URL = "https://models.dev/api.json"
MODELSDEV_TIMEOUT = 5
MODELSDEV_FETCH_COOLDOWN = 300        # 5 min process-wide throttle
MODELSDEV_NEGATIVE_TTL = 24 * 3600    # per-model negative cache
MODELSDEV_MAX_BODY = 32 * 1024 * 1024
MODELSDEV_VALUE_MAX = 10 ** 9
DEFAULT_CONTEXT = 1000000
CACHE_FILENAME = "model.json"

# Vendor-official providers win over aggregator gateways in the index build.
VENDOR_SOURCES = ("zai", "moonshotai", "moonshotai-cn", "openai", "google",
                  "deepseek", "minimax")

# 内建知识表（panel context_catalog.go 移植；值为实测 / models.dev 共识）。
CONTEXT_FALLBACK = {
    # GLM (z-ai)
    "glm-5.2": (1000000, 131072),
    "glm-5.1": (200000, 131072),
    "glm-5.3": (1000000, 131072),
    "glm-5.3-flash": (1000000, 131072),
    "glm-5v-turbo": (200000, 131072),
    # Kimi (moonshot)
    "kimi-k2.7": (256000, 65536),
    "kimi-k2.6": (256000, 262144),
    "kimi-k2.5": (164000, 262144),
    "kimi-k3": (1048576, 131072),
    "kimi-k2.8-preview": (1048576, 0),
    # MiniMax / Hunyuan
    "minimax-m3": (512000, 512000),
    "hy3": (192000, 64000),
    "hy3-preview": (262144, 64000),
    "hy4-preview": (1000000, 64000),
    "hy4-preview-x": (1000000, 64000),
    # DeepSeek
    "deepseek-v4-pro": (1000000, 384000),
    "deepseek-v4-flash": (1000000, 384000),
    "deepseek-v4.1-flash": (1000000, 384000),
    # OpenAI / Google
    "gpt-6-astra": (1050000, 128000),
    "gpt-5.6-sol": (1050000, 128000),
    "gpt-5.6-terra": (1050000, 128000),
    "gpt-5.6-luna": (1050000, 128000),
    "gpt-5.5": (1050000, 128000),
    "gpt-5.4": (1050000, 128000),
    "gpt-5.3-codex": (400000, 128000),
    "gemini-3.5-flash": (1048576, 65536),
    # Global route alias
    "auto": (168000, 0),
}

_lock = threading.Lock()
_index = None            # bare id -> (context, output)
_index_loaded = False
_last_fetch = 0.0
_fetching = False
_negatives = {}


def cache_path(directory):
    return os.path.join(str(directory or "."), CACHE_FILENAME)


def bare_id(model_id):
    """models.dev keys come both bare and namespaced; index by the tail."""
    return str(model_id or "").strip().lower().rsplit("/", 1)[-1]


def _coerce_pair(value):
    if isinstance(value, (list, tuple)) and len(value) == 2:
        try:
            return (int(value[0] or 0), int(value[1] or 0))
        except (TypeError, ValueError):
            return (0, 0)
    if isinstance(value, dict):
        try:
            return (int(value.get("context") or 0), int(value.get("output") or 0))
        except (TypeError, ValueError):
            return (0, 0)
    return (0, 0)


def load_index(directory, force=False):
    """Level 3: read the local model.json index (missing/corrupt -> empty)."""
    global _index, _index_loaded
    with _lock:
        if _index is not None and not force:
            return _index
    data = {}
    try:
        with open(cache_path(directory), encoding="utf-8") as fh:
            payload = json.load(fh)
        if isinstance(payload, dict):
            for key, value in payload.items():
                pair = _coerce_pair(value)
                if pair[0] > 0 or pair[1] > 0:
                    data[bare_id(key)] = pair
    except Exception:
        data = {}
    with _lock:
        _index = data
        _index_loaded = True
    return data


def save_index(directory, index):
    path = cache_path(directory)
    tmp = path + ".tmp"
    try:
        os.makedirs(directory, exist_ok=True)
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump({key: list(value) for key, value in index.items()},
                      fh, ensure_ascii=False)
        os.replace(tmp, path)
        return True
    except OSError:
        return False


def build_index(doc):
    """models.dev api.json -> bare id -> (context, output).

    Vendor-official providers win; otherwise the most common pair wins
    (ties broken by the smaller value). Invalid/absurd values are dropped.
    """
    votes = {}
    for provider, pdata in (doc or {}).items():
        if not isinstance(pdata, dict):
            continue
        models = pdata.get("models")
        if not isinstance(models, dict):
            continue
        for model_id, minfo in models.items():
            if not isinstance(minfo, dict):
                continue
            limit = minfo.get("limit") or {}
            try:
                context = int(limit.get("context") or 0)
                output = int(limit.get("output") or 0)
            except (TypeError, ValueError):
                continue
            if context <= 0 and output <= 0:
                continue
            if context > MODELSDEV_VALUE_MAX or output > MODELSDEV_VALUE_MAX:
                continue
            key = bare_id(model_id)
            entry = votes.setdefault(key, {"values": {}, "vendor_value": None})
            pair = (context, output)
            entry["values"][pair] = entry["values"].get(pair, 0) + 1
            if provider in VENDOR_SOURCES and entry["vendor_value"] is None:
                entry["vendor_value"] = pair
    index = {}
    for key, entry in votes.items():
        if entry["vendor_value"]:
            index[key] = entry["vendor_value"]
        elif entry["values"]:
            index[key] = sorted(entry["values"].items(),
                                key=lambda item: (-item[1], item[0]))[0][0]
    return index


def refresh_async(directory, log=None):
    """Level 4: fetch models.dev in the background (throttled, offline-safe)."""
    global _last_fetch, _fetching
    now = time.time()
    with _lock:
        if _fetching or now - _last_fetch < MODELSDEV_FETCH_COOLDOWN:
            return False
        _fetching = True
        _last_fetch = now

    def worker():
        global _fetching, _index, _index_loaded
        try:
            req = urllib.request.Request(
                MODELSDEV_URL, headers={"User-Agent": "wb2api-hub"})
            with urllib.request.urlopen(req, timeout=MODELSDEV_TIMEOUT) as resp:
                raw = resp.read(MODELSDEV_MAX_BODY)
            index = build_index(json.loads(raw.decode("utf-8")))
            if index:
                save_index(directory, index)
                with _lock:
                    _index = index
                    _index_loaded = True
        except Exception as exc:
            if log:
                log("models.dev refresh skipped: %s" % str(exc)[:120])
        finally:
            with _lock:
                _fetching = False

    threading.Thread(target=worker, daemon=True).start()
    return True


def lookup(model_id, remote_context=0, remote_output=0, directory=None,
           index=None):
    """Resolve (context, output_or_None) for one model.

    Precedence per field: remote > knowledge table > local cache > default
    (context 1M; output omitted). The models.dev fetch itself is asynchronous
    and only warms the cache, so this function never blocks on the network.
    """
    key = bare_id(model_id)
    try:
        remote_context = int(remote_context or 0)
    except (TypeError, ValueError):
        remote_context = 0
    try:
        remote_output = int(remote_output or 0)
    except (TypeError, ValueError):
        remote_output = 0
    table = CONTEXT_FALLBACK.get(key) or (0, 0)
    if index is None:
        index = load_index(directory)
    cached = index.get(key) or (0, 0)
    context = remote_context or table[0] or cached[0] or DEFAULT_CONTEXT
    output = remote_output or table[1] or cached[1] or None
    return context, output, {
        "context": ("remote" if remote_context else
                    "table" if table[0] else
                    "cache" if cached[0] else "default"),
        "output": ("remote" if remote_output else
                   "table" if table[1] else
                   "cache" if cached[1] else "unknown"),
    }
