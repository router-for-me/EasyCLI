#!/usr/bin/env python3
"""WorkBuddy (workbuddy.ai) -> OpenAI-compatible reverse proxy.
Reuses the credentials the WorkBuddy desktop app already stored on this machine
(%%LOCALAPPDATA%%\\CodeBuddyExtension\\Data\\Public\\auth\\*.info), so no separate
login is needed. Exposes:
    GET  /v1/models
    POST /v1/chat/completions     (stream=true and stream=false)
    GET  /health
Only the Python standard library is required.
    python3 wb_proxy.py                    # bind 127.0.0.1:8788
    python3 wb_proxy.py --port 9000
    python3 wb_proxy.py --api-key sk-local # require a bearer token
Launchers: start-wb-proxy.bat / start-wb-proxy-lan.bat on Windows,
start-wb-proxy.command (or ./start-wb-proxy.sh) on macOS/Linux.
"""
import argparse
import bisect
import hashlib
import heapq
import ipaddress
from collections import deque
import re
import json
import os
# 单请求体上限。默认值从 50MB 收紧到 16MB，是「内存护栏」：body 先整体读成 bytes、
# 再 decode 成 str、再 json.loads 成对象，实测 50MB body 的 decode+parse 峰值约
# 109MB、单请求峰值约 160MB；而读 body 发生在 chat 信号量 acquire 之前、线程数又
# 没有上限，路由器（可用内存约 480MB）上几个并发大 body 就能把机器打穿。
# 权衡：另一条路是把 body 读取也纳入 chat 信号量，但那会改动 401/404/413 早退与
# 503 的先后语义、并让慢客户端占着 slot 拖住信号量；只收紧默认值保守得多——16MB
# 对正常客户端绰绰有余（Codex 1M token 上下文的 JSON 也只有几 MB），确需更大可
# 显式设 WB_MAX_PAYLOAD_BYTES，内存代价由运维者自己拍板。
MAX_PAYLOAD_BYTES = int(os.environ.get("WB_MAX_PAYLOAD_BYTES", 16 * 1024 * 1024))  # 16MB limit
# Upstream chat calls may hold a handler thread for up to 600s, and every
# request gets its own thread, so an unbounded pool lets a handful of slow
# clients pin hundreds of threads and the memory behind them. Bound the number
# of chat/responses requests in flight; dashboard and management calls are not
# affected. Excess callers wait briefly, then get a 503 instead of queueing
# forever.
#
# The ceiling is a fixed 32 by default, which has nothing to do with how many
# accounts the pool holds: a 200-account gateway and a 5-account gateway share
# it. Set WB_MAX_CONCURRENT_CHAT=auto to size it from the pool instead - one
# slot per ready account, never below 32. See resize_chat_slots().
_CHAT_SLOTS_ENV = os.environ.get("WB_MAX_CONCURRENT_CHAT", "32").strip().lower()
CHAT_SLOTS_AUTO = _CHAT_SLOTS_ENV in ("auto", "pool", "dynamic")
CHAT_SLOTS_FLOOR = 32
try:
    MAX_CONCURRENT_CHAT = CHAT_SLOTS_FLOOR if CHAT_SLOTS_AUTO else int(_CHAT_SLOTS_ENV)
except ValueError:
    MAX_CONCURRENT_CHAT = CHAT_SLOTS_FLOOR
CHAT_SLOT_WAIT_SECONDS = float(os.environ.get("WB_CHAT_SLOT_WAIT", 30))
import socket
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
import wb_accounts
import wb_activity
import wb_atrest
import wb_catalog
import wb_egress
import wb_ipintel
import wb_pricing
import wb_settings
import wb_upstream_pool
import wb_webtools
import wb_identity
import wb_prompt
import wb_modelsdev
import wb_probes
import wb_updates
IS_WINDOWS = os.name == "nt"

# ---- 一键配置客户端（wb_agents）的可用性判定（issue #246）----
# 这个功能写的是「网关进程所在机器」的客户端配置，所以只有在浏览器和网关
# 同一台机器上打开看板时才成立。服务端 / Docker / OpenWrt 部署下看板都是
# 远程打开的，改了也到不了用户自己的电脑，还会白白付出探测与常驻的开销。
# 因此 wb_agents 改成懒加载：只有下面这道闸门放行、路由真的命中才 import。
_AGENTS_MODULE = None
_SERVER_DEPLOYMENT = None


def agents_module():
    """按需 import wb_agents；用不到它的部署连模块都不载入。"""
    global _AGENTS_MODULE
    if _AGENTS_MODULE is None:
        import wb_agents
        _AGENTS_MODULE = wb_agents
    return _AGENTS_MODULE


def server_deployment_form():
    """进程是否明显跑在服务端形态（容器 / OpenWrt）。

    只认环境标记；「看板是不是本机打开的」由 agents_client_allowed 按请求
    来源地址判断。结果缓存——进程活着期间答案不会变。
    """
    global _SERVER_DEPLOYMENT
    if _SERVER_DEPLOYMENT is None:
        blocked = False
        try:
            if os.path.exists("/.dockerenv") or os.path.exists("/run/.containerenv"):
                blocked = True
            elif os.path.exists("/proc/1/cgroup"):
                with open("/proc/1/cgroup", encoding="utf-8", errors="replace") as fh:
                    cgroup = fh.read().lower()
                blocked = any(m in cgroup for m in ("docker", "containerd", "kubepods"))
        except Exception:
            blocked = False
        if not blocked:
            try:
                if os.path.exists("/etc/openwrt_release"):
                    blocked = True
                elif os.path.exists("/etc/os-release"):
                    with open("/etc/os-release", encoding="utf-8", errors="replace") as fh:
                        blocked = "id=openwrt" in fh.read().lower()
            except Exception:
                blocked = False
        _SERVER_DEPLOYMENT = blocked
    return _SERVER_DEPLOYMENT


def agents_client_allowed(client_address):
    """这个来源地址能不能用一键配置：只认回环。

    非回环说明看板是远程打开的（手机、另一台电脑、容器端口映射），此时
    apply 改的是容器 / 服务器里的配置目录，需求不成立，直接按不可用处理。
    """
    if server_deployment_form():
        return False
    try:
        addr = str(client_address[0])
    except Exception:
        return False
    return addr in ("127.0.0.1", "::1", "::ffff:127.0.0.1")


def launcher_hint(port):
    """Platform-appropriate launcher command for starting on another port."""
    if IS_WINDOWS:
        return "start-wb-proxy.bat %d" % port
    return "./start-wb-proxy.sh %d" % port
def port_owner_hint(port):
    """Command that lists the process holding a local TCP port."""
    if IS_WINDOWS:
        return "netstat -ano | findstr :%d" % port
    return "lsof -nP -iTCP:%d -sTCP:LISTEN" % port
CURRENT_REALM = os.environ.get("WB_PROXY_DEFAULT_REALM", "intl")
def detect_model_realm(model_id):
    if not model_id:
        return CURRENT_REALM
    m = str(model_id).lower()
    intl_only = {
        "gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna",
        "gpt-5.5", "gpt-5.4", "gpt-5.3-codex", "gemini-3.5-flash",
        "grok-4.7"
    }
    if m in intl_only or any(m.startswith(p) for p in ("gpt-", "gemini-")):
        return "intl"
    cn_only = {
        "deepseek-v4-pro", "minimax-m3", "minimax-m2.7", "minimax-m2.5",
        "glm-5.1", "glm-5.0-turbo", "glm-4.6v",
        "kimi-k3-1", "kimi-k2.7", "kimi-k2-thinking",
        "hy3-x", "hy4-preview-dev", "hy4-preview-x"
    }
    if m in cn_only or any(m.startswith(p) for p in ("minimax-", "deepseek-v4-pro")):
        return "cn"
    return CURRENT_REALM
# glm-5.3-flash was listed as cn-only, but the international exit serves it:
# an official intl account posting to www.workbuddy.ai gets HTTP 200, and the
# intl desktop client ships it in its own model list. Only deepseek-v4-pro
# still answers "service info not found" there.
# Models that exist on one side only. Everything else (deepseek-v4.1-flash,
# hy3, glm-5.3 ...) is served by both exits, so it must not be treated as a
# conflict.
INTL_EXCLUSIVE_PREFIXES = ("gpt-", "gemini-")
CN_EXCLUSIVE_PREFIXES = ("minimax-", "deepseek-v4-pro")
INTL_EXCLUSIVE = {
    "gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna",
    "gpt-5.5", "gpt-5.4", "gpt-5.3-codex", "gemini-3.5-flash",
    "grok-4.7",
}
CN_EXCLUSIVE = {
    "deepseek-v4-pro", "glm-5.1", "glm-5v-turbo",
    "kimi-k3-1", "kimi-k2.7", "minimax-m3",
    "hy3-x", "hy4-preview-dev", "hy4-preview-x",
}
def exclusive_realm(model_id):
    """"intl"/"cn" when only that exit serves the model, else ""."""
    if not model_id:
        return ""
    m = str(model_id).lower()
    if m in INTL_EXCLUSIVE or m.startswith(INTL_EXCLUSIVE_PREFIXES):
        return "intl"
    if m in CN_EXCLUSIVE or m.startswith(CN_EXCLUSIVE_PREFIXES):
        return "cn"
    return ""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs
def install_console_close_handler():
    """Release the port when the console window is closed by the user.
    Windows does not kill child processes when a console window closes, so
    the proxy (started by the .bat as a child of cmd.exe) would survive and
    keep the port bound - the next launch then wrongly reports "another
    proxy is already running".
    Closing the window raises CTRL_CLOSE_EVENT in every process attached to
    that console, which is exactly the signal we want. Registering a handler
    for it is event-driven, so unlike polling a parent pid there is no
    chance of a false positive. Harmless when started without a console.
    """
    if os.name != "nt":
        return None
    try:
        import ctypes
        from ctypes import wintypes
        PHANDLER_ROUTINE = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.DWORD)
        CTRL_CLOSE_EVENT = 2
        CTRL_LOGOFF_EVENT = 5
        CTRL_SHUTDOWN_EVENT = 6
        def _handler(event):
            if event in (CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT):
                try:
                    sys.stdout.flush()
                except Exception:
                    pass
                os._exit(0)
            return False
        handler = PHANDLER_ROUTINE(_handler)   # keep the callback referenced
        if not ctypes.windll.kernel32.SetConsoleCtrlHandler(handler, True):
            return None
        return handler
    except Exception:
        return None
UPSTREAM = "https://www.workbuddy.ai"
CHAT_PATH = "/v2/chat/completions"
MODELS_PATH = "/v2/enterprises/personal/models"
DEFAULT_SYSTEM_PROMPT = "You are a helpful assistant."
# The WorkBuddy AI desktop app caches its account product config here on every
# launch. That file carries the real model catalog the app shows in its picker
# (21 models, incl. deepseek-v4.1-flash / gpt-6-astra) - the CLI-facing
# /v2/enterprises/personal/models endpoint returns a narrower list, so prefer
# the cache and fall back to the endpoint.
PRODUCT_CONFIG_CACHE = os.path.join(os.path.expanduser("~"), ".workbuddy-ai", "cache", "acc-product-config-v3.json")
NOISE_KEYS = ("extra_fields", "refusal", "reasoning_content")
class BodyTooLarge(Exception):
    """Raised when a request body exceeds the configured cap."""
    def __init__(self, length):
        super(BodyTooLarge, self).__init__(length)
        self.length = length
class BadJSON(Exception):
    """Raised when a request body is present but not a JSON object."""
# CORS is only needed by browser-based chat clients that call the OpenAI-style
# API from another origin. Management routes (accounts, settings, usage,
# scheduler, panel) serve the dashboard, which is same-origin, so they get no
# ACAO header - that keeps a stray page on the LAN from reading their replies.
CORS_PATH_PREFIXES = ("/v1", "/chat", "/completions", "/models", "/responses")
# Management paths that happen to live under /v1 must not be treated as API:
# /v1/usage reports account-level spend and is gated by the panel session.
MANAGEMENT_PATH_PREFIXES = ("/v1/usage", "/usage", "/accounts", "/settings",
                            "/tasks", "/scheduler", "/panel", "/logs")
def cors_origin_allowed(path):
    """True when the OpenAI-style API path should advertise CORS."""
    path = (path or "").split("?")[0]
    if path.startswith(MANAGEMENT_PATH_PREFIXES):
        return False
    return path.startswith(CORS_PATH_PREFIXES)
_lock = threading.Lock()
_login_lock = threading.Lock()
_chat_slots = threading.BoundedSemaphore(MAX_CONCURRENT_CHAT)
def resize_chat_slots(ready_accounts):
    """Size the chat concurrency ceiling from the pool, when asked to.

    With WB_MAX_CONCURRENT_CHAT=auto the ceiling becomes one slot per ready
    account (never below CHAT_SLOTS_FLOOR), so the gateway scales with the pool
    it actually has instead of a constant that fits neither a 5-account nor a
    200-account deployment. A fixed numeric WB_MAX_CONCURRENT_CHAT keeps the
    1.6.x behaviour and makes this a no-op.

    Only ever grows the ceiling. Requests already holding a slot keep theirs -
    replacing the semaphore cannot revoke a permit - so a shrink would let the
    in-flight count exceed the new ceiling and the surplus releases would raise
    ValueError from BoundedSemaphore. Growing is the direction that matters:
    the pool is loaded once at startup, so in practice this runs before the
    listener accepts anything.
    """
    global _chat_slots, MAX_CONCURRENT_CHAT
    if not CHAT_SLOTS_AUTO:
        return MAX_CONCURRENT_CHAT
    try:
        wanted = max(CHAT_SLOTS_FLOOR, int(ready_accounts or 0))
    except (TypeError, ValueError):
        return MAX_CONCURRENT_CHAT
    if wanted <= MAX_CONCURRENT_CHAT:
        return MAX_CONCURRENT_CHAT
    MAX_CONCURRENT_CHAT = wanted
    _chat_slots = threading.BoundedSemaphore(wanted)
    log("chat slots : %d (one per ready account; WB_MAX_CONCURRENT_CHAT=auto)"
        % wanted)
    return wanted
_login_attempts = {}  # ip -> list of timestamp
def _prune_login_attempts(now=None, window=60):
    """Drop stale per-IP entries so the dict cannot grow without bound.
    Caller must hold _login_lock.
    """
    now = now or time.time()
    for ip in list(_login_attempts.keys()):
        recent = [t for t in _login_attempts[ip] if now - t < window]
        if recent:
            _login_attempts[ip] = recent
        else:
            del _login_attempts[ip]
# --------------------------------------------------- 面板登录限流的来源判定
# 限流按「客户端 IP」分桶（每 IP 5 次失败 / 60 秒），但反代部署下 socket 对端
# 永远不是真实客户端：路由器上 nginx 把 ai.home 转给 127.0.0.1:8788，所有请求
# 的对端都是 127.0.0.1。若照旧拿对端地址当键，任意一个人连错 5 次密码，就会把
# 所有经反代的用户、连同跑在同一台机器上的缓存预热器一起锁死 60 秒——实测里
# 预热器被 429 打掉后直接打印"登录失败（http=429），跳过本轮"，缓存转冷，
# 用户点开统计页要付一次全表扫描。所以键要取「真实来源」，且只在对端可信时
# 才读代理头：对端是回环地址（本机代理）或显式列进 WB_TRUSTED_PROXIES 的代理
# 时，按 X-Real-IP（优先）或 X-Forwarded-For 的第一个地址分桶；其余对端一律
# 忽略这两个头、仍按对端地址分桶——公网客户端不能靠伪造头换个桶绕过限流。
def parse_trusted_proxies(raw):
    """解析 WB_TRUSTED_PROXIES：逗号分隔的 IP 或 CIDR（如 10.0.0.1,fd00::/8）。
    解析不了的条目直接丢弃：一个笔误不该让网关起不来。
    """
    entries = []
    for item in str(raw or "").split(","):
        item = item.strip()
        if not item:
            continue
        try:
            entries.append(ipaddress.ip_network(item, strict=False))
        except ValueError:
            continue
    return tuple(entries)
TRUSTED_PROXIES = parse_trusted_proxies(os.environ.get("WB_TRUSTED_PROXIES", ""))
def peer_is_trusted(peer_ip):
    """对端是否有资格代表它的客户端说话：回环（本机反代）或显式配置的代理。"""
    try:
        addr = ipaddress.ip_address(str(peer_ip))
    except ValueError:
        return False
    # Windows 双栈下连 127.0.0.1 的对端会显示成 ::ffff:127.0.0.1。ipaddress 在
    # Python 3.13 之前不把 IPv4-mapped 地址算作回环（.is_loopback 为 False），
    # 于是本机反代的请求会被当成不可信来源，限流桶退化成所有客户端共用一个。
    mapped = addr.ipv4_mapped if addr.version == 6 else None
    if addr.is_loopback or (mapped is not None and mapped.is_loopback):
        return True
    for net in TRUSTED_PROXIES:
        if addr.version == net.version and addr in net:
            return True
    return False
_PROXY_HEADER_IP_MAX = 64  # 最长的 IPv6 文本也到不了 46 字符，再长就是垃圾
def _proxy_header_ip(value):
    """从代理头里取一个合法 IP 的规范文本；取不到返回 None（绝不抛异常）。

    接受裸 IPv4/IPv6，也接受 nginx 变体可能写出的 host:port / [v6]:port；
    多值（含逗号）、超长、其余畸形串一律判为不可用，由调用方决定回退。
    """
    text = str(value or "").strip()
    if not text or len(text) > _PROXY_HEADER_IP_MAX or "," in text:
        return None
    if text.startswith("["):
        end = text.find("]")
        if end == -1:
            return None
        text = text[1:end]
    elif text.count(":") == 1:
        # v4:port 形式；裸 IPv6 至少两个冒号，不会被这里截断
        host, _, port = text.partition(":")
        if port.isdigit():
            text = host
    try:
        return str(ipaddress.ip_address(text))
    except ValueError:
        return None
def login_rate_limit_key(peer_ip, headers):
    """限流键：对端可信时取代理头里的真实客户端 IP，否则取对端地址。

    X-Real-IP 优先（nginx 里放的就是真实来源），其次 X-Forwarded-For 的
    第一个（链上最左是最初的客户端）。头不可用就退回对端地址：宁可让经
    反代的客户端共用一个桶（退化为旧行为），也不能拿解析不出来的串当键。
    """
    if not peer_is_trusted(peer_ip):
        return peer_ip
    real = _proxy_header_ip(headers.get("X-Real-IP") if headers else None)
    if real:
        return real
    forwarded = (headers.get("X-Forwarded-For") if headers else None) or ""
    first = forwarded.split(",")[0]
    via_chain = _proxy_header_ip(first)
    if via_chain:
        return via_chain
    return peer_ip
_models_cache = {"intl": {"at": 0.0, "data": None}, "cn": {"at": 0.0, "data": None}}
# Usage accounting: every upstream response carries a usage block, and the
# proxy also records one JSONL line per request. Defaults to a folder next to
# this script; override with --usage-dir or WB_PROXY_USAGE_DIR.
USAGE_DIR = os.environ.get("WB_PROXY_USAGE_DIR") \
    or os.path.join(os.path.dirname(os.path.abspath(__file__)), "usage")
USAGE_LOG = os.path.join(USAGE_DIR, "usage.jsonl")
DASHBOARD_HTML = os.path.join(os.path.dirname(os.path.abspath(__file__)), "dashboard.html")
USAGE_FIELDS = ("prompt_tokens", "completion_tokens", "reasoning_tokens",
                "cached_tokens", "total_tokens", "credit")
# Web-panel access control. The panel is gated by its own password (default
# "admin"), independent of the /v1 API key. Sessions live in memory only, so a
# restart forces browsers to log in again.
PANEL = wb_settings.PanelSessions()
API_KEY_FILE_SET = False
def configured_keys():
    """Panel-managed API keys, always read fresh so panel edits apply at once."""
    try:
        return wb_settings.api_keys(ACCOUNTS_DIR)
    except Exception as exc:
        log("could not read api keys: %s" % exc)
        return []
def auth_required():
    """Whether /v1 calls must present a key at all."""
    if wb_settings.auth_disabled(ACCOUNTS_DIR):
        return False
    if any(entry.get("enabled") for entry in configured_keys()):
        return True
    return bool(API_KEY)
def identify_key(supplied):
    """Return the key entry a caller used, or None when nothing matches.
    Once the panel has at least one key, those keys are the only accepted
    credentials - otherwise a launcher key left in a .bat file would silently
    keep working after the panel was locked down.
    """
    extra = () if configured_keys() else (API_KEY,)
    return wb_settings.match_api_key(ACCOUNTS_DIR, supplied, extra_keys=extra)
def _empty_stats():
    return {"requests": 0, "errors": 0, "prompt_tokens": 0, "completion_tokens": 0,
            "reasoning_tokens": 0, "cached_tokens": 0, "total_tokens": 0,
            "credit": 0.0, "cost_cny": 0.0, "cost_missing": {},
            "started": time.time(), "by_model": {},
            # Same aggregation keyed by (model, realm), so the metrics table
            # can show one row per exit for a model that ran through both.
            "by_model_realm": {},
            # And again keyed by (model, realm, account), so a model served
            # by two accounts on the same exit can be split per account.
            "by_model_acct": {},
            # latency accumulators (averages; percentiles come from the JSONL)
            "ttft_ms_sum": 0, "ttft_samples": 0,
            "gen_ms_sum": 0, "gen_samples": 0,
            "wall_ms_sum": 0, "wall_samples": 0}
# 进程启动时刻。快照的 "started"/"since" 从这里取（_usage_snapshot_uncached
# 是它唯一的读者）；requests/ttft/by_model 那些累计字段没有任何读者——每个
# 聚合都重读 usage.jsonl——所以请求路径不再维护它们。
_usage = {"started": time.time()}
def normalize_usage_cache_aliases(usage):
    """Write the best cache-hit value into every alias.

    Some responses carry the real hit in prompt_tokens_details.cached_tokens
    while also emitting cache_read_input_tokens: 0 / cached_tokens: 0
    compatibility aliases; strict downstream parsers may prefer the zero
    aliases and lose the hit. Mutates and returns the usage dict.
    """
    if not isinstance(usage, dict):
        return usage
    best = _best_cached_tokens(usage)
    if best <= 0:
        return usage
    usage["cache_read_input_tokens"] = best
    usage["cached_tokens"] = best
    usage["prompt_cache_hit_tokens"] = best
    prompt_details = dict(usage.get("prompt_tokens_details") or {})
    prompt_details["cached_tokens"] = best
    usage["prompt_tokens_details"] = prompt_details
    if isinstance(usage.get("input_tokens_details"), dict):
        input_details = dict(usage["input_tokens_details"])
        input_details["cached_tokens"] = best
        usage["input_tokens_details"] = input_details
    return usage


def _extract_usage(usage):
    """Normalize the upstream usage block into the fields we track."""
    if not usage:
        return {}
    details = usage.get("completion_tokens_details") or {}
    return {
        "prompt_tokens": usage.get("prompt_tokens") or 0,
        "completion_tokens": usage.get("completion_tokens") or 0,
        "reasoning_tokens": details.get("reasoning_tokens") or 0,
        "cached_tokens": _best_cached_tokens(usage),
        "total_tokens": usage.get("total_tokens") or 0,
        "credit": usage.get("credit") or 0,
    }
def row_realm(row):
    """The realm a log row belongs to.

    Rows written since the field was added carry it directly. Older rows are
    attributed by their account, then by the model's home realm - the same
    order row_matches_realm used, so a filter and a per-realm breakdown can
    never disagree about the same row.
    """
    r = row.get("realm")
    if r:
        return r
    acct_uid = row.get("account")
    if acct_uid and POOL:
        acc = POOL.get(acct_uid)
        if acc:
            return acc.realm
    model = row.get("model")
    if model:
        return detect_model_realm(model)
    return "intl"


def row_matches_realm(row, realm):
    # None means every realm. "all" is accepted here as well so that a caller
    # that forwards the literal cannot silently match nothing: the previous
    # behaviour compared every row's realm against the string "all".
    if not realm or realm == "all": return True
    return row_realm(row) == realm
def realm_scope(realm, fallback=None):
    """Map a caller-supplied realm onto a log filter.

    "all" means every realm, so it becomes None and disables filtering
    entirely: passing the literal through would make row_matches_realm
    compare every row against "all" and match nothing at all. An empty
    or missing value falls back to the second argument: CURRENT_REALM for
    the endpoints whose clients expect the global switch, None (everything)
    for the analytics payload, which has always reported both realms
    combined.
    """
    if realm == "all":
        return None
    return realm or fallback


def _local_midnight(ts=None, days_back=0):
    """Local midnight `days_back` days before `ts` (default: now).

    mktime normalises an out-of-range day, so stepping back past the 1st of a
    month still lands on a real local midnight instead of raising.
    """
    lt = time.localtime(ts if ts is not None else time.time())
    return time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday - days_back, 0, 0, 0, 0, 0, -1))


def _local_hour_start(ts):
    """ts 所属的本地整点（分/秒归零）的 epoch。

    与 _local_midnight 同一套构造：mktime 把本地墙上时间换算回 epoch，
    分/秒归零就是「本地整点」。夏令时切换小时里的时间会被 mktime 规整到
    相邻的真实时刻，所以调用方还要比对返回值是否等于原值——只有能原样
    还原的 ts 才是真的落在整点上（见 _hour_window_key）。
    """
    lt = time.localtime(ts)
    return time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday, lt.tm_hour, 0, 0, 0, 0, -1))


def _epoch_or_none(value):
    """A panel-supplied epoch second, or None when it cannot be trusted.

    A negative or unparseable bound is dropped rather than clamped. The panel
    rejects those before they are ever sent, so one arriving here means a
    hand-written URL, and "no bound on this side" is a much smaller surprise
    than silently slicing the log at 1970.
    """
    if value in (None, "", False):
        return None
    try:
        seconds = float(value)
    except (TypeError, ValueError):
        return None
    return seconds if seconds >= 0 else None


def range_window(value, since=None, until=None):
    """Resolve the dashboard's time-range selector into a (since, until) pair.

    Both bounds are cutoffs on a row's `at`; None means "unbounded on that
    side", and (None, None) - an unknown, empty or missing range - disables
    filtering entirely, which is what every caller did before windows existed,
    so an older panel keeps receiving the full history it used to get.

    today/week/month are calendar windows anchored to local midnight, matching
    the definition the analytics payload has always used for its Today
    figures: two different meanings of "today" on one page would be worse than
    either. The week starts on Monday. Rolling aliases ("7d", "30d") are
    deliberately absent - they would mean "the last seven days", which is a
    different window from "this week" and would make the button's label wrong
    on six days out of seven.

    custom takes the two epochs the panel sends. Either side may be missing
    ("from this date onwards" / "up to this date"), and reversed bounds are
    swapped rather than rejected, because the two inputs are independent and
    an empty end is the normal case.
    """
    v = str(value or "").strip().lower()
    if v in ("today", "day", "1d"):
        return _local_midnight(), None
    if v in ("week", "w"):
        return _local_midnight(days_back=time.localtime().tm_wday), None
    if v in ("month", "m"):
        lt = time.localtime()
        return time.mktime((lt.tm_year, lt.tm_mon, 1, 0, 0, 0, 0, 0, -1)), None
    if v == "custom":
        lo, hi = _epoch_or_none(since), _epoch_or_none(until)
        if lo is not None and hi is not None and hi < lo:
            lo, hi = hi, lo
        return lo, hi
    return None, None


def range_query(query):
    """Pull the three range parameters out of a parsed query string.

    parse_qs hands every key back as a list, and an older panel that sends
    none of them at all is the normal case, so every lookup falls back to
    None - which range_window() reads as "no filter on that side".
    """
    def first(name):
        values = query.get(name) or [None]
        return values[0] if values else None
    return first("range"), first("since"), first("until")
def row_outcome(row):
    """Terminal state of a request row.

    Rows written before the outcome field existed only carry error/status,
    so they fall back to that: an error row is a failure, anything else is a
    completed request. One helper keeps every reader agreeing on the answer.
    """
    o = row.get("outcome")
    if o:
        return o
    return "failed" if row.get("error") else "completed"
def _best_cached_tokens(usage):
    """Best cache-hit value across every alias the upstreams emit (E3).

    Order mirrors the panel: prompt_tokens_details.cached_tokens >
    prompt_cache_hit_tokens > cache_read_input_tokens > cached_tokens >
    input_tokens_details.cached_tokens > completion_tokens_details.
    First positive value wins; zero aliases never shadow a real hit.
    """
    if not isinstance(usage, dict):
        return 0
    prompt_details = usage.get("prompt_tokens_details") or {}
    input_details = usage.get("input_tokens_details") or {}
    details = usage.get("completion_tokens_details") or {}
    for value in (prompt_details.get("cached_tokens"),
                  usage.get("prompt_cache_hit_tokens"),
                  usage.get("cache_read_input_tokens"),
                  usage.get("cached_tokens"),
                  input_details.get("cached_tokens"),
                  details.get("cached_tokens")):
        try:
            number = float(value or 0)
        except (TypeError, ValueError):
            continue
        if number > 0:
            # Token counts must stay integers: the Codex client parses
            # response.completed strictly and rejects 123.0 (invalid number).
            return int(number)
    return 0


def record_usage(model, usage, stream=None, elapsed_ms=None, ttft_ms=None, gen_ms=None, fp=None,
                account=None, outcome="completed", key=None, effort=None):
    """Record one finished request as exactly one JSONL row.

    A request without a usage block still gets a row (flagged usage_missing):
    skipping it entirely used to drop the request from the request count,
    success rate and latency samples, not just from the token totals.

    outcome is the terminal state: completed / client_aborted /
    upstream_aborted / failed. It is deliberately not called status, because
    status already means the HTTP status code on error rows.

    key is the settings id of the client API key that paid for the request,
    never the secret itself. It is passed in explicitly rather than read from
    a thread-local: one keep-alive thread serves many requests, so an implicit
    channel would attribute spend to the wrong key silently, while a missed
    call site only shows up as an extra "no key" row.

    effort is the reasoning effort the request actually ran at, as resolved by
    build_upstream_body(). It is written only when the model has one: a model
    without reasoning controls has nothing to report, and rows written before
    this field existed cannot be told apart from it anyway.
    """
    fields = _extract_usage(usage) or {}
    usage_missing = not fields
    row = {
        "at": time.time(),
        "iso": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "model": model,
        "stream": bool(stream),
        "outcome": outcome,
        "elapsed_ms": elapsed_ms,
        "ttft_ms": ttft_ms,
        "gen_ms": gen_ms,
    }
    if usage_missing:
        row["usage_missing"] = True
    row.update(fields)
    if fp:
        row.update(fp)
    if account:
        row["account"] = account
    acc = POOL.get(account) if (account and POOL) else None
    row["realm"] = acc.realm if acc else CURRENT_REALM
    # Always written, even when the caller presented no key. The empty value is
    # what separates "ran without a key" from rows written before the field
    # existed; the dashboard reports those as two different buckets, and a
    # missing field is the only evidence of the cutover that survives.
    row["key"] = key or ""
    # The price policy this request is measured against, stored as a reference
    # so the table can be de-duplicated and swept. A row written before the
    # table existed has no reference and falls back to the timeline.
    row["cost_policy"] = wb_pricing.current_policy_id(model)
    # Only when the request had one: the panel shows a chip for rows that carry
    # the field, and a model without reasoning controls has nothing to report.
    if effort:
        row["reasoning_effort"] = effort
    # Derived per-request rates (None-safe).
    if gen_ms and gen_ms > 0:
        row["tokens_per_sec"] = round(fields.get("completion_tokens", 0) / (gen_ms / 1000.0), 2)
    # Share the denominator with the aggregate view (compute_usage_analytics),
    # otherwise the per-request row and the rollup disagree on the same data.
    if fields.get("prompt_tokens", 0) > 0:
        row["cache_hit_pct"] = round(fields.get("cached_tokens", 0) * 100.0
                                     / fields["prompt_tokens"], 1)
    _persist_usage(row, "usage persist failed")
    try:
        t_tokens = fields.get("total_tokens", 0)
        dur = f" {elapsed_ms:.0f}ms" if elapsed_ms is not None else ""
        acc_tag = f" acct={account[:8]}" if account else ""
        speed_tag = f" {row.get('tokens_per_sec', 0)}t/s" if row.get("tokens_per_sec") else ""
        miss_tag = " usage=missing" if usage_missing else ""
        log(f"chat done: model={model}{acc_tag}{dur} tokens={t_tokens} (in={fields.get('prompt_tokens',0)} out={fields.get('completion_tokens',0)}){speed_tag}{miss_tag}", tag="chat")
    except Exception:
        pass
    return row


def _persist_usage(row, fail_label):
    """Append one usage row as a JSONL line.

    usage-summary.json used to be rewritten on every single request - a full
    json.dumps of the running totals, a uniquely named temp file and an
    os.replace, plus the deep copy that fed it. Nothing in the tree ever
    loads that file (every aggregate re-reads usage.jsonl), so the work was
    pure overhead on the request path. One append per request now.
    """
    try:
        os.makedirs(USAGE_DIR, exist_ok=True)
        with open(USAGE_LOG, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")
    except Exception as exc:
        log("%s: %s" % (fail_label, exc))

def record_error(model, status, message, elapsed_ms=None, account=None,
                 usage=None, stream=None, ttft_ms=None, gen_ms=None, fp=None,
                 outcome="failed", key=None):
    """Record one failed request as exactly one JSONL row.

    Passing the account uid records which account the request was bound to, so
    per-realm success rates attribute the failure by fact instead of falling
    back to guessing from the model name.

    The usage argument carries whatever the upstream had already reported when
    a stream broke. An aborted stream used to write an error row AND a usage
    row, so one request counted as both a failure and a success; the token
    totals stay accurate here without inflating the request count.

    status stays the HTTP status code; outcome is the terminal state, so the
    two never disagree about what the field means.

    key is the client API key's settings id, same as record_usage. It is the
    only way a rejected request can be attributed to a key, so failures on the
    key's model whitelist still land on the right row instead of vanishing
    into the unattributed bucket.
    """
    fields = _extract_usage(usage) or {}
    row = {
        "at": time.time(),
        "iso": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "model": model,
        "error": True,
        "outcome": outcome,
        "status": status,
        "message": str(message)[:200],
        "elapsed_ms": elapsed_ms,
    }
    if stream is not None:
        row["stream"] = bool(stream)
    if ttft_ms is not None:
        row["ttft_ms"] = ttft_ms
    if gen_ms is not None:
        row["gen_ms"] = gen_ms
    row.update(fields)
    if fp:
        row.update(fp)
    if account:
        row["account"] = account
        acc = POOL.get(account) if POOL else None
        row["realm"] = acc.realm if acc else CURRENT_REALM
    row["key"] = key or ""
    _persist_usage(row, "error persist failed")
    dur = f" {elapsed_ms:.0f}ms" if elapsed_ms is not None else ""
    log(f"request error: model={model}{dur} status={status} msg={str(message)[:180]}", level="ERROR", tag="chat")
    return row
def _pct(values, q):
    """Nearest-rank percentile (no interpolation) - good enough for latency."""
    if not values:
        return None
    ordered = sorted(values)
    idx = int(round((q / 100.0) * (len(ordered) - 1)))
    return ordered[max(0, min(len(ordered) - 1, idx))]


def _perf_block(vals):
    """One metric's summary block, straight from the raw samples.

    D6 要求分位永远从原始样本算（不许聚合近似），所以这里收的是一串原始数值
    ——list 或 deque 都行（增量样本用 deque 做并入/滑出）。排序一次喂三个分位；
    avg 与 max 仍读原序列，于是浮点求和的顺序、以及 avg 的最后一位，与逐次
    排序的旧写法逐位相同。
    """
    if not vals:
        return None
    ordered = sorted(vals)
    count = len(ordered)

    def rank(q):
        idx = int(round((q / 100.0) * (count - 1)))
        return ordered[max(0, min(count - 1, idx))]

    return {
        "avg": round(sum(vals) / count, 1),
        "p50": rank(50),
        "p90": rank(90),
        "p99": rank(99),
        "max": max(vals),
        "samples": count,
    }
_perf_cache = {}
_perf_lock = threading.Lock()


def _perf_stats_scope(sample, realm, range, since, until):
    """(过滤用的 realm, since, until, 缓存键) —— perf_stats() 与它的 ETag 共用。

    抽出来只为一件事：取数和派生 ETag 必须走**同一套**键。两边各写一份的话，
    只要有一处不同（比如一处 realm_scope、一处原始 realm），tag 就会落到另一条
    缓存条目上，客户端拿到的是一个不描述当前响应体的校验符。
    """
    r = realm_scope(realm, CURRENT_REALM)
    lo, hi = range_window(range, since, until)
    try:
        # The key carries the resolved bounds rather than a today/all flag:
        # this week and this month overlap, so a flag cannot tell them apart
        # and one window's latency would be served under the other's label.
        key = (int(sample), r or "all",
               lo if lo is not None else -1, hi if hi is not None else -1)
    except Exception:
        key = (5000, r or "all",
               lo if lo is not None else -1, hi if hi is not None else -1)
    return r, lo, hi, key


def perf_stats(sample=5000, realm=None, ttl=None, range=None, since=None, until=None):
    """Cached wrapper: parsing thousands of rows is CPU-heavy, and the
    dashboard polls this endpoint every few seconds.

    Rebuilds under the lock so a burst of pollers cannot each start their own
    scan of the log. The rebuild itself is incremental: _perf_stats_incremental
    keeps a ring of the last `sample` rows per (sample, realm, window) key and
    only folds the bytes appended since the last rebuild (see there)."""
    ttl = _STATS_TTL if ttl is None else ttl
    r, lo, hi, key = _perf_stats_scope(sample, realm, range, since, until)
    now = time.time()
    with _perf_lock:
        hit = _perf_cache.get(key)
        if hit is not None and (now - hit[0]) < ttl:
            return hit[1]
        data = _perf_stats_incremental(key, key[0], r, lo, hi)
        _perf_cache[key] = (time.time(), data)
    return data


def perf_stats_etag(sample=5000, realm=None, range=None, since=None, until=None):
    """perf_stats() 当前缓存条目的 ETag；条目还不存在时返回 None。

    _json_cached() 会在取数前后各调用一次并比对，两次一致才把 tag 发出去
    （见那里的注释）。所以这里只回答"此刻这条缓存条目的戳是什么"，调用方不需要
    自己安排调用顺序。
    """
    _r, _lo, _hi, key = _perf_stats_scope(sample, realm, range, since, until)
    with _perf_lock:
        hit = _perf_cache.get(key)
    if hit is None:
        return None
    return _cache_etag("perf", repr(key), hit[0])


def _perf_stats_uncached(sample=5000, realm=None, since=None, until=None):
    """Latency percentiles + derived rates, computed from the JSONL log."""
    ttfts, gens, walls, rates, hits, tok_rates = [], [], [], [], [], []
    total = ok = err = aborted = 0
    # 按模型聚合性能指标
    m_buckets = {}
    # Same aggregation keyed by (model, realm), so the metrics table can
    # report a model's latency per exit when it ran through both.
    mr_buckets = {}
    # And keyed by (model, realm, account), so two accounts on one exit can
    # be shown as separate rows.
    ma_buckets = {}
    # 只读日志末尾 sample 行：原先 readlines() 会把整个日志读成字符串列表
    rows = [raw.decode("utf-8", "replace") for raw in _tail_lines(USAGE_LOG, sample)]
    # The tail read stops at `sample` lines, so a window wider than the sample
    # is only described by its newest requests. Both facts are reported so the
    # matrix can say the latency columns cover a partial slice instead of
    # presenting them as the whole window.
    sample_capped = len(rows) >= sample
    sample_from = None
    for line in rows:
        line = line.strip()
        if not line:
            continue
        # 廉价窗口预过滤（复用 _line_outside_window）：解析前的子串检查就能
        # 证明落在窗口外的行不再付 json.loads 的钱。它只丢「下面的窗口判断
        # 本来也会丢」的行，聚合口径一个数都不变；无窗口的调用（since/until
        # 全空）连检查都不做，因此一分钱都不多花。sample_from 还没定下来的
        # 时候也不过滤——第一行解析出来的 at 就是 sample_from 的值，跳过解析
        # 会让它变成窗口内第一行的时间戳，报告的范围就跟着变了。
        if (since or until) and sample_from is not None and \
                _line_outside_window(line, since or None, until or None):
            continue
        try:
            r = json.loads(line)
        except Exception:
            continue
        if sample_from is None:
            sample_from = r.get("at")
        if realm and not row_matches_realm(r, realm):
            continue
        # Same window as the usage snapshot, so the latency and speed columns
        # of the matrix describe the same requests as its token columns.
        at = r.get("at") or 0
        if since and at < since:
            continue
        if until and at > until:
            continue
        total += 1
        outcome = row_outcome(r)
        # Every row reaches the model bucket, whatever its outcome, so a model
        # that only ever saw cancellations still shows up with a zero success
        # count instead of silently vanishing from the per-model table.
        m_id = r.get("model") or "unknown"
        r_realm = row_realm(r)
        mb = m_buckets.setdefault(m_id, {"total": 0, "ok": 0, "err": 0, "aborted": 0,
                                        "ttfts": [], "gens": [], "walls": [],
                                        "tok_rates": [], "hits": []})
        rb = mr_buckets.setdefault(m_id, {}).setdefault(
            r_realm, {"total": 0, "ok": 0, "err": 0, "aborted": 0,
                      "ttfts": [], "gens": [], "walls": [],
                      "tok_rates": [], "hits": []})
        ab = ma_buckets.setdefault(m_id, {}).setdefault(r_realm, {}).setdefault(
            r.get("account") or "(unattributed)",
            {"total": 0, "ok": 0, "err": 0, "aborted": 0,
             "ttfts": [], "gens": [], "walls": [],
             "tok_rates": [], "hits": []})
        mb["total"] += 1
        rb["total"] += 1
        ab["total"] += 1
        # A client that walks away is not a gateway failure, so it counts as
        # neither ok nor err - it gets its own bucket instead of silently
        # dragging the success rate down.
        if outcome == "client_aborted":
            aborted += 1
            for b in (mb, rb, ab):
                b["aborted"] += 1
                if r.get("elapsed_ms"):
                    b["walls"].append(r["elapsed_ms"])
            if r.get("elapsed_ms"):
                walls.append(r["elapsed_ms"])
            continue
        if outcome != "completed":
            err += 1
            for b in (mb, rb, ab):
                b["err"] += 1
                if r.get("elapsed_ms"):
                    b["walls"].append(r["elapsed_ms"])
            if r.get("elapsed_ms"):
                walls.append(r["elapsed_ms"])
            continue
        ok += 1
        for b in (mb, rb, ab):
            b["ok"] += 1
        if r.get("ttft_ms") is not None:
            ttfts.append(r["ttft_ms"])
            for b in (mb, rb, ab):
                b["ttfts"].append(r["ttft_ms"])
        if r.get("gen_ms") is not None:
            gens.append(r["gen_ms"])
            for b in (mb, rb, ab):
                b["gens"].append(r["gen_ms"])
        if r.get("elapsed_ms") is not None:
            walls.append(r["elapsed_ms"])
            for b in (mb, rb, ab):
                b["walls"].append(r["elapsed_ms"])
        if r.get("tokens_per_sec"):
            tok_rates.append(r["tokens_per_sec"])
            for b in (mb, rb, ab):
                b["tok_rates"].append(r["tokens_per_sec"])
        if r.get("cache_hit_pct") is not None:
            hits.append(r["cache_hit_pct"])
            for b in (mb, rb, ab):
                b["hits"].append(r["cache_hit_pct"])
    block = _perf_block
    return {
        "sampled": total,
        # Where the sampled slice starts and whether it was cut short, so a
        # week/month view can admit that its latency columns do not reach back
        # to the window's own start.
        "sample_from": sample_from,
        "sample_capped": sample_capped,
        "success": ok,
        "errors": err,
        "client_aborted": aborted,
        # Success rate is measured against requests the gateway actually
        # finished; client cancellations are reported separately rather than
        # being counted as failures.
        "success_rate_pct": round(ok * 100.0 / (ok + err), 1) if (ok + err) else None,
        "ttft_ms": block(ttfts),
        "generation_ms": block(gens),
        "wall_ms": block(walls),
        "tokens_per_sec": block(tok_rates),
        "cache_hit_pct": block(hits),
        "by_model": {
            mid: {
                "requests": mb["total"],
                "errors": mb["err"],
                "client_aborted": mb.get("aborted", 0),
                "success_rate_pct": round(mb["ok"] * 100.0 / (mb["ok"] + mb["err"]), 1)
                                     if (mb["ok"] + mb["err"]) else None,
                "ttft_ms": block(mb["ttfts"]),
                "generation_ms": block(mb["gens"]),
                "wall_ms": block(mb["walls"]),
                "tokens_per_sec": block(mb["tok_rates"]),
                "cache_hit_pct": block(mb["hits"]),
            } for mid, mb in m_buckets.items()
        },
        "by_model_realm": {
            mid: {
                realm: {
                    "requests": rb["total"],
                    "errors": rb["err"],
                    "client_aborted": rb.get("aborted", 0),
                    "success_rate_pct": round(rb["ok"] * 100.0 / (rb["ok"] + rb["err"]), 1)
                                         if (rb["ok"] + rb["err"]) else None,
                    "ttft_ms": block(rb["ttfts"]),
                    "generation_ms": block(rb["gens"]),
                    "wall_ms": block(rb["walls"]),
                    "tokens_per_sec": block(rb["tok_rates"]),
                    "cache_hit_pct": block(rb["hits"]),
                } for realm, rb in realms.items()
            } for mid, realms in mr_buckets.items()
        },
        "by_model_acct": {
            mid: {
                realm: {
                    acct: {
                        "requests": ab["total"],
                        "errors": ab["err"],
                        "client_aborted": ab.get("aborted", 0),
                        "success_rate_pct": round(ab["ok"] * 100.0 / (ab["ok"] + ab["err"]), 1)
                                             if (ab["ok"] + ab["err"]) else None,
                        "ttft_ms": block(ab["ttfts"]),
                        "generation_ms": block(ab["gens"]),
                        "wall_ms": block(ab["walls"]),
                        "tokens_per_sec": block(ab["tok_rates"]),
                        "cache_hit_pct": block(ab["hits"]),
                    } for acct, ab in accts.items()
                } for realm, accts in realms.items()
            } for mid, realms in ma_buckets.items()
        }
    }


# ---------------------------------------------------------------------------
# perf 的增量样本（T2）
#
# perf_stats 每次重建都要重读日志末尾 sample 行、逐行 json.loads、把每一行的
# 五个数值往全局与三层模型桶各 append 一次，最后每个桶排序求分位。真机
# 558ms/次，TTL 15 秒——面板开着就是每 15 秒付一次。
#
# 这里按 (sample, realm, 窗口) 各记一份**环形样本状态**：
#   ring     最近 sample 条非空行各自的折叠结果（含尾部半行，deque，手工淘汰）
#   agg      这些行对聚合的贡献：全局计数 + 五个数值双端队列 + 三层模型桶
#   offset   已经折到日志的哪个字节；key/tail 是那条前缀的身份证明
# 于是 TTL 过期后的重建只把 offset 之后的新行并进来、把滑出样本的行撤出去，
# 不再重读也不再重新解析；位置校验不过（截断 / 换文件 / 同尺寸重写）才从
# 日志尾部的样本重建一次。
#
# 口径一个都没动：sample 仍是"最近 sample 行"（不是时间窗）、sample_capped
# 仍表示 tail 被截断、realm/窗口预过滤与分位口径照旧（分位始终从原始样本算，
# D6）；三层桶的存在与顺序、成功率的算法、sample_from 的取值规则全部照抄
# _perf_stats_uncached。这份状态**不落盘**（清单 §1.5：重启付一次冷扫即可，
# 落盘会让 checkpoint 涨几百 KB）。
_perf_state = {}
# 面板实际用到的键就两三个（`?realm=` 一个、带窗口的一个），多出来的按最近
# 使用淘汰，免得用户来回切 range 时状态无限堆积。单份状态的内存与 sample 成
# 正比（本机实测：sample=5000 的环+聚合 ≈ 3.4MB，20000 ≈ 13MB），所以上限压
# 到 4 份——面板的键数装得下，最坏内存 ~14MB；淘汰只是下次重建一次，不影响
# 任何输出。
_PERF_STATE_MAX = 4
# ring 条目的形状，见 _perf_line_entry：短元组 = 不贡献聚合的行。
_PERF_REC_LEN = 10


def _perf_new_bucket():
    return {"total": 0, "ok": 0, "err": 0, "aborted": 0,
            "ttfts": deque(), "gens": deque(), "walls": deque(),
            "tok_rates": deque(), "hits": deque()}


def _perf_new_agg():
    return {"total": 0, "ok": 0, "err": 0, "aborted": 0,
            "ttfts": deque(), "gens": deque(), "walls": deque(),
            "tok_rates": deque(), "hits": deque(),
            "m": {}, "mr": {}, "ma": {}}


def _perf_put(vals, value, add, front):
    """一个数值贡献：并入 append，撤销时从对应的那一头弹出。

    环里的行严格按日志顺序进出：滑出样本的是最旧的一行（front），上一轮末尾
    的半行被重折时摘掉的是最新的一行（back）。同一张表的每一头都只对应同一
    行——这正是撤销能 O(1) 做完的原因。
    """
    if add > 0:
        vals.append(value)
    elif front:
        vals.popleft()
    else:
        vals.pop()


def _perf_apply(state, entry, add, front):
    """并入（add=1）或撤销（add=-1）一行对聚合的贡献。

    撤销只做并入做过的那些事：计数减回去、数值表从并进的那一头弹出、桶的
    total 归零就把桶删掉——重建时一个桶存在与否只看样本里还有没有它的行，
    所以"删空桶"正是重建的输出形状；删掉的桶后来再有行进来会重新建在末尾，
    位置与重建时的"第一次出现"一致（旧行都滑出去了，新行就是它现在最早的
    一行）。判定条件与 _perf_stats_uncached 的逐行循环逐句对应：
    elapsed 走真值判断、ttft/cache_hit 走 is not None、tokens_per_sec 走真值。
    """
    if entry is None or len(entry) < _PERF_REC_LEN:
        return
    _at, m_id, r_realm, acct, outcome, elapsed, ttft, gen, tps, hit = entry
    agg = state["agg"]
    if add > 0:
        mb = agg["m"].get(m_id)
        if mb is None:
            mb = agg["m"][m_id] = _perf_new_bucket()
        realms = agg["mr"].get(m_id)
        if realms is None:
            realms = agg["mr"][m_id] = {}
        rb = realms.get(r_realm)
        if rb is None:
            rb = realms[r_realm] = _perf_new_bucket()
        accts = agg["ma"].get(m_id)
        if accts is None:
            accts = agg["ma"][m_id] = {}
        by_realm = accts.get(r_realm)
        if by_realm is None:
            by_realm = accts[r_realm] = {}
        ab = by_realm.get(acct)
        if ab is None:
            ab = by_realm[acct] = _perf_new_bucket()
    else:
        mb = agg["m"][m_id]
        rb = agg["mr"][m_id][r_realm]
        ab = agg["ma"][m_id][r_realm][acct]
    agg["total"] += add
    for b in (mb, rb, ab):
        b["total"] += add
    # A client that walks away is not a gateway failure: it gets its own
    # bucket instead of dragging the success rate down (same split as the
    # full-scan path).
    if outcome == "client_aborted":
        agg["aborted"] += add
        for b in (mb, rb, ab):
            b["aborted"] += add
            if elapsed:
                _perf_put(b["walls"], elapsed, add, front)
        if elapsed:
            _perf_put(agg["walls"], elapsed, add, front)
    elif outcome != "completed":
        agg["err"] += add
        for b in (mb, rb, ab):
            b["err"] += add
            if elapsed:
                _perf_put(b["walls"], elapsed, add, front)
        if elapsed:
            _perf_put(agg["walls"], elapsed, add, front)
    else:
        agg["ok"] += add
        for b in (mb, rb, ab):
            b["ok"] += add
        if ttft is not None:
            _perf_put(agg["ttfts"], ttft, add, front)
            for b in (mb, rb, ab):
                _perf_put(b["ttfts"], ttft, add, front)
        if gen is not None:
            _perf_put(agg["gens"], gen, add, front)
            for b in (mb, rb, ab):
                _perf_put(b["gens"], gen, add, front)
        if elapsed is not None:
            _perf_put(agg["walls"], elapsed, add, front)
            for b in (mb, rb, ab):
                _perf_put(b["walls"], elapsed, add, front)
        if tps:
            _perf_put(agg["tok_rates"], tps, add, front)
            for b in (mb, rb, ab):
                _perf_put(b["tok_rates"], tps, add, front)
        if hit is not None:
            _perf_put(agg["hits"], hit, add, front)
            for b in (mb, rb, ab):
                _perf_put(b["hits"], hit, add, front)
    if add < 0:
        if not mb["total"]:
            del agg["m"][m_id]
        if not rb["total"]:
            del agg["mr"][m_id][r_realm]
            if not agg["mr"][m_id]:
                del agg["mr"][m_id]
        if not ab["total"]:
            del agg["ma"][m_id][r_realm][acct]
            if not agg["ma"][m_id][r_realm]:
                del agg["ma"][m_id][r_realm]
            if not agg["ma"][m_id]:
                del agg["ma"][m_id]


def _perf_at_token(line):
    """预过滤认可的写法里 at 的浮点值（读不到就 None）。

    _line_outside_window 判定为"窗外"的行，其形状已经被它逐条核过：以
    `{"at": ` 开头、值以逗号收尾、值是浮点字面量、后面没有第二个 "at" 键。
    所以这里读到的值与 json.loads 会给的值相同（json 对带小数点/指数的记号
    也走 float()）。只给窗口预过滤跳过的行记 sample_from 用——那些行不解析，
    但旧实现里"样本最老的那一行"会被解析，它的 at 就是 sample_from。
    """
    if not line.startswith(_AT_PREFIX):
        return None
    end = line.find(",", len(_AT_PREFIX))
    if end < 0:
        return None
    token = line[len(_AT_PREFIX):end]
    if "." not in token and "e" not in token and "E" not in token:
        return None
    try:
        return float(token)
    except ValueError:
        return None


def _perf_line_entry(state, line):
    """把一行日志折成 ring 条目（与 _perf_stats_uncached 的逐行循环逐句对应）。

    返回：
      None      这一行没有可用的行数据（坏 JSON，或被窗口预过滤掉且读不出 at）
      (at,)     解析成功但被 realm / 窗口过滤掉——或被窗口预过滤跳过（at 取自
                它自己声明的值）：不贡献聚合，但仍决定 sample_from。旧实现里
                样本最老的那一行无论窗口如何都会被解析，所以被预过滤掉的行也
                要留下 at。
      (at, model, realm, account, outcome, elapsed, ttft, gen, tps, hit)
                通过全部过滤，计入聚合（10 元组，_PERF_REC_LEN）
    """
    since = state["since"]
    until = state["until"]
    realm = state["realm"]
    # 与旧实现同一道廉价预过滤：它只丢"下面的窗口判断本来也会丢"的行。
    # 旧实现的开关是 sample_from is not None（= 样本里已经有一行的 at 非空），
    # 这里用环里 at 非空的行数 n_at 表达同一件事。
    #
    # 多一道 line.endswith("}")：预过滤跳过的行也要记下它声明的 at（供
    # sample_from 用），但一条写了一半的行（尾部没换行、JSON 也没写完）不该
    # 因此报出一个旧实现给不出的 sample_from——完整行才走跳过分支，其余照旧
    # 落到 json.loads 上（坏 JSON 仍然是 None）。完整对象行总是以 } 收尾，
    # 所以这道判断不改变任何合法行省下的解析。
    if (since or until) and state["n_at"] and line.endswith("}") and \
            _line_outside_window(line, since or None, until or None):
        return (_perf_at_token(line),)
    try:
        r = json.loads(line)
    except Exception:
        return None
    at = r.get("at")
    if realm and not row_matches_realm(r, realm):
        return (at,)
    at_value = at or 0
    if since and at_value < since:
        return (at,)
    if until and at_value > until:
        return (at,)
    return (at, r.get("model") or "unknown", row_realm(r),
            r.get("account") or "(unattributed)", row_outcome(r),
            r.get("elapsed_ms"), r.get("ttft_ms"), r.get("gen_ms"),
            r.get("tokens_per_sec"), r.get("cache_hit_pct"))


def _perf_drop_oldest(state):
    """滑出环里最旧的一行（含它对聚合的贡献）。"""
    old = state["ring"].popleft()
    if old is not None and old[0] is not None:
        state["n_at"] -= 1
    _perf_apply(state, old, -1, True)


def _perf_push(state, line):
    """把一行并进环：先腾位置，再折行、并入。

    腾位置放在折行之前——旧实现的窗口预过滤开关是"当前样本里已经有行的 at
    非空"，滑出样本的行不再算数。若先折行再淘汰，新行就可能被预过滤掉一行
    "在旧样本里不算、在新样本里却是第一行"的数据，sample_from 会跟着变。
    """
    sample = state["sample"]
    if sample <= 0:
        return
    ring = state["ring"]
    while len(ring) >= sample:
        _perf_drop_oldest(state)
    entry = _perf_line_entry(state, line)
    if entry is not None and entry[0] is not None:
        state["n_at"] += 1
    _perf_apply(state, entry, 1, False)
    ring.append(entry)


def _perf_pop_last(state):
    """摘掉环尾的条目（上一轮的半行，重折前先撤掉它的贡献）。"""
    ring = state["ring"]
    if not ring:
        return
    old = ring.pop()
    if old is not None and old[0] is not None:
        state["n_at"] -= 1
    _perf_apply(state, old, -1, False)


def _perf_fold(state):
    """把日志 offset 之后的新行并进状态；尾部半行作为样本的最后一行并进去。

    字节纪律与 _count_usage_scan 一致：整行推进、半行不推进 offset（内容随时
    可能被写完整，下次从同一位置按完整内容重折）。但半行在**样本**里算一行
    ——_tail_lines 就是这么数的（它把末尾没有换行的残余当一行）——所以它进环，
    只是每次重折前先摘掉上一轮那一份。

    返回"样本变没变"：没变时渲染可以整段跳过（0 新字节的 TTL 重算就是这条路）。
    """
    ring = state["ring"]
    if state["pending"]:
        _perf_pop_last(state)
        state["pending"] = False
    offset = state["offset"]
    pending_raw = None
    try:
        with open(USAGE_LOG, "rb") as fh:
            fh.seek(offset)
            while True:
                raw = fh.readline()
                if not raw:
                    break
                if not raw.endswith(b"\n"):
                    pending_raw = raw
                    break
                line = raw.decode("utf-8", "replace").strip()
                if line:
                    _perf_push(state, line)
                offset += len(raw)
    except FileNotFoundError:
        pass
    except OSError as exc:
        log("perf incremental read failed: %s" % exc)
    changed = offset != state["offset"] or pending_raw != state["pending_raw"]
    state["offset"] = offset
    state["pending_raw"] = pending_raw
    if pending_raw is not None and pending_raw.strip() and state["sample"] > 0:
        line = pending_raw.decode("utf-8", "replace").strip()
        if line:
            _perf_push(state, line)
            state["pending"] = True
    return changed


def _perf_payload(state):
    """把环形样本的聚合渲染成载荷（键序与字段与旧实现逐字一致）。

    输出里三层桶的顺序 = 各桶在样本里**第一次出现**的顺序（整段重扫时天然
    如此）。增量维护这个顺序要在每次淘汰时重排，比这里一趟扫描还贵；所以
    渲染时扫一遍环把顺序还原出来——只读环、不解析，而且桶都见齐就停。
    """
    agg = state["agg"]
    order = {}          # model -> realm -> account（插入序 = 第一次出现序）
    sample_from = None
    want = len(agg["m"])
    for realms in agg["mr"].values():
        want += len(realms)
    for realms in agg["ma"].values():
        for accts in realms.values():
            want += len(accts)
    seen = 0
    for entry in state["ring"]:
        if entry is None:
            continue
        if sample_from is None and entry[0] is not None:
            sample_from = entry[0]
        if len(entry) < _PERF_REC_LEN:
            continue
        realms = order.get(entry[1])
        if realms is None:
            realms = order[entry[1]] = {}
            seen += 1
        accts = realms.get(entry[2])
        if accts is None:
            accts = realms[entry[2]] = {}
            seen += 1
        if entry[3] not in accts:
            accts[entry[3]] = None
            seen += 1
        if seen >= want:
            break
    by_model = {}
    by_model_realm = {}
    by_model_acct = {}
    for mid, realms in order.items():
        mb = agg["m"][mid]
        by_model[mid] = {
            "requests": mb["total"],
            "errors": mb["err"],
            "client_aborted": mb["aborted"],
            "success_rate_pct": round(mb["ok"] * 100.0 / (mb["ok"] + mb["err"]), 1)
                                 if (mb["ok"] + mb["err"]) else None,
            "ttft_ms": _perf_block(mb["ttfts"]),
            "generation_ms": _perf_block(mb["gens"]),
            "wall_ms": _perf_block(mb["walls"]),
            "tokens_per_sec": _perf_block(mb["tok_rates"]),
            "cache_hit_pct": _perf_block(mb["hits"]),
        }
        realm_out = {}
        acct_out = {}
        for realm, accts in realms.items():
            rb = agg["mr"][mid][realm]
            realm_out[realm] = {
                "requests": rb["total"],
                "errors": rb["err"],
                "client_aborted": rb["aborted"],
                "success_rate_pct": round(rb["ok"] * 100.0 / (rb["ok"] + rb["err"]), 1)
                                     if (rb["ok"] + rb["err"]) else None,
                "ttft_ms": _perf_block(rb["ttfts"]),
                "generation_ms": _perf_block(rb["gens"]),
                "wall_ms": _perf_block(rb["walls"]),
                "tokens_per_sec": _perf_block(rb["tok_rates"]),
                "cache_hit_pct": _perf_block(rb["hits"]),
            }
            accts_out = {}
            for acct in accts:
                ab = agg["ma"][mid][realm][acct]
                accts_out[acct] = {
                    "requests": ab["total"],
                    "errors": ab["err"],
                    "client_aborted": ab["aborted"],
                    "success_rate_pct": round(ab["ok"] * 100.0 / (ab["ok"] + ab["err"]), 1)
                                         if (ab["ok"] + ab["err"]) else None,
                    "ttft_ms": _perf_block(ab["ttfts"]),
                    "generation_ms": _perf_block(ab["gens"]),
                    "wall_ms": _perf_block(ab["walls"]),
                    "tokens_per_sec": _perf_block(ab["tok_rates"]),
                    "cache_hit_pct": _perf_block(ab["hits"]),
                }
            acct_out[realm] = accts_out
        by_model_realm[mid] = realm_out
        by_model_acct[mid] = acct_out
    return {
        "sampled": agg["total"],
        # Where the sampled slice starts and whether it was cut short, so a
        # week/month view can admit that its latency columns do not reach back
        # to the window's own start.
        "sample_from": sample_from,
        "sample_capped": len(state["ring"]) >= state["sample"],
        "success": agg["ok"],
        "errors": agg["err"],
        "client_aborted": agg["aborted"],
        # Success rate is measured against requests the gateway actually
        # finished; client cancellations are reported separately rather than
        # being counted as failures.
        "success_rate_pct": round(agg["ok"] * 100.0 / (agg["ok"] + agg["err"]), 1)
                             if (agg["ok"] + agg["err"]) else None,
        "ttft_ms": _perf_block(agg["ttfts"]),
        "generation_ms": _perf_block(agg["gens"]),
        "wall_ms": _perf_block(agg["walls"]),
        "tokens_per_sec": _perf_block(agg["tok_rates"]),
        "cache_hit_pct": _perf_block(agg["hits"]),
        "by_model": by_model,
        "by_model_realm": by_model_realm,
        "by_model_acct": by_model_acct,
    }


def _log_complete_frontier(path=None):
    """最后一个换行之后的字节偏移：整行折叠从这里开始。

    日志的尾巴可能是写了一半的一行（没有换行符），它算样本的一行但不算已折
    的字节——offset 停在这里，等它写完整再从同一位置重折。向前找换行只在
    冷路径（或位置校验作废）上做，通常第一次读就命中。
    """
    path = path or USAGE_LOG
    try:
        size = os.path.getsize(path)
    except OSError:
        return 0
    chunk = 256 * 1024
    pos = size
    try:
        with open(path, "rb") as fh:
            while pos > 0:
                step = min(chunk, pos)
                pos -= step
                fh.seek(pos)
                idx = fh.read(step).rfind(b"\n")
                if idx >= 0:
                    return pos + idx + 1
    except OSError:
        return 0
    return 0


def _perf_rebuild(key, sample, realm, since, until):
    """冷启动 / 位置校验不过：从日志尾部的 sample 行重建环形状态。"""
    state = {"sample": sample, "realm": realm, "since": since, "until": until,
             "key": None, "offset": 0, "tail": b"", "ring": deque(),
             "n_at": 0, "pending": False, "pending_raw": None,
             "agg": _perf_new_agg(), "at": 0.0,
             "dirty": True, "payload": None}
    raw_lines = _tail_lines(USAGE_LOG, sample)
    frontier = _log_complete_frontier()
    try:
        size = os.path.getsize(USAGE_LOG)
    except OSError:
        size = 0
    pending_raw = None
    if size > frontier:
        try:
            with open(USAGE_LOG, "rb") as fh:
                fh.seek(frontier)
                pending_raw = fh.read()
        except OSError:
            pending_raw = None
    # 尾部半行由 _tail_lines 算进样本的最后一行（末尾残余非空白时），这里把它
    # 从"完整行"里摘出来单独折——下面 offset 停在它前面，重折路径与它一致。
    if raw_lines and pending_raw is not None and raw_lines[-1] == pending_raw:
        raw_lines = raw_lines[:-1]
    for raw in raw_lines:
        line = raw.decode("utf-8", "replace").strip()
        if line:
            _perf_push(state, line)
    state["offset"] = frontier
    if pending_raw is not None and pending_raw.strip() and sample > 0:
        line = pending_raw.decode("utf-8", "replace").strip()
        if line:
            _perf_push(state, line)
            state["pending"] = True
    state["key"] = _usage_log_key()
    state["tail"] = _log_tail_signature(frontier)
    state["at"] = time.time()
    _perf_state[key] = state
    return state


def _perf_trim_states():
    """状态数超上限时按最近使用淘汰（载荷缓存另有 TTL，不受影响）。"""
    while len(_perf_state) > _PERF_STATE_MAX:
        oldest = min(_perf_state, key=lambda k: _perf_state[k]["at"])
        del _perf_state[oldest]


def _perf_stats_incremental(key, sample, realm, since, until):
    """perf_stats 的增量重建：输出与 _perf_stats_uncached 逐字节一致。

    必须在 _perf_lock 里调用（perf_stats 就是这么调的，与"重建在锁里做"的
    旧意图一致）：状态与载荷缓存由同一把锁保护。
    """
    try:
        state = _perf_state.get(key)
        log_key = _usage_log_key()
        if state is None or not _log_resume_ok(state, log_key):
            # 没有状态，或日志被截断/换掉/同尺寸重写（P:2525-2540 那套位置
            # 校验）：从尾部样本重建。
            state = _perf_rebuild(key, sample, realm, since, until)
        else:
            if _perf_fold(state):
                state["dirty"] = True
            state["key"] = _usage_log_key()
            state["tail"] = _log_tail_signature(state["offset"])
        if state["dirty"] or state["payload"] is None:
            # 样本没变就不重渲染：0 新字节的 TTL 重算到此为止（几十微秒），
            # 不再对每个桶重排序求分位。
            state["payload"] = _perf_payload(state)
            state["dirty"] = False
        payload = state["payload"]
        state["at"] = time.time()
        _perf_trim_states()
        return payload
    except Exception as exc:
        # 任何意外都退回旧的整段重建：状态作废（不留半折的账），这一次仍然
        # 给出正确载荷；旧实现遇到同样的输入会抛一样的异常，这里不会更差。
        _perf_state.pop(key, None)
        log("perf incremental failed, falling back to full scan: %s" % exc)
        return _perf_stats_uncached(sample, realm, since=since, until=until)


_snap_cache = {}
_snap_lock = threading.Lock()
# The dashboard polls every 5s. A TTL shorter than the poll interval makes
# every other poll do the full uncached scan; 15s means at most one rebuild
# per three polls while the numbers stay a few seconds stale at worst.
_STATS_TTL = float(os.environ.get("WB_STATS_TTL", 15))


# ---------------------------------------------------------------------------
# Daily usage counters
#
# The upstream caps a free window at a fixed token budget (code 6004), and by
# the time it answers 429 the window is already spent. This counter lets the
# operator park an account at a threshold instead: usage.jsonl is folded into
# uid -> tokens-since-local-midnight, plus two sibling views of the same rows
# - uid -> credit spent today (the daily credit guard) and
# uid -> {model: tokens} (the per-model daily guard).
# AccountPool.apply_daily_token_limit() / apply_daily_credit_limit() /
# apply_model_daily_token_limit() copy the numbers onto the accounts and
# ready() refuses them, so the next request rotates to another account. The
# scan is incremental (byte offset + per-day totals), so the hot path only
# reads rows that arrived since the last scan.
# ---------------------------------------------------------------------------
_daily_usage = {"day": "", "totals": None, "credits": None, "models": None,
                "offset": 0, "at": 0.0}
_daily_usage_lock = threading.Lock()


def _daily_state_copy(source):
    """Copy the cached per-account counters into a fresh scan state.

    Three views of the same rows: uid -> total tokens, uid -> credit spent,
    uid -> {model: tokens}. The copy keeps a later scan from mutating the
    cached dicts in place while readers hold them.
    """
    return {
        "tokens": dict(source.get("totals") or {}),
        "credits": dict(source.get("credits") or {}),
        "models": {k: dict(v) for k, v in (source.get("models") or {}).items()},
    }


def _scan_daily_usage(offset, state):
    """Fold rows at/after today's local midnight into `state`.

    Returns (state, new_offset). A line without its trailing newline is left
    for the next scan: rows are appended whole, so a partial tail only means
    this read raced the writer.
    """
    midnight = _local_midnight()
    with open(USAGE_LOG, encoding="utf-8") as fh:
        fh.seek(offset)
        while True:
            pos = fh.tell()
            line = fh.readline()
            if not line:
                break
            if not line.endswith("\n"):
                return state, pos
            offset = fh.tell()
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except Exception:
                continue
            if (row.get("at") or 0) < midnight:
                continue
            # Same rule as the analytics scan: a client cancellation is not a
            # consumed request, and its token counts are incomplete.
            if row_outcome(row) == "client_aborted":
                continue
            uid = row.get("account")
            if not uid:
                continue
            tokens = row.get("total_tokens") or 0
            state["tokens"][uid] = state["tokens"].get(uid, 0) + tokens
            credit = row.get("credit") or 0
            if credit:
                state["credits"][uid] = state["credits"].get(uid, 0.0) + credit
            mid = row.get("model")
            if mid:
                per = state["models"].setdefault(uid, {})
                per[mid] = per.get(mid, 0) + tokens
    return state, offset


def daily_usage_stats(ttl=None):
    """Today's per-account usage folded from the log, cached for `ttl` seconds.

    Returns {"tokens": uid -> tokens, "credits": uid -> credit spent,
    "models": uid -> {model: tokens}}, or None when the log could not be read
    at all; callers keep that distinct from zero so a failed read never parks
    an account.
    """
    ttl = _STATS_TTL if ttl is None else ttl
    day = time.strftime("%Y-%m-%d")
    now = time.time()
    with _daily_usage_lock:
        c = _daily_usage
        if c["day"] == day and c["totals"] is not None and (now - c["at"]) < ttl:
            return _daily_state_copy(c)
        # A new day keeps the byte offset: everything past it is today's, and
        # the midnight filter drops whatever old rows are still unread.
        if c["day"] == day and c["totals"] is not None:
            state = _daily_state_copy(c)
        else:
            state = {"tokens": {}, "credits": {}, "models": {}}
        offset = int(c["offset"] or 0)
        try:
            size = os.path.getsize(USAGE_LOG)
        except OSError:
            size = 0
        if offset > size:
            state = {"tokens": {}, "credits": {}, "models": {}}
            offset = 0
        try:
            state, offset = _scan_daily_usage(offset, state)
        except Exception as exc:
            log("daily token scan failed: %s" % exc)
            _daily_usage.update({"day": day, "totals": None, "offset": 0,
                                 "at": time.time()})
            return None
        _daily_usage.update({"day": day, "totals": state["tokens"],
                             "credits": state["credits"],
                             "models": state["models"], "offset": offset,
                             "at": time.time()})
        return _daily_state_copy(_daily_usage)


def daily_tokens_by_account(ttl=None):
    """uid -> tokens counted since local midnight, cached for `ttl` seconds.

    None means the log could not be read at all; callers keep that distinct
    from zero so a failed read never parks an account.
    """
    stats = daily_usage_stats(ttl=ttl)
    if stats is None:
        return None
    return stats["tokens"]


def seconds_until_local_midnight():
    """Seconds until the local day rolls over (at least a minute)."""
    lt = time.localtime()
    nxt = time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday + 1, 0, 0, 0, 0, 0, -1))
    return max(60, int(nxt - time.time()))


def apply_daily_token_limit(refresh=False):
    """Push the daily token setting and today's counts into the pool.

    The setting is resolved per realm, so the pool can guard the two exits
    with different numbers while a realm that carries no override still
    follows the global default.
    """
    if POOL is None:
        return 0
    limits = wb_settings.limit_values(ACCOUNTS_DIR, "daily_token_limit")
    usage = None
    if any(value > 0 for value in limits.values()):
        usage = daily_tokens_by_account(ttl=0 if refresh else None)
    return POOL.apply_daily_token_limit(limits, usage)


def apply_daily_credit_limit(refresh=False):
    """Push the daily credit setting, today's spend and the free-model view
    into the pool."""
    if POOL is None:
        return 0
    limits = wb_settings.limit_values(ACCOUNTS_DIR, "daily_credit_limit")
    credits = None
    free_models = None
    if any(value > 0 for value in limits.values()):
        stats = daily_usage_stats(ttl=0 if refresh else None)
        credits = stats["credits"] if stats is not None else None
        free_models = free_models_by_realm()
    return POOL.apply_daily_credit_limit(limits, credits, free_models)


def apply_model_daily_token_limit(refresh=False):
    """Push the per-model daily token setting and today's counts into the pool."""
    if POOL is None:
        return 0
    limits = wb_settings.limit_values(ACCOUNTS_DIR, "model_daily_token_limit")
    per_model = None
    if any(value > 0 for value in limits.values()):
        stats = daily_usage_stats(ttl=0 if refresh else None)
        per_model = stats["models"] if stats is not None else None
    return POOL.apply_model_daily_token_limit(limits, per_model)


def apply_daily_guards(refresh=False):
    """Push all three daily guards into the pool in one pass.

    The guards share their inputs - one settings file and the same folded day
    counters - and the request path used to call them back to back, so each
    one re-read the settings, re-scanned (and re-copied) the same counters and
    re-walked the pool. Here it is one settings read, one fold, three
    publishes; each guard's own semantics are untouched: per-realm limits, the
    free-model exemption on the credit guard, and the "no limit configured ->
    unknown count, never park" rule (an unreadable log leaves the counts None
    for all three, exactly as the separate calls did).

    `refresh` forces the counter fold to bypass its TTL (the settings-save
    path uses it so a limit change is visible immediately). The three
    individual entry points stay for callers that only want one guard.
    """
    if POOL is None:
        return 0
    ttl = 0 if refresh else None
    guards = wb_settings.limit_values_many(
        ACCOUNTS_DIR, ("daily_token_limit", "daily_credit_limit",
                       "model_daily_token_limit"))
    token_limits = guards["daily_token_limit"]
    credit_limits = guards["daily_credit_limit"]
    model_limits = guards["model_daily_token_limit"]
    wants = (any(v > 0 for v in token_limits.values())
             or any(v > 0 for v in credit_limits.values())
             or any(v > 0 for v in model_limits.values()))
    stats = daily_usage_stats(ttl=ttl) if wants else None
    # 池上这三个 apply_* 的返回值是它们各自的 limits 字典（不是计数），调用方
    # 谁也没用过；这里数的是「推了几条护栏」，别拿返回值做算术。
    published = 0
    if any(v > 0 for v in token_limits.values()):
        POOL.apply_daily_token_limit(
            token_limits, stats["tokens"] if stats is not None else None)
        published += 1
    if any(v > 0 for v in credit_limits.values()):
        POOL.apply_daily_credit_limit(
            credit_limits, stats["credits"] if stats is not None else None,
            free_models_by_realm())
        published += 1
    if any(v > 0 for v in model_limits.values()):
        POOL.apply_model_daily_token_limit(
            model_limits, stats["models"] if stats is not None else None)
        published += 1
    return published


# ---------------------------------------------------------------------------
# Remaining usage estimate
#
# 上游给「账号 × 模型」的免费额度是一个 24 小时窗口：窗口被用满时 429（code
# 6004）会带上重置墙钟（intl「usage will reset at …」/ cn「将在 … 重置」），
# 到点计数归零、额度回满。实测（2026-10-10 的 34k 行日志）：重置时刻过后几秒
# 就又开始整额消耗（f793acc1 在 00:50:16 重置，00:50:26 起正常出流），所以
# 「窗口起点 = 重置时刻」；把 429 那一刻往前 24h 的用量加总，跨账号得到高度
# 一致的数字（deepseek-v4.1-flash 7 个样本 179.7M~201.0M，hy4-preview-f 两个
# intl 样本 20.5M/22.1M），也就是该模型窗口预算的估计值——这个数字上游不给，
# 只能在账号真的撞线时反推。
#
# 数据有两个来源，合并去重（键 = 账号 + 模型 + 重置时刻）：
#   - usage/limit-events.jsonl：撞线发生的那一刻由请求路径记下（note_limit_
#     event），带账号、模型、重置时刻与样本。**必须实时记**：一次请求的重试
#     循环可能接连撞好几个账号，但只有最后那个账号会留下一条带账号归属的
#     429 用量行，其余账号的撞线只在 syslog 里出现过；
#   - usage.jsonl 里带账号的 429 行：本功能上线前就存在的历史由它补出预算，
#     新部署时也能兜住事件文件写失败的场合。
# 样本 = 重置前 24h 内该账号在该模型上的成功用量（429 行自身不算用量）；
# 查询时按区域聚合（先按账号平均、再跨账号平均）得预算，再对每个账号算
# 「自窗口起点以来的用量」：剩余 = 预算 − 已用。
# 还在冷却（重置时刻在未来）的组合窗口已用满，剩余记 0；没有撞线记录（或记录
# 已过期、锚不住当前窗口）的组合没有已知的重置时刻，按最近 24h 用量保守估计
# （窗口起点必然落在最近 24h 内，所以这是剩余量的下界），并在载荷里标 estimated
# 供面板区分。
#
# 分段记账：撞线后，那个窗口的记账在撞线处截断（撞线时刻之后的行不再计入——
# 上游已经拒收，残留的行不算这个窗口的消耗）；恢复时刻起新窗口重新记账，旧
# 窗口的用量不得计进新窗口的已用。窗口起点优先取链上上一个重置时刻（两个重置
# 时刻相隔不到 24h 时才是同一个窗口链），否则取重置时刻 − 24h——实测窗口锚定
# 比「固定 24h 链」复杂（有 48h+ 的空档），所以只用上游给的重置时刻这一个事实。
# 载荷里每行带 segments（每段 start/end/used，截断段标 capped），面板按
# 「上段 + 本段」呈现；预算样本同样按段截断（链式撞线时样本从上一个重置时刻
# 起算，旧窗口的用量不进新窗口的样本）。
#
# 锚点来源除事件日志外还有凭证档的 model_caps（持久化，见 wb_accounts）：本次
# 进程没撞过线（日志丢失/被裁）但磁盘上有记录时，冷却、窗口起点与截断点照它算。
#
# 按需过滤：昨天或今天没消耗过的模型不参与剩余用量计算（判定见
# _remaining_activity_floor；撞线锚点本身算消耗证据，冷却中的组合不因旧行被裁
# 掉就消失）。预算样本不因过滤而丢弃——它要撞线才学得到，载荷里标 active=False
# 让面板把它标成「未参与」而不是整条消失（否则每次跨零点面板都抖）。
# 按模型的总剩余 = 可用组的剩余之和：跨账号按 (区域, 模型) 汇总，冷却/已满的
# 组记 0（不参与减法、不把别的组拉低），没有预算的组计入 unknown 不计数。
#
# 按天持久化 + 近 3 天加权（estimate-daily.json）：样本按撞线当天的本地日期
# 分桶，估算是最近 3 天的加权平均（今天 6 / 昨天 3 / 前天 1，就近高权重，只在
# 有样本的天之间归一；某组合 3 天内没有样本就没有预算）。今天用内存里的事件
# 现算；更早的天在跨天时归档进文件、之后直接读——「调昨天的数据不重新计算」。
# 归档发生在构建载荷时发现本地日期变了（或重启后第一次构建）的那一刻：把刚
# 过去的那天算成 {预算, 分段} 合并进文件（该天已在文件里就不覆盖，幂等）。
# 文件里每天的预算是当天样本「先按账号平均、再跨账号平均」的 {avg,min,max,n}；
# 分段是每个 (账号, 模型) 的 pre（当天截断前用量）/ post（当天截断后用量），
# 满足 pre + post == 当天该组合的用量（用户要的「今日截断前 / 截断后」两个数，
# 调昨天的数据不用再算）。保留：预算留最近 3 天（与估算窗口一致），分段只留
# 最近 2 天（用户要求「仅保留到第二天」，多留一天防跨天边界）。
# 归档的计算在状态锁内做（要读事件表与用量缓冲），文件写盘在锁外（与
# checkpoint 同款理由：写盘在状态锁里会构成锁序环）。
#
# 查询侧不做重复计算：
#   - 每个 (账号, 模型) 的用量缓冲是**时间戳 + 累计和**两个列表，窗口
#     求和是两次二分 + 一次相减（O(log n)），不是每次查询把上万行重新加一遍；
#     每行只留时间戳与累计值（旧的 deque-of-tuple 是 tuple + 两个装箱数字），
#     弱设备上省下的是实实在在的内存与遍历；
#   - 载荷挂一个短 TTL（WB_REMAINING_TTL，默认 15 秒）——面板每 5 秒轮询一次，
#     而数字只在用量或撞线发生时变，逐次重建是纯浪费。TTL 内直接返回上次算好的
#     载荷，连日志尾部都不再扫；面板带 If-None-Match 轮询时命中 304，连那点
#     JSON 也不再重传（校验符派生自缓存条目的构建时刻，与其它统计接口同款）。
# ---------------------------------------------------------------------------
LIMIT_WINDOW_SECONDS = 24 * 3600
LIMIT_EVENTS_FILE = os.path.join(USAGE_DIR, "limit-events.jsonl")
# 按天持久化的估算数据（每天的预算口径 + 每个组合的截断前后分段）。与事件
# 日志一样必须跟随 --usage-dir：常量在 import 时按默认目录算过，漏掉它真机上
# 会一直写 ENOENT（2026-10-10 实测 limit-events.jsonl 踩过同一个坑）。
ESTIMATE_DAILY_FILE = os.path.join(USAGE_DIR, "estimate-daily.json")
_ESTIMATE_SCHEMA = 1
# recent 的行偏移索引（页 = 日志尾部的第 N 页）。同样必须跟随 --usage-dir：
# 常量在 import 时按默认目录算过，漏掉它换目录后索引会写到旧目录去。
RECENT_INDEX_FILE = os.path.join(USAGE_DIR, "usage-recent-index.json")
# 估算窗口的权重：今天 6 / 昨天 3 / 前天 1，就近高；只在有样本的天之间归一
# （某天没样本就跳过）。
_ESTIMATE_WEIGHTS = (6, 3, 1)
# 保留：预算留最近这么多天（与估算窗口一致，多留无用——估算读不到）；分段
# 只留最近 _ESTIMATE_SEGMENT_DAYS 天（用户要求「仅保留到第二天」，多留一天
# 防跨天边界），更早的天删掉 segments 字段、预算保留到窗口外一天才整条删。
_ESTIMATE_KEEP_DAYS = len(_ESTIMATE_WEIGHTS)
_ESTIMATE_SEGMENT_DAYS = 2
# 缓冲多留 2h：查询侧的窗口起点最远只能到 now-24h，留出余量避免边界丢行。
_REMAINING_KEEP_SECONDS = 26 * 3600
# 事件只留最近这么多条：预算估计吃的是近期样本，几十条足够，封顶防日志异常
# 时无界增长（每条只记数字与 uid/model，一千条不到 200KB）。文件本身不裁剪，
# 冷启动全读一遍也就几十毫秒。
_REMAINING_MAX_EVENTS = 1000
# 载荷缓存时长（秒）。比面板的 5 秒轮询长、远短于其它统计接口的 900 秒统计 TTL：
# 数字最多滞后这么久，但一轮重建能服务好几轮轮询。
_REMAINING_TTL = float(os.environ.get("WB_REMAINING_TTL", 15))

_remaining_state = {
    "events": [],          # 事件列表，src 标来源（file / log）
    "keys": set(),         # (账号, 模型, 重置时刻) 去重
    "usage": {},           # (uid, model) -> _pair_buffer() 的 {at,cum,head,base}
    "log": {"offset": 0, "key": None, "tail": b""},      # usage.jsonl 的续读位
    "file": {"offset": 0, "key": None, "tail": b""},     # limit-events.jsonl 的
    # 内存字段（不进 checkpoint，schema 不动）：事件表当前覆盖到哪个本地日，
    # 跨天归档的触发依据。重启后为空 -> 第一次构建按「刚过去的那天」试归档
    # 一次（幂等；数据不在就跳过），再置为今天。
    "day": "",
}
_remaining_state_lock = threading.Lock()
# 载荷缓存（与上面的折叠状态分开两把锁：重建要持状态锁做扫描，命中缓存不该等它）。
_remaining_cache = {"at": 0.0, "built_at": 0.0, "data": None}
_remaining_cache_lock = threading.Lock()
# estimate-daily.json 的解析缓存：单键元组一次赋值（(路径, mtime_ns, size),
# days），读侧不加锁也不会看到半份缓存；写盘路径主动换掉它，外部改动靠 stat
# 变化发现。路径也在戳里：--usage-dir 换目录后不会被旧目录的戳骗过去。
_estimate_daily_cache = {"entry": ((None, None, None), {})}
# 文件读-改-写串行锁：只在 _estimate_daily_sync() 里拿一次，且永远在状态锁
# 之外（归档计算在状态锁内、写盘在锁外，同 checkpoint 的理由）。
_estimate_daily_lock = threading.Lock()


def _pair_buffer(state, key):
    """The (at, cum) buffers of one (account, model), created on demand.

    Two parallel lists instead of a deque of (at, tokens) tuples: window sums
    then cost two bisections plus one subtraction over `cum`, and appends stay
    O(1). Only the timestamps and the running sum are kept - a row's token
    count is never read again once it is folded into `cum`, so a third list
    would be pure memory. No new stdlib import either: the packaged runtime's
    trim contract (portable_runtime.REQUIRED_FILES) already vouches for
    bisect, while `array` is not in it (builtin on Windows, a shared extension
    elsewhere) - see tests/_test_release_assets.py.
    `head` is the first live index, `base` the token sum of everything trimmed
    away so far - `cum` keeps counting from the very first row ever buffered,
    so a range whose left edge falls on the first live entry has to subtract
    `base` instead of a predecessor.
    """
    buf = state["usage"].get(key)
    if buf is None:
        buf = state["usage"][key] = {"at": [], "cum": [], "head": 0, "base": 0}
    return buf


def _pair_append(buf, at, tokens):
    """Append one (at, tokens) sample, keeping the buffer sorted by `at`.

    The rows arrive in log order, but `at` is the request *completion* clock:
    two request threads finishing milliseconds apart can append out of order
    (the real log has a pair 4.96ms apart; the router's own checkpoint one
    0.47ms). `_range_sum` bisects these lists, so a single inversion would
    make the window sums skip or double-count whole stretches - the sorted
    insert (O(n) memmove on a pair that holds tens to hundreds of samples)
    is what keeps the bisection premise true. The common in-order append
    stays O(1) exactly as before.
    """
    # coerce: a log row is free text, and these buffers only ever hold numbers
    tokens = int(tokens)
    at = float(at)
    ats = buf["at"]
    if not ats or at >= ats[-1]:
        ats.append(at)
        buf["cum"].append((buf["cum"][-1] if buf["cum"] else 0) + tokens)
        return
    cum = buf["cum"]
    head = buf["head"]
    index = bisect.bisect_right(ats, at)
    if index < head:
        # The sample belongs before the dead prefix: compact it away first
        # (same absolute-base rule as _pair_trim), because the freshly folded
        # row is live by definition and must not land behind `head`.
        buf["base"] = cum[head - 1]
        del ats[:head]
        del cum[:head]
        buf["head"] = 0
        index = bisect.bisect_right(ats, at)
    ats.insert(index, at)
    # cum holds absolute prefix sums; every entry after the insert point
    # moves up by this sample's tokens.
    cum.insert(index, (cum[index - 1] if index else buf["base"]) + tokens)
    for i in range(index + 1, len(cum)):
        cum[i] += tokens


def _pair_repair(buf):
    """Canonicalize a (possibly out-of-order) pair buffer; False = unrepairable.

    A checkpoint written before this fix carries the buffers in log order,
    where `at` can regress by milliseconds (the router's own 674KB file does
    exactly that and was rejected whole). The loading side no longer demands
    an ordered list: this function stable-sorts the live region by `at` and
    rebuilds cum/head/base, so every (at, amount) pairing survives and each
    window sum equals the value recomputed on the sorted sequence. The dead
    prefix is physically dropped here (base keeps the absolute rule of
    _pair_trim's compaction), which leaves the state with exactly the live
    entries a cold fold would have. A negative amount (cum going backwards)
    is corrupt data: False makes the caller reject the entry and fall back
    to the cold fold.
    """
    at = buf["at"]
    cum = buf["cum"]
    head = buf["head"]
    if len(at) != len(cum) or not 0 <= head <= len(at):
        return False
    if head >= len(at):
        return False
    origin = cum[head - 1] if head else buf["base"]
    pairs = []
    prev = origin
    for i in range(head, len(cum)):
        amount = cum[i] - prev
        if amount < 0:
            return False
        pairs.append((at[i], amount))
        prev = cum[i]
    pairs.sort(key=lambda item: item[0])
    rebuilt_at = []
    rebuilt_cum = []
    total = origin
    for value, amount in pairs:
        rebuilt_at.append(value)
        total += amount
        rebuilt_cum.append(total)
    buf["at"] = rebuilt_at
    buf["cum"] = rebuilt_cum
    buf["head"] = 0
    buf["base"] = origin
    return True


def _pair_trim(buf, cutoff):
    """Drop entries older than `cutoff` (batched: the lists are compacted only
    when the dead prefix is big enough to pay for the memmove)."""
    at = buf["at"]
    while buf["head"] < len(at) and at[buf["head"]] < cutoff:
        buf["head"] += 1
    if buf["head"] > 512 and buf["head"] * 2 > len(at):
        # base 是「被裁掉前缀的累计和」的**绝对值**，不是增量：cum 从最早的一行
        # 累计、切片不重置，所以 cum[head-1] 已经把之前裁掉的都算进去了。写成
        # += 会让第二次压缩起把前缀重复计入，之后任何左端落在保留区之前的
        # 区间求和都会变成负数（真机上「上段」显示过 -1293.5M）。
        buf["base"] = buf["cum"][buf["head"] - 1]
        del at[:buf["head"]]
        del buf["cum"][:buf["head"]]
        buf["head"] = 0


def _range_sum(buf, lo, hi):
    """Tokens of one (account, model) with lo <= at <= hi. O(log n)."""
    if buf is None or buf["head"] >= len(buf["at"]):
        return 0
    at = buf["at"]
    j = bisect.bisect_left(at, lo, buf["head"])
    k = bisect.bisect_right(at, hi, buf["head"])
    if k <= j:
        return 0
    # cum 自最早的缓冲行累计；左端落到第一个存活条目时（j 在列表头），
    # 前面的累计值已被裁掉，只能拿 base 补回那段前缀。
    low = buf["cum"][j - 1] if j > 0 else buf["base"]
    return buf["cum"][k - 1] - low


def _range_sum_open(buf, lo, hi):
    """Tokens of one (account, model) with lo <= at < hi. O(log n).

    与 _range_sum 只差右端：日界用左闭右开（恰好落在午夜那一刻的行属于后一
    天），归档的当天用量与分段都按这个口径算，pre + post 的不变式在整秒边界
    上也不会多算一行。
    """
    if buf is None or buf["head"] >= len(buf["at"]):
        return 0
    at = buf["at"]
    j = bisect.bisect_left(at, lo, buf["head"])
    k = bisect.bisect_left(at, hi, buf["head"])
    if k <= j:
        return 0
    low = buf["cum"][j - 1] if j > 0 else buf["base"]
    return buf["cum"][k - 1] - low



def _merge_event(state, event, source):
    """Add one cap event, deduplicated by (account, model, reset).

    The first source to record a given cap wins: the request path's event is
    written the moment the 429 is handled, the log-derived one only appears
    when a scan reaches that row.
    """
    uid = str(event.get("account") or "")
    model = str(event.get("model") or "")
    try:
        reset = float(event.get("reset") or 0)
    except (TypeError, ValueError):
        return
    if not uid or not model or reset <= 0:
        return
    key = (uid, model, reset)
    if key in state["keys"]:
        return
    state["keys"].add(key)
    state["events"].append({
        "at": float(event.get("at") or 0), "reset": reset, "account": uid,
        "model": model, "realm": str(event.get("realm") or ""),
        "sample": int(event.get("sample") or 0), "src": source,
    })
    if len(state["events"]) > _REMAINING_MAX_EVENTS:
        # 从头部丢弃时同步清去重键：丢掉的键将来重新出现（重放日志）会再被
        # 收进来，不会因为键还留着而被静默跳过。
        dropped = state["events"][: len(state["events"]) - _REMAINING_MAX_EVENTS]
        del state["events"][: len(dropped)]
        for old in dropped:
            state["keys"].discard((old["account"], old["model"], old["reset"]))


def _drop_events(state, source):
    """Forget the events one source contributed (its file was rewritten)."""
    kept = [event for event in state["events"] if event["src"] != source]
    for event in state["events"]:
        if event["src"] == source:
            state["keys"].discard((event["account"], event["model"], event["reset"]))
    state["events"] = kept


def _window_sample(usage, uid, model, reset_at, at, start=None):
    """Usage of (uid, model) inside the window ending at `reset_at`.

    `start` 覆盖默认的窗口起点（重置时刻 − 24h）：链式撞线（上一个重置时刻
    落在 24h 之内）时窗口从上一个重置时刻起算，旧窗口的用量不进这个样本。
    """
    lo = reset_at - LIMIT_WINDOW_SECONDS if start is None else start
    return _range_sum(usage.get((uid, model)), lo, at)


def _cap_anchor(entries, at, reset):
    """把一个撞线锚点（撞线时刻 at + 恢复时刻 reset）并进 entries。

    entries 以 reset 为键去重：同一个重置时刻从事件日志与凭证档各来一份时
    只算一个锚点；at 取两者中较晚的（这个窗口最后被观测到还在消耗的时刻，
    截断点宁晚勿早——截早了会漏掉真实计过量的行）。
    """
    try:
        at = float(at or 0)
        reset = float(reset or 0)
    except (TypeError, ValueError):
        return
    if reset <= 0:
        return
    old = entries.get(reset)
    if old is None or at > old["at"]:
        entries[reset] = {"at": at, "reset": reset}


def _cap_anchors(events, uid, model):
    """(账号, 模型) 的锚点表，来自事件日志里该组合的全部事件。"""
    entries = {}
    for event in events:
        if event["account"] == uid and event["model"] == model:
            _cap_anchor(entries, event["at"], event["reset"])
    return entries


def _cap_window_start(entries, reset):
    """窗口起点：链上上一个重置时刻（落在 24h 内时），否则重置时刻 − 24h。

    上游的窗口锚定比「固定 24h 链」复杂（实测同组合的重置时刻有 48h+ 的空
    档），所以只用上游给的重置时刻这一个事实：上一个重置时刻落在 24h 窗口
    之内时它就是本窗口的起点（链式撞线，旧窗口的用量不得计进新窗口）；之
    外时窗口不超过 24h，起点取重置时刻 − 24h。
    """
    prev = 0.0
    for other in entries:
        if other < reset and other > prev:
            prev = other
    return max(prev, reset - LIMIT_WINDOW_SECONDS)


def _remaining_activity_floor(now):
    """「昨日 00:00」：本地时区里 now 所在日的前一天零点。

    参与剩余用量计算的判定下界（昨天或今天有消耗才算在用）。取整天边界而
    不是 now − 24h，因为需求写的是「昨日或当日」；同一天里这个下界不变，
    面板因此不会在轮询之间来回抖动。
    """
    lt = time.localtime(now)
    midnight = time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday, 0, 0, 0, 0, 0, -1))
    return midnight - LIMIT_WINDOW_SECONDS


def _save_account_caps(account, model, reset_at):
    """把一个 6004 的恢复时刻写进凭证档（best-effort）。

    写盘失败只影响「重启后是否记得」，不影响本次请求——真正的停用由内存里的
    模型冷却负责，重启后的补挂交给 AccountPool.restore_model_caps()。
    """
    try:
        account.save(os.path.dirname(account.path) if account.path else ACCOUNTS_DIR)
    except Exception as exc:
        log("account %s: model cap write failed: %s" % (account.uid[:8], exc))


def note_limit_event(account, model, reset_at):
    """Record a model-scoped upstream cap as it happens (429 / code 6004).

    Called from the request path the moment the 429 is classified: the sample
    is computed against the usage fold refreshed right here, because the rows
    that led to the cap are already in the log but may not have been scanned
    yet. Everything is best-effort - a failure to record must never break the
    request that is already failing.
    """
    try:
        uid = getattr(account, "uid", "") or ""
        if not uid or not model or not reset_at:
            return
        at = time.time()
        with _remaining_state_lock:
            _remaining_refresh(_remaining_state)
            anchors = _cap_anchors(_remaining_state["events"], uid, model)
            # 凭证档里的最近一次撞线（note_model_cap 还没记这次）也能锚住窗口
            # 起点：日志里没有上一个重置时刻时它是唯一的分段依据。
            cap = (getattr(account, "model_caps", None) or {}).get(model)
            if isinstance(cap, dict):
                _cap_anchor(anchors, cap.get("at"), cap.get("reset"))
            sample = _window_sample(_remaining_state["usage"], uid, model,
                                    reset_at, at,
                                    _cap_window_start(anchors, reset_at))
        event = {
            "at": at, "account": uid, "model": model,
            "realm": getattr(account, "realm", "") or "",
            "reset": float(reset_at), "sample": sample,
        }
        try:
            os.makedirs(USAGE_DIR, exist_ok=True)
            with open(LIMIT_EVENTS_FILE, "a", encoding="utf-8") as fh:
                fh.write(json.dumps(event, ensure_ascii=False) + "\n")
        except Exception as exc:
            log("limit event not persisted: %s" % exc)
        with _remaining_state_lock:
            _merge_event(_remaining_state, event, "file")
    except Exception as exc:
        log("limit event not recorded: %s" % exc)


def _fold_remaining(row, state):
    """Fold one usage.jsonl row into the remaining-usage state.

    The row order is the log's append order, so the trim below can use each
    row's own `at` as "now": rows only ever get newer, and a row older than
    the retention window can never be needed again.
    """
    uid = row.get("account")
    model = row.get("model")
    if not uid or not model:
        return
    at = row.get("at") or 0
    if not at:
        return
    key = (uid, model)
    if row.get("status") == 429:
        reset = parse_rate_limit_reset(str(row.get("message") or ""))
        if reset is None:
            return
        # 样本按段截断：链式撞线时从上一个重置时刻起算（旧窗口的用量不进
        # 新窗口的样本），没撞过线时就是默认的重置时刻 − 24h。
        anchors = _cap_anchors(state["events"], uid, model)
        _merge_event(state, {
            "at": at, "reset": reset, "account": uid, "model": model,
            "realm": row.get("realm") or "",
            "sample": _window_sample(state["usage"], uid, model, reset, at,
                                     _cap_window_start(anchors, reset)),
        }, "log")
        return
    if row.get("error") or row.get("usage_missing"):
        return
    if row_outcome(row) == "client_aborted":
        return
    buf = _pair_buffer(state, key)
    _pair_append(buf, at, row.get("total_tokens") or 0)
    _pair_trim(buf, at - _REMAINING_KEEP_SECONDS)


def _remaining_refresh(state):
    """Fold both files' new rows into `state`; caller holds the state lock.

    Same pass rules as the other folds: an unreadable or rewritten file resets
    the contributions it fed instead of publishing a half-folded one. The
    usage log is folded first so the event journal's samples (computed against
    it at cap time) and the log-derived events describe the same buffers.

    A brand-new process asks the checkpoint first: the cold scan is the only
    cost here that grows with the log (a few MB a day), so restarting should
    not pay it again. Missing or stale -> fold from zero, as before.
    """
    if state["log"].get("key") is None:
        _usage_cache_adopt_remaining(state, _usage_log_key())
    for source, path in (("log", USAGE_LOG), ("file", LIMIT_EVENTS_FILE)):
        slot = state[source]
        log_key = _usage_log_key(path)
        if not _log_resume_ok(slot, log_key, path):
            # 文件被换掉/截短：这个来源的贡献全部作废，从零重折。
            state[source] = {"offset": 0, "key": log_key, "tail": b""}
            if source == "log":
                state["usage"].clear()
            _drop_events(state, source)
        fold = (lambda row: _fold_remaining(row, state)) if source == "log" \
            else (lambda row: _merge_event(state, row, "file"))
        offset, error = _scan_usage_from(state[source]["offset"], fold, path=path)
        if error is None:
            state[source]["offset"] = offset
            state[source]["tail"] = _log_tail_signature(offset, path)
        else:
            log("remaining usage fold failed (%s): %s" % (source, error))
            state[source] = {"offset": 0, "key": log_key, "tail": b""}
            if source == "log":
                state["usage"].clear()
            _drop_events(state, source)


def _budget_means(per_account):
    """{账号: [样本]} -> {avg, min, max, n}：先按账号平均、再跨账号平均。

    一个账号反复撞线只算一个样本（它自己样本的均值），所以跨账号平均称的是
    账号而不是事件；min/max/n 都是账号均值的极值与个数（沿用既有口径）。
    """
    means = [int(round(sum(v) / float(len(v)))) for v in per_account.values()]
    return {"avg": int(round(sum(means) / float(len(means)))),
            "min": min(means), "max": max(means), "n": len(means)}


def _estimate_budget_entry_ok(entry):
    """归档文件里一天的口径：{avg,min,max,n} 都是数字且 n > 0 才算数。

    文件在数据目录里、可能被手改或半写坏：任何一项不合形状就跳过这一天，
    估算退化成「那天没样本」，绝不因此让载荷构建报错。
    """
    if not isinstance(entry, dict):
        return False
    if not _usage_cache_int(entry.get("n")) or entry["n"] <= 0:
        return False
    return all(_usage_cache_number(entry.get(field))
               for field in ("avg", "min", "max"))


def _estimate_day_keys(now):
    """估算窗口的日键，就近在前：[今天, 昨天, 前天]。"""
    return [_local_day_key(_local_midnight(now, days_back=back))
            for back in range(_ESTIMATE_KEEP_DAYS)]


def _estimate_daily_stamp():
    """estimate-daily.json 的 (路径, mtime_ns, size)；文件不在时后两项为 None。

    路径也在戳里：--usage-dir 换目录后，不会被旧目录的戳骗过去（两个目录里
    恰好有 mtime/大小相同的文件几乎不可能，但也没必要赌）。
    """
    try:
        st = os.stat(ESTIMATE_DAILY_FILE)
        return (ESTIMATE_DAILY_FILE, st.st_mtime_ns, st.st_size)
    except OSError:
        return (ESTIMATE_DAILY_FILE, None, None)


def _estimate_daily_read():
    """estimate-daily.json 的 days 表；坏文件/读不到按空表处理。

    按戳记忆解析结果：写盘路径主动刷新缓存，外部改动（手改、另一个进程）靠
    stat 变化发现。这是估算的加速件，任何一步不成立都只是回到「没有归档」
    （今天现算、更早的天跳过），绝不能因此报错。
    """
    stamp = _estimate_daily_stamp()
    cached_stamp, cached_days = _estimate_daily_cache["entry"]
    if cached_stamp == stamp:
        return cached_days
    days = {}
    if stamp[1] is not None:
        try:
            with open(ESTIMATE_DAILY_FILE, "r", encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, ValueError, UnicodeDecodeError):
            data = None
        if isinstance(data, dict) and data.get("schema") == _ESTIMATE_SCHEMA \
                and isinstance(data.get("days"), dict):
            days = {day: record for day, record in data["days"].items()
                    if _usage_cache_day_key_ok(day) and isinstance(record, dict)}
    _estimate_daily_cache["entry"] = (stamp, days)
    return days


def _estimate_daily_write(days):
    """原子写 estimate-daily.json（同目录 .tmp + os.replace，同 checkpoint）。"""
    os.makedirs(USAGE_DIR, exist_ok=True)
    tmp = ESTIMATE_DAILY_FILE + ".tmp"
    text = json.dumps({"schema": _ESTIMATE_SCHEMA, "days": days},
                      ensure_ascii=False, separators=(",", ":"))
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
        os.replace(tmp, ESTIMATE_DAILY_FILE)
    except OSError:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def _estimate_daily_view(daily, archived):
    """文件表 + 本次归档的内存视图：该天已在文件里就用文件里的（幂等口径）。

    本次构建的估算直接读这份视图，跨天那一轮的载荷不用等下一轮才看到昨天；
    已被持久化下来的那天不会被这次重算的值盖掉（「调昨天的数据不重新计算」）。
    """
    if not archived:
        return daily
    view = dict(daily)
    for day, record in archived.items():
        view.setdefault(day, record)
    return view


def _estimate_daily_sync(archived, now):
    """把归档记录合并进文件、按保留策略裁剪；内容不变就不写盘。

    幂等：该天已在文件里就不覆盖（重启/重复构建不会写第二遍、也不会用重算的
    值盖掉持久化下来的值）。保留：只留估算窗口内的最近 3 天，更早的天整条
    删掉；分段只留最近 2 天（用户要求「仅保留到第二天」，多留一天防跨天
    边界），更早的天删掉 segments 字段、预算保留到 3 天。

    写盘在状态锁外做（与 checkpoint 同款理由：写盘在状态锁里会构成锁序环），
    用写锁串行、原子替换；失败只记日志——这是估算的加速件，不影响请求。
    """
    with _estimate_daily_lock:
        days = dict(_estimate_daily_read())
        changed = False
        for day, record in (archived or {}).items():
            if day not in days:
                days[day] = record
                changed = True
        keep = _estimate_day_keys(now)
        segment_days = set(keep[:_ESTIMATE_SEGMENT_DAYS])
        pruned = {}
        for day, record in days.items():
            if day not in keep:
                changed = True
                continue
            if day not in segment_days and record.get("segments"):
                record = dict(record)
                record.pop("segments", None)
                changed = True
            pruned[day] = record
        if changed:
            try:
                _estimate_daily_write(pruned)
            except Exception as exc:
                log("estimate daily write failed: %s" % exc)
        # 缓存换成刚算好的表（含本次归档与裁剪）：写失败也不重试——这份表就是
        # 「本进程认为文件里有什么」，与 checkpoint 的「记尝试不记成功」同款。
        _estimate_daily_cache["entry"] = (_estimate_daily_stamp(), pruned)


def _remaining_archive_due(state, now, realm_of):
    """跨天（或重启后第一次构建）时归档刚过去的那天。

    调用方持有状态锁（归档要读事件表与用量缓冲）。返回 {日键: 记录}，没跨天
    或没有可归档的数据返回 None。state["day"] 是内存字段、不进 checkpoint：
    重启后为空，第一次构建按「刚过去的那天」试一次——幂等，数据不在就跳过，
    再置为今天（无论归档成没成）。
    """
    today = _local_day_key(now)
    stale = state.get("day") or ""
    if stale == today:
        return None
    state["day"] = today
    if not stale or stale >= today:
        # 重启后为空（或时钟回拨、stale 落在未来）：按「刚过去的那天」= 昨天。
        stale = _local_day_key(_local_midnight(now, days_back=1))
    if stale == today:
        return None
    record = _remaining_archive_day(state, stale, realm_of)
    return {stale: record} if record else None


def _remaining_archive_day(state, day, realm_of):
    """一天的量归档：当天样本的预算口径 + 每个组合的截断前后分段。

    budgets：撞线时刻 event["at"] 落在那天的样本，先按账号平均、再跨账号
    平均（沿用查询侧口径）。segments：那天每个 (uid, model) 的 pre/post。
    两样都空（数据不在缓冲里 / 当天没有样本）返回 None——调用方跳过不写。
    """
    start = _day_key_start(day)
    end = start + 86400
    events = [event for event in state["events"]
              if _local_day_key(event["at"]) == day]
    samples = {}
    for event in events:
        if event["sample"] <= 0:
            continue
        realm = event["realm"] or realm_of.get(event["account"], "")
        samples.setdefault((realm, event["model"]), {}) \
               .setdefault(event["account"], []).append(event["sample"])
    budgets = {"%s\x1f%s" % key: _budget_means(per_account)
               for key, per_account in samples.items()}
    segments = _remaining_archive_segments(state, events, start, end)
    if not budgets and not segments:
        return None
    return {"budgets": budgets, "segments": segments}


def _remaining_archive_segments(state, events, start, end):
    """那天每个 (uid, model) 的截断前 / 截断后用量。

    post = 恢复时刻落在当天的窗口在当天内的用量之和（从恢复时刻到「下一个
    撞线时刻或当天结束」）；pre = 当天该组合总用量 − post（不变式：pre + post
    == 当天该组合的用量，用缓冲的区间求和算）。两个都是 0 的组合不写。
    """
    caps = {}
    for event in events:
        caps.setdefault((event["account"], event["model"]), []).append(
            (event["at"], event["reset"]))
    segments = {}
    for (uid, model), buf in state["usage"].items():
        total = _range_sum_open(buf, start, end)
        pair_caps = caps.get((uid, model)) or ()
        post = 0
        for _at, reset in pair_caps:
            if not start <= reset < end:
                continue
            nxt = end
            for other_at, _other_reset in pair_caps:
                if reset < other_at < nxt:
                    nxt = other_at
            post += _range_sum_open(buf, reset, min(nxt, end))
        if total <= 0 and post <= 0:
            continue
        segments["%s\x1f%s" % (uid, model)] = {"pre": total - post, "post": post}
    return segments


def _remaining_budgets(events, realm_of, now, daily):
    """(realm, model) -> 近 3 天加权的窗口预算估计。

    样本按撞线当天的本地日期分桶，取最近 3 天（今天/昨天/前天）：今天用内存
    里的事件现算，更早的天直接读 estimate-daily.json 里归档好的当天口径（不
    再重算——这就是持久化的意义）。权重就近高（今天 6 / 昨天 3 / 前天 1，见
    _ESTIMATE_WEIGHTS），只在有样本的天之间归一，某天没样本就跳过。只有撞过
    6004 的窗口才有样本（天然成立），某组合 3 天内没有样本就没有预算。

    载荷里保留 avg/min/max/n（min/max 取各天账号均值的极值，n 为各天账号数
    之和），并新增 days 列出各天构成（{day, w, avg, n}）供面板/排查用。
    """
    keys = _estimate_day_keys(now)
    per_day = {}
    samples = {}
    for event in events:
        if event["sample"] <= 0 or _local_day_key(event["at"]) != keys[0]:
            continue
        realm = event["realm"] or realm_of.get(event["account"], "")
        samples.setdefault((realm, event["model"]), {}) \
               .setdefault(event["account"], []).append(event["sample"])
    for key, per_account in samples.items():
        per_day.setdefault(key, {})[keys[0]] = _budget_means(per_account)
    # 昨天/前天：直接读归档文件里当天的口径（avg/min/max/n 都是现成的）。
    for day in keys[1:]:
        record = (daily or {}).get(day) or {}
        for pair_key, entry in (record.get("budgets") or {}).items():
            if not _estimate_budget_entry_ok(entry):
                continue
            realm, _, model = pair_key.partition("\x1f")
            if not model:
                continue
            per_day.setdefault((realm, model), {})[day] = entry
    budgets = {}
    for key, days in per_day.items():
        rows = []
        for index, day in enumerate(keys):
            entry = days.get(day)
            if not entry or entry.get("n", 0) <= 0:
                continue
            rows.append({"day": day, "w": _ESTIMATE_WEIGHTS[index],
                         "avg": int(round(entry["avg"])), "n": int(entry["n"])})
        if not rows:
            continue
        weight = sum(row["w"] for row in rows)
        budgets[key] = {
            "avg": int(round(sum(row["w"] * row["avg"] for row in rows)
                             / float(weight))),
            "min": min(int(round(days[row["day"]]["min"])) for row in rows),
            "max": max(int(round(days[row["day"]]["max"])) for row in rows),
            "n": sum(row["n"] for row in rows),
            "days": rows,
        }
    return budgets


def _remaining_payload(now=None, accounts=None):
    """Per-account remaining usage against the estimated per-model budget.

    Refreshes the fold and builds the payload in one pass under the state
    lock: nothing is copied out (the buffers stay put and are read through
    their cumulative sums), so a rebuild costs what the new rows cost plus
    O(pairs * log n), not a re-sum of every buffered row.

    Every enabled account is reported for every model that has a budget in its
    realm, plus every model it actually used inside the retention window - an
    account at 0 usage is the honest "full budget" answer, not noise. A pair
    still cooling on a spent window reports remaining 0 and the reset clock
    that hands the quota back.

    Models nobody burned since yesterday 00:00 do not take part at all
    (activity is judged per realm + model, see _remaining_activity_floor);
    their budget samples stay in the payload marked inactive so the panel can
    say "not counted" instead of dropping a budget it would take another cap
    to relearn.

    跨天归档：本地日期与 state["day"] 不同（或重启后为空）时，在状态锁内把
    刚过去的那天算成 {预算, 分段}，本次构建的估算立刻把它并进视图；文件写盘
    在锁外（见 _estimate_daily_sync）。
    """
    now = time.time() if now is None else now
    if accounts is None:
        accounts = list(POOL.accounts) if POOL else []
    accounts = [a for a in accounts
                if getattr(a, "enabled", True) and getattr(a, "uid", "")]
    with _remaining_state_lock:
        state = _remaining_state
        _remaining_refresh(state)
        realm_of = {a.uid: (getattr(a, "realm", "") or "") for a in accounts}
        archived = _remaining_archive_due(state, now, realm_of)
        daily = _estimate_daily_view(_estimate_daily_read(), archived)
        payload = _remaining_payload_locked(state, now, accounts, realm_of, daily)
        offset = state["log"].get("offset") or 0
    # 归档与 checkpoint 的写盘都在锁外做（见 checkpoint 一节）：写盘要逐把读
    # 各份状态，在状态锁里再取别的状态锁会构成锁序环。折叠失败的那次 offset
    # 已归零，落盘函数会跳过。
    _estimate_daily_sync(archived, now)
    _usage_cache_maybe_save("remaining", offset)
    return payload


def _remaining_payload_locked(state, now, accounts, realm_of, daily):
    """The payload itself; caller holds the state lock and has refreshed."""
    budgets = _remaining_budgets(state["events"], realm_of, now, daily)
    # strftime/localtime is one of the few per-row costs left (30 rows ≈ 0.12ms,
    # 74% of the build) and rows overwhelmingly share their window start / reset
    # clock - so format each distinct second once per payload.
    iso_cache = {}

    def iso_at(ts):
        key = int(ts)
        out = iso_cache.get(key)
        if out is None:
            out = iso_cache[key] = time.strftime("%Y-%m-%d %H:%M:%S",
                                                 time.localtime(ts))
        return out

    # 撞线锚点：事件日志 + 凭证档里的 model_caps，按 (账号, 模型, 重置时刻)
    # 去重——重启后日志可能丢失/被裁，凭证档里持久化的记录是那时唯一的依据；
    # 反之凭证档写失败时日志还在。锚点决定冷却、窗口起点与截断点。
    anchors = {}
    for event in state["events"]:
        _cap_anchor(anchors.setdefault((event["account"], event["model"]), {}),
                    event["at"], event["reset"])
    for acct in accounts:
        caps = getattr(acct, "model_caps", None)
        if not isinstance(caps, dict):
            continue
        try:
            items = list(caps.items())
        except RuntimeError:
            # 恰好并发在记新撞线（note_model_cap 在另一把锁下改这张表）：
            # 这轮少一个锚点，下一轮重建就补上，不值得为它拿别人的锁。
            continue
        for model, entry in items:
            if isinstance(entry, dict):
                _cap_anchor(anchors.setdefault((acct.uid, str(model)), {}),
                            entry.get("at"), entry.get("reset"))

    # 按需过滤：昨天或今天没消耗过的模型不参与（判定见 _remaining_activity_floor）。
    # 消耗 = 缓冲里有行（最后一行的时间戳就是该组合最后一次用量），或撞线锚点
    # 落在下界之后——撞线是「把窗口用满」的证据，冷却中的组合不能因为旧行被裁
    # 掉就从面板上消失。
    floor = _remaining_activity_floor(now)
    active = set()
    for (uid, model), buf in state["usage"].items():
        realm = realm_of.get(uid)
        if realm is None or not buf["at"]:
            continue
        if buf["at"][-1] >= floor:
            active.add((realm, model))
    for (uid, model), entries in anchors.items():
        realm = realm_of.get(uid)
        if realm is None:
            continue
        if any(max(entry["at"], entry["reset"]) >= floor
               for entry in entries.values()):
            active.add((realm, model))

    rows = []
    totals = {}
    for acct in accounts:
        uid = acct.uid
        realm = realm_of.get(uid, "")
        models = set(model for (r, model) in budgets
                     if r == realm and (r, model) in active)
        models.update(model for (u, model) in state["usage"]
                      if u == uid and (realm, model) in active)
        for model in sorted(models):
            entries = anchors.get((uid, model)) or {}
            buf = state["usage"].get((uid, model))
            past = [e for e in entries.values() if e["reset"] <= now]
            future = [e for e in entries.values() if e["reset"] > now]
            last = max(past, key=lambda e: e["reset"]) if past else None
            upcoming = min(future, key=lambda e: e["reset"]) if future else None
            segments = []
            if upcoming is not None:
                # 窗口已用满：正在冷却，额度到恢复时刻才回来。这个窗口的记账
                # 在撞线处截断——已用到撞线时刻为止（上游已经拒收，撞线之后
                # 残留的行不算这个窗口的消耗），所以已用量≈预算，与剩余 0 对
                # 得上。窗口起点是链上上一个重置时刻（链式撞线时），否则重
                # 置时刻 − 24h。
                reset_at = upcoming["reset"]
                window_start = _cap_window_start(entries, reset_at)
                end = min(now, upcoming["at"] or reset_at)
                used = _range_sum(buf, window_start, end)
                segments.append({"start": window_start, "end": end, "used": used,
                                 "capped": True, "current": True,
                                 "start_iso": iso_at(window_start),
                                 "end_iso": iso_at(end)})
                cooling = True
                estimated = False
            elif last is not None:
                reset_at = last["reset"]
                window_start = max(reset_at, now - LIMIT_WINDOW_SECONDS)
                used = _range_sum(buf, window_start, now)
                cooling = False
                # 重置时刻超过 24h 的组合锚不住当前窗口：它那个窗口早已到期，
                # 现在这个窗口的起点未知，落在最近 24h 内的用量只是下界，与
                # 从没撞过线的组合同样标 estimated。
                estimated = reset_at < now - LIMIT_WINDOW_SECONDS
                if not estimated:
                    # 分段相加：上一段（上一个窗口，在它的撞线处截断）+ 本段
                    # （自恢复时刻起重新记账，旧窗口的用量不计进本段的已用）。
                    if last["at"] > 0:
                        prev_end = min(last["at"], reset_at)
                        prev_start = _cap_window_start(entries, reset_at)
                        segments.append({
                            "start": prev_start, "end": prev_end,
                            "used": _range_sum(buf, prev_start, prev_end),
                            "capped": True, "current": False,
                            "start_iso": iso_at(prev_start),
                            "end_iso": iso_at(prev_end)})
                    segments.append({"start": reset_at, "end": now, "used": used,
                                     "capped": False, "current": True,
                                     "start_iso": iso_at(reset_at),
                                     "end_iso": iso_at(now)})
            else:
                window_start = now - LIMIT_WINDOW_SECONDS
                used = _range_sum(buf, window_start, now)
                reset_at = None
                cooling = False
                estimated = True
            budget = budgets.get((realm, model))
            remaining = None
            pct = None
            if cooling:
                # 冷却中的组剩余记 0（没有预算也照记）：额度到恢复时刻才回来。
                remaining = 0
                pct = round(used * 100.0 / budget["avg"], 1) \
                    if budget and budget["avg"] else None
            elif budget:
                remaining = max(0, budget["avg"] - used)
                pct = round(used * 100.0 / budget["avg"], 1) if budget["avg"] else None
            rows.append({
                "uid": uid,
                "nickname": getattr(acct, "nickname", "") or "",
                "realm": realm,
                "model": model,
                "used": used,
                "budget": budget["avg"] if budget else None,
                "budget_min": budget["min"] if budget else None,
                "budget_max": budget["max"] if budget else None,
                "samples": budget["n"] if budget else 0,
                "remaining": remaining,
                "used_pct": pct,
                "window_start": window_start,
                "window_iso": iso_at(window_start),
                "reset_at": reset_at,
                "reset_iso": iso_at(reset_at) if reset_at else None,
                "cooling": cooling,
                "estimated": estimated,
                "segments": segments,
            })
            # 按模型总剩余 = 可用组的剩余之和：冷却/已满的组 remaining 本身就是
            # 0（只加不减，绝不把别的组拉低），没有预算的组计入 unknown 不计数。
            total = totals.get((realm, model))
            if total is None:
                total = totals[(realm, model)] = {
                    "realm": realm, "model": model, "total_remaining": None,
                    "budget": budget["avg"] if budget else None,
                    "pairs": 0, "available": 0, "cooling": 0,
                    "unknown": 0, "estimated": 0,
                }
            total["pairs"] += 1
            if cooling:
                total["cooling"] += 1
            if estimated:
                total["estimated"] += 1
            if remaining is None:
                total["unknown"] += 1
            else:
                total["total_remaining"] = (total["total_remaining"] or 0) + remaining
                if not cooling:
                    total["available"] += 1
    rows.sort(key=lambda r: (r["remaining"] is None,
                             r["remaining"] if r["remaining"] is not None else 0,
                             r["realm"], r["nickname"], r["model"]))
    return {
        "window_seconds": LIMIT_WINDOW_SECONDS,
        "generated_at": now,
        "generated_iso": iso_at(now),
        "activity_floor": floor,
        "activity_floor_iso": iso_at(floor),
        "budgets": [dict(budget, realm=realm, model=model,
                         active=(realm, model) in active)
                    for (realm, model), budget in sorted(budgets.items())],
        "models": sorted(totals.values(),
                         key=lambda m: (m["total_remaining"] is None,
                                        -(m["total_remaining"] or 0),
                                        m["realm"], m["model"])),
        "rows": rows,
    }


def remaining_usage(ttl=None):
    """Cached payload: the panel polls every 5s, the numbers change far slower.

    Within the TTL the last payload is returned as is - no rescan of either
    file, no rebuild - which is what makes the 5s polling cost nothing between
    rebuilds. The TTL is deliberately short (WB_REMAINING_TTL, 15s default):
    long enough to serve several polls per rebuild, short enough that a cap or
    a burst of usage shows up while the operator is looking at the panel.
    """
    ttl = _REMAINING_TTL if ttl is None else ttl
    now = time.time()
    with _remaining_cache_lock:
        entry = _remaining_cache
        if entry["data"] is not None and (now - entry["at"]) < ttl:
            return entry["data"]
    # 重建在缓存锁外：它自己会拿状态锁扫描，命中等它的调用不该被拖住。
    data = _remaining_payload()
    with _remaining_cache_lock:
        _remaining_cache.update({"at": time.time(), "built_at": time.time(),
                                 "data": data})
    return data


def remaining_usage_etag():
    """ETag of the current /usage/remaining cache entry, or None.

    与 usage_by_account_etag() 同款：键是常量（这个视图只有一条缓存），配对
    调用由 _json_cached() 保证。轮询在 TTL 内命中同一份载荷 → 同一枚校验符
    → 304；重建后自动换戳。
    """
    with _remaining_cache_lock:
        entry = _remaining_cache
        if entry["data"] is None:
            return None
        built_at = entry["built_at"]
    return _cache_etag("remaining", "all", built_at)


# ---------------------------------------------------------------------------
# 剩余用量优先调度（开关默认关，见 wb_settings.remaining_priority_enabled）
#
# 「快达到估计限额的 (账号, 模型) 先被交出」：估计剩余占预算的比例越小权重
# 越高，分派侧（AccountPool._pick_remaining_first）只对权重 > 1 的组合做平滑
# 加权轮询。分档而不是线性：面板上说得清（剩不到 30% 开始加权、5% 以内最重），
# 测试里也钉得住确切数字。权重表由请求路径每请求推给池——数据方向与
# apply_daily_* 一致，池不反向依赖本模块。
# ---------------------------------------------------------------------------
_REMAINING_PRIORITY_BANDS = ((0.05, 4), (0.15, 3), (0.30, 2))


def _remaining_schedule_weight(remaining, budget):
    """估计剩余对应的调度权重：1 = 不加权，2..4 = 越接近耗尽越重。

    没有可比较的尺度时不猜：预算样本缺失（budget 为 None/0）或剩余未知
    （remaining 为 None）一律不加权。剩余 <= 0 也不加权——它没有「先用掉」
    的意义（真正用满的组合正在冷却，本来就不可用；estimated 口径下它只是
    下界，不值得把流量往一个可能已经耗尽的组合上压）。
    """
    try:
        remaining = float(remaining)
        budget = float(budget)
    except (TypeError, ValueError):
        return 1
    if remaining <= 0 or budget <= 0:
        return 1
    ratio = remaining / budget
    for ceiling, weight in _REMAINING_PRIORITY_BANDS:
        if ratio <= ceiling:
            return weight
    return 1


def remaining_priority_enabled():
    """优先调度开关（默认关）。"""
    return wb_settings.remaining_priority_enabled(ACCOUNTS_DIR) is True


def remaining_schedule_weights():
    """{(uid, model): 权重}：估计剩余接近耗尽的组合，权重 > 1。

    只读剩余估算载荷的既有字段（rows 的 uid / model / remaining / budget），
    不碰折叠与预算的实现。载荷自带短 TTL：面板在轮询时本来就维持着它，请求
    路径这里拿到的基本都是缓存，重建只发生在 TTL 到点时（与面板同款成本）。
    best-effort：任何失败都只意味着这一次不加权，绝不能连累请求本身。
    """
    try:
        payload = remaining_usage()
        weights = {}
        for row in payload.get("rows") or []:
            weight = _remaining_schedule_weight(row.get("remaining"),
                                                row.get("budget"))
            if weight > 1:
                weights[(row.get("uid") or "", row.get("model") or "")] = weight
        return weights
    except Exception as exc:
        log("remaining priority weights unavailable: %s" % exc)
        return {}


_free_models_cache = {"at": 0.0, "data": None}
_FREE_MODELS_TTL = 60.0


def credits_is_free(value):
    """True when a catalogue credits string means "this one costs nothing".

    The same test curate_remote_catalog() uses to pick free siblings; a
    missing or unparsable value is NOT free, so an unknown model stays
    under the credit guard instead of slipping past it.
    """
    return str(value or "").strip().lower() in ("x0.00", "x0", "0", "0.00")


def free_models_by_realm():
    """realm -> set of model ids the catalogue marks free ("x0.00").

    Built from the bundled snapshot and the desktop cache file - both local
    reads, no network - and cached for a minute so the request path pays
    nothing. The daily credit guard uses it to tell paid models from free
    ones, per realm: the same id can be free on one exit and paid on the
    other.
    """
    now = time.time()
    data = _free_models_cache.get("data")
    if data is not None and (now - _free_models_cache.get("at", 0.0)) < _FREE_MODELS_TTL:
        return data
    out = {}
    for realm, source in (("intl", getattr(wb_catalog, "STATIC_INTL_MODELS", [])),
                          ("cn", getattr(wb_catalog, "STATIC_CN_MODELS", []))):
        free = set()
        for item in source or []:
            if not isinstance(item, dict):
                continue
            mid = str(item.get("id") or "").strip()
            if mid and credits_is_free(item.get("credits")):
                free.add(mid)
        cached = read_cached_remote_catalog(realm)
        if cached:
            for mid, item in (cached[1] or {}).items():
                if isinstance(item, dict) and credits_is_free(item.get("credits")):
                    free.add(str(mid))
        out[realm] = free
    _free_models_cache.update({"at": now, "data": out})
    return out


def _usage_snapshot_scope(realm, range, since, until):
    """(过滤用的 realm, since, until, 缓存键) —— usage_snapshot() 与它的 ETag 共用。

    同 _perf_stats_scope()：键只允许有一个来源，否则 ETag 可能指向另一条缓存。
    """
    r = realm_scope(realm, CURRENT_REALM)
    lo, hi = range_window(range, since, until)
    # Bounds, not a today/all flag: this week and this month overlap, so a
    # flag would let one window serve the other's totals from the cache.
    key = "%s|%s|%s" % (r or "all",
                        lo if lo is not None else "", hi if hi is not None else "")
    return r, lo, hi, key


def usage_snapshot(realm=None, ttl=None, range=None, since=None, until=None):
    """Cached wrapper: the dashboard polls this every few seconds.

    The rebuild happens while holding the lock on purpose. Releasing it first
    let every concurrent caller run its own full scan of the JSONL when the
    entry expired, so a single dashboard refresh could trigger several scans
    of the same file.
    """
    ttl = _STATS_TTL if ttl is None else ttl
    r, lo, hi, key = _usage_snapshot_scope(realm, range, since, until)
    now = time.time()
    with _snap_lock:
        hit = _snap_cache.get(key)
        if hit is not None and (now - hit[0]) < ttl:
            return hit[1]
        data = _usage_snapshot_uncached(r, since=lo, until=hi)
        _snap_cache[key] = (time.time(), data)
    return data


def usage_snapshot_etag(realm=None, range=None, since=None, until=None):
    """usage_snapshot() 当前缓存条目的 ETag；条目不存在时返回 None。

    配对调用（取数前后各一次、一致才发）由 _json_cached() 负责，理由见那里。
    """
    _r, _lo, _hi, key = _usage_snapshot_scope(realm, range, since, until)
    with _snap_lock:
        hit = _snap_cache.get(key)
    if hit is None:
        return None
    return _cache_etag("usage", key, hit[0])


def _fold_cost(bucket, cost, model):
    """Fold one row's estimated cost into a stats bucket.

    Unpriced models land in cost_missing (id -> count) instead of quietly
    vanishing from the totals, so the panel can name what the price
    snapshot does not cover yet. A row the master switch left unpriced is not
    "missing a price" - the whole feature is off - so it is not listed there.
    """
    if cost["known"]:
        bucket["cost_cny"] = (bucket.get("cost_cny") or 0.0) + cost["cny"]
    elif not cost.get("disabled"):
        missing = bucket.setdefault("cost_missing", {})
        mid = model or "unknown"
        missing[mid] = missing.get(mid, 0) + 1


# ---------------------------------------------------------------------------
# Incremental all-time aggregates
#
# The dashboard's all-time panels used to re-read the whole usage.jsonl on
# every cache miss, and the log grows by thousands of rows a day. Those totals
# only change when a row is appended, so each aggregate keeps the byte offset
# it has already folded and a refresh reads just the rows past it - the shape
# daily_usage_stats() already uses for today's counters, truncation guard
# included. A windowed query still walks the file, but the pre-filter above
# drops out-of-window lines before the parse, so it pays only for the rows it
# keeps.
#
# A carried-over total is only valid while everything the fold read is
# unchanged, so a state also carries a fingerprint of its other inputs: the
# price tables (a new policy can re-price a row that has no reference of its
# own) and the realm a row without a realm field is attributed to. When one of
# those moves, the fold starts over - an offset alone would happily keep
# yesterday's prices forever.
#
# One state per realm and per aggregate: each holds a few models and a few
# accounts, so keeping one around costs nothing next to the scan it replaces.
# ---------------------------------------------------------------------------

# What wb_pricing hands back while its file is missing. Without these, every
# call would get a fresh empty dict, the identity test below would never
# match, and a deployment with no price files would rescan on every refresh.
_EMPTY_POLICIES = {}
_EMPTY_TIMELINE = []


def _usage_log_key(path=None):
    """(size, device, inode) of the usage log, or a zero key when it is gone.

    One stat answers both questions an incremental fold has to ask. The size
    says whether the bytes the cached offset counted are still there: a
    shorter file was truncated, or replaced by a shorter one. The device and
    inode say whether it is still the same file at all - a log copied over in
    place keeps its inode and is caught by the size test, while one moved into
    position is a different file and a stale offset would point into the
    middle of unrelated rows.

    `path` defaults to the usage log; the remaining-usage fold keys its own
    event journal with the same rule.
    """
    try:
        info = os.stat(path or USAGE_LOG)
        return (info.st_size, info.st_dev, info.st_ino)
    except OSError:
        return (0, 0, 0)


def _log_offset_holds(previous, current, offset):
    """True when `offset` still sits inside the file `previous` described.

    False means the cached fold counted bytes that are gone or belong to a
    different file now, so it has to start over.
    """
    return (previous is not None
            and previous[1] == current[1] and previous[2] == current[2]
            and current[0] >= offset)


# How many bytes of the already-counted prefix are read back on every
# refresh to prove that prefix still holds the same bytes. Long enough
# that a rewrite cannot match it by accident, short enough to cost
# nothing on the poll path.
_TAIL_SIGNATURE = 64


def _log_tail_signature(offset, path=None):
    """The last bytes before `offset`, or None when they cannot be read.

    An incremental fold assumes the bytes it has already counted are still
    there. Size and inode catch a log that was replaced or cut short, but
    not one that was emptied in place and has since grown past the old
    offset: same file, only longer, which is what truncating the log - or
    a log rotation with copytruncate - leaves behind. Reading the seam back
    is what tells those apart, because an append leaves it alone and a
    rewrite does not.
    """
    if offset <= 0:
        return b""
    try:
        with open(path or USAGE_LOG, "rb") as fh:
            start = max(0, offset - _TAIL_SIGNATURE)
            fh.seek(start)
            return fh.read(offset - start)
    except OSError:
        return None


def _log_resume_ok(state, log_key, path=None):
    """True when the cached offset still sits on the log's current prefix.

    False means the fold cannot be carried over: the file shrank, it is a
    different file, or the bytes it counted were rewritten under it.
    """
    if not _log_offset_holds(state["key"], log_key, state["offset"]):
        return False
    return _log_tail_signature(state["offset"], path) == state["tail"]


def _pricing_inputs(pricing_on=None):
    """Everything cost_for_row() reads besides the row itself.

    Identity, not equality: wb_pricing caches each of these behind its own
    file key, so an edited file hands back a different object, and comparing
    identities keeps the check off the refresh path - walking the whole price
    table to notice a change would cost more than the change does. Holding the
    objects here is also what makes the test sound: a live object cannot have
    its id handed to a replacement.

    With the master switch off cost_for_row() never opens a table at all, so
    the flag is the whole input. It is read through the same no-argument
    call cost_for_row() makes - passing a directory here would answer from
    a different settings file whenever the two disagree, and the fingerprint
    would then watch an input the fold never read.
    """
    # pricing_on 由调用方传入时不再自己问一次总开关：一次扫描里它只该被问一次
    # （见 _usage_snapshot_uncached / _compute_usage_analytics_uncached），指纹
    # 与折叠读到的因此是同一个值。
    try:
        if not (wb_pricing.pricing_enabled() if pricing_on is None else pricing_on):
            return (False, None, None, None)
        return (True,
                wb_pricing.load_policies() or _EMPTY_POLICIES,
                wb_pricing.load_timeline() or _EMPTY_TIMELINE,
                wb_pricing.load_pricing())
    except Exception as exc:
        log("pricing inputs unreadable: %s" % exc)
        return None


def _pricing_unchanged(previous, current):
    """True when two _pricing_inputs() tuples describe the same prices.

    只留给行级结果记忆（_row_cost_remember）：那里按「价表代次」整表判定，
    保守但正确。状态指纹走 _pricing_used_unchanged()（T15 的收窄判定）。
    """
    if previous is None or current is None:
        return False
    return (previous[0] == current[0] and previous[1] is current[1]
            and previous[2] is current[2] and previous[3] is current[3])


# ---------------------------------------------------------------------------
# 「用到的价」状态指纹（T15）：状态指纹收窄成折叠行真正用到的价
#
# 价表内容一变（哪怕只是新增一个无关模型的价格），带价格指纹的 5 条状态
# （snapshot×3 + analytics×2）此前全部判废、各重折 12-15s；但已折行的成本
# 在设计上就是历史价——带 cost_policy 的行按不可变策略计价，无引用老行由
# 时间线解析（见 _fold_usage_snapshot 的注释："a later price change cannot
# rewrite yesterday's totals"）——所以绝大多数重折是白付。
#
# 这里把状态指纹从「(policies, timeline, pricing_doc) 三张表的对象身份」收窄
# 成一份记录（state["pricing"]，checkpoint 的 "pricing" 字段，schema 5）：
#   * refs           折叠解析出来的每个 pid 的策略内容摘要（_object_digest 按
#                    对象身份记忆，热路径 O(1)）；pid 当时查不到记 None（=该
#                    pid 必须继续缺席，出现即作废）。
#   * timeline       时间线依赖的投影摘要（没有依赖时 None）。
#   * timeline_at    无引用且成本非零的行的 at 上界。
#   * timeline_models 上述行的模型集合。
#   * unresolved     上述行里整条时间线都没提到的模型（解析扫过全部行）。
#   * backfilled     上述行里出现过回填解析（解析扫过全部行）。
#   * builtin        用到内置快照时，内置价目表文档的内容摘要。
#
# 判定侧 _pricing_used_unchanged() 从当前价表重算同一份记录并逐项比对，不等
# 就判废重折；相等的部分保证「重折会给出同样的数」。
#
# 不动的东西：_row_cost / _row_cost_remember（行级记忆代次仍用整表判定）、
# series（不算价）、任何聚合口径与窗口路径。
# ---------------------------------------------------------------------------

# 记录里的固定字段（进程内构造与 checkpoint 采纳都按这一组校验）。
_PRICED_FIELDS = ("refs", "timeline", "timeline_at", "timeline_models",
                  "unresolved", "backfilled", "builtin")


def _pricing_record_new():
    """一份空的状态指纹记录（见上面一节；字段固定、可 JSON 往返）。"""
    return {"refs": {}, "timeline": None, "timeline_at": None,
            "timeline_models": [], "unresolved": [], "backfilled": False,
            "builtin": None}


def _pricing_record_ok(record):
    """状态指纹记录的形状；不合格的记录按「价表变了」处理（判废重折）。"""
    if not isinstance(record, dict):
        return False
    for field in _PRICED_FIELDS:
        if field not in record:
            return False
    refs = record["refs"]
    if not isinstance(refs, dict):
        return False
    for pid, digest in refs.items():
        if not isinstance(pid, str) or not (digest is None
                                            or isinstance(digest, str)):
            return False
    for field in ("timeline", "builtin"):
        value = record[field]
        if not (value is None or isinstance(value, str)):
            return False
    at = record["timeline_at"]
    if not (at is None or _usage_cache_number(at)):
        return False
    for field in ("timeline_models", "unresolved"):
        value = record[field]
        if not isinstance(value, list) or not all(isinstance(v, str)
                                                  for v in value):
            return False
    return isinstance(record["backfilled"], bool)


def _copy_pricing_record(record):
    """记录的深拷贝：落盘在状态锁外序列化，记录必须和活状态脱钩。"""
    if record is None:
        return None
    copy = dict(record)
    copy["refs"] = dict(record.get("refs") or {})
    copy["timeline_models"] = list(record.get("timeline_models") or ())
    copy["unresolved"] = list(record.get("unresolved") or ())
    return copy


def _timeline_at_value(value):
    """一个 at 值按数字比（坏值按 0，与折叠里的 `at or 0` 同口径）。"""
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def _timeline_row_at(row):
    """时间线一行的 at。"""
    return _timeline_at_value(row.get("at"))


# 时间线投影摘要的记忆：值是一个元组 (时间线对象, 参数键, 摘要)，一次 get 拿全
# （与 _object_digest 的记忆同一个写法）——两个状态锁下的线程同时算也不会读到
# 拼起来的值；元组里握着时间线对象，身份不会被回收复用。
_pricing_timeline_memo = {}


def _pricing_timeline_digest(timeline, record):
    """记录里时间线依赖的投影摘要（折叠侧收尾与判定侧共用一份定义）。

    投影 = [[行 at, 该行里 timeline_models 的 {model: pid} 子字典], ...]，
    摘要 = 排序 JSON + sha1：

      * backfilled 为真（有行靠回填解析）：整条时间线——那批行的解析扫过全部
        行，任何追加都可能改变结果（保守）；
      * 否则：at <= timeline_at 的行，外加最早那一行（二分夹逼 index=0 时会读
        它）；每行只取 timeline_models 里的子字典，无关模型的价变动不进依赖
        ——这正是 T15 要收掉的重折。行本身（连 at）都要进投影：at 的增减会
        挪动二分的结果；
      * unresolved（整条时间线都没提到过的模型）：整条时间线里「提到它的行」
        ——那批行的解析扫过全部行，之后任何时刻补一行都会改变结果。

    timeline_models 为空（没有无引用非零成本的行）时没有依赖，返回 None。
    按（时间线对象, 记录参数）记忆：一次刷新里 5 条状态共用同一份时间线，
    记录没变时只算一次。
    """
    models = record.get("timeline_models") or ()
    if not models:
        return None
    key = (record.get("timeline_at"), tuple(models),
           tuple(record.get("unresolved") or ()),
           bool(record.get("backfilled")))
    hit = _pricing_timeline_memo.get(id(timeline))
    if hit is not None and hit[0] is timeline and hit[1] == key:
        return hit[2]
    wanted = set(models)
    rows = list(timeline or ())
    parts = []
    if key[3]:
        scope = rows
    else:
        bound = _timeline_at_value(record.get("timeline_at"))
        scope = [row for row in rows if _timeline_row_at(row) <= bound]
        if rows and not scope:
            scope = rows[:1]
    parts.append([[_timeline_row_at(row),
                   {m: pid for m, pid in (row.get("policies") or {}).items()
                    if m in wanted}]
                  for row in scope])
    for model in sorted(record.get("unresolved") or ()):
        parts.append([[_timeline_row_at(row), (row.get("policies") or {})[model]]
                      for row in rows
                      if model in (row.get("policies") or {})])
    try:
        payload = json.dumps(parts, sort_keys=True, ensure_ascii=False,
                             separators=(",", ":")).encode("utf-8")
    except Exception:
        return None
    digest = hashlib.sha1(payload).hexdigest()
    if len(_pricing_timeline_memo) > 8:
        # 正常进程里时间线只在价格变化时换代；这里只是给长期运行兜底
        # （与 _object_digest 的记忆同一条规则）。
        _pricing_timeline_memo.clear()
    _pricing_timeline_memo[id(timeline)] = (timeline, key, digest)
    return digest


def _priced_note(priced, row, cost, pricing):
    """把一行「用到的价」记进状态指纹记录（折叠侧；见上面一节）。

    refs 收下解析出来的 pid 的内容摘要（显式引用 + 时间线解析）；无显式引用
    且成本非零的行另记时间线依赖。总开关关掉（cost 带 disabled）或价表读不
    出来（pricing 为 None）时不记——前者折叠没读任何价、后者记不完整，两种
    都由判定侧按「记录对不上」兜底作废。

    热路径开销：每行几次 dict 查找 + 按对象身份记忆的摘要（第一次之后 O(1)），
    只有无引用非零成本的行（旧前缀，一次性折过）才会碰列表；那批行里落到
    内置快照的（模型从未进过时间线）再问一次 policy_ref_at 把依赖记准。
    """
    if priced is None or pricing is None or not cost:
        return
    if cost.get("disabled"):
        return
    source = cost.get("source")
    if source and source != "builtin":
        if source not in priced["refs"]:
            # 一次扫描里价表不变，同一个 pid 的摘要只算一次：命中记忆之后
            # 这一步就只剩一次成员检查（真机 4.7 万行里只有几个不同 pid）。
            policy = (pricing[1] or {}).get(source)
            priced["refs"][source] = (_object_digest(policy)
                                      if policy is not None else None)
    else:
        # 内置快照：文档摘要一变，这些行的价就变（同一份文档只算一次摘要）。
        doc = pricing[3]
        if doc is not None and priced["builtin"] is None:
            priced["builtin"] = _object_digest(doc)
    pid = row.get("cost_policy")
    if pid:
        # 显式引用：价由 refs 看住，与时间线无关；引用的策略当前查不到时
        # 按「不存在」记，它一旦出现（内容当然也就变了）必须作废重折。
        if pid not in priced["refs"]:
            priced["refs"][pid] = None
        return
    if not cost.get("known") or not cost.get("cny"):
        return                       # 零成本：数值上不依赖价表（refs 已记）
    at = row.get("at") or 0
    bound = priced["timeline_at"]
    if bound is None or at > bound:
        priced["timeline_at"] = at
    model = row.get("model")
    if model is not None and model not in priced["timeline_models"]:
        priced["timeline_models"].append(model)
        priced["timeline_models"].sort()
    backfilled = bool(cost.get("backfilled"))
    if source == "builtin":
        # 解析结果没进 cost（pid 为空、或它的策略查不到，两条都落到内置快照）：
        # 再问一次时间线，把「解析出没出 pid / 是不是回填」记准。
        resolved, scan_backfilled = wb_pricing.policy_ref_at(at, model)
        if resolved is None:
            # 整条时间线都没提到这个模型：解析扫过全部行，之后任何时刻补一
            # 行都会改变结果，所以它按「整条时间线里提到它的行」记账。
            if model is not None and model not in priced["unresolved"]:
                priced["unresolved"].append(model)
                priced["unresolved"].sort()
        else:
            backfilled = backfilled or bool(scan_backfilled)
            if resolved not in priced["refs"]:
                priced["refs"][resolved] = None
    if backfilled:
        priced["backfilled"] = True


def _pricing_record_close(priced, pricing):
    """一次成功的折叠收尾：把时间线投影摘要刷成当前参数下的值。

    记录在折叠过程中增量更新（新行进来只加不改），摘要必须跟着参数走——
    下一次判定拿它跟「当前时间线按同一组参数算出来的投影」比。按时间线对象
    与参数记忆，参数没动时这里只是一次查表。
    """
    if priced is None or pricing is None:
        return
    priced["timeline"] = _pricing_timeline_digest(pricing[2], priced)


def _pricing_used_unchanged(record, pricing):
    """折叠行真正用到的价，与当前价表一致？（T15 的状态判废判定）

    只读记录与当前价表、不碰日志：refs 逐个 pid 比对内容摘要（查不到 = 变化；
    折叠时查不到的 pid 出现也是变化），时间线依赖重算同一份投影摘要，builtin
    比对文档摘要。任何一项对不上就返回 False（状态作废、重折）。

    pricing 为 None（价表读不出来）时不能确认，一律 False；总开关关着时折叠
    没读任何价，只有「折叠时也没用价」（record 为 None）才算一致。
    """
    if pricing is None:
        return False
    if not pricing[0]:
        return record is None
    if not _pricing_record_ok(record):
        return False
    policies = pricing[1]
    for pid, digest in record["refs"].items():
        policy = policies.get(pid) if policies else None
        current = _object_digest(policy) if policy is not None else None
        if current != digest:
            return False
    if record["timeline"] != _pricing_timeline_digest(pricing[2], record):
        return False
    if record["builtin"] is not None:
        if _object_digest(pricing[3]) != record["builtin"]:
            return False
    return True


def _realm_inputs():
    """What row_realm() consults for a row that carries no realm field.

    Rows written before the field existed - an error row with no account - are
    attributed to the account that served them, or to the model's home realm.
    Importing or dropping an account can therefore move such a row from one
    exit to the other, and a cached aggregate must not outlive that.

    这两样（当前出口 + uid→realm 归属）从**磁盘**同步读，而不是从活账号池的
    内存映射读。池是启动时从同一份磁盘加载的快照，但「它什么时候加载完」会让
    同一个进程在不同时刻算出不同的指纹：重启后第一刻的进程算出的是启动期那
    一份，于是刚写下的 checkpoint 反而被判无效——而那正是它存在的意义（重启
    后免冷扫）。从磁盘读，任何时刻、任何进程算出来都一样。

    池仍然要查，但只作为「这次折叠能不能被冷进程复现」的判据（见
    _realm_fold_reproducible），不再进指纹的值。
    """
    disk = _account_realm_map()
    if disk is None:
        return None
    return (CURRENT_REALM, disk)


_realm_disk_cache = {"key": None, "map": None}
_realm_disk_lock = threading.Lock()


def _account_files_key(directory):
    """账号目录的便宜指纹：文件名 + (size, mtime)；读不到返回 None。"""
    try:
        names = sorted(n for n in os.listdir(directory) if n.endswith(".json"))
    except OSError:
        return None
    entries = []
    for name in names:
        try:
            info = os.stat(os.path.join(directory, name))
        except OSError:
            return None
        entries.append((name, info.st_size, info.st_mtime_ns))
    return (directory, tuple(entries))


def _account_realm_map(directory=None):
    """账号文件里的 uid→realm 映射（排序后的元组）；读不到返回 None。

    用账号池自己的加载器推导，而不是在这里复制一份 uid/realm 解析逻辑：复制
    出来的那份迟早会和 Account 里的逻辑漂移，而漂移意味着指纹描述的东西和
    折叠读的东西不再是同一个。整份读一次的开销由缓存键挡着——目录下每个
    *.json 的名字/size/mtime 都没变就直接用上次的结果。
    """
    directory = ACCOUNTS_DIR if directory is None else directory
    key = _account_files_key(directory)
    if key is None:
        return None
    with _realm_disk_lock:
        cached = _realm_disk_cache
        if cached["key"] == key:
            return cached["map"]
    try:
        pool = wb_accounts.AccountPool(directory, log=None)
        pool.load()
        value = tuple(sorted((a.uid, a.realm) for a in pool.accounts if a.uid))
    except Exception as exc:
        log("account realm map unreadable: %s" % exc)
        return None
    with _realm_disk_lock:
        _realm_disk_cache.update({"key": key, "map": value})
    return value


def _realm_fold_reproducible(disk):
    """这次折叠用的归属，冷启动的进程能不能按同一份磁盘原样复现。

    折叠读的是活账号池，指纹读的是磁盘。两者一致（正常情况：池就是启动时从
    这份磁盘加载的）时没有疑问；磁盘被改过、池还没重载时两者会短暂不一致，
    那种折叠写下的 checkpoint 会被冷进程按新归属当成有效，所以不写。池整个
    不存在时折叠根本不查账号映射——只有磁盘上也没有账号，两边才一致。
    """
    if POOL is None:
        return not disk
    try:
        pool = tuple(sorted((a.uid, a.realm) for a in POOL.accounts))
    except Exception as exc:
        log("realm inputs unreadable: %s" % exc)
        return False
    return pool == disk


def _scan_usage_from(offset, fold, stop_at=None, skip=None, skip_raw=None, path=None):
    """Fold every row past `offset` into `fold(row)`; returns (offset, error).

    The offset handed back is the end of the last row the pass got through.
    Rows are appended whole, so a line without its trailing newline is left
    for the next pass rather than half-parsed - the rule _scan_daily_usage()
    follows.

    `skip(line)` drops a line before the parse; the windowed readers pass
    _line_outside_window() here, so a line the range excludes costs a substring
    search instead of a json.loads. `skip_raw(raw)` is the same test on the
    undecoded line, run before the decode: the windowed readers pass
    _line_outside_window_bytes(), which only answers True for a line the text
    check above would drop as well, so a line it drops skips the decode too
    and no other line reads differently. Both are optional and either may be
    given alone.

    `stop_at` caps the read at the offset an all-time fold has already covered,
    so a windowed payload built on top of that fold describes exactly the same
    bytes as its all-time half.

    `path` defaults to the usage log; the remaining-usage fold folds its own
    event journal through the same pass rules.

    The log is read in binary and the offset advances by the length of each
    line: a tell() per line costs more than the parse it would be tracking,
    and an offset only has to be a byte count. Every line that reaches the
    text checks is decoded to text before it is looked at, which is the order
    the text-mode readers read in and the one the text-mode pre-filter
    expects; a line `skip_raw` drops never gets that far.

    `error` is the exception that stopped the read, or None. A row the fold
    cannot swallow ends the pass exactly where it used to - the readers have
    always kept what they had folded so far - and the caller reports it with
    its own label. The offset handed back with an error is the end of that
    row: the fold may have half applied it before it raised, so the caller
    starts its own fold over instead of resuming after it, while a windowed
    pass built on the state still sees the row - and stops there - so the
    two halves keep describing the same bytes.
    """
    if stop_at is not None and offset >= stop_at:
        return offset, None
    try:
        with open(path or USAGE_LOG, "rb") as fh:
            fh.seek(offset)
            while True:
                if stop_at is not None and offset >= stop_at:
                    break
                raw = fh.readline()
                if not raw:
                    break
                if not raw.endswith(b"\n"):
                    break
                end = offset + len(raw)
                if stop_at is not None and end > stop_at:
                    # The line straddles the covered bytes, so the all-time
                    # fold never saw it either: leave it out of this pass too.
                    break
                # The bytes-level pre-filter runs first: a line it can read
                # exactly and place outside the window never becomes text at
                # all. It only answers for lines the text filter below would
                # drop too (see _line_outside_window_bytes), so the rows this
                # pass folds are exactly the rows it folded before.
                if skip_raw is not None and skip_raw(raw):
                    offset = end
                    continue
                try:
                    # Decoded before anything else, exactly as the text-mode
                    # readers do: a line no reader can decode stops them where
                    # it stands, so it stops this pass too rather than letting
                    # it count rows the full scan never reached. Stripping the
                    # text (not the bytes) keeps the two agreeing on what
                    # counts as an empty line.
                    line = raw.decode("utf-8").strip()
                except UnicodeDecodeError as exc:
                    return end, exc
                if not line:
                    offset = end
                    continue
                if skip is not None and skip(line):
                    offset = end
                    continue
                try:
                    row = json.loads(line)
                except Exception:
                    offset = end
                    continue
                try:
                    fold(row)
                except Exception as exc:
                    return end, exc
                offset = end
    except FileNotFoundError:
        pass
    except Exception as exc:
        return offset, exc
    return offset, None


def _copy_usage_bucket(bucket):
    """Copy one by_model* bucket, its accounts dict included."""
    out = dict(bucket)
    out["accounts"] = dict(bucket["accounts"])
    return out


def _copy_usage_snapshot(snap):
    """A snapshot copy the caller may decorate and hand out.

    The fold is cached and shared, so what leaves this module is a copy - the
    same reason _daily_state_copy() exists for the daily counters. Only the
    three per-model views nest; every other value is a scalar.
    """
    out = dict(snap)
    out["cost_missing"] = dict(snap["cost_missing"])
    out["by_model"] = {m: _copy_usage_bucket(b) for m, b in snap["by_model"].items()}
    out["by_model_realm"] = {m: {rr: _copy_usage_bucket(b) for rr, b in realms.items()}
                             for m, realms in snap["by_model_realm"].items()}
    out["by_model_acct"] = {
        m: {rr: {acct: _copy_usage_bucket(b) for acct, b in accts.items()}
            for rr, accts in realms.items()}
        for m, realms in snap["by_model_acct"].items()}
    return out


# ---------------------------------------------------------------------------
# 行级结果记忆：把「同一行算出来的东西」跨视图复用
#
# 一次冷启动里同一份日志被三个视图各扫一遍（snapshot / analytics / series），
# 每一行都要重新算一次本地日键（_local_day_key，strftime 真机 43.4µs/行），
# snapshot 与 analytics 还要各算一次 cost_for_row（真机 72.4µs/行）。这两样都是
# 行的纯函数——日键只看 at（进程内时区固定），定价只看行的定价字段加价表本身
# ——所以第二个视图不必再算：记下结果，直接取。
#
# 记的是结果，不是状态：五个聚合仍各持各的折叠状态（D1/D2/D3 不受影响），这里
# 没有任何一份聚合能被另一个视图借走，只有「这行折进哪天」和「这行值多少钱」
# 两个标量。位置/指纹校验、realm 归属、日桶语义都照旧。
#
# 键不能是行对象 id()：三次扫描各自 json.loads 出各自的行对象，旧对象释放后 id
# 会被复用，按 id 记忆会静默拿到别的行的值。日键按 at 记（纯函数，跨扫描安全）；
# 定价按它真正读的那几个字段记（model / cost_policy / at / 三个 token 数），外加
# 价表代次——代次就是扫描开头取到的那份 _pricing_inputs() 元组，价表换代时元组
# 里的对象不再是同一个，旧条目自然不命中（判定与 _pricing_unchanged 同一条规则）。
# 同一代次内的价表内容按定义不变，所以命中值必然等于现算值。
#
# 两张表都必须有界（路由器上不能拿内存换速度换到失控）：上限按条目数，满了以后
# 只收「不比表里最旧那条更旧」的行。扫描总是从头往尾走，表里留住的因此是最近的
# 一批行，第二个视图走到尾部照样命中，被拒的只是已经过去的头段。单条体积实测
# 日键 60B、定价 402B（键里留着行的模型名/时间戳/三个 token 数），所以两张表的
# 上限分开给，都可用环境变量调（WB_ROW_MEMO_MAX / WB_ROW_COST_MEMO_MAX，
# 0 = 关掉这张表）。
#
# 不落盘：扫描期的临时记忆，重启后重算一遍即可（清单 §1.5）。
# ---------------------------------------------------------------------------

# 条目上限（条数，不是字节）。默认按「典型日志装得下大半、内存留在个位数 MB」
# 给：日键表 6.4 万条（≤3.8MB）；定价表 16384 条（≤6.3MB）。日志更大、内存又
# 宽裕的部署可以把 _ROW_COST_MEMO_MAX 调到行数以上，拿满跨视图复用（路由器上
# 整份日志约 4 万行，满覆盖约 15MB）；内存紧的调到 0 就是关掉那张表。
#
# 两个值在导入时从环境变量读一次（WB_ROW_MEMO_MAX / WB_ROW_COST_MEMO_MAX，
# 0 = 关掉那张表）：这是给部署定上限的开关，不是逐次可切的现场开关——而
# os.environ.get 走「键不存在」那条路要 ~2µs，放在每行都走的插入路径上会把
# 省下来的时间原样还回去。运行期要改直接改模块属性（测试就是这么做的）。
_ROW_MEMO_MAX = 65536
_ROW_COST_MEMO_MAX = 16384


def _row_memo_env_cap(name, default):
    """导入时读一次上限；读不到或解析不出来就按默认。"""
    raw = os.environ.get(name)
    if raw is None:
        return default
    try:
        return max(0, int(raw))
    except (TypeError, ValueError):
        return default


_ROW_MEMO_MAX = _row_memo_env_cap("WB_ROW_MEMO_MAX", _ROW_MEMO_MAX)
_ROW_COST_MEMO_MAX = _row_memo_env_cap("WB_ROW_COST_MEMO_MAX", _ROW_COST_MEMO_MAX)

_row_day_memo = {}          # at -> "YYYY-MM-DD"
_row_cost_memo = {}         # (model, cost_policy, at, pt, ct, comp) -> (价表代次, 定价结果)
_row_cost_gen = None        # 定价表当前代次（_pricing_inputs() 的元组）
_row_memo_tz = time.tzname  # 日键表绑在哪个时区上（见 _row_memo_tz_changed）


def _row_memo_at(value):
    """记忆表的「新旧」比较键：只有数字能比大小，别的（None / 坏行）一律按 0。

    比大小只决定「这条记不记」，不决定结果——比不动就不比，照记。
    """
    return value if isinstance(value, (int, float)) else 0


def _row_memo_tz_changed():
    """进程时区被 tzset() 换过 → 日键表整体作废（定价表不受影响）。

    日键是本地时区的函数，跨时区复用旧值会算错天。CPython 的 time.tzset()
    每次都会重建 time.tzname 这个元组对象，所以「对象换了」就是「时区换过」；
    没换时这里只是一次身份比较。生产里进程时区不会变，这一句就是白送的。
    """
    global _row_memo_tz
    now = time.tzname
    if _row_memo_tz is now:
        return False
    _row_memo_tz = now
    _row_day_memo.clear()
    return True


def _cost_entry(pricing, cost):
    """cost_for_row(details=False) 的返回值 -> 记忆条目（定长元组）。"""
    return (pricing, cost["known"], cost["cny"], cost.get("band"),
            cost.get("rates"), cost.get("source"), cost.get("source_at"),
            cost.get("backfilled"), bool(cost.get("disabled")))


def _cost_from_memo(entry):
    """记忆条目 -> 与 cost_for_row(details=False) 逐键同序的 dict。

    两条分支的形状与原函数一一对应：总开关关掉那一支不带 rates、多一个
    disabled；有价/没价那一支带 rates。键序照抄原函数的插入顺序，别处
    按字节对拍时看到的是一样的 dict。
    """
    cost = {"cny": entry[2], "known": entry[1], "band": entry[3]}
    if entry[8]:
        cost["source"] = "builtin"
        cost["source_at"] = None
        cost["backfilled"] = False
        cost["disabled"] = True
        return cost
    cost["rates"] = entry[4]
    cost["source"] = entry[5]
    cost["source_at"] = entry[6]
    cost["backfilled"] = entry[7]
    return cost


def _row_cost(row, pricing, pricing_on):
    """一行的估算成本，跨视图复用（见「行级结果记忆」一节）。

    pricing 是本次扫描开头取到的 _pricing_inputs() 元组：命中要求条目与它
    同代。pricing 为 None（价表读不出来）时不进表，照旧每行现算——那条路径
    本来就不该被记忆；空行（falsy row）同理，它走的是原函数的短路分支。

    返回的形状与 wb_pricing.cost_for_row(..., details=False) 逐键相同；命中
    时按条目重建一份，调用方拿到的是自己的副本，与现算路径一致。
    """
    if pricing is None or not row:
        return wb_pricing.cost_for_row(row, details=False, enabled=pricing_on)
    key = (row.get("model"), row.get("cost_policy"), row.get("at"),
           row.get("prompt_tokens"), row.get("cached_tokens"),
           row.get("completion_tokens"))
    memo = _row_cost_memo
    try:
        entry = memo.get(key)
    except TypeError:
        # 不可哈希的字段（坏行）：原函数到 (doc.get("models") or {}).get(model)
        # 那一步也会抛 TypeError，这里不抢先，交给它
        return wb_pricing.cost_for_row(row, details=False, enabled=pricing_on)
    if entry is not None and _pricing_unchanged(entry[0], pricing):
        return _cost_from_memo(entry)
    cost = wb_pricing.cost_for_row(row, details=False, enabled=pricing_on)
    _row_cost_remember(key, pricing, cost)
    return cost


def _row_cost_remember(key, pricing, cost):
    """把一行的定价结果记进有界表；价表换代时整表作废。

    换代只可能来自价表文件被改写（策略表/时间线/价目表任一新对象），发生时
    旧条目按定义全部失效，直接清掉，别让它们白占位置。判定用的是
    _pricing_unchanged——与状态指纹同一条规则，两处不会漂移。
    """
    global _row_cost_gen
    memo = _row_cost_memo
    if _row_cost_gen is None or not _pricing_unchanged(_row_cost_gen, pricing):
        memo.clear()
        _row_cost_gen = pricing
    cap = _ROW_COST_MEMO_MAX
    if cap <= 0:
        return
    if len(memo) >= cap:
        oldest = next(iter(memo))
        if _row_memo_at(key[2]) < _row_memo_at(oldest[2]):
            # 比表里最旧的还旧：记了也会立刻被顶掉，不如不记
            return
        memo.pop(oldest, None)
    memo[key] = _cost_entry(pricing, cost)


def _fold_usage_snapshot(row, snap, r, since, until, pricing_on=None, also=None,
                         pricing=None, priced=None):
    """Fold one parsed row into a usage snapshot.

    One definition for both callers: the all-time fold (fed only the rows past
    its offset) and the windowed one (fed the whole file, out-of-window lines
    dropped before the parse). The realm and window tests come first, so every
    total below - requests, tokens, per-model and per-account breakdowns -
    describes the same slice of the log.

    pricing_on 是本次扫描开头问过一次的总开关：聚合只读 known/cny（_fold_cost
    再读 disabled），悬停明细构造出来就被丢掉，所以整扫不建明细、开关也不逐行
    重问。代价是设置改动从下一次扫描起生效；逐行展示路径不受影响，仍然每行
    查、改设置当场可见。

    `also` 是同一行还要折进的其余同形状快照（按日/按小时分桶的窗口半），
    一个元组，空/None 表示没有。过滤、算价与字段提取只做一次，几份各自累加：
    折叠本来就要求每份各记一笔，而 cost_for_row 与 realm 判定是每行最贵的
    那部分，只该付一次。

    `pricing` 是本次扫描开头取到的 _pricing_inputs() 元组（调用方从同一个
    值里取指纹，两处看到的是同一份价表）；传给 _row_cost 当记忆的代次，
    None 表示价表读不出来、不进表。

    `priced` 是本次扫描要填的「用到的价」记录（见「用到的价」状态指纹一节），
    只有推进状态的那条折叠传它；窗口半/冷扫描传 None，一行都不记。
    """
    if r and not row_matches_realm(row, r):
        return
    at = row.get("at") or 0
    if since and at < since:
        return
    if until and at > until:
        return
    outcome = row_outcome(row)
    # Each row is priced against the version that was in force when it
    # happened, so a later price change cannot rewrite yesterday's totals.
    # _row_cost 按行记忆（见「行级结果记忆」）：同一个视图里这一行只算一次，
    # 第二个视图直接取上次的结果；pricing 是本次扫描的价表代次。
    cost = _row_cost(row, pricing, pricing_on)
    if outcome != "client_aborted":
        # 折进去的每一行都把「用到的价」记进状态指纹（中止行只记 errors，
        # 不碰价，所以不记）。记的是这次折叠真正读到的价，价表无关的变动
        # 因此不再判废这条状态（见「用到的价」状态指纹一节）。
        _priced_note(priced, row, cost, pricing)
    model = row.get("model")
    targets = (snap,) if not also else (snap,) + tuple(also)
    if outcome != "completed":
        for tgt in targets:
            tgt["errors"] += 1
            # Credit is money already spent: a request that failed after the
            # upstream had billed for it still consumed credit, so it is summed
            # here exactly like the analytics page sums it. Token totals keep
            # the completed-only rule this page has always used, and a client
            # abort is skipped because its usage block is incomplete.
            if outcome != "client_aborted":
                tgt["credit"] += (row.get("credit") or 0)
                _fold_cost(tgt, cost, model)
        return
    # Read each tracked field once: the row lands in four buckets below and
    # every one of them walks the same fields.
    present = [(k, row[k] or 0) for k in USAGE_FIELDS if k in row]
    m = model or "unknown"
    rr = row_realm(row)
    acct_id = row.get("account")
    acct_key = acct_id or "(unattributed)"
    known = cost["known"]
    cny = cost["cny"] if known else 0.0
    for tgt in targets:
        tgt["requests"] += 1
        for k, value in present:
            tgt[k] += value
        _fold_cost(tgt, cost, model)
        per = tgt["by_model"].setdefault(m, {"requests": 0, "accounts": {}, "cost_cny": 0.0, **{k: 0 for k in USAGE_FIELDS}})
        per_realm = tgt["by_model_realm"].setdefault(m, {}).setdefault(
            rr, {"requests": 0, "accounts": {}, "cost_cny": 0.0, **{k: 0 for k in USAGE_FIELDS}})
        per_acct = (tgt["by_model_acct"].setdefault(m, {})
                    .setdefault(rr, {})
                    .setdefault(acct_key, {"requests": 0, "accounts": {},
                                           "cost_cny": 0.0,
                                           **{k: 0 for k in USAGE_FIELDS}}))
        for bucket in (per, per_realm, per_acct):
            bucket["requests"] += 1
            for k, value in present:
                bucket[k] += value
            if known:
                bucket["cost_cny"] += cny
            if acct_id:
                accounts = bucket["accounts"]
                accounts[acct_id] = accounts.get(acct_id, 0) + 1


_usage_snap_state = {}
_usage_snap_state_lock = threading.Lock()


def _usage_snap_state_advance(r, pricing_on):
    """（不折叠）取/建状态、采纳 checkpoint、判废归零、记身份指纹。

    调用方必须已持有 _usage_snap_state_lock。返回本次扫描取到的价表代次
    （_pricing_inputs 的元组）：调用方把它传给折叠当行级记忆的代次，与记进
    状态指纹的是同一份，两处不会看到两份价表。

    状态指纹是「折叠行真正用到的价」记录（T15，见「用到的价」状态指纹一节）：
    折叠时逐行更新（state["pricing"]），扫描成功后把时间线投影摘要刷成当前
    参数下的值（_pricing_record_close）——下一次判废拿它跟「当前价表按同一组
    参数算出来的投影」比（_pricing_used_unchanged）。

    状态是否「冷」（这次要付一次全量重折）看折过的位置：state["offset"] > 0
    表示有可续折的数据；0 表示折叠要从文件头开始——状态刚建、刚被价表 /
    realm / 日志判废、或 checkpoint 采纳失败。判定只看已折位置与三份指纹，
    一行都不折，所以窗口路径拿它分流是纯加速：重折之后两边给出的字节完全
    一致（见 _usage_snap_state_cold）。

    采纳/判废只有这一段定义：_usage_snap_state_ready（all-time 折叠）与
    窗口路径的冷热判定共用它，两边的规则因此不会漂移。
    """
    state = _usage_snap_state.get(r)
    if state is None:
        state = _usage_snap_state[r] = {"snap": None, "offset": 0, "key": None,
                                        "pricing": None, "realm": None,
                                        "tail": b"", "days": {}, "days_floor": None,
                                        "hours_floor_at": None,
                                        "hits": [], "hits_overflow": False}
    pricing = _pricing_inputs(pricing_on)
    realm = _realm_inputs()
    log_key = _usage_log_key()
    # 重折时状态指纹从这份空记录重新长起；总开关关着 / 价表读不出来时
    # 记 None（折叠没读任何价，判定侧只认「记录也是 None」这一种一致）。
    fresh = _pricing_record_new() if (pricing is not None
                                      and pricing[0]) else None
    if state["snap"] is None:
        # 全新状态（进程刚起来）：先问 checkpoint，问不到才从零折起。
        if not _usage_cache_adopt_snapshot(r, state, log_key, pricing, realm):
            state.update({"snap": _empty_stats(), "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "hours_floor_at": None,
                          "hits": [], "hits_overflow": False, "pricing": fresh})
    elif (not _pricing_used_unchanged(state["pricing"], pricing)
            or state["realm"] != realm
            or not _log_resume_ok(state, log_key)):
        state.update({"snap": _empty_stats(), "offset": 0, "tail": b"",
                      "days": {}, "days_floor": None, "hours_floor_at": None,
                      "hits": [], "hits_overflow": False, "pricing": fresh})
    state.update({"realm": realm, "key": log_key})
    return pricing


def _usage_snap_state_cold(r, pricing_on):
    """窗口请求现在会不会付一次全量重折？True = 会（窗口改走冷扫描）。

    调用方必须已持有 _usage_snap_state_lock。判定就是 all-time 折叠的前半段
    （_usage_snap_state_advance），不折任何行；为真时窗口半改走按日/按小时
    分组的冷扫描（_scan_usage_snapshot_window_days / _hours，~1s 量级），
    重折留给 all-time 视图（_usage_alltime_snapshot）与预热器。

    这条路不写折叠结果，也不改「重折该不该发生」：状态里 offset 仍是 0，
    下一个 all-time 请求照样从文件头折到文件尾——窗口请求不会把状态冻在
    冷态。重折发生之后，走不走这条路给出的字节完全相同（冷扫描与日桶/小时
    桶之和本就是同一批行、同一个求和口径，见两个冷路径函数的 docstring）。
    """
    _usage_snap_state_advance(r, pricing_on)
    return _usage_snap_state[r]["offset"] <= 0


def _usage_snap_state_ready(r, pricing_on):
    """把 `r` 的 all-time 折叠推进到文件尾；返回 (state, offset, error)。

    调用方必须已持有 _usage_snap_state_lock。日桶与 all-time 折叠共用这一次
    扫描（见 _fold_usage_snapshot 的 also），所以窗口半与 all-time 半描述的
    永远是同一段字节——这正是旧实现用 stop_at 想保证的事。

    采纳 checkpoint 与判废归零在 _usage_snap_state_advance 里（窗口路径的
    冷热判定也用那一段，见 _usage_snap_state_cold），这里只管折到文件尾。
    """
    pricing = _usage_snap_state_advance(r, pricing_on)
    state = _usage_snap_state[r]
    priced = state["pricing"]

    def fold(row):
        # 一行折三处：all-time 半 + 它所属那天的窗口半 + 那个小时的小时桶。
        # 过滤与算价在 _fold_usage_snapshot 里只做一次。
        day = _usage_day_bucket(state, row, r, _day_snapshot_bucket)
        hour = None if day is None else _usage_hour_bucket(state, row, day)
        extras = () if day is None else ((day,) if hour is None else (day, hour))
        _fold_usage_snapshot(row, state["snap"], r, None, None, pricing_on,
                             also=extras, pricing=pricing, priced=priced)

    offset, error = _scan_usage_from(state["offset"], fold)
    if error is None:
        state["offset"] = offset
        state["tail"] = _log_tail_signature(offset)
        _pricing_record_close(priced, pricing)
    return state, offset, error


def _usage_alltime_snapshot(r, pricing_on=None):
    """A private copy of the cached all-time fold for realm filter `r`
    (None = every realm).

    One state per realm: the filter decides which rows the fold ever sees, so
    `intl` and `cn` are two folds and neither may inherit the other's rows.
    The state holds the raw fold only - `started`, `since`, `accounts_map` and
    the rest are re-derived on every call, exactly as the full scan re-derived
    them.
    """
    with _usage_snap_state_lock:
        state, offset, error = _usage_snap_state_ready(r, pricing_on)
        if error is None:
            # Copied while the lock is held: the caller decorates what it
            # gets, and a concurrent refresh must never be seen half-applied.
            snap = _copy_usage_snapshot(state["snap"])
        else:
            # A row the fold could not swallow may have half applied itself,
            # so the carried-over totals cannot be resumed from there: this
            # call answers what was folded - the full scan reports the same
            # partial totals - and the next refresh starts over, which is
            # what the full scan does on every call.
            log(f"usage snapshot read failed: {error}")
            snap = state["snap"]
            state.update({"snap": None, "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "hours_floor_at": None,
                          "hits": [], "hits_overflow": False})
    # 落盘决定在锁外做（见 checkpoint 一节）：写盘要逐把读三份状态，在状态
    # 锁里再取别的状态锁会构成锁序环。折叠失败的那次不写——它的 offset 已经
    # 归零，落盘函数也会跳过。
    if error is None:
        _usage_cache_maybe_save("snapshot", offset)
    return snap


def _scan_usage_snapshot_window(r, since, until, pricing_on=None):
    """One pass over the log, keeping only the rows inside the window.

    Nothing is carried over here - a window moves with the clock - but the
    pre-filter drops the parse for every line it can prove is outside it, and
    its bytes-level half drops the decode of those lines as well, which is
    what keeps a cold day or week view cheap on a multi-day log.

    The bounds reach the pre-filter through `or None` because that is the
    test the fold itself applies (`if since and at < since`): a bound of 0
    is a bound the fold ignores, and the pre-filter must not read it as
    "keep nothing after the epoch" and drop rows the fold would have
    counted.

    价表在这里取一次（与 all-time 折叠同一条口径：一次扫描一份价表快照），
    传给折叠当行级记忆的代次；这次扫描里的行因此可以和别的视图共享定价结果。
    """
    pricing = _pricing_inputs(pricing_on)
    snap = _empty_stats()
    _, error = _scan_usage_from(
        0,
        lambda row: _fold_usage_snapshot(row, snap, r, since, until, pricing_on,
                                         pricing=pricing),
        skip=lambda line: _line_outside_window(line, since or None,
                                                 until or None),
        skip_raw=lambda raw: _line_outside_window_bytes(raw, since or None,
                                                        until or None))
    if error is not None:
        log(f"usage snapshot read failed: {error}")
    return snap


def _usage_snapshot_window_from_days(r, since, until, pricing_on):
    """窗口半快照 = 窗口内的日桶之和；日桶不可用时返回 None。

    先把 all-time 折叠推进到文件尾：日桶与它同一次扫描推进，窗口半与
    all-time 半因此描述同一段字节。

    until 开放（None）时窗口从起点一路到文件尾；until 给出时它必须是本地
    午夜（调用方已按格点判过，这里再校验一次），窗口就是「起点那天起、
    until 那天之前」的日桶之和。桶按 at 严格切分（恰好落在 until 上的行归
    until 那天那一桶，见 _usage_hour_bucket），桶切不出那一刻的行，所以
    那一刻有整秒行时整条路退回 None（见 _closed_window_edge_ok），调用方
    接着整段扫描，结果不变。

    状态冷（从没折过/刚被价表、realm、日志判废/采纳失败）时直接返回 None：
    那条路要先付一次全量重折，窗口请求不背它——调用方退回按日分组的冷扫描
    （同一批行、同一个求和口径，逐字节相同），重折留给 all-time 视图与
    预热器（见 _usage_snap_state_cold）。
    """
    with _usage_snap_state_lock:
        if _usage_snap_state_cold(r, pricing_on):
            return None
        state, offset, error = _usage_snap_state_ready(r, pricing_on)
        if error is not None:
            # 折叠失败时状态已经归零，日桶跟着清空：这一次退回整段扫描，
            # 与「没有缓存」时的行为一致。
            log(f"usage snapshot read failed: {error}")
            state.update({"snap": None, "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "hours_floor_at": None,
                          "hits": [], "hits_overflow": False})
            return None
        day_key = _window_day_key_safe(since)
        if day_key is None or not _days_cover(state, day_key):
            return None
        until_key = None
        if until is not None:
            until_key = _window_day_key_safe(until)
            if until_key is None or not _closed_window_edge_ok(state, until):
                return None
        snap = _empty_stats()
        days = state["days"]
        for key in sorted(days):
            if key < day_key:
                continue
            if until_key is not None and key >= until_key:
                break
            _add_usage_snapshot(snap, days[key])
    _usage_cache_maybe_save("snapshot", offset)
    return snap


def _usage_snapshot_window_from_hours(r, since, until, pricing_on):
    """窗口半快照 = 整点窗口内的小时桶之和；小时层不可用时返回 None。

    与日桶版同一条规矩：先把 all-time 折叠推进到文件尾（小时桶与它同一次
    扫描推进），再从存量桶求和。成立条件缺一不可，任何一条不成立就返回
    None（调用方退回按小时分组的冷路径）：

      * 窗口起点是本地整点（见 _hour_window_key）；
      * days_floor 覆盖窗口起点——小时桶挂在日桶上，剪枝跟着日桶走；
      * 被选中的每个日桶都带小时表。某天的小时格点不成立时整张表被摘掉
        （见 _usage_hour_bucket），这时整个窗口退回扫描，不能只跳过那一天；
      * 每个小时桶的 at 落在「窗口起点 + k*3600」的格点上，且与所属日桶
        对得上（日桶 at + 序号*3600）。这一条挡住夏令时把日边界挪动的
        情形——与 _series_slice 的格点校验同源；
      * until 给出时（闭窗口）它也必须落在同一套小时格点上，且直到那一刻
        没有整秒行（见 _closed_window_edge_ok）：桶按 at 严格切分，窗口
        只取 at < until 的那批桶，恰好落在 until 上的行由边沿账挡回整段
        扫描。

        情形——与 _series_slice 的格点校验同源。

    状态冷（从没折过/刚被价表、realm、日志判废/采纳失败）时直接返回 None：
    那条路要先付一次全量重折，窗口请求不背它——调用方退回按小时分组的冷
    扫描（同一批行、同一个求和口径，逐字节相同），重折留给 all-time 视图
    与预热器（见 _usage_snap_state_cold）。
    """
    key = _hour_window_key(since)
    if key is None:
        return None
    day_key, k0 = key
    until_key = None
    if until is not None:
        until_key = _hour_window_key(until)
        if until_key is None:
            return None
    with _usage_snap_state_lock:
        if _usage_snap_state_cold(r, pricing_on):
            return None
        state, offset, error = _usage_snap_state_ready(r, pricing_on)
        if error is not None:
            log(f"usage snapshot read failed: {error}")
            state.update({"snap": None, "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "hours_floor_at": None,
                          "hits": [], "hits_overflow": False})
            return None
        if not _days_cover(state, day_key):
            return None
        if not _hours_cover(state, since):
            return None
        if until_key is not None and not _closed_window_edge_ok(state, until):
            return None
        lo = int(since)
        snap = _empty_stats()
        days = state["days"]
        for key in sorted(days):
            if key < day_key:
                continue
            if until_key is not None and key > until_key[0]:
                break
            bucket = days[key]
            hours = bucket.get("hours")
            if hours is None:
                return None
            day_at = int(bucket["at"])
            for hour in sorted(hours, key=int):
                if key == day_key and int(hour) < k0:
                    # 起点那天只看起点之后的整点：更早的桶属于窗口外，跳过
                    # 它们不影响正确性（桶按 at 严格切分，见 _usage_hour_bucket）。
                    continue
                sub = hours[hour]
                at = int(sub["at"])
                if at != day_at + int(hour) * 3600:
                    return None
                if (at - lo) % 3600:
                    return None
                if until_key is not None and (key, int(hour)) >= until_key:
                    # 闭窗口：until 那一刻归后一桶（桶按 at 严格切分），
                    # 只取 at < until 的桶；那一刻有整秒行时上面已经退回。
                    break
                _add_usage_snapshot(snap, sub)
    _usage_cache_maybe_save("snapshot", offset)
    return snap


def _usage_snapshot_window(r, since, until, pricing_on=None):
    """窗口半快照：对齐窗口由日桶/小时桶相加，其余保持整段扫描。

    只有「两端都落在同一套格点上」的窗口拆得成存量桶的并集：起点是本地
    午夜、终点（开放或闭）也是本地午夜时用日桶（today / week / month 与
    「到某天零点」都是），两端落在本地整点上时用小时桶（自定义区间的常见
    形态，见 _hour_window_key）。闭窗口的右边界按「at < until 的桶」切
    （见 _closed_window_edge_ok）：那一刻有整秒行时退回整段扫描，结果与
    改动前逐字节一致——闭窗口本来走的就是这条兜底。起点不落在格点上、
    或 until 不落格点的窗口同样保持整段扫描。
    """
    if until is None:
        day_key = _window_day_key_safe(since)
        if day_key is not None and _usage_day_buckets_enabled():
            snap = _usage_snapshot_window_from_days(r, since, None, pricing_on)
            if snap is not None:
                return snap
            # 日桶没盖住窗口起点（还没折过/被剪枝/折叠出错）：冷路径现折一份
            # 按日分组的窗口半，口径与日桶状态一致，两条路径的字节因此相同。
            return _scan_usage_snapshot_window_days(r, since, None, pricing_on)
        if _usage_day_buckets_enabled() and _window_hour_key_safe(since) is not None:
            snap = _usage_snapshot_window_from_hours(r, since, None, pricing_on)
            if snap is not None:
                return snap
            # 小时层没盖住窗口起点（还没折过/被剪枝/那天的小时格点不成立）：
            # 冷路径现折一份按小时分组的窗口半，口径与小时桶状态一致。
            return _scan_usage_snapshot_window_hours(r, since, None, pricing_on)
        return _scan_usage_snapshot_window(r, since, until, pricing_on)
    # 闭窗口：两端都落在格点上才有桶可借（与上面同一套格点校验）。
    if _usage_day_buckets_enabled():
        if (_window_day_key_safe(since) is not None
                and _window_day_key_safe(until) is not None):
            snap = _usage_snapshot_window_from_days(r, since, until, pricing_on)
            if snap is not None:
                return snap
            return _scan_usage_snapshot_window_days(r, since, until, pricing_on)
        if (_window_hour_key_safe(since) is not None
                and _window_hour_key_safe(until) is not None):
            snap = _usage_snapshot_window_from_hours(r, since, until, pricing_on)
            if snap is not None:
                return snap
            return _scan_usage_snapshot_window_hours(r, since, until, pricing_on)
    return _scan_usage_snapshot_window(r, since, until, pricing_on)


def _usage_snapshot_uncached(realm=None, since=None, until=None):
    # None means every realm; usage_snapshot() has already mapped "all"
    # onto it, so the filter below is simply skipped.
    r = realm
    rep = POOL.representative(realm=r) if POOL else current_account()
    # 总开关整扫只问一次，值一路传进折叠与价格指纹：每行重问一次就是每行一次
    # os.stat，2.7 万行上很可观（见 tests/_test_usage_aggregate_fold.py）。
    pricing_on = wb_pricing.pricing_enabled()
    if since is None and until is None:
        # An open window covers every row, and the rows only ever arrive at
        # the end of the log, so the all-time totals are carried over from the
        # last fold and only the rows appended since then are read.
        snap = _usage_alltime_snapshot(r, pricing_on)
    else:
        snap = _usage_snapshot_window(r, since, until, pricing_on)
    snap["started"] = _usage.get("started", time.time())
    snap["since"] = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(snap.get("started", time.time())))
    snap["log_file"] = USAGE_LOG
    snap["realm"] = r or "all"
    # Cost figures are CNY; the panel divides by this rate to show USD
    # without a round trip.
    snap["usd_cny"] = wb_pricing.usd_cny()
    snap["accounts_map"] = {a.uid: {"nickname": a.nickname, "realm": a.realm} for a in POOL.accounts} if POOL else {}
    snap["account"] = {
        "uid": (rep.uid if rep else ""),
        "domain": (rep.domain if rep else ""),
        "issuer": (wb_accounts.jwt_issuer(rep.access_token) if rep else ""),
        "credential_file": (os.path.basename(rep.path) if rep and rep.path else ""),
        "expires_at": (rep.expires_at if rep else 0),
        "accounts": (len(POOL.accounts) if POOL else 0),
        "accounts_ready": (POOL.count_ready() if POOL else 0),
    }
    return snap
def _tail_lines(path, max_lines, chunk=256 * 1024):
    """Return up to the last `max_lines` non-empty lines, oldest first.

    The usage log passes 20MB within a day. Scanning it end to end on every
    dashboard poll was the dominant cost behind slow /usage/* responses, so
    the reader walks the file backwards in `chunk`-sized blocks and splits
    each block exactly once: `buf` carries only the fragment that straddles
    the boundary into the next (older) block, never the blocks already read.

    Lines that hold nothing but whitespace are not rows and do not count
    towards max_lines; a trailing chunk with no newline still counts as a
    line when it holds anything else (record_usage always writes the
    newline, so that case is a write caught mid-flight).

    逐行的活只剩"这一行要不要"：整块的非空行在 C 层用列表推导滤出来、一次
    extend 收进 lines（上限再截断），比逐行 strip()+append 少一层 Python
    循环。字节口径与逐行写法逐字相同（套件里留了参考实现对拍）。
    """
    lines = []
    try:
        with open(path, "rb") as fh:
            fh.seek(0, os.SEEK_END)
            pos = fh.tell()
            buf = b""
            while pos > 0 and len(lines) < max_lines:
                step = min(chunk, pos)
                pos -= step
                fh.seek(pos)
                buf = fh.read(step) + buf
                parts = buf.split(b"\n")
                buf = parts[0]
                got = [raw for raw in parts[1:] if raw and not raw.isspace()]
                if got:
                    lines.extend(reversed(got))
                    if len(lines) > max_lines:
                        # 与逐行写法一样：收满 max_lines 就停，剩下的（更老的）
                        # 行属于更早的块，本来也不会被收进来。
                        del lines[max_lines:]
                        break
            if len(lines) < max_lines and buf.strip():
                lines.append(buf)
    except FileNotFoundError:
        return []
    except Exception as exc:
        log("tail read failed: %s" % exc)
        return []
    lines.reverse()
    return lines


_count_cache = {}
_count_lock = threading.Lock()
# The count only feeds the "N records" label. The poll interval is 5s, so a
# TTL of the same length would miss on nearly every poll; 30s turns a full
# scan per poll into one scan per six polls while the label stays current
# enough for a record total that only ever grows.
_COUNT_TTL = float(os.environ.get("WB_COUNT_TTL", 30))

# 增量计数的状态（offset/tail/n/n_realm），由 _count_state_lock 保护。
# 与 by_account 折叠同一套位置校验（_usage_log_key + _log_offset_holds +
# _log_tail_signature）：日志被截断、被替换或同尺寸重写时从零重数，宁多扫
# 一次也不能给出错的页数。TTL 缓存按 realm 分键，这份状态则与 realm 无关
# ——一次扫描同时维护总数与每个 realm 的计数，任何 realm 的查询都复用同一
# 条进度，不再各自全量扫一遍。
_count_state_lock = threading.Lock()
_count_state = {"offset": 0, "key": None, "tail": b"", "n": 0, "n_realm": {}}


_series_cache = {}
_series_lock = threading.Lock()


def _usage_timeseries_scope(realm, range, since, until, bucket_seconds):
    """(realm, lo, hi, step, 缓存键) —— usage_timeseries() 与它的 ETag 共用。

    lo/hi 是这次真正要扫的窗口（未钉住的边界在这里补成"现在/往前 24 小时"），
    键只收**钉住的**边界，理由见下面 wrapper 里的注释。
    """
    r = realm_scope(realm, CURRENT_REALM)
    lo, hi = range_window(range, since, until)
    # Which bounds the caller pinned, captured before the fallbacks below fill
    # the rest in. Only a pinned bound may enter the cache key as a value:
    # "now" (and the 24h-before-now default lo) is recomputed on every call,
    # so a key holding it could never be reused while the panel polls the same
    # chart, and the cache would buy nothing. The "auto" sentinel keeps the two
    # kinds of bound apart, so a rolling window is never served a pinned
    # window's series.
    pinned_lo, pinned_hi = lo, hi
    if hi is None:
        hi = time.time()
    if lo is None:
        lo = hi - 86400
    span = max(1.0, hi - lo)
    if bucket_seconds:
        step = max(60, int(bucket_seconds))
    elif span <= 6 * 3600:
        step = 60
    elif span <= 14 * 86400:
        step = 3600
    else:
        step = 86400
    # Bounds, not a range flag: this week and this month overlap, so a flag
    # would let one window serve the other's series from the cache. The
    # bucket step joins the key for the same reason: an explicit bucket
    # changes every bucket's width, and the auto-scaled one flips with the
    # span, so two different steps are two different payloads.
    key = (r or "all",
           pinned_lo if pinned_lo is not None else "auto",
           pinned_hi if pinned_hi is not None else "auto",
           step)
    return r, lo, hi, step, key


def usage_timeseries(realm=None, range=None, since=None, until=None,
                     bucket_seconds=None, ttl=None):
    """Bucketed token/credit series for the analytics chart.

    Bucket size auto-scales with the window: minute (<=6h), hour (<=14d),
    day otherwise; an explicit bucket_seconds overrides it. Completed
    requests contribute tokens; every non-client-aborted row contributes
    credit (money already spent). The most recent credited requests ride
    along so the panel can show a credit history without another endpoint.

    Cached wrapper: the Token chart polls this endpoint whenever the metrics
    tab is open, and one uncached call re-reads and re-parses the whole log.
    Same shared TTL as its siblings, and the rebuild runs under the lock so a
    burst of pollers cannot each start their own scan.
    """
    ttl = _STATS_TTL if ttl is None else ttl
    r, lo, hi, step, key = _usage_timeseries_scope(realm, range, since, until,
                                                   bucket_seconds)
    now = time.time()
    with _series_lock:
        hit = _series_cache.get(key)
        if hit is not None and (now - hit[0]) < ttl:
            return hit[1]
        data = _usage_timeseries_uncached(r, lo, hi, step)
        _series_cache[key] = (time.time(), data)
    return data


def usage_timeseries_etag(realm=None, range=None, since=None, until=None,
                          bucket_seconds=None):
    """usage_timeseries() 当前缓存条目的 ETag；没有条目时返回 None。

    配对调用由 _json_cached() 负责。注意键里只有钉住的边界，所以"滚动窗口"这一类
    请求始终落在同一条目上：条目没重建，响应体（含它自己的 until / 桶边界）就逐
    字节不变，这正是可以放心回 304 的原因。
    """
    _r, _lo, _hi, _step, key = _usage_timeseries_scope(realm, range, since, until,
                                                       bucket_seconds)
    with _series_lock:
        hit = _series_cache.get(key)
    if hit is None:
        return None
    return _cache_etag("series", repr(key), hit[0])


# Every row this process writes starts with `{"at": <float>,` - record_usage
# and record_error put the timestamp first and json.dumps keeps that order -
# which is what makes the window pre-filter below possible.
_AT_PREFIX = '{"at": '
# The same prefix as bytes: _line_outside_window_bytes() reads the undecoded
# line, and deriving the bytes from the text keeps the two in step.
_AT_PREFIX_B = _AT_PREFIX.encode("ascii")


def _line_outside_window(line, lo, hi):
    """True when this log line's `at` is provably outside [lo, hi].

    Either bound may be None, which means "unbounded on that side": the
    ranges the dashboard asks for ("today", "this week") only pin a start,
    and an open side must never drop a row.

    The scan uses it to drop out-of-window lines without paying for
    json.loads, which is what made a cold pass over the log expensive. False
    means "parse it as before" - both for lines that match the shape and fall
    inside the window, and for anything the check cannot read exactly:

    * another key order or a hand-edited line: no leading `{"at": `, or no
      comma to end the value;
    * an integer token: json.loads would hand back an int, and int/float
      comparison stays exact where float(token) rounds once the value passes
      2**53 (a float token round-trips through float() exactly, which is why
      those are the only ones accepted);
    * a later `"at"` key: json.loads keeps the last duplicate, so the first
      value read here would be the wrong one. Escaped quotes inside string
      values (\\") cannot produce a false `"at"`, so this check only ever
      costs a parse - and it runs only for lines that would be skipped, so a
      line the scan is going to parse anyway never pays for it.
    """
    if not line.startswith(_AT_PREFIX):
        return False
    end = line.find(",", len(_AT_PREFIX))
    if end < 0:
        return False
    token = line[len(_AT_PREFIX):end]
    if "." not in token and "e" not in token and "E" not in token:
        return False
    try:
        at = float(token)
    except ValueError:
        return False
    if not ((lo is not None and at < lo) or (hi is not None and at > hi)):
        return False
    return line.find('"at"', end) < 0


def _line_outside_window_bytes(raw, lo, hi):
    """_line_outside_window() on the undecoded line: True only when the text
    check would answer True too.

    The scan reads bytes and only then decodes, so the same test can run
    before the decode and a line it drops never becomes text at all - which
    is the whole point, since a narrow window leaves most of a multi-day log
    outside it. The rules are the text version's, read off the bytes: same
    prefix, same first comma, same float-token requirement, same window
    comparison, same "no later `"at"` key" test (see that docstring for what
    each one is protecting).

    One extra condition: the line has to be ASCII from end to end. Two things
    ride on it. It makes the byte read exact - every character the text check
    would see is the byte the check saw - and it is what keeps a line no
    reader can decode out of the skip path. On the text path that line stops
    the pass with UnicodeDecodeError before any filter runs (see
    _scan_usage_from), so skipping it here would fold in rows the old pass
    never reached. ASCII is a subset of UTF-8, so an all-ASCII line always
    decodes, and a line with anything else in it returns False and is decoded
    and filtered exactly as before.

    False therefore means "decode it and check as before", never "this line
    is inside the window". Bounds must be the ones handed to
    _line_outside_window(): this check is only allowed to drop lines that one
    drops.
    """
    if not raw.startswith(_AT_PREFIX_B):
        return False
    end = raw.find(b",", len(_AT_PREFIX_B))
    if end < 0:
        return False
    token = raw[len(_AT_PREFIX_B):end]
    # 46/101/69 are `.`/`e`/`E` as byte values. The test is spelled with int
    # needles because on bytes an int needle is a plain single-byte scan
    # (memchr), where `b"." in token` runs the general substring search - about
    # four times the cost on a token this short, paid once per skipped line.
    # The answer is the text version's either way.
    if 46 not in token and 101 not in token and 69 not in token:
        return False
    try:
        at = float(token)
    except ValueError:
        return False
    if not ((lo is not None and at < lo) or (hi is not None and at > hi)):
        return False
    if raw.find(b'"at"', end) >= 0:
        return False
    return raw.isascii()


def _usage_timeseries_scan(realm, lo, hi, step):
    """One uncached pass over the log, folding rows into fixed-width buckets.

    这是窗口序列的兜底路径：日/小时桶切片不成立（见 _series_slice）时用它，
    口径一字未改。
    """
    buckets = {}
    # 窗口内 credit 前 50 条：大小 50 的最小堆，堆顶是当前的第 50 名。旧实现
    # 把整窗的计费行全 append 进列表、扫完再 sort 整份切前 50——排序的是上万
    # 条，留下的只有 50 条。堆把每行压到一次「比不比堆顶大」的比较，进不了
    # 前 50 的行连载荷字典都不建。
    # 堆元素是 (at, -序号, 载荷)：序号取负，让相同 at 时日志靠前的行更大，与
    # 稳定排序（at 倒序、同 at 保持日志顺序）以及 _series_credit_push 的缓冲
    # 逐条一致（键唯一，堆的替换规则与稳定排序的取舍等价）。
    credits = []
    credit_seq = 0
    try:
        with open(USAGE_LOG, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                # Cheap window test first: a day or week window covers a small
                # slice of a multi-day log, and the json.loads this skips is
                # what made a cold scan expensive. A line the check cannot read
                # exactly falls through to the full parse and the full test
                # below, so a skipped row is always one the parse would have
                # skipped as well - same output, fewer parses.
                if _line_outside_window(line, lo, hi):
                    continue
                try:
                    row = json.loads(line)
                except Exception:
                    continue
                if realm and not row_matches_realm(row, realm):
                    continue
                at = row.get("at") or 0
                if at < lo or at > hi:
                    continue
                key = int((at - lo) // step)
                bucket = buckets.setdefault(key, {
                    "at": lo + key * step, "requests": 0, "errors": 0,
                    "prompt_tokens": 0, "completion_tokens": 0,
                    "reasoning_tokens": 0, "cached_tokens": 0,
                    "total_tokens": 0, "credit": 0.0,
                })
                outcome = row_outcome(row)
                if outcome == "completed":
                    bucket["requests"] += 1
                    for field in ("prompt_tokens", "completion_tokens",
                                  "reasoning_tokens", "cached_tokens",
                                  "total_tokens"):
                        bucket[field] += (row.get(field) or 0)
                else:
                    bucket["errors"] += 1
                credit = row.get("credit") or 0
                if outcome != "client_aborted":
                    bucket["credit"] += credit
                if credit > 0:
                    seq = -credit_seq
                    credit_seq += 1
                    if len(credits) < _SERIES_CREDITS_KEEP:
                        heapq.heappush(credits,
                                       (at, seq, _series_credit_entry(at, row)))
                    elif (at, seq) > (credits[0][0], credits[0][1]):
                        heapq.heapreplace(credits,
                                          (at, seq, _series_credit_entry(at, row)))
    except FileNotFoundError:
        pass
    except Exception as exc:
        log("usage timeseries read failed: %s" % exc)
    # 堆里就是窗口内的前 50 条；按同一把键排出（键唯一，等于旧实现的稳定
    # 排序结果：at 倒序、同 at 保持日志顺序）。
    credits.sort(key=lambda item: (item[0], item[1]), reverse=True)
    return {
        "ok": True, "realm": realm or "all", "bucket_seconds": step,
        "since": lo, "until": hi,
        "series": [buckets[key] for key in sorted(buckets)],
        "credits": [item[2] for item in credits],
    }


def _usage_timeseries_uncached(realm, lo, hi, step):
    """窗口序列：优先由日/小时桶/分钟环切片，不成立时整段扫描。

    切片只在「桶宽与窗口对齐」时才是同一件事，所以先按桶宽判断要不要惊动
    状态：日/小时桶要求窗口起点是本地午夜，分钟环要求起点落在分钟格点上
    （lo 是 60 的整数倍）。两条都不成立的自定义区间根本不折状态，也就不会
    为它们白扫一遍。
    """
    if step in (3600, 86400):
        day_key = _window_day_key(lo)
    elif step == 60:
        # 分钟窗口：起点落在格点上才可能与存量分钟桶一一对应（见
        # _series_slice_minutes）；不在格点上的窗口连状态都不惊动。
        day_key = _series_window_day_key(lo)
        if day_key is not None and not _series_minute_grid_ok(lo):
            day_key = None
    else:
        day_key = None
    if day_key is not None and _usage_day_buckets_enabled():
        with _series_state_lock:
            state, offset, error = _usage_series_state_ready(realm)
            out = None
            if error is not None:
                # 折叠失败：状态归零（下一次从零折起），这次退回整段扫描。
                log("usage timeseries fold failed: %s" % error)
                state.update({"days": {}, "days_floor": None, "max_at": 0,
                              "credits": [], "minutes": {}, "minutes_from": None,
                              "hits": [], "hits_overflow": False,
                              "offset": 0, "tail": b""})
            else:
                out = _series_slice(state, realm, lo, hi, step, day_key)
        if error is None:
            _usage_cache_maybe_save("series", offset)
        if out is not None:
            return out
    return _usage_timeseries_scan(realm, lo, hi, step)


def count_usage_rows(realm=None):
    """Cached row count - incremental, substring match instead of a full parse.

    Rows written before the `realm` field existed (they are all error rows)
    have to fall back to the account/model heuristic in row_realm, so those
    few are still parsed properly - once, when the fold first reaches them.

    This runs on every /usage/recent poll purely to render the page total,
    and a full scan of the file dominated that endpoint (measured at ~50% of
    its cost on a 45MB log). The fold is carried over and refreshed with the
    rows appended since the last call, so a poll costs what the new traffic
    costs instead of what the whole log costs; the short TTL still keeps the
    number honest while removing even that from the poll path.
    """
    # 先归一化：桶键就是 realm 字符串本身，字面 "all" 会被当成一个永远不存在
    # 的桶（realm_scope 把它映射成 None = 总数）。
    realm = realm_scope(realm)
    r = realm or ""
    now = time.time()
    with _count_lock:
        hit = _count_cache.get(r)
        if hit is not None and (now - hit[0]) < _COUNT_TTL:
            return hit[1]
    n = _count_usage_rows_uncached(realm)
    with _count_lock:
        _count_cache[r] = (time.time(), n)
    return n


# `"realm"` 的键文本。旧实现拿 `'"realm": "<值>"'` / `'"realm":"<值>"'` 两个
# needle 做子串命中，新实现按同一对间距从行里取字段值（见 _count_usage_line）
# ——口径逐字相同，只是不再为每个 realm 各扫一遍文件。
_REALM_KEY = '"realm"'
_REALM_KEY_LEN = len(_REALM_KEY)


def _count_usage_line(state, line):
    """折一行：总数 + 这一行声明的 realm 桶。

    总数数的是**所有非空行**（包括解析不了的行），与旧实现一致。带 realm
    字段的行按字段值归桶：取值只认旧 needle 覆盖的两种间距（`"realm": "x"`
    与 `"realm":"x"`），间距再花哨的写法旧实现本来就匹配不到任何 realm，
    这里也不为它建桶；值读到下一个引号为止——查询用的 realm 不会带引号，
    字符串里的转义引号也拼不出 `"realm"` 这个键文本（同 _line_outside_window
    的说明），所以不会误判。同一行重复声明 `"realm"`（手改过的行）按每一处
    声明的值各计一次，与旧 needle 子串命中的行为一致；只有完全没有该字段的
    老行（都是错误行）才解析 JSON 走 row_realm() 的账号/模型回退——旧实现
    每次查询都要为它们付一次 json.loads，这里只在折叠时付一次。
    """
    state["n"] += 1
    counts = state["n_realm"]
    pos = line.find(_REALM_KEY)
    if pos < 0:
        # 没有 realm 字段（老错误行）：解析回退。解析不了就只算进总数。
        try:
            realm = row_realm(json.loads(line))
        except Exception:
            return
        counts[realm] = counts.get(realm, 0) + 1
        return
    while pos >= 0:
        end = pos + _REALM_KEY_LEN
        if line.startswith(': "', end):
            start = end + 3
        elif line.startswith(':"', end):
            start = end + 2
        else:
            start = -1
        if start >= 0:
            quote = line.find('"', start)
            if quote >= 0:
                value = line[start:quote]
                counts[value] = counts.get(value, 0) + 1
        pos = line.find(_REALM_KEY, end)


def _count_usage_scan(state):
    """把日志 offset 之后的每一行折进 state；返回 (offset, error)。

    与 _scan_usage_from 同一套字节纪律（整行推进、半行留给下一趟、解码失败
    即停），但折叠的是**行文本**而不是解析出的 row：计数的总口径里有一类行
    根本不是合法 JSON（总行数照数），而 _scan_usage_from 在 fold 之前就把
    它们丢掉了。逐行调用在这里直接写死（不走回调），全量重数一次要过几万行，
    每行省一次间接调用是这条冷路径上最便宜的一笔。
    """
    offset = state["offset"]
    try:
        with open(USAGE_LOG, "rb") as fh:
            fh.seek(offset)
            while True:
                raw = fh.readline()
                if not raw:
                    break
                if not raw.endswith(b"\n"):
                    break
                end = offset + len(raw)
                try:
                    line = raw.decode("utf-8").strip()
                except UnicodeDecodeError as exc:
                    return end, exc
                if line:
                    try:
                        _count_usage_line(state, line)
                    except Exception as exc:
                        return end, exc
                offset = end
    except FileNotFoundError:
        pass
    except Exception as exc:
        return offset, exc
    return offset, None


def _count_usage_rows_uncached(realm=None):
    """增量计数：把折叠推进到文件尾，返回 realm 对应的行数。

    状态按「本进程数到哪」记账（offset/tail/n/n_realm），每次调用只数新增
    的字节；位置校验不过（截断、换文件、同尺寸重写）才从零重数。realm 为
    空返回总行数，否则返回该 realm 桶里的行数。
    """
    state = _count_state
    with _count_state_lock:
        log_key = _usage_log_key()
        if not _log_resume_ok(state, log_key):
            state.update({"offset": 0, "tail": b"", "n": 0, "n_realm": {}})
        state["key"] = log_key
        offset, error = _count_usage_scan(state)
        n = state["n"]
        counts = state["n_realm"]
        if error is None:
            state["offset"] = offset
            state["tail"] = _log_tail_signature(offset)
        else:
            # 出错的那一行（非法 UTF-8 才会走到这里）可能已经半折进去了：
            # 这份状态整个作废，下次从零重数——by_account 折叠同款处理。
            # 这一次仍返回已经折到的部分：旧实现的文本层按块解码，坏行落在
            # 第一块里时整次报 0、落在后面时给部分数；新实现稳定地给出坏行
            # 之前的行数，任何情况下都不会更差。
            log("usage count failed: %s" % error)
            state.update({"offset": 0, "tail": b"", "n": 0, "n_realm": {}})
        if not realm:
            return n
        return counts.get(realm, 0)


# ---------------------------------------------------------------------------
# recent 的行偏移索引（页 = 日志尾部的第 N 页）
# ---------------------------------------------------------------------------
#
# recent_usage 的每一页都要从日志尾部往回数到第 N×limit 条记录才定位得到：
# 第 1 页至少要读回一个 256KB 块，realm 过滤命中率低时要读回整个文件，页深
# 越大代价越大。这里给日志建一份行偏移索引——每 64KB 记一条「行边界字节偏移
# + 到该处的累计记录数」，页 N 直接 seek 到起点附近再顺序读回页大小量级的
# 数据，读取量与页深无关。
#
# 索引落在数据目录的独立文件里（不并进 checkpoint：checkpoint 是五份聚合
# 状态的落盘，这份索引的对象只有日志本身，校验与写盘策略都跟着日志走）：
#
#   * 位置校验复用聚合那套现成机制（_usage_log_key + _log_offset_holds +
#     _log_tail_signature，P:2504-2573）：日志被截断、被替换、同尺寸重写都会
#     让索引作废，从零重扫一遍——宁多扫一次，也不给出一页错的记录；
#   * 两个口径都在同一次扫描里现算，不复制第二份实现：页数标签走
#     _count_usage_line（D5：总数含坏 JSON 的非空行），行定位走 recent 自己的
#     row_matches_realm 归属；
#   * 「没带 realm 字段的行」的归属要查活账号池（row_realm），跨进程不一定
#     复现得了：有这类行时，realm 相关的数字只在「活池 == 磁盘归属」且 uid→
#     realm 与折进来时一致时才用索引（_realm_fold_reproducible），否则退回
#     老路——老路永远用活池，永远是当前该有的数字。
_RECENT_INDEX_SCHEMA = 1
# 索引粒度：每 64KB 记一条行边界。4 万行的日志（约 23MB）只有 350 条上下，
# 页读取的起点最多偏一段，读取量是「一段 + 页本身」。
_RECENT_INDEX_SEGMENT = 65536
# realm 表容量。真实日志只有 intl/cn（外加没带字段的老行归属），超过这个数
# 说明 realm 字段被人乱写了，索引直接不用（老路照常给出正确结果）。
_RECENT_INDEX_REALM_CAP = 6
# realm 值长度上限：手改过的行可以带出很长的值，那只会把索引撑大。
_RECENT_INDEX_REALM_MAX = 128


def _recent_index_enabled():
    """索引总开关，默认开；WB_RECENT_INDEX=0 关（现场应急 / 测试切换）。

    与 WB_USAGE_CACHE 同款：每次调用读环境变量，不在导入时定死。
    """
    return os.environ.get("WB_RECENT_INDEX", "0").strip().lower() in (
        "1", "true", "yes", "on")


def _recent_index_min_bytes():
    """写盘节流：推进不足这么多字节就不写。默认 256KB。

    这个阈值就是「重启后最多回放多少」：索引加载回来后要把没索引的尾部重新
    折一遍（每行一次 json.loads），真机 1MB ≈ 0.14s、256KB ≈ 35ms。索引本身
    只有几 KB，写盘很便宜（日志每天涨 6MB 上下 → 一天几十次写、几百 KB），
    所以阈值取得比 checkpoint 的 1MB 小一档，把重启后的第一页压在几十毫秒。
    """
    try:
        return max(0, int(os.environ.get("WB_RECENT_INDEX_MIN_BYTES", 256 * 1024)))
    except (TypeError, ValueError):
        return 256 * 1024


def _recent_index_min_seconds():
    """写盘节流：距上次尝试不足这么多秒就不写。默认 900s。"""
    try:
        return max(0.0, float(os.environ.get("WB_RECENT_INDEX_MIN_SECONDS", 900)))
    except (TypeError, ValueError):
        return 900.0


def _recent_index_stamp():
    """索引文件的 (路径, mtime_ns, size)；文件不在时后两项为 None。

    路径也在戳里（同 estimate-daily）：换目录后不会被旧目录的戳骗过去。
    """
    try:
        st = os.stat(RECENT_INDEX_FILE)
        return (RECENT_INDEX_FILE, st.st_mtime_ns, st.st_size)
    except OSError:
        return (RECENT_INDEX_FILE, None, None)


def _recent_index_realm_map_json(disk):
    """磁盘那份 uid→realm 的 JSON 形态（与索引文件里的比）。"""
    if disk is None:
        return None
    return [[uid, realm] for uid, realm in disk]


def _recent_index_realm_name(value):
    """realm 表里的键：只收能出现在查询里的短字符串，其余让索引作废。

    row_realm 对带字段的行原样返回字段值，手改过的行可以带出数字或超长的
    值：它们既进不了 JSON 键、也不会等于任何字符串查询，但会把表撑大，索性
    让整份索引不可用（老路照常给出正确结果）。
    """
    if isinstance(value, str) and len(value) <= _RECENT_INDEX_REALM_MAX:
        return value
    return None


def _recent_index_realm_ok(state):
    """realm 相关的数字能不能信这份索引。

    只有「没带 realm 字段、带 account 的行」会查到活池（row_realm），别的行
    归属只由行内字段决定，任何进程、任何时刻算出来都一样。有这类行时要求：
    折进来时的 uid→realm（磁盘那份）与现在一致，且活池与磁盘一致
    （_realm_fold_reproducible）——两者合起来把归属钉死，索引算出的每个
    realm 计数与现在这台机器上会算出来的那份逐字相同。
    """
    if not state["legacy"]:
        return True
    realm_map = state["realm_map"]
    if realm_map is None:
        return False
    disk = _account_realm_map()
    if _recent_index_realm_map_json(disk) != realm_map:
        return False
    return _realm_fold_reproducible(disk)


def _recent_index_blank():
    """一份空索引的核心字段（写盘形态与内存形态共用）。"""
    return {"key": None, "size": 0, "tail": b"", "realm_map": None,
            "legacy": False, "realms": [], "counts": {}, "row_counts": {},
            "lines": 0, "rows": 0, "segs": []}


_recent_index_lock = threading.Lock()
# 索引状态（由 _recent_index_lock 保护）。字段：
#   stamp          —— 上次读到/写下的索引文件戳（外部改动靠它发现）；
#   usable         —— 位置校验通过、可以服务查询；
#   giveup         —— 扫描撞上索引处理不了的行，本进程不再重试（日志换掉才重来）；
#   key/size/tail  —— 与聚合同款的位置信息：索引覆盖到 size（行边界）；
#   realm_map      —— 折进来时的磁盘 uid→realm（realm 数字的指纹）；
#   legacy         —— 日志里有归属要查活池的行；
#   realms/counts/row_counts —— realm 表与两个口径的计数（count 口径只有总数，
#                      recent 口径按段存，供定位）；
#   segs           —— [offset, rows, <row_counts 各列>]，累计值；
#   wrote_size/at  —— 写盘节流（记「尝试」而非成功，失败也退避）。
_recent_index_state = {
    "stamp": None, "usable": False, "giveup": False, "giveup_key": None,
    "key": None, "size": 0, "tail": b"", "realm_map": None, "legacy": False,
    "realms": [], "counts": {}, "row_counts": {}, "lines": 0, "rows": 0,
    "segs": [], "wrote_size": 0, "wrote_at": 0.0,
}


def _recent_index_restart(state, realm_map):
    """清回零重新折（保留文件戳与写盘节流）：位置或归属口径变了才走这里。"""
    state.update(_recent_index_blank())
    state.update({"usable": False, "giveup": False, "giveup_key": None,
                  "realm_map": realm_map})


def _recent_index_add_realm(state, name):
    """把一个新 realm 值加进表，段数组同步补一列零。

    表就是段数组的列序：某一段里没出现过的值，计数本来就是 0，补零不改变
    任何累计值。
    """
    if name in state["realms"]:
        return True
    if len(state["realms"]) >= _RECENT_INDEX_REALM_CAP:
        return False
    state["realms"].append(name)
    for seg in state["segs"]:
        seg.append(0)
    state["counts"].setdefault(name, 0)
    state["row_counts"].setdefault(name, 0)
    return True


def _recent_index_dirty(state):
    """扫描撞上索引处理不了的行：这份状态不能用了，本进程不再重试。

    与「干净停下」的区别：_count_usage_line 可能已经把半个计数写进状态，
    续读会得到错的累计值，所以整份丢掉、记住日志位置——日志被换掉（key 变）
    才再试一次，否则每次调用都重扫一遍比老路还贵。
    """
    state.update(_recent_index_blank())
    state.update({"usable": False, "giveup": True,
                  "giveup_key": _usage_log_key()})
    return False


def _recent_index_fold_line(state, line):
    """一行折进两个口径；返回 False 表示这行让索引不能用了（见上）。"""
    counts = state["counts"]
    before = len(counts)
    shim = {"n": state["lines"], "n_realm": counts}
    try:
        _count_usage_line(shim, line)
    except Exception:
        return _recent_index_dirty(state)
    state["lines"] = shim["n"]
    if len(counts) != before:
        for name in list(counts):
            if name in state["realms"]:
                continue
            good = _recent_index_realm_name(name)
            if good is None or not _recent_index_add_realm(state, good):
                return _recent_index_dirty(state)
    try:
        row = json.loads(line)
    except Exception:
        return True                     # 坏 JSON：算行数、不算记录（D5 同款）
    if not isinstance(row, dict):
        # 老实现会把 null / 数组这类行原样放进 matching（随后计价时炸掉）：
        # 这种日志不用索引，让老路照旧处理。
        return _recent_index_dirty(state)
    if not row.get("realm") and row.get("account"):
        state["legacy"] = True          # 这一行的归属要查活池
    try:
        name = row_realm(row)
    except Exception:
        return _recent_index_dirty(state)
    name = _recent_index_realm_name(name)
    if name is None or not _recent_index_add_realm(state, name):
        return _recent_index_dirty(state)
    state["rows"] += 1
    state["row_counts"][name] = state["row_counts"].get(name, 0) + 1
    return True


def _recent_index_extend(state):
    """把索引推进到日志尾部；返回是否可用。

    与 _count_usage_scan 同一套字节纪律：整行推进、半行留给下一趟、解码失败
    即停。停下的位置就是 size（最后一条完整行的末尾），下一趟从那里接着读，
    所以半行与坏字节只让索引暂时不可用，不会让每次调用都重扫整份日志。
    """
    if state["giveup"]:
        return False
    offset = state["size"]
    try:
        with open(USAGE_LOG, "rb") as fh:
            fh.seek(offset)
            while True:
                raw = fh.readline()
                if not raw:
                    break
                if not raw.endswith(b"\n"):
                    state["size"] = offset      # 半行（写入中/掉电碎片）
                    return False
                end = offset + len(raw)
                try:
                    line = raw.decode("utf-8").strip()
                except UnicodeDecodeError:
                    state["size"] = offset      # 解码失败：索引停在这行之前
                    return False
                if line and not _recent_index_fold_line(state, line):
                    if not state["giveup"]:
                        state["size"] = offset
                    return False
                offset = end
                last = state["segs"][-1][0] if state["segs"] else 0
                if offset - last >= _RECENT_INDEX_SEGMENT:
                    state["segs"].append(
                        [offset, state["rows"]]
                        + [state["row_counts"].get(r, 0)
                           for r in state["realms"]])
    except OSError:
        state["size"] = offset
        return False
    except Exception:
        state["size"] = offset
        return False
    state["size"] = offset
    state["key"] = _usage_log_key()
    state["tail"] = _log_tail_signature(offset)
    if state["tail"] is None:
        return False                        # 签名读不回来：位置校验立不住
    state["usable"] = True
    return True


def _recent_index_payload(state):
    """索引的 JSON 形态：段用增量写（offset 差 + 各列计数差）。

    增量写出来的数字都小（offset 差就是段宽，计数差是段内的记录数），4 万行
    的日志整份不到 10KB；累计值在加载时一次加回去。不排序键：段数组是位置
    敏感的，形状固定，无需靠键序表达什么。
    """
    segs = []
    prev_off = 0
    prev = [0] * (2 + len(state["realms"]))
    for seg in state["segs"]:
        row = [seg[0] - prev_off]
        for i in range(1, len(seg)):
            row.append(seg[i] - prev[i])
        segs.append(row)
        prev_off = seg[0]
        prev = seg
    return {
        "schema": _RECENT_INDEX_SCHEMA,
        "seg": _RECENT_INDEX_SEGMENT,
        "key": list(state["key"] or (0, 0, 0)),
        "tail": state["tail"].hex(),
        "realm_map": state["realm_map"],
        "legacy": bool(state["legacy"]),
        "realms": list(state["realms"]),
        "totals": {"lines": state["lines"], "rows": state["rows"],
                   "counts": dict(state["counts"]),
                   "row_counts": dict(state["row_counts"])},
        "segs": segs,
    }


def _recent_index_write(state):
    """原子写索引（同目录 .tmp + os.replace，同 checkpoint / estimate-daily）。"""
    os.makedirs(USAGE_DIR, exist_ok=True)
    tmp = RECENT_INDEX_FILE + ".tmp"
    text = json.dumps(_recent_index_payload(state), ensure_ascii=False,
                      separators=(",", ":"))
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
        os.replace(tmp, RECENT_INDEX_FILE)
    except OSError:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def _recent_index_parse(data):
    """磁盘上的索引 → 内存状态；任何一处不合形状都返回 None（当作没有索引）。"""
    if not isinstance(data, dict) or data.get("schema") != _RECENT_INDEX_SCHEMA:
        return None
    if data.get("seg") != _RECENT_INDEX_SEGMENT:
        return None
    realms = data.get("realms")
    if not isinstance(realms, list) or len(realms) > _RECENT_INDEX_REALM_CAP:
        return None
    for name in realms:
        if _recent_index_realm_name(name) is None:
            return None
    if len(set(realms)) != len(realms):
        return None
    totals = data.get("totals")
    if not isinstance(totals, dict):
        return None
    lines = totals.get("lines")
    rows = totals.get("rows")
    counts = totals.get("counts")
    row_counts = totals.get("row_counts")
    if (not _usage_cache_int(lines) or lines < 0
            or not _usage_cache_int(rows) or rows < 0):
        return None
    if not _usage_cache_counts_ok(counts) or not _usage_cache_counts_ok(row_counts):
        return None
    key = data.get("key")
    if (not isinstance(key, list) or len(key) != 3
            or not all(_usage_cache_int(v) and v >= 0 for v in key)):
        return None
    tail = data.get("tail")
    if not isinstance(tail, str):
        return None
    try:
        tail_bytes = bytes.fromhex(tail)
    except ValueError:
        return None
    realm_map = data.get("realm_map")
    if realm_map is not None:
        if not isinstance(realm_map, list):
            return None
        for item in realm_map:
            if (not isinstance(item, list) or len(item) != 2
                    or not isinstance(item[0], str)
                    or not isinstance(item[1], str)):
                return None
    if not isinstance(data.get("legacy"), bool):
        # 老行归属要不要查池：这个标记决定 realm 数字信不信这份索引，写成
        # 别的一律当坏文件（宁可重扫）。
        return None
    segs_raw = data.get("segs")
    if not isinstance(segs_raw, list):
        return None
    width = 2 + len(realms)
    segs = []
    off = 0
    prev = [0] * width
    for raw in segs_raw:
        if (not isinstance(raw, list) or len(raw) != width
                or not all(_usage_cache_int(v) and v >= 0 for v in raw)):
            return None
        off += raw[0]
        cur = [off]
        for i in range(1, width):
            cur.append(prev[i] + raw[i])
        segs.append(cur)
        prev = cur
    state = _recent_index_blank()
    state.update({
        "key": tuple(key), "size": key[0], "tail": tail_bytes,
        "realm_map": realm_map, "legacy": data.get("legacy") is True,
        "realms": list(realms), "counts": dict(counts),
        "row_counts": dict(row_counts), "lines": lines, "rows": rows,
        "segs": segs,
    })
    return state


def _recent_index_adopt(state, stamp):
    """文件戳变了：把磁盘那份读进来。

    只接受「比内存这份更新」（同一份日志上更长的索引，或内存这份已经不成立）
    的文件——内存里那份是同一份日志的续读，回退到更短的磁盘版本等于白扫。
    读不出来或校验不过就当没有索引：下一趟从零重建（坏文件不报错）。
    """
    data = None
    if stamp[1] is not None:
        try:
            with open(RECENT_INDEX_FILE, "r", encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, ValueError, UnicodeDecodeError):
            data = None
    loaded = _recent_index_parse(data) if data is not None else None
    if loaded is not None:
        log_key = _usage_log_key()
        if (_log_offset_holds(loaded["key"], log_key, loaded["size"])
                and _log_tail_signature(loaded["size"]) == loaded["tail"]):
            if not state["usable"] or loaded["size"] > state["size"]:
                state.update(loaded)
                state.update({"usable": True, "giveup": False,
                              "giveup_key": None, "stamp": stamp,
                              "wrote_size": loaded["size"],
                              "wrote_at": time.time()})
                return
    state["stamp"] = stamp
    if not state["usable"]:
        _recent_index_restart(
            state, _recent_index_realm_map_json(_account_realm_map()))


def _recent_index_maybe_write(state):
    """按节流阈值把索引落盘（默认 256KB / 900s）。

    第一次建起来立即写——那正是「重启后免重扫」的那一份；之后只在推进满
    256KB 或距上次尝试满 900s 时写（空闲不重写）。记「尝试」而非成功：只读
    文件系统上失败会一直成立，不退避的话每次刷新都去试写一遍。归属复现不了
    的索引不写：写下去别的进程也不敢用。
    """
    now = time.time()
    size = state["size"]
    if size <= state["wrote_size"]:
        return
    if (size - state["wrote_size"] < _recent_index_min_bytes()
            and (now - state["wrote_at"]) < _recent_index_min_seconds()):
        return
    if not _recent_index_realm_ok(state):
        return
    state["wrote_at"] = now
    state["wrote_size"] = size
    try:
        _recent_index_write(state)
    except Exception as exc:
        log("recent index write failed: %s" % exc)
        return
    state["stamp"] = _recent_index_stamp()


def _recent_index_snapshot(state):
    """给调用方的只读拷贝（锁外读页时状态可能被别的线程推进）。"""
    return {
        "size": state["size"],
        "realms": list(state["realms"]),
        "counts": dict(state["counts"]),
        "row_counts": dict(state["row_counts"]),
        "lines": state["lines"],
        "rows": state["rows"],
        "segs": [list(seg) for seg in state["segs"]],
    }


def _recent_index_resume_ok(state, log_key):
    """位置校验：索引覆盖的字节还是这份日志的前缀。

    与聚合折叠同款判据（_log_offset_holds + _log_tail_signature）：size/inode
    挡住截断与换文件，size 前的 64B 签名挡住同尺寸重写。
    """
    return (_log_offset_holds(state["key"], log_key, state["size"])
            and _log_tail_signature(state["size"]) == state["tail"])


def _recent_index_ready(realm_filtered=False):
    """扩到文件尾的索引快照；None 表示这次查询走老路（count + 反向读）。

    顺序固定：先看文件戳（外部改动）→ 位置校验（截断/替换/同尺寸重写）→
    归属口径（realm_map 变了整份重折）→ 续读到尾 → realm 查询再验一次归属。
    """
    if not _recent_index_enabled():
        return None
    with _recent_index_lock:
        state = _recent_index_state
        stamp = _recent_index_stamp()
        if state["stamp"] != stamp:
            _recent_index_adopt(state, stamp)
        log_key = _usage_log_key()
        if state["giveup"]:
            if state["giveup_key"] == log_key:
                return None
            state["giveup"] = False             # 日志被换掉了：再给一次机会
        if state["size"] > 0:
            if not _recent_index_resume_ok(state, log_key):
                # 位置对不上（截断/换文件/同尺寸重写）：整份重折
                _recent_index_restart(
                    state, _recent_index_realm_map_json(_account_realm_map()))
            elif state["legacy"]:
                realm_map = _recent_index_realm_map_json(_account_realm_map())
                if state["realm_map"] != realm_map:
                    # 归属口径变了（uid→realm 改过）：整份重折
                    # ——与聚合折叠同款政策。
                    _recent_index_restart(state, realm_map)
        if not state["usable"] and not state["segs"] and not state["lines"]:
            # 从零开始：把这次的归属口径记下来（后面续读都按它校验）
            state["realm_map"] = _recent_index_realm_map_json(
                _account_realm_map())
        # 每次都续读到文件尾：日志只会追加，续读的代价就是新字节（没有新
        # 字节时只是一次 seek 到底的空读）。
        if not _recent_index_extend(state):
            return None
        if realm_filtered and not _recent_index_realm_ok(state):
            return None
        _recent_index_maybe_write(state)
        return _recent_index_snapshot(state)


def _recent_index_total(snap, realm):
    """页数标签：口径与 count_usage_rows 完全一致（总数含坏 JSON 的非空行）。"""
    if not realm:
        return snap["lines"]
    return snap["counts"].get(realm, 0)


def _recent_index_page(snap, realm, page, limit):
    """第 page 页的记录（新的在前）；None 表示索引答不了，调用方走老路。

    老实现的页 = 从尾部往回数第 (page-1)*limit+1 .. page*limit 条记录，切片
    两端都夹在「真的存在这么多条」上（页数标签含坏行、记录数不含，最后几页
    因此可能短、甚至为空）。这里把同一件事换算成从头数的序号：第 k 条匹配
    记录在两套里是同一行，于是页的字节区间就是 [第 K_lo 条起点, 第 K_hi 条
    终点]，从 K_lo 所在段的起点顺序读过去即可。
    """
    if realm:
        if realm not in snap["realms"]:
            return []                        # 一条也不匹配：老路也是空页
        col = 2 + snap["realms"].index(realm)
        total_rows = snap["row_counts"].get(realm, 0)
    else:
        col = 1
        total_rows = snap["rows"]
    target = page * limit
    k_hi = total_rows - (page - 1) * limit
    if k_hi < 1:
        return []                            # 页落在记录总数之外：老路是空切片
    k_lo = max(1, total_rows - target + 1)
    segs = snap["segs"]
    # 二分：第一个「到该段起点为止的记录数 >= K_lo」的段；它前一段就是起点
    lo, hi = 0, len(segs)
    while lo < hi:
        mid = (lo + hi) // 2
        if segs[mid][col] < k_lo:
            lo = mid + 1
        else:
            hi = mid
    start = segs[lo - 1][0] if lo > 0 else 0
    have = segs[lo - 1][col] if lo > 0 else 0
    # 段起点之前的记录数是 have，段里第一条就是第 have+1 条：跳过
    # k_lo-have-1 条，下一条正好是第 k_lo 条（K_lo > have 由二分保证）。
    rows = _recent_index_read(start, realm, k_lo - have - 1, k_hi - k_lo + 1)
    if rows is None:
        return None
    rows.reverse()
    return rows


def _recent_index_read(start, realm, skip, take):
    """从 start 顺序读到第 take 条匹配记录；对不上就返回 None（走老路）。

    读取量 = 一段（64KB）+ 页本身的字节，与页深无关——这正是老实现最贵的
    地方（第 1 页也要读满一个 256KB 块，realm 过滤的页要读回整个文件）。
    读到的行与老实现逐字一致：同一套 strip/解码（"replace"）与 json.loads，
    realm 过滤走同一个 row_matches_realm。
    """
    rows = []
    seen = 0
    try:
        with open(USAGE_LOG, "rb") as fh:
            fh.seek(start)
            buf = b""
            while len(rows) < take:
                # 64KB 一读：正好一段的量级，页读完就停手（读取量 = 一段 +
                # 页本身的字节，与页深无关）。
                chunk = fh.read(64 * 1024)
                if not chunk:
                    break
                buf += chunk
                parts = buf.split(b"\n")
                buf = parts.pop()
                for raw in parts:
                    st = raw.strip()
                    if not st:
                        continue
                    try:
                        item = json.loads(st.decode("utf-8", "replace"))
                    except Exception:
                        continue
                    if not isinstance(item, dict):
                        return None     # 索引覆盖的字节里不该有这种行
                    if realm and not row_matches_realm(item, realm):
                        continue
                    if seen < skip:
                        seen += 1
                        continue
                    rows.append(item)
                    if len(rows) >= take:
                        break
    except Exception as exc:
        log("recent index read failed: %s" % exc)
        return None
    if len(rows) < take:
        return None
    return rows


def _recent_backward_page(realm, page, limit):
    """老路：从尾部按 256KB 块往回读，凑够 page*limit 条再切片。

    页深越大读得越多（realm 过滤命中率低时要读回整个文件），所以索引可用时
    不走这里；索引答不了（半行、坏字节、池没就绪……）时它是唯一的正确答案，
    逐字保留原实现的行为。
    """
    target_count = page * limit
    matching = []
    chunk = 256 * 1024
    try:
        with open(USAGE_LOG, "rb") as fh:
            fh.seek(0, os.SEEK_END)
            pos = fh.tell()
            buf = b""
            while pos > 0 and len(matching) < target_count:
                step = min(chunk, pos)
                pos -= step
                fh.seek(pos)
                buf = fh.read(step) + buf
                parts = buf.split(b"\n")
                buf = parts[0]
                for raw in reversed(parts[1:]):
                    st = raw.strip()
                    if not st:
                        continue
                    try:
                        item = json.loads(st.decode("utf-8", "replace"))
                    except Exception:
                        continue
                    if realm and not row_matches_realm(item, realm):
                        continue
                    matching.append(item)
                    if len(matching) >= target_count:
                        break
            if len(matching) < target_count and buf.strip():
                try:
                    item = json.loads(buf.strip().decode("utf-8", "replace"))
                    if not realm or row_matches_realm(item, realm):
                        matching.append(item)
                except Exception:
                    pass
    except FileNotFoundError:
        pass
    except Exception as exc:
        log("recent_usage read failed: %s" % exc)
    start_idx = (page - 1) * limit
    end_idx = start_idx + limit
    return matching[start_idx:end_idx]


def recent_usage(limit=100, realm=None, page=1):
    """Paginated rows from the tail of the log (page 1 is latest).

    Reading backward in chunks keeps this in the millisecond range while
    accurately fetching any requested page without missing rows across realms.

    索引可用时页从索引直接定位（见上面那一节）：页数标签与页内容都来自同一次
    扫描，读取量是页大小量级；索引答不了就整段退回 _recent_backward_page()，
    两条路的输出逐字相同。
    """
    try:
        limit = max(1, int(limit))
    except Exception:
        limit = 100
    try:
        page = max(1, int(page))
    except Exception:
        page = 1
    realm = realm_scope(realm)
    index = _recent_index_ready(realm_filtered=bool(realm))
    page_rows = None
    if index is not None:
        total = _recent_index_total(index, realm)
        total_pages = max(1, (total + limit - 1) // limit) if total > 0 else 1
        page = min(page, total_pages)
        page_rows = _recent_index_page(index, realm, page, limit)
        if page_rows is None:
            index = None
    if index is None:
        total = count_usage_rows(realm)
        total_pages = max(1, (total + limit - 1) // limit) if total > 0 else 1
        page = min(page, total_pages)
        page_rows = _recent_backward_page(realm, page, limit)
    # Equivalent-token cost per row at OpenRouter list prices, computed here
    # so every consumer of /usage/recent gets the same number. Each row is
    # priced against the version that was in force when it happened, and says
    # where that price came from. cost_cny stays None for models the version
    # cannot price; cost_band is the index of the conditional band the row
    # landed in (None when the model has one flat price, or none matched).
    # cost_rates / cost_unit / cost_currency / cost_usd_cny / cost_or_id /
    # cost_via / cost_inherited_from / cost_override_from / cost_band_note /
    # cost_via_derived are the same figure taken apart for the panel's hover
    # card: which band's three unit prices, at which rate, matched how. They
    # are all None when the row is unpriced - the panel must say so rather
    # than show a made-up 0.
    for r in page_rows:
        cost = wb_pricing.cost_for_row(r)
        r["cost_cny"] = round(cost["cny"], 6) if cost["known"] else None
        r["cost_band"] = cost["band"] if cost["known"] else None
        # cost_policy stays as the row recorded it; cost_source is what the
        # lookup actually resolved to (the same id, or "builtin").
        r["cost_source"] = cost["source"] if cost["known"] else None
        r["cost_source_at"] = cost["source_at"] if cost["known"] else None
        r["cost_backfilled"] = bool(cost["backfilled"]) if cost["known"] else False
        r["cost_rates"] = cost["rates"] if cost["known"] else None
        r["cost_unit"] = cost["unit"] if cost["known"] else None
        r["cost_currency"] = cost["currency"] if cost["known"] else None
        r["cost_usd_cny"] = cost["usd_cny"] if cost["known"] else None
        r["cost_or_id"] = cost["or_id"] if cost["known"] else None
        r["cost_via"] = cost["via"] if cost["known"] else None
        r["cost_inherited_from"] = (cost["inherited_from"] if cost["known"]
                                    else None)
        r["cost_override_from"] = (cost["override_from"] if cost["known"]
                                   else None)
        r["cost_band_note"] = cost["band_note"] if cost["known"] else None
        r["cost_via_derived"] = (bool(cost["via_derived"]) if cost["known"]
                                 else None)
    return {
        "total": total,
        "page": page,
        "limit": limit,
        "total_pages": total_pages,
        "usd_cny": wb_pricing.usd_cny(),
        "rows": page_rows
    }
POOL = None
SCHEDULER = None
PRICING = None
CREDITS_REFRESHER = None
# Release discovery only - the checker never downloads or installs anything.
UPDATES = None
ACCOUNTS_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'accounts')
def realm_state_file():
    """Path of the persisted realm switch.

    Computed on every access rather than cached in a module constant: the
    constant was built from the default ACCOUNTS_DIR at import time, so a
    later --accounts-dir (or a Docker volume pointing somewhere else) still
    read and wrote the realm switch next to the script - the panel then
    reported an exit that did not match the configured account store.
    """
    return os.path.join(ACCOUNTS_DIR, "active_realm.json")


def load_persisted_realm():
    global CURRENT_REALM
    path = realm_state_file()
    if os.path.isfile(path):
        try:
            with open(path, "r", encoding="utf-8") as fh:
                d = json.load(fh)
                r = d.get("realm")
                if r in ("intl", "cn"):
                    CURRENT_REALM = r
                    return CURRENT_REALM
        except Exception as e:
            log("could not load active realm: %s" % e)
    return CURRENT_REALM
def save_persisted_realm(realm):
    global CURRENT_REALM
    if realm in ("intl", "cn"):
        CURRENT_REALM = realm
        try:
            os.makedirs(ACCOUNTS_DIR, exist_ok=True)
            with open(realm_state_file(), "w", encoding="utf-8") as fh:
                json.dump({"realm": realm, "updated_at": time.time(), "updated_iso": time.strftime("%Y-%m-%d %H:%M:%S")}, fh, indent=2)
            log("persisted active realm '%s' to disk" % realm)
        except Exception as exc:
            log("failed to persist active realm: %s" % exc)
    return CURRENT_REALM
API_KEY = None
SYSTEM_PROMPT = DEFAULT_SYSTEM_PROMPT
def import_desktop_accounts(realm=None):
    imported = []
    for p, r in wb_accounts.desktop_credential_candidates():
        if realm and r != realm:
            continue
        try:
            account = POOL.import_desktop_credential(path=p, realm=r)
            imported.append(account)
            log("imported %s (%s) from %s" % (account.uid[:8], account.realm, os.path.basename(p)))
        except Exception as exc:
            log("skip %s: %s" % (os.path.basename(p), exc))
    return imported
def desktop_credential_scan():
    """Read-only scan of the desktop client credentials on this machine."""
    return wb_accounts.scan_desktop_credentials()
def account_views(realm=None):
    """List view of every account, including a live readiness flag."""
    if not POOL:
        return []
    return POOL.list_public(realm=realm)

# --------------------------------------------------------- 积分获取历史（发放记录）
# 上游对每个账号返回一份积分包清单（免费套餐、每日活跃奖励的 Bonus Pack、活动包…），
# 每个包带面额、发放时间与到期时间。看板的「积分获取历史」就是这份清单的合并视图：
# 只读内存里的账号积分快照，**不发任何上游请求**（要更新的数字先点「一键刷新积分」），
# 所以它同时给出快照时刻，让面板标明数据有多旧。
GRANT_STATUS_ACTIVE = "active"        # 上游标了 in_usage：当前正从它扣减
GRANT_STATUS_AVAILABLE = "available"  # 有剩余、未过期，只是当前不参与扣减
GRANT_STATUS_USED_UP = "used_up"      # 用完了
GRANT_STATUS_EXPIRED = "expired"      # 已过期


def _parse_stamp_epoch(text):
    """'2026-10-10 00:42:45' -> epoch；认不出来返回 None。"""
    if not text:
        return None
    try:
        return time.mktime(time.strptime(str(text)[:19], "%Y-%m-%d %H:%M:%S"))
    except Exception:
        return None


def _grant_status(pkg, now):
    """一个积分包现在的状态；判定顺序是「过期 > 用完 > 在扣减 > 可用」。"""
    try:
        remain = float(pkg.get("remain") or 0)
    except (TypeError, ValueError):
        remain = 0.0
    expired = bool(pkg.get("is_expired"))
    expire_at = _parse_stamp_epoch(pkg.get("expire_time"))
    if expire_at is not None and expire_at <= now:
        expired = True
    if expired:
        return GRANT_STATUS_EXPIRED
    if remain <= 0:
        return GRANT_STATUS_USED_UP
    return GRANT_STATUS_ACTIVE if pkg.get("in_usage") else GRANT_STATUS_AVAILABLE


# 发放记录与本机动作的关联窗口：动作在发放前 2 小时内算「这次动作带来的」，
# 上游记的发放时间可能比我们那条动作记录早几秒，所以向后也留一点余量。
GRANT_ACTION_BEFORE = 2 * 3600
GRANT_ACTION_AFTER = 600


def _grant_actions():
    """uid -> [{at, ts, task, ok}]（按时间升序），只含签到与每日活跃的真实尝试。

    历史文件是磁盘上的东西：读取一律先过 wb_activity.project()，多出来的键
    （别的写入方、手工编辑、被塞进来的凭证）不会跟着载荷出去；任何读取失败都
    退化成「没有关联动作」，绝不让一次查询挂掉。
    """
    index = {}
    try:
        rows = wb_activity.load()
    except Exception:
        return index
    for raw in rows or []:
        row = wb_activity.project(raw)
        task = row["task"]
        if task not in (wb_activity.TASK_CHECKIN, wb_activity.TASK_DAILY_CHAT):
            continue
        at = wb_activity._parse_ts(row["ts"])
        if at is None:
            continue
        index.setdefault(row["uid"], []).append(
            {"at": at, "ts": row["ts"], "task": task, "ok": bool(row["ok"])})
    for items in index.values():
        items.sort(key=lambda item: item["at"])
    return index


def _grant_action(index, uid, at):
    """发放时刻之前最近的一次本机签到 / 每日活跃尝试；窗口外没有就 None。"""
    if at is None:
        return None
    for item in reversed(index.get(uid) or ()):
        if item["at"] > at + GRANT_ACTION_AFTER:
            continue
        if item["at"] < at - GRANT_ACTION_BEFORE:
            break
        return {"task": item["task"], "ts": item["ts"], "ok": item["ok"],
                "delta_seconds": int(round(at - item["at"]))}
    return None

def credit_grants(now=None):
    """每个积分包一行（新的在前）＋汇总；只读快照，不发上游请求。"""
    now = time.time() if now is None else now
    actions = _grant_actions()
    rows = []
    fetched = 0.0
    for account in (POOL.accounts if POOL else []):
        credits = getattr(account, "credits", None) or {}
        try:
            fetched = max(fetched, float(credits.get("updated_at") or 0))
        except (TypeError, ValueError):
            pass
        for pkg in credits.get("packages") or []:
            if not isinstance(pkg, dict):
                continue
            def _num(key):
                try:
                    return float(pkg.get(key) or 0)
                except (TypeError, ValueError):
                    return 0.0
            expire_iso = pkg.get("expire_time") or pkg.get("cycle_end_time") or ""
            create_at = _parse_stamp_epoch(pkg.get("create_time"))
            rows.append({
                "uid": account.uid,
                "nickname": account.nickname or (account.uid or "")[:8],
                "realm": account.realm,
                "name": str(pkg.get("name") or pkg.get("package_code") or "Package"),
                "product": str(pkg.get("sub_product_name") or pkg.get("product_name") or ""),
                "grant_reason": str(pkg.get("grant_reason") or ""),
                "size": _num("size"),
                "remain": _num("remain"),
                "used": _num("used"),
                "unit": str(pkg.get("unit") or "credit"),
                "create_at": create_at,
                "create_iso": str(pkg.get("create_time") or ""),
                "expire_at": _parse_stamp_epoch(expire_iso),
                "expire_iso": str(expire_iso),
                "no_expiry": bool(pkg.get("no_expiry")),
                "days_left": pkg.get("days_left"),
                "status": _grant_status(pkg, now),
                "action": _grant_action(actions, account.uid, create_at),
            })
    # 新的在前；同一秒按账号、包名稳定排序，免得每次刷新顺序都在跳。
    rows.sort(key=lambda r: (-(r["create_at"] or 0), r["nickname"], r["name"]))
    return {
        "ok": True,
        "generated_at": now,
        "generated_iso": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(now)),
        "fetched_at": fetched or None,
        "fetched_iso": (time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(fetched))
                        if fetched else None),
        "rows": rows,
        "summary": {
            "count": len(rows),
            "size": round(sum(r["size"] for r in rows), 2),
            "remain": round(sum(r["remain"] for r in rows), 2),
            "used": round(sum(r["used"] for r in rows), 2),
            "accounts": len(set(r["uid"] for r in rows)),
        },
    }


# --------------------------------------------------------------- 任务状态快照
# GET /tasks 每次都要向上游要两份数据（成长任务 + 汇总），实测约 2 秒；而看板切区域、
# 切账号、切页面都会各打一次，用户在界面上就是"点了要等几秒才变"。这里按账号放一份
# 短 TTL 快照：只有这条只读路径吃缓存，任何会改变任务状态的动作（执行/旅行/签到）都
# 先清掉它，所以"刚点完执行却看到旧状态"不会发生。TTL 可用 WB_TASKS_CACHE_TTL 调，
# 设 0 即关闭缓存、回到每次直连上游。
TASKS_CACHE_TTL = float(os.environ.get("WB_TASKS_CACHE_TTL") or "20")
_tasks_cache = {}
_tasks_cache_lock = threading.Lock()

def invalidate_tasks_cache():
    """任务状态变了就清掉快照，下一次读重新向上游取。"""
    with _tasks_cache_lock:
        _tasks_cache.clear()

def growth_snapshot(account):
    """成长任务 + 汇总；TTL 内直接复用上一份快照，过期或没缓存才请求上游。

    两条查询互不依赖（汇总那边自己还要问三个接口），所以并发发出：冷启动时这块
    从"两条串起来等"变成一个来回，实测约 2 秒降到 1.5 秒以内。
    """
    if TASKS_CACHE_TTL <= 0:
        from wb_tasks import fetch_growth_tasks, fetch_growth_summary
        return _both(fetch_growth_tasks, fetch_growth_summary, account)
    now = time.time()
    with _tasks_cache_lock:
        hit = _tasks_cache.get(account.uid)
        if hit and hit[0] > now:
            return hit[1]
    from wb_tasks import fetch_growth_tasks, fetch_growth_summary
    data = _both(fetch_growth_tasks, fetch_growth_summary, account)
    with _tasks_cache_lock:
        _tasks_cache[account.uid] = (time.time() + TASKS_CACHE_TTL, data)
    return data

def _both(tasks_fn, summary_fn, account):
    """并发跑两条上游查询；任一条抛错就照旧往外抛（不写进快照）。"""
    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(max_workers=2) as pool:
        tasks_future = pool.submit(tasks_fn, account)
        summary_future = pool.submit(summary_fn, account)
        return tasks_future.result(), summary_future.result()



PROXY_DISCOVER_HOST = os.environ.get("WB_PROXY_DISCOVER_HOST") or "cli-proxy-mihomo"


def _proxy_port_range():
    raw = os.environ.get("WB_PROXY_DISCOVER_PORTS") or "17901-17910"
    if "-" in raw:
        lo, _, hi = raw.partition("-")
        if lo.strip().isdigit() and hi.strip().isdigit():
            return range(int(lo), int(hi) + 1)
    if raw.strip().isdigit():
        return [int(raw)]
    return range(17901, 17911)


def probe_proxy_exit(proxy_url, timeout=12):
    """Return (exit_ip, error) for one proxy URL."""
    try:
        opener = wb_accounts.opener_for_proxy(proxy_url)
        if opener is None:
            return "", "empty proxy url"
        req = urllib.request.Request("https://api.ipify.org", method="GET")
        with opener.open(req, timeout=timeout) as resp:
            return resp.read().decode("utf-8", "replace").strip(), ""
    except Exception as exc:
        return "", str(exc)[:160]


def probe_proxy_intel(proxy_url, timeout=12):
    """Probe one proxy: its exit IP, and where and what that exit is.

    Both steps live here so the discovery list, the per-slot test and the stored
    slot all describe an exit the same way. `latency_ms` covers the proxy probe
    only - the lookup goes out directly and would otherwise inflate it.
    """
    started = time.time()
    exit_ip, error = probe_proxy_exit(proxy_url, timeout=timeout)
    latency_ms = int((time.time() - started) * 1000)
    intel = wb_ipintel.lookup(exit_ip) if exit_ip else wb_ipintel.empty()
    # Two steps, two outcomes. `ok` covers the proxy probe only: the lookup is a
    # separate third-party call that fails on its own (blocked, timeout, 429, a
    # reply without a status). Callers that already know an exit need to tell
    # "the lookup did not answer" from "the lookup answered with nothing", or a
    # blip at the geo service would erase what the last good probe learned.
    intel_ok = any(str(intel.get(field) or "").strip() for field in wb_ipintel.FIELDS)
    return {
        "ok": not error,
        "exit_ip": exit_ip,
        "latency_ms": latency_ms,
        "error": error,
        "intel_ok": intel_ok,
        "country": intel["country"],
        "country_code": intel["country_code"],
        "ip_type": intel["ip_type"],
        "isp": intel["isp"],
        "asn": intel["asn"],
    }


def discover_proxy_slots():
    """Probe the configured mihomo host/ports and report reachable exits."""
    out = []
    for port in _proxy_port_range():
        url = "http://%s:%d" % (PROXY_DISCOVER_HOST, port)
        probe = probe_proxy_intel(url)
        probe["url"] = url
        probe["reachable"] = probe["ok"]
        out.append(probe)
    return out


def slot_label(entry):
    """What to call a slot: the operator's name, else its exit, else its id."""
    name = str((entry or {}).get("name") or "").strip()
    if name:
        return name
    auto = wb_ipintel.slot_name(entry.get("country"), entry.get("ip_type"))
    return auto or str((entry or {}).get("id") or "")


def proxy_slots_view():
    """Proxy slots plus how many enabled accounts are bound to each."""
    counts = {}
    if POOL:
        for account in POOL.accounts:
            slot_id = account.proxy_slot
            if slot_id and account.enabled:
                counts[slot_id] = counts.get(slot_id, 0) + 1
    out = []
    for entry in wb_settings.proxy_slots(ACCOUNTS_DIR):
        item = dict(entry)
        item["bound"] = counts.get(entry["id"], 0)
        item["label"] = slot_label(entry)
        out.append(item)
    return out


_byacct_cache = {"at": 0.0, "data": None}
_byacct_lock = threading.Lock()


def usage_by_account(ttl=None):
    """Cached wrapper: full aggregation over the whole log is expensive.

    Rebuilds under the lock, same reasoning as usage_snapshot."""
    ttl = _STATS_TTL if ttl is None else ttl
    now = time.time()
    with _byacct_lock:
        if _byacct_cache["data"] is not None and (now - _byacct_cache["at"]) < ttl:
            return _byacct_cache["data"]
        data = _usage_by_account_uncached()
        _byacct_cache["at"] = time.time()
        _byacct_cache["data"] = data
    return data


def usage_by_account_etag():
    """usage_by_account() 当前缓存条目的 ETag；没有条目时返回 None。

    这个视图只有一条缓存（没有窗口/区域参数），所以键是常量。配对调用由
    _json_cached() 负责。
    """
    with _byacct_lock:
        if _byacct_cache["data"] is None:
            return None
        built_at = _byacct_cache["at"]
    return _cache_etag("by-account", "all", built_at)


_byacct_state = {"buckets": None, "offset": 0, "key": None, "tail": b""}
# The wrapper above holds _byacct_lock while it rebuilds and then calls in
# here, and _usage_by_account_uncached() is also called straight from the
# bench and the tests; a lock of its own keeps the refresh from re-entering
# the wrapper's - that one is not reentrant, so sharing it hung the process
# on the first cache miss.
_byacct_state_lock = threading.Lock()


def _fold_usage_by_account(row, buckets):
    """Fold one row into the per-account buckets.

    No window and no realm filter here - this view has always described the
    whole log - so the fold only skips the rows the original skipped.
    """
    if row.get("error"):
        return
    key = row.get("account") or "(unattributed)"
    bucket = buckets.get(key)
    if bucket is None:
        bucket = buckets[key] = {
            "account": key, "requests": 0, "prompt_tokens": 0,
            "completion_tokens": 0, "reasoning_tokens": 0,
            "cached_tokens": 0, "total_tokens": 0, "models": {},
        }
    bucket["requests"] += 1
    bucket["prompt_tokens"] += row.get("prompt_tokens") or 0
    bucket["completion_tokens"] += row.get("completion_tokens") or 0
    bucket["reasoning_tokens"] += row.get("reasoning_tokens") or 0
    bucket["cached_tokens"] += row.get("cached_tokens") or 0
    bucket["total_tokens"] += row.get("total_tokens") or 0
    model = row.get("model") or "?"
    models = bucket["models"]
    models[model] = models.get(model, 0) + 1


def _usage_by_account_buckets():
    """The cached per-account fold, refreshed and copied for the caller.

    The copy matters: _usage_by_account_uncached() replaces each bucket's
    `models` dict with a sorted list, and the next refresh must still find the
    counts dict where it left it.
    """
    with _byacct_state_lock:
        log_key = _usage_log_key()
        if _byacct_state["buckets"] is None:
            # 全新状态（进程刚起来）：先问 checkpoint，问不到才从零折起。
            if not _usage_cache_adopt_by_account(_byacct_state, log_key):
                _byacct_state.update({"buckets": {}, "offset": 0, "tail": b""})
        elif not _log_resume_ok(_byacct_state, log_key):
            _byacct_state.update({"buckets": {}, "offset": 0, "tail": b""})
        _byacct_state["key"] = log_key
        offset, error = _scan_usage_from(
            _byacct_state["offset"],
            lambda row: _fold_usage_by_account(row, _byacct_state["buckets"]))
        buckets = _byacct_state["buckets"]
        if error is None:
            _byacct_state["offset"] = offset
            _byacct_state["tail"] = _log_tail_signature(offset)
        else:
            # The row that stopped the fold may have half applied
            # itself, so this view starts over next call - the full
            # scan it replaces does the same on every call.
            log("usage_by_account failed: %s" % error)
            _byacct_state.update({"buckets": {}, "offset": 0, "tail": b""})
        out = {k: dict(v) for k, v in buckets.items()}
    if error is None:
        _usage_cache_maybe_save("by_account", offset)
    return out


def _usage_by_account_uncached():
    """Aggregate the JSONL log per account id.

    Incremental: the fold is carried over and refreshed with the rows appended
    since the last call, so this costs what the new traffic costs instead of
    what the whole log costs.
    """
    buckets = _usage_by_account_buckets()
    out = sorted(buckets.values(), key=lambda b: -b["total_tokens"])
    for item in out:
        item["models"] = sorted(item["models"].items(), key=lambda kv: -kv[1])[:5]
    return out
_analytics_cache = {}
_analytics_lock = threading.Lock()


def _analytics_scope(realm, range, since, until):
    """(since, until, 缓存键) —— compute_usage_analytics() 与它的 ETag 共用。

    注意键里用的是**原始** realm（`realm or "all"`），与这个接口一直以来的键
    一致；折叠时用 realm_scope(realm) 另有其义。两者都不能顺手"统一"，否则就是
    换了一条缓存键。
    """
    lo, hi = range_window(range, since, until)
    cache_key = "%s|%s|%s" % (realm or "all",
                              lo if lo is not None else "", hi if hi is not None else "")
    return lo, hi, cache_key


def compute_usage_analytics(ttl=None, realm=None, range=None, since=None, until=None):
    """Cached analytics payload.

    Unlike perf_stats/usage_snapshot/usage_by_account this used to run
    uncached, re-reading the whole JSONL on every call while the metrics tab
    polls it every 5 seconds. Same shared TTL as its siblings now, and the
    rebuild runs under the lock so parallel pollers do not each scan the log.

    The window joins the cache key for the same reason it does in the other
    readers: the payload's window bucket is what the KPI cards print, and this
    week and this month overlap, so one entry cannot serve both.
    """
    ttl = _STATS_TTL if ttl is None else ttl
    now = time.time()
    lo, hi, cache_key = _analytics_scope(realm, range, since, until)
    with _analytics_lock:
        entry = _analytics_cache.get(cache_key)
        if entry is not None and (now - entry["at"]) < ttl:
            return entry["data"]
        data = _compute_usage_analytics_uncached(realm=realm_scope(realm), since=lo, until=hi)
        _analytics_cache[cache_key] = {"at": time.time(), "data": data}
    return data


def usage_analytics_etag(realm=None, range=None, since=None, until=None):
    """compute_usage_analytics() 当前缓存条目的 ETag；无条目返回 None。

    配对调用由 _json_cached() 负责，理由见那里。
    """
    _lo, _hi, cache_key = _analytics_scope(realm, range, since, until)
    with _analytics_lock:
        entry = _analytics_cache.get(cache_key)
    if entry is None:
        return None
    return _cache_etag("analytics", cache_key, entry["at"])


def _new_analytics_stat():
        return {
            "requests": 0, "errors": 0,
            "prompt_tokens": 0, "completion_tokens": 0, "reasoning_tokens": 0,
            "cached_tokens": 0, "total_tokens": 0,
            "credit": 0.0,
            "cost_cny": 0.0,
            "ttft_sum": 0.0, "ttft_n": 0,
            "speed_sum": 0.0, "speed_n": 0,
            "elapsed_sum": 0.0, "elapsed_n": 0,
        }


# Rows the per-key table folds traffic into when no real key id applies.
# `before` and `anon` are told apart by whether the row carries a `key` field
# at all: rows written before this feature existed have none, and their number
# can only ever shrink, while "no key configured" deployments keep adding rows
# with an empty key. The two need different responses, so they never merge.
KEY_BUCKET_BEFORE = "__before_keys__"
KEY_BUCKET_ANON = "__no_key__"
KEY_BUCKET_UNKNOWN = "__unknown_key__"
# The per-key model breakdown is capped: a deployment with 50 keys would
# otherwise ship a few thousand pills to a page that repaints every 5 seconds.
# The accounts table can afford to list every model because it has one row per
# upstream account, not one per caller.
KEY_MODEL_TOP_N = 5


# One half of the analytics payload folds into four maps: the window summary,
# the per-account entries, the per-model entries and the per-key entries. The
# all-time half and the windowed half hold the same maps and differ only in
# which stat on each entry they own, so the row fold below is written once and
# told which half it is feeding.
_ANALYTICS_ALL = ("all_time", "all_models")
_ANALYTICS_WINDOW = ("window", "window_models")


def _new_analytics_maps():
    """The four maps one half of the payload folds into."""
    return {"summary": _new_analytics_stat(), "accts": {}, "models": {}, "keys": {}}


def _analytics_row_values(row):
    """The per-row numbers the folds below need, read once per row.

    feed()/bump_models() used to call row.get() for each of them on every
    invocation - eight stat objects per row, all walking the same handful of
    fields - which was the single biggest cost of a cold analytics pass.
    """
    return (row.get("prompt_tokens") or 0,
            row.get("completion_tokens") or 0,
            row.get("reasoning_tokens") or 0,
            row.get("cached_tokens") or 0,
            row.get("total_tokens") or 0,
            row.get("credit") or 0,
            row.get("ttft_ms"),
            row.get("tokens_per_sec"),
            row.get("elapsed_ms"))


def _feed_analytics(stat_obj, vals, is_err, cost_cny):
    """Fold one row's numbers into one analytics stat object.

    Token totals follow actual consumption, so a request that failed after the
    upstream had already billed for tokens still shows them; only the
    request/error counters depend on the outcome. An unpriced model passes
    cost_cny = 0.0, which adds nothing: the accumulator starts as a float in
    _new_analytics_stat(), so the sum is exactly what the guarded
    `if cost["known"]` used to leave behind.
    """
    if is_err:
        stat_obj["errors"] += 1
    else:
        stat_obj["requests"] += 1
    stat_obj["prompt_tokens"] += vals[0]
    stat_obj["completion_tokens"] += vals[1]
    stat_obj["reasoning_tokens"] += vals[2]
    stat_obj["cached_tokens"] += vals[3]
    stat_obj["total_tokens"] += vals[4]
    stat_obj["credit"] += vals[5]
    stat_obj["cost_cny"] += cost_cny
    if vals[6]:
        stat_obj["ttft_sum"] += vals[6]
        stat_obj["ttft_n"] += 1
    if vals[7]:
        stat_obj["speed_sum"] += vals[7]
        stat_obj["speed_n"] += 1
    if vals[8]:
        stat_obj["elapsed_sum"] += vals[8]
        stat_obj["elapsed_n"] += 1


def _bump_analytics_models(tgt, m_id, vals, is_err, cost_cny):
    """Fold one row into one {model: {...}} bucket.

    Model distribution counts successful requests only: a failed call
    attributed to a model would show up as demand for it when the caller got
    nothing.
    """
    if is_err:
        return
    tm = tgt.get(m_id)
    if tm is None:
        tm = tgt[m_id] = {"requests": 0, "tokens": 0, "reasoning": 0, "cost_cny": 0.0}
    tm["requests"] += 1
    tm["tokens"] += vals[4]
    tm["reasoning"] += vals[2]
    tm["cost_cny"] += cost_cny


def _fold_usage_analytics(row, realm, maps, half, pricing_on=None, also=None,
                          pricing=None, priced=None):
    """Fold one parsed row into one half of the analytics payload.

    `maps` holds the four maps that half folds into and `half` names the stat
    each entry of that half owns, so the incremental all-time fold (every
    entry's all_time stat) and the windowed pass (the same entries' window
    stat) share one definition of what a row contributes.

    The key axis is separate from the account axis on purpose: one key can be
    served by many upstream accounts, and one account can serve many keys, so
    the two tables are views of the same spend, not a decomposition of it.

    `also` 是同一行还要折进的那几份 (maps, half) 的序列（按日分桶与按小时
    分桶的窗口半）。过滤、算价与字段提取只做一次，各份各自累加：折叠本来
    就要求每侧各记一笔，而 cost_for_row 与 realm 判定是每行最贵的那部分，
    只该付一次。

    `pricing` 是本次扫描开头取到的 _pricing_inputs() 元组（见
    _fold_usage_snapshot 的同名参数）。

    `priced` 是本次扫描要填的「用到的价」记录（见「用到的价」状态指纹一节），
    只有推进状态的那条折叠传它；窗口半/冷扫描传 None。
    """
    if realm and not row_matches_realm(row, realm):
        return
    # Only a genuine gateway/upstream failure is an error. A client
    # cancellation is not: its token counts are incomplete, and folding them
    # into the ratios this page reports would understate cache hit and speed.
    # It is counted in perf_stats instead.
    outcome = row_outcome(row)
    if outcome == "client_aborted":
        return
    is_err = outcome != "completed"
    cost = _row_cost(row, pricing, pricing_on)
    # 折进去的每一行都把「用到的价」记进状态指纹（中止行在上面整行跳过）。
    _priced_note(priced, row, cost, pricing)
    cost_cny = cost["cny"] if cost["known"] else 0.0
    vals = _analytics_row_values(row)
    acct_uid = row.get("account") or "(unattributed)"
    m_id = row.get("model") or "(unknown)"
    # A row written before this feature existed has no `key` field at all; a
    # row from a deployment that never configured a key has one, and it is
    # empty.
    if "key" in row:
        k_id = row.get("key") or KEY_BUCKET_ANON
    else:
        k_id = KEY_BUCKET_BEFORE
    k_realm = row_realm(row) or ""
    at = row.get("at") or 0
    targets = [(maps, half)]
    if also is not None:
        targets.extend(also)
    for tgt, (stat_key, models_key) in targets:
        _feed_analytics(tgt["summary"], vals, is_err, cost_cny)

        entry = tgt["accts"].get(acct_uid)
        if entry is None:
            entry = tgt["accts"][acct_uid] = {
                "uid": acct_uid,
                "nickname": acct_uid,
                "realm": row.get("realm", ""),
                "domain": "",
                stat_key: _new_analytics_stat(),
                models_key: {},
            }
        _feed_analytics(entry[stat_key], vals, is_err, cost_cny)
        _bump_analytics_models(entry[models_key], m_id, vals, is_err, cost_cny)

        model = tgt["models"].get(m_id)
        if model is None:
            model = tgt["models"][m_id] = {"model": m_id,
                                           stat_key: _new_analytics_stat()}
        _feed_analytics(model[stat_key], vals, is_err, cost_cny)

        km = tgt["keys"].get(k_id)
        if km is None:
            km = tgt["keys"][k_id] = {
                "key": k_id,
                stat_key: _new_analytics_stat(),
                models_key: {},
                # realm -> row count. A key bound to one exit only ever sees
                # that exit; a key with no binding follows the model, and its
                # credit column then adds up two different products. Kept as a
                # dict because this ends up in JSON.
                "realms": {},
                "last_at": 0,
            }
        km["realms"][k_realm] = km["realms"].get(k_realm, 0) + 1
        if at and at > km["last_at"]:
            km["last_at"] = at
        _feed_analytics(km[stat_key], vals, is_err, cost_cny)
        _bump_analytics_models(km[models_key], m_id, vals, is_err, cost_cny)


_analytics_state = {}
_analytics_state_lock = threading.Lock()


def _analytics_all_time(realm, pricing_on=None):
    """The cached all-time fold for one realm filter; returns (maps, offset).

    One state per realm: the filter decides which rows the fold ever sees, so
    two realms are two folds and neither may inherit the other's rows. Only
    the all-time half is cached - the windowed half moves with the clock - and
    the offset comes back with it so a windowed pass can stop where the fold
    stopped.
    """
    with _analytics_state_lock:
        state, offset, error = _analytics_state_ready(realm, pricing_on)
        if error is None:
            # Copied under the lock, same rule as the snapshot state: what
            # the caller decorates must not be the live fold.
            maps = _copy_analytics_all_time(state["maps"])
        else:
            # Same rule as the snapshot state: the row that stopped the
            # fold may have half applied itself, so the state starts
            # over next call. The offset still goes back - it is the
            # end of that row - because the windowed half has to see
            # it, and stop there, to describe the same bytes the
            # all-time half already folded.
            log("compute_usage_analytics failed: %s" % error)
            maps = state["maps"]
            state.update({"maps": _new_analytics_maps(), "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "max_at": 0})
    if error is None:
        _usage_cache_maybe_save("analytics", offset)
    return maps, offset


def _analytics_state_advance(realm, pricing_on=None):
    """（不折叠）取/建状态、采纳 checkpoint、判废归零、记指纹。

    调用方必须已持有 _analytics_state_lock。返回本次扫描取到的价表代次
    （与记进状态指纹的是同一份）。状态是否「冷」（这次要付一次全量重折）看
    state["offset"]：> 0 表示有可续折的数据，0 表示折叠要从文件头开始——
    状态刚建、刚被价表 / realm / 日志判废、或 checkpoint 采纳失败。与
    snapshot 侧同一套分工：采纳/判废只有这一段定义，_analytics_state_ready
    与窗口路径的冷热判定（_analytics_state_cold）共用它，两边不会漂移。
    """
    state = _analytics_state.get(realm)
    if state is None:
        state = _analytics_state[realm] = {"maps": _new_analytics_maps(),
                                           "offset": 0, "key": None,
                                           "pricing": None, "realm": None,
                                           "tail": b"", "days": {},
                                           "days_floor": None, "max_at": 0,
                                           "hours_keep": True,
                                           "hits": [], "hits_overflow": False}
    pricing = _pricing_inputs(pricing_on)
    realm_inputs = _realm_inputs()
    log_key = _usage_log_key()
    fresh = _pricing_record_new() if (pricing is not None
                                      and pricing[0]) else None
    if state["key"] is None:
        # 全新状态（进程刚起来）：先问 checkpoint，问不到才从零折起。
        if not _usage_cache_adopt_analytics(realm, state, log_key, pricing, realm_inputs):
            state.update({"maps": _new_analytics_maps(), "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "max_at": 0,
                          "hits": [], "hits_overflow": False, "pricing": fresh})
    elif (not _pricing_used_unchanged(state["pricing"], pricing)
            or state["realm"] != realm_inputs
            or not _log_resume_ok(state, log_key)):
        state.update({"maps": _new_analytics_maps(), "offset": 0, "tail": b"",
                      "days": {}, "days_floor": None, "max_at": 0,
                      "hits": [], "hits_overflow": False, "pricing": fresh})
    state.update({"realm": realm_inputs, "key": log_key})
    return pricing


def _analytics_state_cold(realm, pricing_on=None):
    """窗口请求现在会不会付一次全量重折？True = 会（窗口改走冷扫描）。

    调用方必须已持有 _analytics_state_lock。语义与 _usage_snap_state_cold
    逐条相同：判定不折叠、不写结果，窗口请求不会把状态冻在冷态（offset 仍是
    0，下一个 all-time 半的请求照样从零折到文件尾），重折之后两边字节一致。
    """
    _analytics_state_advance(realm, pricing_on)
    return _analytics_state[realm]["offset"] <= 0


def _analytics_state_ready(realm, pricing_on=None):
    """把 `realm` 的 all-time 折叠推进到文件尾；返回 (state, offset, error)。

    调用方必须已持有 _analytics_state_lock。日桶、小时子表与 all-time 折叠
    共用这一次扫描（见 _fold_usage_analytics 的 also），窗口半与 all-time 半
    因此描述同一段字节。

    `hours_keep` 标记这份状态要维护小时子表（剪枝线比日桶短，见
    _prune_day_buckets）；`max_at` 是折过的最大 at（含 realm 不匹配的行），
    小时窗口靠它判断窗口上界有没有切掉桶里的行。

    状态指纹是「折叠行真正用到的价」记录（T15，见「用到的价」状态指纹一节）：
    折叠时逐行更新（state["pricing"]），扫描成功后刷时间线投影摘要
    （_pricing_record_close），判废走 _pricing_used_unchanged。

    采纳 checkpoint 与判废归零在 _analytics_state_advance 里（窗口路径的
    冷热判定也用那一段），这里只管折到文件尾。
    """
    pricing = _analytics_state_advance(realm, pricing_on)
    state = _analytics_state[realm]
    priced = state["pricing"]

    def fold(row):
        # 一行折三处：all-time 半 + 它所属那天的窗口半 + 那天里它所属那小时
        # 的窗口半（过滤与算价只做一次）。
        at = row.get("at") or 0
        if at > state["max_at"]:
            state["max_at"] = at
        day = _usage_day_bucket(state, row, realm, _day_analytics_bucket)
        hour = _analytics_hour_bucket(day, at)
        if day is None:
            also = None
        elif hour is None:
            also = ((day, _ANALYTICS_WINDOW),)
        else:
            also = ((day, _ANALYTICS_WINDOW), (hour, _ANALYTICS_WINDOW))
        _fold_usage_analytics(
            row, realm, state["maps"], _ANALYTICS_ALL, pricing_on, also=also,
            pricing=pricing, priced=priced)

    offset, error = _scan_usage_from(state["offset"], fold)
    if error is None:
        state["offset"] = offset
        state["tail"] = _log_tail_signature(offset)
        _pricing_record_close(priced, pricing)
    return state, offset, error


def _analytics_window_from_days(realm, day_key, until, pricing_on):
    """窗口半 maps = 窗口内的日桶之和；日桶不可用时返回 None。

    先确保 all-time 折叠推进到文件尾：日桶与它同一次扫描推进，窗口半与
    all-time 半因此描述同一段字节（旧实现用 stop_at 保证的同一件事）。

    until 给出时（闭窗口）它必须是本地午夜（调用方已按格点判过，这里再
    校验一次），窗口只取 at < until 的那批日桶：恰好落在 until 上的行归
    until 那天那一桶（桶按 at 严格切分，见 _usage_hour_bucket），桶切不出
    那一刻的行，所以那一刻有整秒行时整条路退回 None（见
    _closed_window_edge_ok），调用方接着整段扫描，结果不变。

    状态冷（从没折过/刚被价表、realm、日志判废/采纳失败）时直接返回 None：
    那条路要先付一次全量重折，窗口请求不背它——调用方退回按日分组的冷扫描
    （同一批行、同一个求和口径，逐字节相同），重折留给 all-time 半与预热器
    （见 _analytics_state_cold）。
    """
    with _analytics_state_lock:
        if _analytics_state_cold(realm, pricing_on):
            return None
        state, offset, error = _analytics_state_ready(realm, pricing_on)
        if error is not None:
            log("compute_usage_analytics failed: %s" % error)
            state.update({"maps": _new_analytics_maps(), "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "max_at": 0,
                          "hits": [], "hits_overflow": False})
            return None
        if not _days_cover(state, day_key):
            return None
        until_key = None
        if until is not None:
            until_key = _window_day_key_safe(until)
            if until_key is None or not _closed_window_edge_ok(state, until):
                return None
        maps = _new_analytics_maps()
        days = state["days"]
        for key in sorted(days):
            if key < day_key:
                continue
            if until_key is not None and key >= until_key:
                break
            _add_analytics_window_maps(maps, days[key])
    _usage_cache_maybe_save("analytics", offset)
    return maps


# ---------------------------------------------------------------------------
# analytics 的小时层：自定义区间窗口的就近取值
#
# 日桶只拆得开「起点是本地午夜 + until 开放」的窗口；面板的自定义区间起点是
# 任意时刻，那条路径一直整段扫描（路由器上冷 49.4s）。而 analytics 的 token
# 口径含失败行（见 _feed_analytics），与 snapshot/series 不同，借不了别人的
# 层——所以这里在 analytics 自己的日桶里再挂一层小时子表（analytics 算价，
# 小时桶跟着带成本字段），起点落在「本地午夜 + k*3600」格点上的窗口就是窗口
# 内小时桶之和，O(小时数)。
#
# 与日桶同一条纪律：对齐窗口的求和统一用「按小时分组」的口径，冷扫描
# （_scan_usage_analytics_window_hours）、小时桶、加载 checkpoint 三条路径
# 给出的字节完全一致；非整小时的起点（面板按分钟选的时间）拆不开，仍然整段
# 扫描，正确性优先。WB_USAGE_DAY_BUCKETS=0 一并关掉小时层。
#
# 保留比日桶短得多（WB_USAGE_HOUR_KEEP_HOURS，默认 48h）：小时桶只为近期的
# 自定义窗口服务，每个桶的形状与日桶一样（summary + 每账号/每模型/每 key），
# 跟着 32 天的日桶全留会让 checkpoint 膨胀十几倍。剪枝线以外的天由查询侧按
# 「这一天还有没有 hours」自己表达（没有就退回扫描），所以不另存 floor。
# ---------------------------------------------------------------------------


def _analytics_window_from_hours(realm, since, until, pricing_on):
    """窗口半 maps = 窗口内的小时桶之和；小时层不可用时返回 None。

    成立条件（缺一不可，任何一条不成立就退回扫描——正确性优先）：
      * 窗口起点落在本地午夜起算的小时格点上（调用方已判过，这里再按桶校验）；
      * until 开放，或 until 不低于折过的最大 at（否则窗口上界会切掉桶里的行），
        或 until 落在同一套小时格点上且直到那一刻没有整秒行（见
        _closed_window_edge_ok）——这时窗口只取 at < until 的那批桶，恰好
        落在 until 上的行由边沿账挡回整段扫描；
      * days_floor 覆盖窗口起点所在的天（更早的天没有桶）；
      * 窗口起点所在的天及其后每一天都有 hours 子表（被剪枝线剪掉、或本地
        午夜落在不存在的小时里而摘掉的那天，都会让窗口整体退回扫描）。

    状态冷（从没折过/刚被价表、realm、日志判废/采纳失败）时直接返回 None：
    那条路要先付一次全量重折，窗口请求不背它——调用方退回按小时分组的冷
    扫描（同一批行、同一个求和口径，逐字节相同），重折留给 all-time 半与
    预热器（见 _analytics_state_cold）。
    """
    with _analytics_state_lock:
        if _analytics_state_cold(realm, pricing_on):
            return None
        state, offset, error = _analytics_state_ready(realm, pricing_on)
        if error is not None:
            log("compute_usage_analytics failed: %s" % error)
            state.update({"maps": _new_analytics_maps(), "offset": 0, "tail": b"",
                          "days": {}, "days_floor": None, "max_at": 0,
                          "hits": [], "hits_overflow": False})
            return None
        cut_at = None
        if until is not None:
            if until < state["max_at"]:
                # 闭窗口切进桶里：until 必须落在同一套小时格点上、且直到
                # 那一刻没有整秒行，桶才切得出这个窗口。
                if (_window_hour_key_safe(until) is None
                        or not _closed_window_edge_ok(state, until)):
                    return None
                cut_at = int(until)
        day_key = _local_day_key(since)
        if not _days_cover(state, day_key):
            return None
        lo_i = int(since)
        maps = _new_analytics_maps()
        for key in sorted(state["days"]):
            if key < day_key:
                continue
            if cut_at is not None and _day_key_start(key) > cut_at:
                break
            bucket = state["days"][key]
            hours = bucket.get("hours")
            if hours is None:
                return None
            day_at = int(bucket["at"])
            for hour in sorted(hours, key=int):
                sub = hours[hour]
                if int(sub["at"]) != day_at + int(hour) * 3600:
                    return None
                # 格点校验对起点所在那天的每个桶都做（含 at < since 的桶）：
                # 时区偏移不是整小时的夏令时（极少数时区）会让格点整体错开
                # 半小时，错开的桶可能横跨起点、把 at >= since 的行关在里面
                # ——跳过它就会漏行，所以宁可整个窗口退回扫描。
                if (int(sub["at"]) - lo_i) % 3600:
                    return None
                if int(sub["at"]) < lo_i:
                    continue
                if cut_at is not None and int(sub["at"]) >= cut_at:
                    # 闭窗口：until 那一刻归后一桶（桶按 at 严格切分），
                    # 只取 at < until 的桶；那一刻有整秒行时上面已经退回。
                    break
                _add_analytics_window_maps(maps, sub)
    _usage_cache_maybe_save("analytics", offset)
    return maps


def _analytics_window(realm, since, until, stop_at, pricing_on):
    """窗口半 analytics：对齐窗口由日桶/小时桶相加，其余保持整段扫描。

    与 _usage_snapshot_window 同一条规矩，外加一层小时桶：起点是本地午夜、
    终点开放或也是本地午夜的窗口拆得成整天的并集；起点落在整小时的其它窗口
    （自定义区间）拆得成整小时的并集，由 analytics 自己的小时层回答（见
    「小时层」一节）；其余一律保持整段扫描。闭窗口的右边界按「at < until
    的桶」切，那一刻有整秒行时退回扫描（见 _closed_window_edge_ok）。
    """
    day_key = _window_day_key_safe(since)
    until_day = None if until is None else _window_day_key_safe(until)
    if (day_key is not None and _usage_day_buckets_enabled()
            and (until is None or until_day is not None)):
        maps = _analytics_window_from_days(realm, day_key, until, pricing_on)
        if maps is not None:
            return maps
        # 日桶没盖住窗口起点：冷路径现折一份按日分组的窗口半，口径一致。
        return _scan_usage_analytics_window_days(realm, since, until, stop_at,
                                                 pricing_on)
    if _window_hour_aligned(since) and _usage_day_buckets_enabled():
        maps = _analytics_window_from_hours(realm, since, until, pricing_on)
        if maps is not None:
            return maps
        # 小时层没盖住：冷路径现折一份按小时分组的窗口半，口径与小时层一致。
        return _scan_usage_analytics_window_hours(realm, since, until, stop_at,
                                                  pricing_on)
    return _scan_usage_analytics_window(realm, since, until, stop_at=stop_at,
                                        pricing_on=pricing_on)


def _scan_usage_analytics_window(realm, since, until, stop_at=None, pricing_on=None):
    """One pass over the log, folding only the rows inside the window.

    The all-time half is not rebuilt here - it comes from the incremental fold
    - and the read stops where that fold stopped, so both halves of the
    payload describe the same bytes even when a row lands mid-rebuild.

    价表在这里取一次（与 all-time 折叠同一条口径），传给折叠当行级记忆的
    代次：这次窗口扫描里的行可以和 all-time 半共享定价结果。
    """
    pricing = _pricing_inputs(pricing_on)
    maps = _new_analytics_maps()

    def fold(row):
        # Same bounds as /usage and /usage/perf, so the three readers agree on
        # what the selected range contains.
        at = row.get("at", 0)
        if since is not None and at < since:
            return
        if until is not None and at > until:
            return
        _fold_usage_analytics(row, realm, maps, _ANALYTICS_WINDOW, pricing_on,
                              pricing=pricing)

    _, error = _scan_usage_from(
        0, fold, stop_at=stop_at,
        skip=lambda line: _line_outside_window(line, since, until),
        skip_raw=lambda raw: _line_outside_window_bytes(raw, since, until))
    if error is not None:
        log("compute_usage_analytics failed: %s" % error)
    return maps


def _copy_analytics_models(models):
    """Copy one {model: {...}} bucket; its inner dicts are only ever read."""
    return {mid: dict(entry) for mid, entry in models.items()}


def _copy_analytics_all_time(maps):
    """A private copy of the cached all-time maps.

    The payload build decorates what it is given (finalize writes the
    derived ratios into the stat dicts) and the cached fold keeps growing
    under a concurrent refresh, so the caller gets a copy taken while the
    state lock is held rather than a live view of it.
    """
    out = {"summary": dict(maps["summary"]), "accts": {}, "models": {}, "keys": {}}
    for uid, entry in maps["accts"].items():
        copy = dict(entry)
        copy["all_time"] = dict(entry["all_time"])
        copy["all_models"] = _copy_analytics_models(entry["all_models"])
        out["accts"][uid] = copy
    for m_id, entry in maps["models"].items():
        out["models"][m_id] = {"model": m_id, "all_time": dict(entry["all_time"])}
    for k_id, km in maps["keys"].items():
        copy = dict(km)
        copy["all_time"] = dict(km["all_time"])
        copy["all_models"] = _copy_analytics_models(km["all_models"])
        out["keys"][k_id] = copy
    return out


def _merge_analytics_halves(all_maps, win_maps):
    """Combine the cached all-time fold with the windowed one.

    `win_maps` is `all_maps` itself when no range was selected - every row is
    inside an open window, so the windowed half is the all-time fold over
    again. The halves are copied apart in that case, which is what a second
    pass would have produced without the second pass, and it keeps a caller
    that mutates one half out of the other (and out of the cache).

    An entry only the windowed half knows cannot exist: the windowed pass
    stops where the all-time fold stopped, so every row it saw is already in
    there.
    """
    shared = win_maps is all_maps
    acct_map = {}
    for uid, entry in all_maps["accts"].items():
        win = win_maps["accts"].get(uid)
        if shared:
            window_stat = dict(entry["all_time"])
            window_models = _copy_analytics_models(entry["all_models"])
        elif win is None:
            window_stat = _new_analytics_stat()
            window_models = {}
        else:
            window_stat = win["window"]
            window_models = win["window_models"]
        acct_map[uid] = {
            "uid": uid,
            "nickname": uid,
            "realm": entry["realm"],
            "domain": "",
            "window": window_stat,
            "all_time": dict(entry["all_time"]),
            "window_models": window_models,
            "all_models": _copy_analytics_models(entry["all_models"]),
        }
    model_map = {}
    for m_id, entry in all_maps["models"].items():
        stat = entry["all_time"]
        win = win_maps["models"].get(m_id)
        if shared:
            window_stat = dict(stat)
        elif win is None:
            window_stat = _new_analytics_stat()
        else:
            window_stat = win["window"]
        model_map[m_id] = {"model": m_id, "window": window_stat,
                           "all_time": dict(stat)}
    key_map = {}
    for k_id, km in all_maps["keys"].items():
        win = win_maps["keys"].get(k_id)
        if shared:
            window_stat = dict(km["all_time"])
            window_models = _copy_analytics_models(km["all_models"])
        elif win is None:
            window_stat = _new_analytics_stat()
            window_models = {}
        else:
            window_stat = win["window"]
            window_models = win["window_models"]
        key_map[k_id] = {
            "key": k_id,
            "window": window_stat,
            "all_time": dict(km["all_time"]),
            "window_models": window_models,
            "all_models": _copy_analytics_models(km["all_models"]),
            "realms": dict(km["realms"]),
            "last_at": km["last_at"],
        }
    return {"summary": dict(all_maps["summary"]),
            "window_summary": (dict(all_maps["summary"]) if shared
                               else win_maps["summary"]),
            "accts": acct_map, "models": model_map, "keys": key_map}


def _add_analytics_stat(dst, src):
    """Field-wise sum of two _new_analytics_stat() dicts, in place."""
    for field, value in src.items():
        if isinstance(value, (int, float)):
            dst[field] += value


# ---------------------------------------------------------------------------
# 按日分桶：窗口半的增量
#
# all-time 半已经是增量（按字节偏移折叠 + 落盘 checkpoint），但窗口半
# （range=today / week / month）仍然是「从头扫到窗口」：窗口越宽扫得越多，
# 路由器上一次 range=week 要 5 秒。窗口的边界全是本地午夜（range_window()），
# 而每行只属于一天，所以只要在折叠时顺手把每一行折进「它所属那天的桶」，
# 任何窗口就退化成「把窗口内的日桶相加」——O(天数)，checkpoint 带着日桶
# 时冷缓存也是瞬间。
#
# 时间边界只认一个规则：行的本地日由行自己的 at 决定（_local_day_key），
# 与 range_window() 的 today/week/month 用的是同一套本地午夜。折叠时刻不
# 参与切天——迟写/补写的行必须落进它时间戳所属的那天，按折叠时刻切会让
# 窗口相加错位。跨夏令时同理：日桶的键是本地日历日，只要窗口起点是本地
# 午夜，「日键 >= 窗口日键」就是「at >= since」的等价写法（见 _window_day_key）。
#
# 已封口的日桶不会再变（新行只落进今天那一桶），窗口相加时旧桶原样复用；
# 状态只在新建桶时剪枝（_prune_day_buckets），剪掉的只是任何窗口都够不到
# 的天。剪枝会抬高 days_floor，任何够不到的天都因此退回整段扫描——宁可多
# 扫一次，不能少算一天。
#
# analytics 的日桶还挂一层小时子表（保留线更短，只给自定义区间用，见
# 「analytics 的小时层」一节）：那不是日桶的一部分，日桶求和原样忽略它。
#
# 浮点求和的口径：窗口 = 日桶逐日相加，与旧实现「整段扫描逐行累加」的区别
# 只是浮点的结合顺序（credit / cost_cny / speed_sum 可能差最后一两个 ulp，
# 整数字段逐位相同）。逐行顺序无法从日聚合还原，这是日桶推导的固有代价；
# 本文件里所有对齐窗口的路径统一用「按日分组」的口径（冷扫描、日桶、加载
# checkpoint 三条路径给出的字节完全一致），把「像没有缓存一样正确」保成
# 路径之间的一致性；analytics 的小时窗口同理，只是分组的粒度换成小时。
# WB_USAGE_DAY_BUCKETS=0 可整体退回旧的逐行口径。
#
# 小时层（T9）：日桶只服务「起点是本地午夜」的窗口，而面板的自定义区间
# （range=custom）起点是任意时刻——最常见的形态就是整点（datetime-local
# 输入框给的是分钟粒度，操作者选整点的概率远高于选 37 分）。于是每个日桶
# 里再挂一张小时子表（_usage_hour_bucket），起点落在本地整点上的窗口由
# 「整小时桶之和」回答：token/requests/credit 与成本字段（cost_cny /
# cost_missing / by_model_acct）全部来自同一份桶，口径与整段扫描一致，不
# 需要像「借时间序列的小时桶」那样丢掉成本列（series 不算价，D3）。
# 小时层比日桶短命（_usage_hour_keep_days），覆盖不到就退回按小时分组的
# 冷路径——两条路径的字节逐字节相同，语义与改动前一致（只是浮点结合顺序
# 按小时而不是按行）。
# ---------------------------------------------------------------------------

# 日桶保留天数：月窗口最多回溯 30 天（当月 1 号），32 天留两天余量。更早的
# 天没有任何窗口够得到，留着只让 checkpoint 白白变大——路由器上每次写盘都
# 要重写整份文件。调小它（WB_USAGE_DAY_KEEP_DAYS）会让更宽的窗口退回整段
# 扫描（结果仍然正确，只是慢）：对 flash 写入敏感的部署可以调到 8，只保周。
_USAGE_DAY_KEEP_DAYS = 32

# 小时桶保留天数：小时层只服务面板的自定义整点窗口（操作者回头看最近一两
# 天的某个整点），保留 2 天 = 最少 48 小时、最多 72 小时（剪枝按天走，见
# _prune_hour_buckets）。再往前的小时桶没有任何现实窗口够得到，而每个小时
# 桶都是完整的一份窗口半快照（含 by_model_acct 三层结构，实测约 2KB/桶）
# ——路由器上 checkpoint 每多一份都要跟着重写，flash 写入慢且费寿命。覆盖
# 不到的窗口退回冷路径，结果正确，只是慢。WB_USAGE_HOUR_KEEP_DAYS 可调；
# 对 flash 特别敏感的部署可以调到 1（只保今天，24~48 小时）。
_USAGE_HOUR_KEEP_DAYS = 2

# 日桶求和要跳过的字段：started 是每份快照各自的创建时刻，不是累计量。
# 与 _USAGE_CACHE_SNAPSHOT_NUMBERS 同源（同一份构造函数的数字字段），只是
# 在模块加载顺序上不能引用它（那段常量在文件更靠后），所以这里自己推导。
_USAGE_DAY_SNAPSHOT_FIELDS = tuple(
    k for k, v in _empty_stats().items()
    if isinstance(v, (int, float)) and not isinstance(v, bool) and k != "started")


def _usage_day_keep_days():
    """日桶保留天数，默认 32；WB_USAGE_DAY_KEEP_DAYS 可调。

    每次调用都读环境变量，理由与 WB_USAGE_CACHE 相同（现场调、测试里来回切）。
    """
    try:
        return max(1, int(os.environ.get("WB_USAGE_DAY_KEEP_DAYS",
                                         _USAGE_DAY_KEEP_DAYS)))
    except (TypeError, ValueError):
        return _USAGE_DAY_KEEP_DAYS


def _usage_hour_keep_days():
    """小时桶保留天数，默认 2；WB_USAGE_HOUR_KEEP_DAYS 可调。

    同 _usage_day_keep_days：每次调用都读环境变量。取值再小也保留一天
    （max(1, ...)）：小时层至少要能服务「今天的整点窗口」，否则等于没建。
    """
    try:
        return max(1, int(os.environ.get("WB_USAGE_HOUR_KEEP_DAYS",
                                         _USAGE_HOUR_KEEP_DAYS)))
    except (TypeError, ValueError):
        return _USAGE_HOUR_KEEP_DAYS


def _usage_day_buckets_enabled():
    """日桶总开关，默认开；WB_USAGE_DAY_BUCKETS=0 退回逐行扫描口径。

    与 WB_USAGE_CACHE 同样的用法：现场紧急关掉、测试在一个进程里来回切。
    关掉时既不折日桶也不用它们，窗口路径与改动前逐字节一致。小时层挂在
    日桶上（每个日桶里的 hours 子表），所以这个开关一并管住小时层：关掉
    时既不折小时桶，整点窗口也退回逐行口径。
    """
    return os.environ.get("WB_USAGE_DAY_BUCKETS", "1").strip().lower() not in (
        "0", "false", "no", "off")


# analytics 小时子表的保留小时数：小时层只为近期的自定义窗口服务，而每个小时
# 桶的形状与日桶一样重（summary + 每账号/每模型/每 key），跟着 32 天的日桶
# 全留会让 checkpoint 膨胀十几倍。48 小时覆盖「从昨天某个整点起」这类最常用
# 的手动窗口，剪枝又在每天新建日桶时跑一次，实际保留 48~72 小时。调大只是让
# checkpoint 变大（层覆盖更宽），调小只是让更早的自定义窗口退回整段扫描
# （结果仍然正确，只是慢）。
_USAGE_HOUR_KEEP_HOURS = 48


def _usage_hour_keep_hours():
    """小时子表保留小时数，默认 48；WB_USAGE_HOUR_KEEP_HOURS 可调。

    每次调用都读环境变量，理由与 WB_USAGE_DAY_KEEP_DAYS 相同（现场调、测试
    里来回切）。
    """
    try:
        return max(1, int(os.environ.get("WB_USAGE_HOUR_KEEP_HOURS",
                                         _USAGE_HOUR_KEEP_HOURS)))
    except (TypeError, ValueError):
        return _USAGE_HOUR_KEEP_HOURS


def _local_day_key(ts):
    """ts 所属的本地日历日，"YYYY-MM-DD"。

    用字符串而不是午夜 epoch 当日桶的键：窗口与日桶的比较就是一次字符串
    比较（ISO 日期天然有序），查询不必再算一遍本地午夜，也不会被夏令时的
    日长变化影响——同一套本地日历日规则在折叠与查询两侧都成立。

    结果按 ts 记忆（见「行级结果记忆」）：一次冷启动里三个视图各扫一遍日志，
    每一行都要问一次日键，第二、第三遍直接取。时区被 tzset() 换过时表整体
    作废；不可哈希的 ts（坏行）不进表，照旧真算、照旧抛同样的异常。
    """
    memo = _row_day_memo
    if _row_memo_tz is not time.tzname:
        _row_memo_tz_changed()
    try:
        hit = memo.get(ts)
    except TypeError:
        return time.strftime("%Y-%m-%d", time.localtime(ts))
    if hit is not None:
        return hit
    key = time.strftime("%Y-%m-%d", time.localtime(ts))
    cap = _ROW_MEMO_MAX
    if cap > 0:
        if len(memo) >= cap:
            # 满了：只收「不比表里最旧那条更旧」的 ts。扫描从头往尾走，表里
            # 留住的因此总是最近的一批行，第二遍走到尾部照样命中。
            oldest = next(iter(memo))
            if _row_memo_at(ts) < _row_memo_at(oldest):
                return key
            memo.pop(oldest, None)
        memo[ts] = key
    return key


def _day_key_start(key):
    """日键 -> 该日的本地午夜 epoch。"""
    lt = time.strptime(key, "%Y-%m-%d")
    return time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday, 0, 0, 0, 0, 0, -1))


def _window_day_key(since):
    """窗口起点能由日桶推导时返回它的日键，否则 None。

    只有「起点正好是本地午夜」的窗口才拆得成整天的并集：at >= 本地午夜
    等价于「日键 >= 那天的键」。自定义区间的起点是任意时刻，拆不开——那条
    路径保持整段扫描（正确性优先）。
    """
    if since is None:
        return None
    if _local_midnight(since) != since:
        return None
    return _local_day_key(since)


def _hour_window_key(since):
    """窗口起点能由小时桶推导时返回 (日键, 小时序号)，否则 None。

    只有「起点正好是本地整点」的窗口才拆得成整小时桶的并集：小时桶按
    「离那天本地午夜多少个 3600 秒」切格点（见 _usage_hour_bucket），所以
    起点必须同时满足两条——本地墙上时间落在整点（_local_hour_start 能原样
    还原），并且离本地午夜的距离是 3600 的整数倍。夏令时把日边界挪动、
    或时区按半小时偏移的情形，只要 since 不落在格点上就返回 None（退回
    扫描，正确性优先）。

    整点起点也可能是本地午夜（两者同时成立）：调用方先问日桶，所以这条
    只在「不是午夜」时兜住自定义区间的整点窗口。
    """
    if since is None:
        return None
    if _local_hour_start(since) != since:
        return None
    day_key = _local_day_key(since)
    day_at = _day_key_start(day_key)
    k = int((since - day_at) // 3600)
    if day_at + k * 3600 != since:
        return None
    return day_key, k


def _window_hour_aligned(since):
    """窗口起点是否落在「本地午夜 + k*3600」的小时格点上。

    小时桶的格点是「离那天的本地午夜多少秒」（见 _analytics_hour_bucket），
    所以起点减它所属那天的本地午夜必须是 3600 的整数倍。自定义区间的起点是
    任意分钟时拆不开——那条路径保持整段扫描（正确性优先）；起点正好是本地
    午夜时恒成立（0 是 3600 的整数倍），与 _window_day_key 相容。
    """
    if since is None:
        return False
    return (_local_midnight(since) - since) % 3600 == 0


# 闭窗口边沿账的上限：只有「at 恰好落在整秒上」的行进账，真实日志里几乎为
# 零（at 是 time.time() 的浮点秒，落在整秒上的概率约百万分之一）。手改过的
# 日志可能有很多，超过上限就停记并留痕——闭窗口退回整段扫描，正确性优先。
_CLOSED_WINDOW_HITS_MAX = 256


def _closed_window_hit_record(state, at):
    """行恰好落在整秒上时记一笔（闭窗口走桶求和的边沿账）。

    桶按 at 严格切分（见 _usage_hour_bucket）：闭窗口 [since, until] 与
    「at < until 的那批桶」只差「恰好落在 until 上的行」，所以直到那一刻
    没有整秒行时，桶的和就是窗口的全部（见 _closed_window_edge_ok）。这份
    账只收真正进了桶的那些行（realm/剪枝线与折叠同一套判定），记录点因此
    在 _usage_day_bucket 的过滤之后。

    at 不是有限数（手改日志的 Infinity/NaN）时整秒判定不成立就跳过——
    `at // 1` 对这些值不抛异常，不把一次折叠打断在这里。
    """
    if at != at // 1:
        return
    hits = state["hits"]
    if len(hits) >= _CLOSED_WINDOW_HITS_MAX:
        state["hits_overflow"] = True
        return
    hits.append(int(at))


def _closed_window_edge_ok(state, until):
    """闭窗口的右边界能否由存量桶回答：直到 until 为止没有行落在整秒上。

    直到那一刻没有整秒行时，「[since, until] 的行」与「at < until 的桶」是
    同一批（桶按 at 严格切分，恰好落在 until 上的行归后一桶）；真有一行
    恰好落在 until 上时桶切不出它，只能退回整段扫描。账记满（见
    _closed_window_hit_record）同样退回。
    """
    if state["hits_overflow"]:
        return False
    return until not in state["hits"]


def _window_day_key_safe(ts):
    """_window_day_key 的取值安全版：取值离谱时返回 None。

    手写的极大 since/until（例如 6e18）会让 _local_midnight 里的
    time.localtime 抛 OSError——那只是「这个窗口服务不了」的一种，按退回
    整段扫描处理，不该把端点打成 500（与 _series_window_day_key 同一个用意）。
    """
    try:
        return _window_day_key(ts)
    except (OSError, OverflowError, ValueError):
        return None


def _window_hour_key_safe(ts):
    """_hour_window_key 的取值安全版：取值离谱时返回 None（同上）。"""
    try:
        return _hour_window_key(ts)
    except (OSError, OverflowError, ValueError):
        return None


def _new_window_snapshot_bucket(at):
    """一份带格点 at 的窗口半快照桶（日桶与小时桶共用这个形状）。

    started 不参与求和（见 _add_usage_snapshot），形状保留只是让校验与
    all-time 半共用同一套 _usage_cache_snapshot_ok。at 是这个桶的本地格点
    （日的午夜 / 小时的整点）epoch：小时桶按它切 3600 的格点，求和侧按它
    做格点校验——与时间序列的桶同一套做法（_new_series_bucket）。
    """
    bucket = _empty_stats()
    bucket["at"] = at
    return bucket


def _day_snapshot_bucket(key, day_start):
    """一天一份的窗口半快照桶；形状与 _empty_stats() 一致。"""
    return _new_window_snapshot_bucket(day_start)


def _day_analytics_bucket(key, day_start):
    """一天一份的窗口形状 analytics maps（带 at 与小时子表，见「小时层」一节）。

    at 是那天的本地午夜：小时子表按「离本地午夜多少秒」切，行的归属因此
    不依赖折叠时刻（迟写/补写的行落回它时间戳所属的那天那小时）。
    """
    bucket = _new_analytics_maps()
    bucket["at"] = day_start
    bucket["hours"] = {}
    return bucket


def _new_analytics_hour_bucket(at):
    """一小时一份的窗口形状 analytics maps（带 at；见「小时层」一节）。"""
    bucket = _new_analytics_maps()
    bucket["at"] = at
    return bucket


def _analytics_hour_bucket(day, at):
    """这一行该折进的小时桶（day 桶的 hours 子表）；不该折时返回 None。

    小时键是「离那天的本地午夜多少秒」的整小时数（字符串，与 JSON 往返后的
    形状一致，同时间序列的小时子表）。本地午夜落在不存在的小时里（夏令时在
    午夜切换的极少数时区）时这一行放不进格点：把整天的 hours 摘掉，让覆盖
    这一天的小时窗口整体退回扫描（日桶本身照常，不受影响）。
    """
    if day is None:
        return None
    hours = day.get("hours")
    if hours is None:
        return None
    day_at = int(day["at"])
    if at < day_at:
        day.pop("hours", None)
        return None
    hour = str(int((at - day_at) // 3600))
    sub = hours.get(hour)
    if sub is None:
        sub = hours[hour] = _new_analytics_hour_bucket(day_at + int(hour) * 3600)
    return sub


def _day_series_bucket(key, day_start):
    """一天一份的时间序列桶（带小时子表，见 _fold_usage_series）。"""
    bucket = _new_series_bucket(day_start)
    bucket["hours"] = {}
    return bucket


def _usage_day_bucket(state, row, r, factory):
    """这一行该折进哪个日桶；必要时新建（顺带剪枝），不该折时返回 None。

    行的归属只看它自己的 at：补写/迟到的行必须落进它时间戳所属的那天。
    realm 不匹配的行折叠本来就会跳过，这里也不为它建桶；总开关关掉时连
    日桶都不折（窗口路径也不会用它们，见 _usage_day_buckets_enabled）。
    """
    if not _usage_day_buckets_enabled():
        return None
    if r and not row_matches_realm(row, r):
        return None
    at = row.get("at") or 0
    key = _local_day_key(at)
    floor = state.get("days_floor")
    if floor is not None and key < floor:
        # 剪枝线以下的天：那些桶已经被丢掉，而且任何可服务的窗口都从
        # days_floor 那天起，这一行不在其中——跳过不影响窗口。
        return None
    _closed_window_hit_record(state, at)
    days = state["days"]
    bucket = days.get(key)
    if bucket is None:
        bucket = days[key] = factory(key, _day_key_start(key))
        _prune_day_buckets(state, days, key)
    return bucket


def _prune_day_buckets(state, days, newest_key):
    """丢掉任何窗口都够不到的旧日桶，并把 days_floor 抬到剪枝线。

    days_floor 的语义是「这份状态从哪天起是完整的」：它就是剪枝线（最新那天
    往前 _usage_day_keep_days() 天），没剪过时第一次折到新的一天也会把它记
    下来——比这条线更早的窗口一律退回整段扫描（时钟大幅回拨、日志里有远古
    补写行这类情况），宁可多扫一次，不能少算一天。

    analytics 状态还挂着一条更短的小时子表保留线（state["hours_keep"]）：
    小时层只为近期的自定义窗口服务，只摘旧天的 "hours" 子表，日桶本身照常
    留满保留天数。被摘掉的天由查询侧按「这一天还有没有 hours」自己表达
    （见 _analytics_window_from_hours），所以这里不用另存一个 floor。
    """
    horizon = _local_day_key(_day_key_start(newest_key)
                             - _usage_day_keep_days() * 86400)
    for old in [k for k in days if k < horizon]:
        del days[old]
    floor = state.get("days_floor")
    if floor is None or floor < horizon:
        state["days_floor"] = horizon
    # 带小时层的状态（snapshot，见 _usage_hour_bucket）顺带剪小时表：小时层
    # 的保留期比日桶短，剪枝线要跟着一起抬。analytics / series 的状态没有
    # hours_floor_at 这个键，不会被这条带到。
    if "hours_floor_at" in state:
        _prune_hour_buckets(state, days, newest_key)
    # analytics 状态还挂着一条更短的小时子表保留线（state["hours_keep"]）：
    # 小时层只为近期的自定义窗口服务，只摘旧天的 "hours" 子表，日桶本身照常
    # 留满保留天数。
    if state.get("hours_keep"):
        hour_horizon = _local_day_key(_day_key_start(newest_key)
                                      - _usage_hour_keep_hours() * 3600)
        for key, bucket in days.items():
            if key < hour_horizon:
                bucket.pop("hours", None)


def _days_cover(state, day_key):
    """这份状态的日桶能否服务以 day_key 为起点的窗口。

    days_floor 是剪枝线：比它早的窗口可能少了被剪掉的天，只能退回扫描。
    它最早也要折到第一个新的一天才会记下（没折过就是 None，此时谁都不服务
    ——调用方总是先折到文件尾，所以实际不会遇到）。
    """
    floor = state.get("days_floor")
    return floor is not None and day_key >= floor


def _usage_hour_bucket(state, row, day):
    """这一行该折进哪个小时桶；不该折时返回 None。

    小时桶挂在日桶里（`hours` 子表），键是「离当天本地午夜的 3600 秒个数」
    的字符串——与时间序列的小时子桶同一套格点规则（见 _fold_usage_series）：
    桶的 at = 日桶 at + 序号*3600，所以每个桶恰好覆盖一个整点区间，跨桶
    求和不会重复也不会漏。

    at 早于日桶 at 的行（极少数时区在本地午夜切换夏令时，午夜那一刻不存在）
    放不进格点：整张小时表摘掉并留痕（`hours` 记为 None），求和侧看到没有表
    就退回按小时分组的冷路径（日桶本身照常，日窗口不受影响）。留痕是为了
    挡住「摘掉之后又来了这一天的行、把表重建出来」——重建的表缺了被摘掉的
    那批行，一旦被求和侧选中就是少算。

    hours_floor_at 是小时层的剪枝线：比它早的天不再折小时表。迟写/补写的
    行可能落回很久以前那天（日桶还留着），但那张表早被剪掉，重新建起来
    只会得到一张缺了其余小时的表——求和侧一旦用它就会少算，所以这里直接
    不折（求和侧另有 _hours_cover 兜底）。
    """
    floor = state.get("hours_floor_at")
    if floor is None or day["at"] < floor:
        return None
    at = row.get("at") or 0
    day_at = day["at"]
    hours = day.get("hours")
    if hours is None and "hours" in day:
        # 这天的小时表被摘过（午夜兜底）：不再重建，见 docstring。
        return None
    if at < day_at:
        day["hours"] = None
        return None
    if hours is None:
        hours = day["hours"] = {}
    hour = str(int((at - day_at) // 3600))
    sub = hours.get(hour)
    if sub is None:
        sub = hours[hour] = _new_window_snapshot_bucket(day_at + int(hour) * 3600)
    return sub


def _prune_hour_buckets(state, days, newest_key):
    """丢掉保留窗口之外的日桶里的小时表，并把 hours_floor_at 抬到剪枝线。

    小时层比日桶短命：它只服务最近几天的整点窗口（见 _USAGE_HOUR_KEEP_DAYS）。
    剪枝按天走（与 _prune_day_buckets 同一套算术），所以实际保留在
    keep_days 与 keep_days+1 天之间。hours_floor_at 的语义与 days_floor 相同
    ——「这份状态的小时层从哪个时刻起是完整的」——更早的整点窗口一律退回
    冷路径，宁可多扫一次，不能少算一个桶。
    """
    horizon_at = _day_key_start(newest_key) - _usage_hour_keep_days() * 86400
    horizon = _local_day_key(horizon_at)
    for key in list(days):
        if key < horizon and "hours" in days[key]:
            # 与午夜兜底同一个留痕：表置 None 而不是删键。剪枝线（下面那条）
            # 本来也挡着重建，但留痕让「这张表不可用」成为桶自己的事实，
            # 求和侧与折叠侧不必再依赖两条线的一致性。
            days[key]["hours"] = None
    floor = state.get("hours_floor_at")
    if floor is None or floor < horizon_at:
        state["hours_floor_at"] = horizon_at


def _hours_cover(state, since):
    """这份状态的小时层能否服务起点为 `since` 的窗口。

    hours_floor_at 是剪枝线（见 _prune_hour_buckets）：比它早的窗口可能少了
    被剪掉的小时表，只能退回扫描。它最早也要折到第一个新的一天才会记下
    （没折过就是 None，此时谁都不服务——调用方总是先折到文件尾）。
    """
    floor = state.get("hours_floor_at")
    return floor is not None and since >= floor


def _hours_floor_on_adopt(days, stored):
    """采用 checkpoint 时的小时层剪枝线：取「文件里的」与「按最新一天现算」
    的较晚者。

    较晚者一定安全：剪枝线只会往前抬，抬高了最多是少服务几天（那些窗口
    退回冷路径），永远不会服务一张被剪过的表。两条都挡住同一条路——拿
    不完整的表当完整（那是这个层唯一会算错数的方向）：文件里那条可能是
    别的保留天数下写的，现算这条跟着当前 WB_USAGE_HOUR_KEEP_DAYS 走。
    """
    recomputed = None
    if days:
        recomputed = _day_key_start(max(days)) - _usage_hour_keep_days() * 86400
    if stored is None:
        return recomputed
    if recomputed is None:
        return stored
    return max(stored, recomputed)


def _add_usage_bucket(dst_map, key, src):
    """把一个 by_model* 桶并入同键的桶（不存在就整只拷进来）。"""
    dst = dst_map.get(key)
    if dst is None:
        dst_map[key] = _copy_usage_bucket(src)
        return
    for field in _USAGE_CACHE_BUCKET_NUMBERS:
        dst[field] += src.get(field) or 0
    for acct, count in src["accounts"].items():
        dst["accounts"][acct] = dst["accounts"].get(acct, 0) + count


def _add_usage_snapshot(dst, src):
    """把一份窗口半快照按字段并入另一份（日桶求和用）。

    逐字段相加，不是重折：日桶是不可变的聚合，窗口只是它们的和。started
    是每份快照各自的创建时刻，不是累计量，跳过。
    """
    for field in _USAGE_DAY_SNAPSHOT_FIELDS:
        dst[field] += src.get(field) or 0
    for mid, count in src["cost_missing"].items():
        dst["cost_missing"][mid] = dst["cost_missing"].get(mid, 0) + count
    for mid, bucket in src["by_model"].items():
        _add_usage_bucket(dst["by_model"], mid, bucket)
    for mid, realms in src["by_model_realm"].items():
        for rr, bucket in realms.items():
            _add_usage_bucket(dst["by_model_realm"].setdefault(mid, {}), rr, bucket)
    for mid, realms in src["by_model_acct"].items():
        for rr, accts in realms.items():
            tgt = dst["by_model_acct"].setdefault(mid, {}).setdefault(rr, {})
            for acct, bucket in accts.items():
                _add_usage_bucket(tgt, acct, bucket)


def _add_analytics_models(dst, src):
    """把一份 {model: {requests, tokens, reasoning, cost_cny}} 并入另一份。"""
    for mid, stat in src.items():
        tgt = dst.get(mid)
        if tgt is None:
            dst[mid] = dict(stat)
            continue
        for field in ("requests", "tokens", "reasoning", "cost_cny"):
            tgt[field] += stat.get(field) or 0


def _add_analytics_window_maps(dst, src):
    """把一份窗口形状的 analytics maps 并入另一份（日桶求和用）。

    日桶本身就是 _ANALYTICS_WINDOW 那一半的形状（window / window_models），
    所以这里只动这两个键；realms / last_at 照并，虽然 _merge_analytics_halves
    最终取的是 all-time 侧的值。
    """
    _add_analytics_stat(dst["summary"], src["summary"])
    for uid, entry in src["accts"].items():
        tgt = dst["accts"].get(uid)
        if tgt is None:
            tgt = dst["accts"][uid] = {
                "uid": uid, "nickname": uid, "realm": entry["realm"],
                "domain": "", "window": _new_analytics_stat(),
                "window_models": {}}
        _add_analytics_stat(tgt["window"], entry["window"])
        _add_analytics_models(tgt["window_models"], entry["window_models"])
    for mid, entry in src["models"].items():
        tgt = dst["models"].get(mid)
        if tgt is None:
            tgt = dst["models"][mid] = {"model": mid,
                                        "window": _new_analytics_stat()}
        _add_analytics_stat(tgt["window"], entry["window"])
    for kid, entry in src["keys"].items():
        tgt = dst["keys"].get(kid)
        if tgt is None:
            tgt = dst["keys"][kid] = {
                "key": kid, "window": _new_analytics_stat(),
                "window_models": {}, "realms": {}, "last_at": 0}
        _add_analytics_stat(tgt["window"], entry["window"])
        _add_analytics_models(tgt["window_models"], entry["window_models"])
        for rr, count in entry["realms"].items():
            tgt["realms"][rr] = tgt["realms"].get(rr, 0) + count
        if entry["last_at"] > tgt["last_at"]:
            tgt["last_at"] = entry["last_at"]


def _copy_analytics_window_maps(maps):
    """一份窗口形状 maps 的深拷贝（落盘用，形状同 _new_analytics_maps()）。

    日桶/小时桶还会多带 at（两者都有）与 hours 子表（日桶）：照原样拷贝、
    键序保持折叠时的插入顺序——checkpoint 不排序，加载回来要与冷启动逐字节
    一致（见 _usage_cache_write）。
    """
    out = {"summary": dict(maps["summary"]), "accts": {}, "models": {}, "keys": {}}
    for uid, entry in maps["accts"].items():
        copy = dict(entry)
        copy["window"] = dict(entry["window"])
        copy["window_models"] = _copy_analytics_models(entry["window_models"])
        out["accts"][uid] = copy
    for mid, entry in maps["models"].items():
        out["models"][mid] = {"model": mid, "window": dict(entry["window"])}
    for kid, km in maps["keys"].items():
        copy = dict(km)
        copy["window"] = dict(km["window"])
        copy["window_models"] = _copy_analytics_models(km["window_models"])
        copy["realms"] = dict(km["realms"])
        out["keys"][kid] = copy
    if "at" in maps:
        out["at"] = maps["at"]
    hours = maps.get("hours")
    if hours is not None:
        out["hours"] = {h: _copy_analytics_window_maps(sub)
                        for h, sub in hours.items()}
    return out


def _scan_usage_snapshot_window_days(r, since, until, pricing_on=None):
    """窗口半的冷路径：整段扫一遍，先按日分桶再相加。

    日桶状态不可用（还没折过、被剪枝、折叠出错）时走这里。结果必须与日桶
    状态给出的窗口半逐字节一致：窗口起点是本地午夜（until 给出时终点也是
    本地午夜），两边装的是同一批行、同一个「按日分组」的求和口径，只是桶
    一个现折、一个存量。

    状态冷时它是窗口请求的直接答案（不折状态，重折留给 all-time 视图与
    预热器，见 _usage_snap_state_cold）；其余情形它紧跟一次 all-time 折叠
    （_usage_snapshot_window_from_days 先折到文件尾才发现日桶不够用），
    行级记忆几乎全中：同一批行刚在 all-time 半算过价与日键。窗口外的行由
    skip 挡在解析之前，这条路只付窗口内那批行。价表照样在本函数开头取一次。
    """
    pricing = _pricing_inputs(pricing_on)
    days = {}

    def fold(row):
        at = row.get("at") or 0
        if since and at < since:
            return
        if until and at > until:
            return
        key = _local_day_key(at)
        bucket = days.get(key)
        if bucket is None:
            bucket = days[key] = _empty_stats()
        _fold_usage_snapshot(row, bucket, r, None, None, pricing_on,
                             pricing=pricing)

    _, error = _scan_usage_from(
        0, fold,
        skip=lambda line: _line_outside_window(line, since or None,
                                               until or None),
        skip_raw=lambda raw: _line_outside_window_bytes(raw, since or None,
                                                        until or None))
    if error is not None:
        log(f"usage snapshot read failed: {error}")
    snap = _empty_stats()
    for key in sorted(days):
        _add_usage_snapshot(snap, days[key])
    return snap


def _scan_usage_snapshot_window_hours(r, since, until, pricing_on=None):
    """窗口半的冷路径：整段扫一遍，先按小时分桶再相加。

    小时层状态不可用（还没折过、被剪枝、折叠出错、那天的格点不成立）时走
    这里。结果必须与小时桶状态给出的窗口半逐字节一致：窗口起点是本地整点
    （until 给出时终点也落在同一套格点上），两边装的是同一批行、同一个
    「按（日，小时）分组」的求和口径，只是桶一个现折、一个存量。

    分组与 _usage_hour_bucket 同一套算术（日键 + 离午夜多少个 3600 秒），
    但这里对 at 早于本地午夜的行照折不误（键可以是负的）：状态侧那种行会
    把整张小时表摘掉、窗口退回这条路径，所以这条路径必须把它们算进去。
    排序按 (日键, 序号) 数值序，与状态侧的遍历顺序一致。

    价表在本函数开头取一次，行级记忆（见「行级结果记忆」）跟着这一批扫描
    走：同一批行通常刚在 all-time 半算过价与日键，这里几乎全中。
    """
    pricing = _pricing_inputs(pricing_on)
    hours = {}
    day_ats = {}

    def fold(row):
        at = row.get("at") or 0
        if since and at < since:
            return
        if until and at > until:
            return
        key = _local_day_key(at)
        day_at = day_ats.get(key)
        if day_at is None:
            day_at = day_ats[key] = _day_key_start(key)
        hour = int((at - day_at) // 3600)
        bucket = hours.get((key, hour))
        if bucket is None:
            bucket = hours[(key, hour)] = _empty_stats()
        _fold_usage_snapshot(row, bucket, r, None, None, pricing_on,
                             pricing=pricing)

    _, error = _scan_usage_from(
        0, fold,
        skip=lambda line: _line_outside_window(line, since or None,
                                               until or None),
        skip_raw=lambda raw: _line_outside_window_bytes(raw, since or None,
                                                        until or None))
    if error is not None:
        log(f"usage snapshot read failed: {error}")
    snap = _empty_stats()
    for key in sorted(hours):
        _add_usage_snapshot(snap, hours[key])
    return snap


def _scan_usage_analytics_window_days(realm, since, until, stop_at=None,
                                      pricing_on=None):
    """窗口半 analytics 的冷路径：按日分桶再相加（见 snapshot 版）。"""
    pricing = _pricing_inputs(pricing_on)
    days = {}

    def fold(row):
        at = row.get("at") or 0
        if since is not None and at < since:
            return
        if until is not None and at > until:
            return
        key = _local_day_key(at)
        maps = days.get(key)
        if maps is None:
            maps = days[key] = _new_analytics_maps()
        _fold_usage_analytics(row, realm, maps, _ANALYTICS_WINDOW, pricing_on,
                              pricing=pricing)

    _, error = _scan_usage_from(
        0, fold, stop_at=stop_at,
        skip=lambda line: _line_outside_window(line, since, until),
        skip_raw=lambda raw: _line_outside_window_bytes(raw, since, until))
    if error is not None:
        log("compute_usage_analytics failed: %s" % error)
    out = _new_analytics_maps()
    for key in sorted(days):
        _add_analytics_window_maps(out, days[key])
    return out


def _scan_usage_analytics_window_hours(realm, since, until, stop_at=None,
                                       pricing_on=None):
    """窗口半 analytics 的冷路径（起点整小时）：按小时分桶再相加。

    小时层不可用（还没折过、被剪枝、折叠出错、until 切在桶中间）时走这里。
    结果必须与小时桶状态给出的窗口半逐字节一致：两边装的是同一批行、同一个
    「按小时分组」的求和口径，只是桶一个现折、一个存量。

    分桶与折叠侧同一条规则（离那天的本地午夜多少秒）；本地午夜落在不存在
    的小时里时这一天的小时键会变成负数，求和照样把它算进去（分组只是浮点
    结合顺序的载体），所以这条冷路径不依赖格点是否成立。

    价表在本函数开头取一次，行级记忆（见「行级结果记忆」）跟着这一批扫描走。
    """
    pricing = _pricing_inputs(pricing_on)
    days = {}

    def fold(row):
        at = row.get("at") or 0
        if since is not None and at < since:
            return
        if until is not None and at > until:
            return
        key = _local_day_key(at)
        day = days.get(key)
        if day is None:
            day = days[key] = (_day_key_start(key), {})
        day_at, hours = day
        hour = str(int((at - day_at) // 3600))
        maps = hours.get(hour)
        if maps is None:
            maps = hours[hour] = _new_analytics_maps()
        _fold_usage_analytics(row, realm, maps, _ANALYTICS_WINDOW, pricing_on,
                              pricing=pricing)

    _, error = _scan_usage_from(
        0, fold, stop_at=stop_at,
        skip=lambda line: _line_outside_window(line, since, until),
        skip_raw=lambda raw: _line_outside_window_bytes(raw, since, until))
    if error is not None:
        log("compute_usage_analytics failed: %s" % error)
    out = _new_analytics_maps()
    for key in sorted(days):
        _day_at, hours = days[key]
        for hour in sorted(hours, key=int):
            _add_analytics_window_maps(out, hours[hour])
    return out


# ---------------------------------------------------------------------------
# 时间序列的日/小时/分钟桶
#
# usage_timeseries() 本来就把行按时间装桶，但每次缓存未命中都要整段扫描
# （路由器上 range=week 约 2 秒）。窗口边界同样是本地午夜，而它的桶宽只有
# 3600 与 86400 两种能与窗口对齐；span<=6h 的窗口（自定义区间，选择器
# 见 _usage_timeseries_scope）给出 step=60，边界是任意分钟格点，日/小时桶
# 都答不了——这一档由「最近 6 小时的分钟环」回答（见 _fold_usage_series_minute）。
# 折叠时顺手维护这三层，窗口就是它们的切片：每个输出桶恰好等于一个存量桶
# （不做跨桶相加），所以连浮点求和都是逐位一致的——不像 snapshot/analytics
# 的窗口半需要把日桶相加。
# ---------------------------------------------------------------------------
_series_state = {}
_series_state_lock = threading.Lock()
# credit 历史只保留最近这么多条（整段扫描返回的也是窗口内的前 50 条）。
_SERIES_CREDITS_KEEP = 50
# 分钟环的保留窗口：6 小时，与选择器给出 step=60 的最大跨度（span<=6h）一一
# 对应。分钟桶比小时桶密 60 倍，跟着日桶的 32 天保留会让 checkpoint 翻好几倍，
# 而更早的窗口本来就该由小时层（step=3600）回答——所以环只留最近 6 小时。
# 保留线用「桶的 at 严格小于 max_at - 窗口」判定，窗口服务条件写成
# lo >= max_at - 窗口；两者一配对，服务得上的窗口需要的桶恰好一个不落
# （窗口起点必须落在分钟格点上，见 _series_minute_grid_ok）。
_SERIES_MINUTE_WINDOW = 6 * 3600
# 折叠途中环的临时上限：超过就按保留线剪一次。剪枝是 O(环大小)、而环的
# 正常大小 <= 361（6 小时 + 两个端点），720 是给「冷折时环里还堆着更早的
# 桶」留的余量——这样整趟冷折的内存有界（满环约 200KB），不用等折完才剪。
_SERIES_MINUTE_RING_CAP = 720


def _new_series_bucket(at):
    """一个时间序列桶；键序与 _usage_timeseries_uncached 的输出逐字一致。"""
    return {"at": at, "requests": 0, "errors": 0, "prompt_tokens": 0,
            "completion_tokens": 0, "reasoning_tokens": 0, "cached_tokens": 0,
            "total_tokens": 0, "credit": 0.0}


def _feed_series_bucket(bucket, row):
    """一行折进一个序列桶（日桶或小时桶）。

    口径与 _usage_timeseries_uncached 的折叠逐条一致：completed 计 requests
    与令牌，其它结果（含客户端中止）计 errors；credit 只要不是客户端中止就
    累加。
    """
    outcome = row_outcome(row)
    if outcome == "completed":
        bucket["requests"] += 1
        for field in ("prompt_tokens", "completion_tokens", "reasoning_tokens",
                      "cached_tokens", "total_tokens"):
            bucket[field] += (row.get(field) or 0)
    else:
        bucket["errors"] += 1
    if outcome != "client_aborted":
        bucket["credit"] += (row.get("credit") or 0)


def _series_credit_entry(at, row):
    """一条计费行的载荷：整段扫描与增量缓冲共用的字段表。

    两边的条目会出现在同一个响应的 credits 数组里，键序也是响应字节的
    一部分，所以只能有一份定义。
    """
    return {"at": at, "iso": row.get("iso") or "",
            "model": row.get("model") or "",
            "account": row.get("account") or "",
            "credit": row.get("credit") or 0,
            "total_tokens": row.get("total_tokens") or 0}


def _series_credit_push(credits, row, at):
    """把一条带 credit 的行放进「最近 50 条」缓冲。

    整段扫描返回的 credits 是窗口内按 at 倒序的前 50 条；缓冲里始终留着
    最新的一批（至少 50 条，涨到 100 条时裁回 50），于是任何后缀窗口
    （today/week/month 都是）的前 50 条都在这批里——比缓冲最旧一条还旧的
    行不可能是窗口的前 50 条。at 相同时两边都是稳定排序，顺序（日志顺序）
    一致。
    """
    credits.append(_series_credit_entry(at, row))
    if len(credits) > _SERIES_CREDITS_KEEP * 2:
        credits.sort(key=lambda item: item.get("at") or 0, reverse=True)
        del credits[_SERIES_CREDITS_KEEP:]


def _series_minute_key(at):
    """行 -> 它所属的分钟桶键：epoch 分钟（向下取整）。

    键用 epoch 分钟而不是「离本地午夜多少秒」：分钟窗口的起点是任意分钟格点
    （自定义区间的 since），不锚在本地午夜上，所以桶的格点也取绝对分钟；
    这样跨午夜、跨小时都是同一张表，切片不必按天分组。
    """
    return int(at // 60)


def _fold_usage_series_minute(state, row, at):
    """一行折进分钟环（与日/小时桶同一 realm 过滤、同一折叠口径）。

    保留线以下的桶不建（建了也会被下一次剪枝丢掉）：窗口要服务得上的行都在
    max_at - 6h 之上，更早的行折进来只是白建一个马上被剪的桶。
    """
    key = _series_minute_key(at)
    if key * 60 < state["max_at"] - _SERIES_MINUTE_WINDOW:
        return
    minutes = state["minutes"]
    bucket = minutes.get(key)
    if bucket is None:
        bucket = minutes[key] = _new_series_bucket(key * 60)
        if len(minutes) > _SERIES_MINUTE_RING_CAP:
            _prune_series_minutes(state)
    _feed_series_bucket(bucket, row)


def _prune_series_minutes(state):
    """丢掉保留线（max_at - 6h）以下的分钟桶。

    只按 max_at 剪，不跟日桶的 32 天剪枝线：分钟环的覆盖范围就是它自己的
    保留窗口，与日桶保留多少天无关。判定与 _fold_usage_series_minute 的
    跳过条件同一条（桶的 at 严格小于保留线），两边不会出现「建的桶马上被剪」
    或「剪了还需要的桶」。
    """
    horizon = state["max_at"] - _SERIES_MINUTE_WINDOW
    minutes = state["minutes"]
    for key in [k for k in minutes if k * 60 < horizon]:
        del minutes[key]


def _fold_usage_series(row, r, state):
    """一行折进时间序列状态：日桶 + 小时桶 + 分钟环 + credit 缓冲。"""
    at = row.get("at") or 0
    if at > state["max_at"]:
        # 折过的最大 at：窗口上界（hi=now）只要不低于它，存量桶就都在窗口内。
        # 越界的行（时钟回拨/补写）会让它偏大，那只是更保守，方向安全。
        state["max_at"] = at
    bucket = _usage_day_bucket(state, row, r, _day_series_bucket)
    if bucket is None:
        return
    _feed_series_bucket(bucket, row)
    # 分钟环放在小时桶的午夜兜底之前：兜底会 return（那一天的 hours 摘掉），
    # 而分钟环与日/小时格点无关，漏折一行就会让它的覆盖出现空洞。
    _fold_usage_series_minute(state, row, at)
    if at < bucket["at"]:
        # 本地午夜落在不存在的小时里（极少数时区的夏令时正好在午夜切换）：
        # 这一天的行放不进小时格点，把 hours 摘掉让切片对这一天整体退回扫描
        # （日桶本身照常，step=86400 的切片不受影响）。
        bucket.pop("hours", None)
        return
    # 小时子桶按「离那天的本地午夜多少秒」切：窗口起点是本地午夜时，窗口
    # 序列的桶序号 k = 天序号*24 + 小时序号（夏令时不在日边界上时也一样，
    # 因为两边都是纯 epoch 算术；日边界被夏令时挪动的情形由 _series_slice
    # 的格点校验挡住）。键写成字符串，与 JSON 往返后的形状一致。
    hour = str(int((at - bucket["at"]) // 3600))
    sub = bucket["hours"].get(hour)
    if sub is None:
        sub = bucket["hours"][hour] = _new_series_bucket(
            bucket["at"] + int(hour) * 3600)
    _feed_series_bucket(sub, row)
    if (row.get("credit") or 0) > 0:
        _series_credit_push(state["credits"], row, at)


def _usage_series_state_ready(r):
    """把 `r` 的时间序列状态推进到文件尾；返回 (state, offset, error)。

    调用方必须已持有 _series_state_lock。时间序列折叠不算价，所以状态只带
    realm 指纹（行没有 realm 字段时 row_realm() 会读它）。

    状态里的 `minutes_from` 是分钟环的完整线：从零折起（含 refold）时它是
    None（环对整份日志完整）；采用了一份没有分钟环的旧 checkpoint 时，环只
    对采用点之后折进来的行完整，那条线就是采用时的 max_at（见
    _usage_cache_adopt_series）。
    """
    state = _series_state.get(r)
    if state is None:
        state = _series_state[r] = {
            "days": {}, "days_floor": None, "max_at": 0, "credits": [],
            "minutes": {}, "minutes_from": None,
            "hits": [], "hits_overflow": False,
            "offset": 0, "key": None, "realm": None, "tail": b""}
    realm = _realm_inputs()
    log_key = _usage_log_key()
    if state["key"] is None:
        # 全新状态（进程刚起来）：先问 checkpoint，问不到才从零折起。
        if not _usage_cache_adopt_series(r, state, log_key, realm):
            state.update({"days": {}, "days_floor": None, "max_at": 0,
                          "credits": [], "minutes": {}, "minutes_from": None,
                          "hits": [], "hits_overflow": False,
                          "offset": 0, "tail": b""})
    elif state["realm"] != realm or not _log_resume_ok(state, log_key):
        state.update({"days": {}, "days_floor": None, "max_at": 0,
                      "credits": [], "minutes": {}, "minutes_from": None,
                      "hits": [], "hits_overflow": False,
                      "offset": 0, "tail": b""})
    state.update({"realm": realm, "key": log_key})
    offset, error = _scan_usage_from(
        state["offset"], lambda row: _fold_usage_series(row, r, state))
    if error is None:
        state["offset"] = offset
        state["tail"] = _log_tail_signature(offset)
        # 折叠途中的剪枝是按上限触发的，收尾再剪一次，落盘的环就是保留窗口
        # 以内的那一批（状态不折时这里也不花时间：环大小 <= 上限）。
        _prune_series_minutes(state)
    return state, offset, error


def _series_output_bucket(bucket):
    """存量桶 -> 输出桶（键序固定；日桶的 hours 子表不进输出）。"""
    return {"at": bucket["at"], "requests": bucket["requests"],
            "errors": bucket["errors"], "prompt_tokens": bucket["prompt_tokens"],
            "completion_tokens": bucket["completion_tokens"],
            "reasoning_tokens": bucket["reasoning_tokens"],
            "cached_tokens": bucket["cached_tokens"],
            "total_tokens": bucket["total_tokens"], "credit": bucket["credit"]}


def _series_minute_grid_ok(lo):
    """窗口起点能否落在分钟环的格点上：lo 必须是 60 的整数倍。

    整段扫描的桶边界是 lo + k*60，存量分钟桶的边界是 epoch 分钟的整数倍；
    两组边界一一对应（可以整桶取用）当且仅当 lo 自己落在格点上。面板的
    datetime-local 输入给的就是整分钟（fromLocalInputValue 只到分钟），
    手写的带小数的 since 一律退回整段扫描。
    """
    return int(lo) == lo and int(lo) % 60 == 0


def _series_window_day_key(lo):
    """窗口起点 lo 的本地日键；取值超出平台 epoch 范围时返回 None。

    _local_day_key 走 time.localtime，手写的极大 since（例如 6e18）会让它抛
    OSError——那只是「这个窗口服务不了」的一种，按退回整段扫描处理，不该把
    端点打成 500。分钟切片拿这个键做 days_floor 覆盖判断，拿不到就退回扫描。
    """
    try:
        return _local_day_key(lo)
    except (OSError, OverflowError, ValueError):
        return None


def _series_closed_cut_ok(state, lo_i, hi, step):
    """闭窗口的右边界能否由存量桶回答：hi 落在桶格点上，且直到它没有整秒行。

    桶按 at 严格切分（日/小时桶锚在本地午夜，分钟环锚在 epoch 分钟），所以
    hi 落在与桶同一条格点上时，「[lo, hi] 的行」与「at < hi 的桶」只差
    「恰好落在 hi 上的行」；那一刻有整秒行时桶切不出它，只能退回整段扫描
    （见 _closed_window_edge_ok）。取值离谱的 hi（手写 URL）按不成立处理，
    退回整段扫描——与 _series_window_day_key 的取值保护同一个用意。
    """
    if not _closed_window_edge_ok(state, hi):
        return False
    if step == 60:
        try:
            return _series_minute_grid_ok(hi)
        except (OverflowError, ValueError):
            return False
    try:
        hi_i = int(hi)
    except (OverflowError, ValueError):
        return False
    if hi_i != hi:
        return False
    return (hi_i - lo_i) % step == 0


def _series_slice_minutes(state, realm, lo, hi, day_key):
    """分钟环的切片；不满足对齐/覆盖条件时返回 None（退回整段扫描）。

    成立条件（缺一不可，任何一条不成立就退回整段扫描——正确性优先）：
      * 窗口起点是分钟格点（见 _series_minute_grid_ok）；
      * hi 不低于折过的最大 at：否则窗口上界会切进桶里，而桶是聚合值，
        切不出一半来（与日/小时切片同一条）。hi 切进数据里（闭窗口）时
        还要过闭窗口闸门（见 _series_closed_cut_ok）：hi 落在分钟格点上、
        且直到那一刻没有整秒行，才按「at < hi 的桶」切，恰好落在 hi 上的
        行由边沿账挡回整段扫描；
      * 窗口起点不早于 max_at - 6h：环的保留线，更早的窗口环里没有桶；
      * 窗口起点晚于 minutes_from：环只从那条线之后才完整（采用过没有
        分钟环的旧 checkpoint 时才有这条线，见 _usage_series_state_ready）；
      * 每个桶的 at 落在键*60 上且不晚于 max_at（形状校验之外的防御，
        与 _series_slice 对小时子桶的格点校验同一个用意）。
    输出桶与整段扫描逐条相同（同一批行、同一个网格、同一个累加口径），
    credits 与日/小时切片共用下面同一段选择。
    """
    minutes = state.get("minutes")
    if not minutes:
        # 空环（或状态里没有环）：这一份状态没有任何行落在保留窗口里。空环
        # 答不出窗口里的行，也不能把窗口静默答成空序列——退回整段扫描，
        # 由扫描给出真值（正确的空序列也由它给）。
        return None
    lo_i = int(lo)
    cut = False
    if hi < state["max_at"]:
        # 上界切进环里：闭窗口闸门不成立就退回整段扫描（结果不变）。
        if not _series_closed_cut_ok(state, lo_i, hi, 60):
            return None
        cut = True
    if not _series_minute_grid_ok(lo):
        return None
    if lo_i < state["max_at"] - _SERIES_MINUTE_WINDOW:
        return None
    minutes_from = state.get("minutes_from")
    if minutes_from is not None and lo_i <= minutes_from:
        return None
    if day_key is None:
        # 直接调用（测试）时补上；取值离谱（见 _series_window_day_key）就退回。
        day_key = _series_window_day_key(lo)
        if day_key is None:
            return None
    if not _days_cover(state, day_key):
        return None
    series = []
    for key in sorted(minutes):
        sub = minutes[key]
        at = int(sub["at"])
        if at != key * 60 or at > state["max_at"]:
            return None
        if at < lo_i:
            continue
        if cut and at >= hi:
            # 闭窗口：hi 那一刻归后一桶（桶按 at 严格切分），只取 at < hi
            # 的桶；那一刻有整秒行时上面已经退回。
            break
        out = _series_output_bucket(sub)
        # 整段扫描的桶 at 是 lo + k*60，类型跟着 lo 走（int 或 float）；
        # 存量桶的 at 是整数，这里按 lo 的类型对齐，两边的 JSON 字节才一致。
        out["at"] = int(at) if isinstance(lo, int) else float(at)
        series.append(out)
    return _series_payload(state, realm, lo, hi, 60, series)


def _series_payload(state, realm, lo, hi, step, series):
    """切片载荷的公共部分：窗口头 + credit 历史。

    credit 历史：缓冲按 at 倒序稳定排序后取窗口内的前 50 条——与整段扫描
    的选择逐条相同（见 _series_credit_push 的论证）。窗口的两端都要过滤：
    缓冲收的是整份日志的计费行，闭窗口的上界切进缓冲里时（hi < max_at），
    at > hi 的行不属于窗口，整段扫描不会收它们，切片也必须裁掉。
    """
    credits = sorted(state["credits"], key=lambda item: item.get("at") or 0,
                     reverse=True)
    credits = [c for c in credits
               if lo <= (c.get("at") or 0) <= hi][:_SERIES_CREDITS_KEEP]
    return {"ok": True, "realm": realm or "all",
            "bucket_seconds": step, "since": lo, "until": hi,
            "series": series, "credits": credits}


def _series_slice(state, realm, lo, hi, step, day_key):
    """窗口序列 = 日桶/小时桶/分钟环的切片；不满足对齐条件时返回 None。

    成立条件（缺一不可，任何一条不成立就退回整段扫描——正确性优先）：
      * 窗口起点是本地午夜（day_key 由 _window_day_key 给出）；
      * 桶宽是 3600 或 86400（与窗口对齐的两种）；
      * days_floor 覆盖窗口起点（剪枝/更早的天没折过就只能整段扫）；
      * hi 不低于折过的最大 at，否则窗口上界会切掉桶里的行；hi 切进数据里
        （闭窗口）时还要过闭窗口闸门（见 _series_closed_cut_ok）：hi 落在
        同一套桶格点上、且直到那一刻没有整秒行，才按「at < hi 的桶」切，
        恰好落在 hi 上的行由边沿账挡回整段扫描；
      * 每个桶的 at 落在 lo + k*step 的格点上。这一条同时挡住夏令时：日长
        不是 86400 秒的那天之后，每一天的本地午夜都不再落在 86400 的格点上，
        于是自动退回整段扫描。时区没有夏令时（cn/路由器）时恒真。

    桶宽 60 走分钟环（条件见 _series_slice_minutes），它不要求窗口起点是
    本地午夜，只要求落在分钟格点上。
    """
    if step == 60:
        return _series_slice_minutes(state, realm, lo, hi, day_key)
    if step not in (3600, 86400):
        return None
    lo_i = int(lo)
    cut = False
    if hi < state["max_at"]:
        # 上界切进桶里：闭窗口闸门不成立就退回整段扫描（结果不变）。
        if not _series_closed_cut_ok(state, lo_i, hi, step):
            return None
        cut = True
    if not _days_cover(state, day_key):
        return None
    series = []
    done = False
    for key in sorted(state["days"]):
        if key < day_key or done:
            continue
        bucket = state["days"][key]
        day_at = int(bucket["at"])
        if (day_at - lo_i) % 86400:
            return None
        if step == 86400:
            if cut and day_at >= hi:
                # 闭窗口：hi 那一刻归后一桶（桶按 at 严格切分），只取
                # at < hi 的桶；那一刻有整秒行时上面已经退回。这一天及其后
                # 的桶都在窗口外，done 把循环收口。
                done = True
                continue
            series.append(_series_output_bucket(bucket))
            continue
        hours = bucket.get("hours")
        if hours is None:
            # 这一天的小时格点不成立（见 _fold_usage_series 的午夜兜底）：
            # 整个窗口退回整段扫描，不能只跳过这一天。
            return None
        for hour in sorted(hours, key=int):
            sub = hours[hour]
            if int(sub["at"]) != day_at + int(hour) * 3600:
                return None
            if (int(sub["at"]) - lo_i) % 3600:
                return None
            if cut and int(sub["at"]) >= hi:
                # 闭窗口的右边界（放在格点校验之后——错开的桶要先退回扫描）。
                done = True
                break
            series.append(_series_output_bucket(sub))
    return _series_payload(state, realm, lo, hi, step, series)


def _top_models(bucket, top_n=KEY_MODEL_TOP_N):
    """Split a model bucket into the N busiest models plus one remainder.

    The remainder is flagged with `other` rather than being recognised by its
    label, so a model genuinely called "(其他)" cannot be mistaken for it.
    """
    items = sorted(bucket.items(), key=lambda kv: (-kv[1]["tokens"], -kv[1]["requests"], kv[0]))
    head = [{"model": mid, "requests": s["requests"], "tokens": s["tokens"],
             "reasoning": s["reasoning"]} for mid, s in items[:top_n]]
    rest = items[top_n:]
    other = None
    if rest:
        other = {
            "model": "(其他)",
            "other": True,
            "models": len(rest),
            "requests": sum(s["requests"] for _, s in rest),
            "tokens": sum(s["tokens"] for _, s in rest),
            "reasoning": sum(s["reasoning"] for _, s in rest),
        }
    return head, other


def _build_key_rows(key_map, realm=None):
    """Merge observed per-key traffic with the configured key roster.

    Every key the panel can still see gets a row even with no traffic in the
    window: a key that was used yesterday and not today is a fact about
    today's spend, and dropping it would read as "the key is gone". A key that
    is disabled *and* idle in this window is dropped, because that row says
    nothing about the selected range.

    The reverse direction matters just as much: an id seen in the log that the
    roster does not know still gets a row, so the per-key table always adds up
    to the per-account table. Those are folded into a single row - ids that
    cannot be named are an anomaly, not a dimension worth splitting.
    """
    key_map = key_map or {}
    rows = {}

    def empty_row(k_id, name, declared_realm, enabled, source):
        return {
            "key": k_id,
            "name": name,
            "realm": declared_realm,
            "enabled": enabled,
            "source": source,
            "window": _new_analytics_stat(),
            "all_time": _new_analytics_stat(),
            "window_models": {},
            "all_models": {},
            "realms": {},
            "last_at": 0,
        }

    for entry in configured_keys():
        k_id = entry.get("id") or ""
        if not k_id:
            continue
        declared = entry.get("realm") or ""
        # A key bound to the other exit can never have rows in this view.
        if realm and declared and declared != realm:
            continue
        rows[k_id] = empty_row(k_id, entry.get("name") or k_id, declared,
                               entry.get("enabled", True) is not False, "panel")
    # The launcher key (--api-key / API_KEY) lives in no settings file, so it
    # is only ever visible as an id in the log. It is not an attribution
    # dimension: once the panel holds any key of its own the launcher key is
    # no longer accepted at all (see identify_key), so an unused one is a
    # permanent zero that says nothing about who spent what. Give it a row
    # only when the log shows it was actually used - the only state in which
    # it carries history worth reconciling.
    if "launcher" not in rows and "launcher" in key_map:
        rows["launcher"] = empty_row("launcher", "启动参数", "", True, "launcher")

    def adopt(row, km):
        row["window"] = km["window"]
        row["all_time"] = km["all_time"]
        row["window_models"] = km["window_models"]
        row["all_models"] = km["all_models"]
        row["realms"] = km["realms"]
        row["last_at"] = km["last_at"]

    unknown = None
    for k_id, km in key_map.items():
        if k_id == KEY_BUCKET_BEFORE:
            rows[k_id] = empty_row(k_id, "(切换前)", "", False, "bucket")
            adopt(rows[k_id], km)
        elif k_id == KEY_BUCKET_ANON:
            rows[k_id] = empty_row(k_id, "(无 key)", "", False, "bucket")
            adopt(rows[k_id], km)
        elif k_id in rows:
            adopt(rows[k_id], km)
        else:
            if unknown is None:
                unknown = rows[KEY_BUCKET_UNKNOWN] = empty_row(
                    KEY_BUCKET_UNKNOWN, "(未知 key)", "", False, "bucket")
            _add_analytics_stat(unknown["window"], km["window"])
            _add_analytics_stat(unknown["all_time"], km["all_time"])
            for tgt, src in ((unknown["window_models"], km["window_models"]),
                             (unknown["all_models"], km["all_models"])):
                for mid, s in src.items():
                    dst = tgt.setdefault(mid, {"requests": 0, "tokens": 0, "reasoning": 0})
                    dst["requests"] += s["requests"]
                    dst["tokens"] += s["tokens"]
                    dst["reasoning"] += s["reasoning"]
            for k_realm, count in km["realms"].items():
                unknown["realms"][k_realm] = unknown["realms"].get(k_realm, 0) + count
            unknown["last_at"] = max(unknown["last_at"], km["last_at"])

    out = []
    for row in rows.values():
        if (row["source"] == "panel" and not row["enabled"]
                and not row["window"]["requests"] and not row["window"]["errors"]):
            continue
        _finalize_analytics_stat(row["window"])
        _finalize_analytics_stat(row["all_time"])
        row["models"], row["models_other"] = _top_models(row["window_models"])
        # A key with no realm binding follows the model it is asked for, so it
        # can serve both exits - and then its credit column adds up two
        # different products' prices. The page has to be able to say so.
        row["cross_realm"] = len([x for x in row["realms"] if x]) > 1
        out.append(row)
    out.sort(key=lambda r: (-r["window"]["total_tokens"], -r["all_time"]["total_tokens"], r["name"]))
    return out


def _enrich_accounts_from_pool(acct_map, realm=None):
    """Attach nickname/realm/credits for accounts that saw no traffic."""
    if POOL:
        for a in POOL.accounts:
            if realm and a.realm != realm:
                continue
            if a.uid in acct_map:
                acct_map[a.uid]["nickname"] = a.nickname
                acct_map[a.uid]["realm"] = a.realm
                acct_map[a.uid]["domain"] = a.domain
                acct_map[a.uid]["credits"] = getattr(a, "credits", None) or {}
            else:
                    acct_map[a.uid] = {
                        "uid": a.uid,
                        "nickname": a.nickname,
                        "realm": a.realm,
                        "domain": a.domain,
                        "credits": getattr(a, "credits", None) or {},
                        "window": _new_analytics_stat(),
                        "all_time": _new_analytics_stat(),
                        "window_models": {},
                        "all_models": {},
                    }


def _finalize_analytics_stat(stat_obj):
        p = stat_obj["prompt_tokens"]
        c = stat_obj["cached_tokens"]
        out = stat_obj["completion_tokens"]
        reas = stat_obj["reasoning_tokens"]
        stat_obj["cache_hit_pct"] = round((c / p * 100), 1) if p > 0 else 0.0
        stat_obj["reasoning_ratio"] = round((reas / out * 100), 1) if out > 0 else 0.0
        stat_obj["ttft_ms_avg"] = round(stat_obj["ttft_sum"] / stat_obj["ttft_n"]) if stat_obj["ttft_n"] > 0 else 0
        stat_obj["speed_avg"] = round(stat_obj["speed_sum"] / stat_obj["speed_n"], 1) if stat_obj["speed_n"] > 0 else 0.0
        stat_obj["elapsed_ms_avg"] = round(stat_obj["elapsed_sum"] / stat_obj["elapsed_n"]) if stat_obj["elapsed_n"] > 0 else 0
        return stat_obj

def _compute_usage_analytics_uncached(realm=None, since=None, until=None):
    """Detailed analytics for Token, Cache, and Reasoning metrics page."""
    # 与 /usage 快照同一条规矩：一次扫描里总开关只问一次，值传进折叠与指纹。
    pricing_on = wb_pricing.pricing_enabled()
    all_maps, offset = _analytics_all_time(realm, pricing_on)
    if since is None and until is None:
        # An open window covers every row, so the windowed half is the
        # all-time fold over again; the merge copies the two apart.
        win_maps = all_maps
    else:
        # The all-time half is not re-folded: the windowed half comes from
        # the day buckets that fold advanced (or, failing that, from a pass
        # that stops where that fold stopped), so one page never mixes two
        # reads.
        win_maps = _analytics_window(realm, since, until, offset, pricing_on)
    parts = _merge_analytics_halves(all_maps, win_maps)
    acct_map = parts["accts"]
    model_map = parts["models"]
    key_map = parts["keys"]
    _enrich_accounts_from_pool(acct_map, realm=realm)
    all_summary = parts["summary"]
    window_summary = parts["window_summary"]
    _finalize_analytics_stat(all_summary)
    _finalize_analytics_stat(window_summary)
    for a in acct_map.values():
        _finalize_analytics_stat(a["window"])
        _finalize_analytics_stat(a["all_time"])
    for m in model_map.values():
        _finalize_analytics_stat(m["window"])
        _finalize_analytics_stat(m["all_time"])
    accts_list = sorted(acct_map.values(), key=lambda a: (-a["window"]["total_tokens"], -a["all_time"]["total_tokens"]))
    models_list = sorted(model_map.values(), key=lambda m: (-m["window"]["total_tokens"], -m["all_time"]["total_tokens"]))
    keys_list = _build_key_rows(key_map, realm=realm)
    return {
        # The resolved window travels with the payload so the page can label
        # its first column from what the server actually applied, not from
        # what the panel hoped it sent.
        "window": {"since": since, "until": until},
        "realm": realm or "all",
        # Cost figures are CNY; the panel divides by this rate to show USD.
        "usd_cny": wb_pricing.usd_cny(),
        "summary": {"window": window_summary, "all_time": all_summary},
        "accounts": accts_list,
        "models": models_list,
        # Same rows, folded by the API key that called the gateway instead of
        # by the upstream account that served the call. One key can be served
        # by several accounts and one account can serve several keys, so this
        # is a second view of the same spend, not a breakdown of it.
        "keys": keys_list,
    }


# ---------------------------------------------------------------------------
# 聚合状态落盘（重启后免冷扫）
#
# 上面的几份增量聚合（_usage_alltime_snapshot / _usage_by_account_buckets /
# _analytics_all_time / 时间序列 / 剩余用量）把状态放在模块级字典里，进程一
# 重启就全丢：第一次刷新仍是全量扫一遍日志。日志每天涨一万多行，冷扫描成本
# 跟着线性上升——路由器上 2.7 万行的一次冷启动要 6~8 秒，一个月后接近两分钟。
# 这里把每份状态连同它们的位置信息写进数据目录，重启后第一次刷新只读文件尾
# 部的新增字节。
#
# v4 起**一类状态一份文件**（usage-aggregate-cache-<kind>.json）：五份状态
# 各有各的位置/指纹校验与写入节流，加载端按需解析——只跑 /usage 的进程不再
# 为其余四份（真机上 remaining 一份就比其余加起来还大）付解析成本；写侧也只
# 重写「有推进的那一类」。升级前的单文件（v3）只读兼容：某一类还没有自己的
# 文件时从旧文件里取它那一节（旧文件不重写、也不删除，等各类都落过一次盘就
# 自然被忽略）。
#
# 文件放在数据目录里是刻意的：它和日志同生命周期，跟着 sysupgrade 一起走，
# 也和日志一起被搬走、清掉。读写失败（只读文件系统、磁盘满、权限不足）一律
# 静默降级——checkpoint 只是加速，丢的是一次加速，绝不能影响服务本身。
#
# 加载校验复用 #189 已有的原语，不另造一套：offset 必须仍落在同一个文件的
# 现有字节里（_log_offset_holds），offset 前的最后 64 字节必须与写下时一致
# （_log_tail_signature），再加上价格/realm 指纹和 schema 版本。任何一项对不
# 上就当没有缓存、走全量——宁可多扫一次，也不能给出错的数字。
# ---------------------------------------------------------------------------

# 数据目录下的文件名。usage/*.json 已被 .gitignore 覆盖。
# 带 <kind> 的是 v4 布局（一类一份）；不带的是升级前的单文件，只读不写。
_USAGE_CACHE_NAME = "usage-aggregate-cache.json"
_USAGE_CACHE_KIND_NAME = "usage-aggregate-cache-%s.json"
# 状态结构或折叠口径一变就 +1，旧文件整体作废。这是唯一挡在「旧结构喂进新
# 代码」前面的东西，改动状态形状时别忘了它。
# v2：写盘不再排序（v1 用 sort_keys 写，加载回来所有 dict 变字母序，响应
# 与冷启动逐字节不同）。键顺序算状态结构的一部分，所以旧文件必须整体作废
# ——否则那份字母序会一直粘在内存里，直到日志尾部被重写才会被清掉。
# v3：状态多了按日分桶（窗口半的增量）与时间序列的日/小时桶。载荷形状变了，
# 旧文件同样整体作废：升级后第一次刷新做一次全量折叠（一次性代价，之后
# checkpoint 照旧）——这正是把窗口半也变成增量的入场费。
# v4：一类状态一份文件（每份自带 schema 与 kind 标签），并且快照/analytics 的
# 日桶里多了小时子表（整点窗口的小时层）、series 多了分钟环。载荷形状的变化由
# 各类自己的校验器挡住：升级前的单文件（v3）只读兼容，作为各类还没有自己文件
# 时的回退来源——能通过新校验器的段（by_account / series / remaining）照常
# 采纳，通不过的（缺小时表的 snapshot / analytics）在升级后第一次刷新时做一次
# 全量折叠，之后 checkpoint 照旧。
# v5：snapshot / analytics 的 "pricing" 字段从「三张表的内容摘要」（["on", d1,
# d2, d3] / ["off"]）换成「折叠行真正用到的价」记录（T15）。旧条目的字段形状
# 对不上新判定，采纳端直接拒——升级后第一次刷新一次性重折（与 v3→v4 同处置），
# 之后价表再变只按「用到的价」判废（无关模型的价变动不再重折）。
_USAGE_CACHE_SCHEMA = 5
# 升级前唯一布局的版本号：v3 单文件。只出现在只读回退路径上。
_USAGE_CACHE_LEGACY_SCHEMA = 3
# 五类状态的固定集合（文件名、kind 标签与写盘循环都用它）。
_USAGE_CACHE_KINDS = ("snapshot", "by_account", "analytics", "series",
                      "remaining")


def _usage_cache_enabled():
    """落盘总开关，默认开；WB_USAGE_CACHE=0 关。

    每次调用都读环境变量而不是在导入时定死：现场用它紧急关掉，测试用它在一
    个进程里来回切，而一次 os.environ.get 的开销在刷新路径上可以忽略。
    """
    return os.environ.get("WB_USAGE_CACHE", "1").strip().lower() not in (
        "0", "false", "no", "off")


def _usage_cache_path():
    """升级前单文件（v3）的路径；只读回退与探针脚本用。

    每次调用重算，而不是在导入时算成常量：--usage-dir、测试和探针脚本都会
    在导入之后改 USAGE_DIR，常量会写到旧目录去。
    """
    return os.path.join(USAGE_DIR, _USAGE_CACHE_NAME)


def _usage_cache_kind_path(kind):
    """某一类状态的 checkpoint 路径（v4 布局；同上，每次调用重算）。"""
    return os.path.join(USAGE_DIR, _USAGE_CACHE_KIND_NAME % kind)


def _usage_cache_min_bytes():
    """写盘节流：折叠推进不足这么多字节就不写。默认 1MB。

    1MB 是「重启后最多回放多少」的上限——路由器上扫描约 0.5s/MB，回放 1MB
    等于冷启动成本多半秒；而日志每天涨 6MB 上下，正常流量下一天也未必触发
    一次。写小了是写放大（路由器 flash 写入慢且费寿命），写大了重启变慢。
    """
    try:
        return max(0, int(os.environ.get("WB_USAGE_CACHE_MIN_BYTES", 1024 * 1024)))
    except (TypeError, ValueError):
        return 1024 * 1024


def _usage_cache_min_seconds():
    """写盘节流：距上次尝试不足这么多秒就不写。默认 900s（15 分钟）。

    这条兜住「字节阈值还没到、但偏移已经推进了一点」的情况：不设它，一段
    短流量之后停下的日志要等到下次凑满 1MB 才会落盘。15 分钟让面板开着时
    一天最多约百次写盘（每次几十 KB），对 flash 友好；重启回放的上限则是
    min(1MB, 15 分钟流量)。
    """
    try:
        return max(0.0, float(os.environ.get("WB_USAGE_CACHE_MIN_SECONDS", 900)))
    except (TypeError, ValueError):
        return 900.0


_usage_cache_lock = threading.Lock()
# 写盘串行锁：只被 _usage_cache_maybe_save() 拿一次、且永远是最外层（拿它
# 之后才拿状态锁/节流锁），所以它不参与任何锁序环。
_usage_cache_write_lock = threading.Lock()
# 进程启动后按「类」只读一次 checkpoint：读到就记下来，读不到也记 None——
# 坏文件、只读文件系统都不该让每次刷新都去解析一遍。只解析调用方真正要的
# 那一类，这正是拆文件的意义（只跑 /usage 的进程不解析 remaining）。
# 升级前的单文件（v3）也只读一次，作为某一类还没有自己文件时的回退来源。
_usage_cache_kind_cache = {}
_USAGE_CACHE_LEGACY_UNREAD = object()
_usage_cache_legacy_data = _USAGE_CACHE_LEGACY_UNREAD
# 写盘节流状态，全部由 _usage_cache_lock 保护：
#   progress[kind]     —— 本进程见过该类状态折叠到的最远 offset；
#   checkpointed[kind] —— 该类状态上次「尝试」落盘时的 offset（失败也记，
#                         只读文件系统上不能每次刷新都去试写一遍）；
#   last_attempt       —— 上次尝试写盘的时刻，时间阈值与失败退避都靠它。
# 按「类」记账而不是记一个总数：某类状态第一次折叠（或中途重扫）时 offset
# 从 0 跳到文件尾，这一类要尽快落盘，而刚采用过 checkpoint 的类不能被别的
# 类的推进带着白写一遍。v4 起写盘也按类分开：只重写「有推进的那一类」。
_usage_cache_progress = {}
_usage_cache_checkpointed = {}
_usage_cache_last_attempt = time.time()


def _usage_cache_forget():
    """忘掉本进程读过的 checkpoint（模拟一次重启；测试与探针用）。

    读记忆按「类」记账，所以要清的是一个集合；调用点不许持有任何状态锁，
    本函数自己取 _usage_cache_lock（与读取路径同一把）。
    """
    global _usage_cache_legacy_data
    with _usage_cache_lock:
        _usage_cache_kind_cache.clear()
        _usage_cache_legacy_data = _USAGE_CACHE_LEGACY_UNREAD


def _usage_cache_kind_entries(kind):
    """某一类状态的条目列表（读一次、按类记忆）；没有可用来源时返回 None。

    本类文件不存在或读不出时回退到升级前的单文件里对应的那一节：升级后的
    第一次刷新因此不必把整个日志重折一遍。本类文件一旦存在（schema 与 kind
    都对得上）就是唯一来源，旧文件不再参与该类——旧文件里那份可能更旧，也
    可能已经和日志对不上，位置校验各自负责。

    调用点可能在状态锁内（采用阶段），所以本函数会取 _usage_cache_lock；
    反向的锁序不存在——写盘路径取状态锁之前一定先放开本锁，见
    _usage_cache_maybe_save。
    """
    if not _usage_cache_enabled():
        return None
    with _usage_cache_lock:
        if kind not in _usage_cache_kind_cache:
            entries = _usage_cache_read_kind(kind)
            if entries is None:
                entries = _usage_cache_read_legacy_kind(kind)
            _usage_cache_kind_cache[kind] = entries
        return _usage_cache_kind_cache[kind]


def _usage_cache_read_kind(kind):
    """读某一类的 checkpoint 文件并做顶层校验；任何一步不成立都返回 None。

    None 和「文件不存在」同义：调用方按没有缓存处理，绝不因此报错。
    调用方持有 _usage_cache_lock（解析在锁内做，与 v3 的单文件读取一致）。
    """
    try:
        with open(_usage_cache_kind_path(kind), "rb") as fh:
            raw = fh.read()
    except OSError:
        return None
    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return None
    if not isinstance(data, dict):
        return None
    # 先看类型再看值：JSON 的 true 在 Python 里等于 1，schema: true 不能算
    # 「版本 1」。
    if not _usage_cache_int(data.get("schema")):
        return None
    if data["schema"] != _USAGE_CACHE_SCHEMA:
        return None
    # kind 标签是双保险：文件被复制/改名成另一类的名字时，宁可当没有缓存。
    if data.get("kind") != kind:
        return None
    entries = data.get("entries")
    if not isinstance(entries, list):
        return None
    return entries


def _usage_cache_read_legacy_kind(kind):
    """升级前单文件（v3）里某一类的条目，或 None；文件只解析一次。

    调用方持有 _usage_cache_lock。条目列表是解析结果的一部分，采用路径从
    它里面 pop（取过即删），与 v3 时的用法一致。
    """
    global _usage_cache_legacy_data
    if _usage_cache_legacy_data is _USAGE_CACHE_LEGACY_UNREAD:
        _usage_cache_legacy_data = _usage_cache_read_legacy_file()
    data = _usage_cache_legacy_data
    if not isinstance(data, dict):
        return None
    entries = data.get(kind)
    if not isinstance(entries, list):
        return None
    return entries


def _usage_cache_read_legacy_file():
    """升级前的单文件（v3）解析结果，或 None。

    只做顶层校验（对象 + schema 恰好是 3）。不校验它的 kind 字段——v3 的
    文件本来就没有这个字段，各类小节就是顶层键。任何一步不成立都返回 None。
    """
    try:
        with open(_usage_cache_path(), "rb") as fh:
            raw = fh.read()
    except OSError:
        return None
    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return None
    if not isinstance(data, dict):
        return None
    if not _usage_cache_int(data.get("schema")):
        return None
    if data["schema"] != _USAGE_CACHE_LEGACY_SCHEMA:
        return None
    return data


_usage_cache_digest_memo = {}


def _object_digest(obj):
    """一段 JSON 内容摘要，按对象身份记忆；算不出来返回 None。

    记忆表的值强引用对象本身，所以 id() 不会被回收复用；超过几条就整体清空
    ——正常进程里价表对象只在价格变化时换代，这里只是给长期运行兜底。
    """
    hit = _usage_cache_digest_memo.get(id(obj))
    if hit is not None and hit[0] is obj:
        return hit[1]
    try:
        payload = json.dumps(obj, sort_keys=True, ensure_ascii=False,
                             separators=(",", ":")).encode("utf-8")
        digest = hashlib.sha1(payload).hexdigest()
    except Exception:
        return None
    if len(_usage_cache_digest_memo) > 8:
        _usage_cache_digest_memo.clear()
    _usage_cache_digest_memo[id(obj)] = (obj, digest)
    return digest


def _realm_inputs_key(inputs):
    """realm 指纹本来就是值比较（不是身份），JSON 化后原样可比。"""
    if inputs is None:
        return None
    realm, accounts = inputs
    return [realm, [[uid, r] for uid, r in accounts]]


# 折叠会就地累加/读取的字段集合直接从构造函数推导：上游往 _empty_stats() 或
# _new_analytics_stat() 里加字段时这里自动跟着收——漏一个，放行的缓存就会在
# 折叠半路抛 KeyError，那次刷新报出半份数字（折叠的错误路径只保留已折的
# 部分）。
_USAGE_CACHE_SNAPSHOT_NUMBERS = tuple(
    k for k, v in _empty_stats().items()
    if isinstance(v, (int, float)) and not isinstance(v, bool))
_USAGE_CACHE_SNAPSHOT_MAPS = ("cost_missing", "by_model", "by_model_realm", "by_model_acct")
_USAGE_CACHE_ANALYTICS_NUMBERS = tuple(_new_analytics_stat().keys())
_USAGE_CACHE_BUCKET_NUMBERS = ("requests", "cost_cny") + USAGE_FIELDS
# 时间序列桶的数字字段（_new_series_bucket 的键，含 at）。从构造函数推导，
# 与上面两条同一个道理：上游加字段时校验自动跟着收。
_USAGE_CACHE_SERIES_NUMBERS = tuple(_new_series_bucket(0).keys())


def _usage_cache_number(value):
    """折叠累加的数字。bool 是 int 的子类，必须挡掉。"""
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _usage_cache_int(value):
    """严格整数：JSON 里写成 12345.0 的不算。"""
    return isinstance(value, int) and not isinstance(value, bool)


def _usage_cache_counts_ok(counts):
    """{str: 数字} 形状的计数表（cost_missing / models / realms）。"""
    if not isinstance(counts, dict):
        return False
    for key, value in counts.items():
        if not isinstance(key, str) or not _usage_cache_number(value):
            return False
    return True


def _usage_cache_bucket_ok(bucket):
    """一个 by_model* 桶：折叠会就地累加这些键。"""
    if not isinstance(bucket, dict):
        return False
    for field in _USAGE_CACHE_BUCKET_NUMBERS:
        if not _usage_cache_number(bucket.get(field)):
            return False
    return _usage_cache_counts_ok(bucket.get("accounts"))


def _usage_cache_snapshot_bucket_ok(snap):
    """一个窗口半快照桶的形状（all-time 半、日桶、小时桶共用）。

    少一个键，折叠或 _copy_usage_snapshot() 就会半路抛错，那次刷新报出半份
    数字——校验必须挡在采用之前。
    """
    if not isinstance(snap, dict):
        return False
    for field in _USAGE_CACHE_SNAPSHOT_NUMBERS:
        if not _usage_cache_number(snap.get(field)):
            return False
    for field in _USAGE_CACHE_SNAPSHOT_MAPS:
        if not isinstance(snap.get(field), dict):
            return False
    if not _usage_cache_counts_ok(snap["cost_missing"]):
        return False
    for _, bucket in snap["by_model"].items():
        if not _usage_cache_bucket_ok(bucket):
            return False
    for _, realms in snap["by_model_realm"].items():
        if not isinstance(realms, dict):
            return False
        for _, bucket in realms.items():
            if not _usage_cache_bucket_ok(bucket):
                return False
    for _, realms in snap["by_model_acct"].items():
        if not isinstance(realms, dict):
            return False
        for _, accts in realms.items():
            if not isinstance(accts, dict):
                return False
            for _, bucket in accts.items():
                if not _usage_cache_bucket_ok(bucket):
                    return False
    return True


def _usage_cache_snapshot_ok(snap):
    """窗口半快照桶 + 它可能挂着的小时子表（见 _usage_hour_bucket）。

    小时子表的校验与时间序列那份（_usage_cache_series_bucket_ok）同一套：
    键是数字字符串（折叠侧从一开始就写 str(序号)，与 JSON 往返后的形状
    一致），子桶形状同快照桶且不再嵌表（表只挂在日桶上）。at 是格点，
    没有它求和侧就做不了格点校验，所以这里也要求它是数字。
    """
    if not _usage_cache_snapshot_bucket_ok(snap):
        return False
    hours = snap.get("hours")
    if hours is None:
        return True
    if not isinstance(hours, dict):
        return False
    for hour, sub in hours.items():
        if not (isinstance(hour, str) and hour.isdigit()):
            return False
        if not isinstance(sub, dict) or "hours" in sub:
            return False
        if not _usage_cache_number(sub.get("at")):
            return False
        if not _usage_cache_snapshot_bucket_ok(sub):
            return False
    return True


def _usage_cache_stat_ok(stat):
    """一个 analytics 统计桶：折叠会就地累加它的每一个字段。"""
    if not isinstance(stat, dict):
        return False
    for field in _USAGE_CACHE_ANALYTICS_NUMBERS:
        if not _usage_cache_number(stat.get(field)):
            return False
    return True


def _usage_cache_models_ok(models):
    """analytics 的 {model: {requests, tokens, reasoning, cost_cny}} 表。"""
    if not isinstance(models, dict):
        return False
    for mid, stat in models.items():
        if not isinstance(mid, str) or not isinstance(stat, dict):
            return False
        for field in ("requests", "tokens", "reasoning", "cost_cny"):
            if not _usage_cache_number(stat.get(field)):
                return False
    return True


def _usage_cache_analytics_window_ok(maps):
    """窗口形状的 analytics maps（日桶/小时桶）：同一批表，装的是 window 那一半。

    与 _usage_cache_analytics_ok 是两份形状，不能互相顶替：日桶只折窗口半，
    少 all_time/all_models 两个键，用 all-time 的校验器会把整份 analytics
    entry 拒掉（连它自己那份 all-time 也一起丢），窗口就退回整段扫描。
    """
    if not isinstance(maps, dict):
        return False
    if not _usage_cache_stat_ok(maps.get("summary")):
        return False
    accts, models, keys = maps.get("accts"), maps.get("models"), maps.get("keys")
    if not isinstance(accts, dict) or not isinstance(models, dict) or not isinstance(keys, dict):
        return False
    for uid, entry in accts.items():
        if not isinstance(uid, str) or not isinstance(entry, dict):
            return False
        for field in ("nickname", "realm", "domain"):
            if not isinstance(entry.get(field), str):
                return False
        if not _usage_cache_stat_ok(entry.get("window")):
            return False
        if not _usage_cache_models_ok(entry.get("window_models")):
            return False
    for mid, entry in models.items():
        if not isinstance(mid, str) or not isinstance(entry, dict):
            return False
        if not isinstance(entry.get("model"), str):
            return False
        if not _usage_cache_stat_ok(entry.get("window")):
            return False
    for kid, entry in keys.items():
        if not isinstance(kid, str) or not isinstance(entry, dict):
            return False
        if not isinstance(entry.get("key"), str):
            return False
        if not _usage_cache_stat_ok(entry.get("window")):
            return False
        if not _usage_cache_models_ok(entry.get("window_models")):
            return False
        if not _usage_cache_counts_ok(entry.get("realms")):
            return False
        if not _usage_cache_number(entry.get("last_at")):
            return False
    return True


def _usage_cache_analytics_hours_ok(hours):
    """analytics 日桶的小时子表：{小时键: 窗口形状 maps + at}。

    小时键是数字字符串（JSON 的对象键只能是字符串，折叠侧从一开始就用
    str(小时序号)），子桶就是一份窗口形状 maps 再带它自己的 at——形状与
    时间序列的小时子表同一个道理。
    """
    if not isinstance(hours, dict):
        return False
    for hour, sub in hours.items():
        if not (isinstance(hour, str) and hour.isdigit()):
            return False
        if not isinstance(sub, dict) or "hours" in sub:
            return False
        if not _usage_cache_number(sub.get("at")):
            return False
        if not _usage_cache_analytics_window_ok(sub):
            return False
    return True


def _usage_cache_analytics_day_ok(bucket):
    """analytics 的日桶：窗口形状 maps + at（本地午夜）+ hours 子表。

    小时层要靠 at 与小时子表回答自定义窗口，两者缺一这份状态就只能服务
    按日对齐的窗口——旧版本写下的日桶（没有这两个键）因此在这里被拒，
    升级后 analytics 整体重折一次（一次性代价），小时层立刻就有历史可借；
    snapshot/series 等其余四份状态照旧采用，不受影响。
    """
    if not isinstance(bucket, dict):
        return False
    if not _usage_cache_number(bucket.get("at")):
        return False
    if not _usage_cache_analytics_hours_ok(bucket.get("hours")):
        return False
    return _usage_cache_analytics_window_ok(bucket)


def _usage_cache_analytics_ok(maps):
    """analytics all-time maps：summary/accts/models/keys 四张表。"""
    if not isinstance(maps, dict):
        return False
    if not _usage_cache_stat_ok(maps.get("summary")):
        return False
    accts, models, keys = maps.get("accts"), maps.get("models"), maps.get("keys")
    if not isinstance(accts, dict) or not isinstance(models, dict) or not isinstance(keys, dict):
        return False
    for uid, entry in accts.items():
        if not isinstance(uid, str) or not isinstance(entry, dict):
            return False
        for field in ("nickname", "realm", "domain"):
            if not isinstance(entry.get(field), str):
                return False
        if not _usage_cache_stat_ok(entry.get("all_time")):
            return False
        if not _usage_cache_models_ok(entry.get("all_models")):
            return False
    for mid, entry in models.items():
        if not isinstance(mid, str) or not isinstance(entry, dict):
            return False
        if not isinstance(entry.get("model"), str):
            return False
        if not _usage_cache_stat_ok(entry.get("all_time")):
            return False
    for kid, entry in keys.items():
        if not isinstance(kid, str) or not isinstance(entry, dict):
            return False
        if not isinstance(entry.get("key"), str):
            return False
        if not _usage_cache_stat_ok(entry.get("all_time")):
            return False
        if not _usage_cache_models_ok(entry.get("all_models")):
            return False
        if not _usage_cache_counts_ok(entry.get("realms")):
            return False
        if not _usage_cache_number(entry.get("last_at")):
            return False
    return True


def _usage_cache_buckets_ok(buckets):
    """by_account 的 {account: 桶}：折叠会就地累加每个桶。"""
    if not isinstance(buckets, dict):
        return False
    for key, bucket in buckets.items():
        if not isinstance(key, str) or not isinstance(bucket, dict):
            return False
        if not isinstance(bucket.get("account"), str):
            return False
        for field in ("requests", "prompt_tokens", "completion_tokens",
                      "reasoning_tokens", "cached_tokens", "total_tokens"):
            if not _usage_cache_number(bucket.get(field)):
                return False
        if not _usage_cache_counts_ok(bucket.get("models")):
            return False
    return True


def _usage_cache_remaining_ok(entry):
    """剩余用量折叠：事件表 + 每个 (账号, 模型) 的 at/cum 两个平行列表。

    cum 必须单调（回退=负增量=坏数据）且与 at 等长、base 不可能超过 cum[0]
    ——这些是加载后一切查询的前提，坏一条就会静默算出错数字，所以逐元素走
    一遍：一次重启付 1~3ms，换掉一次全量冷扫。
    `at` **不要求有序**：写入侧的 at 是请求完成时刻，多线程下会回退几毫秒
    （真机日志与路由器自己写出的 checkpoint 都踩过），旧版在这里整份拒绝、
    每次重启都退回冷扫。加载端改为由 _pair_repair 做一次稳定排序（配对不变、
    前缀和重算），所以有序性校验搬到了那里，形状检查留在这里。
    """
    if not isinstance(entry, dict):
        return False
    if not _usage_cache_int(entry.get("offset")) or entry.get("offset") <= 0:
        return False
    if not isinstance(entry.get("tail"), str):
        return False
    key = entry.get("key")
    if not isinstance(key, list) or len(key) != 3 or not all(
            _usage_cache_int(v) for v in key):
        return False
    file_pos = entry.get("file")
    if not isinstance(file_pos, dict) or not _usage_cache_int(file_pos.get("offset")):
        return False
    if not isinstance(file_pos.get("tail"), str):
        return False
    file_key = file_pos.get("key")
    if not isinstance(file_key, list) or len(file_key) != 3 or not all(
            _usage_cache_int(v) for v in file_key):
        return False
    events = entry.get("events")
    if not isinstance(events, list) or len(events) > _REMAINING_MAX_EVENTS:
        return False
    for event in events:
        if not isinstance(event, dict):
            return False
        if not isinstance(event.get("account"), str) or not isinstance(event.get("model"), str):
            return False
        for field in ("at", "reset", "sample"):
            if not _usage_cache_number(event.get(field)):
                return False
    usage = entry.get("usage")
    if not isinstance(usage, dict):
        return False
    for pair_key, pair in usage.items():
        if (not isinstance(pair_key, str) or "\x1f" not in pair_key
                or not isinstance(pair, dict)):
            return False
        at = pair.get("at")
        cum = pair.get("cum")
        if not isinstance(at, list) or not isinstance(cum, list) or len(at) != len(cum):
            return False
        if not _usage_cache_int(pair.get("head")) or not _usage_cache_int(pair.get("base")):
            return False
        if pair["head"] < 0 or pair["head"] > len(at) or pair["base"] < 0:
            return False
        for value in at:
            if not _usage_cache_number(value):
                return False
        last_cum = None
        for value in cum:
            if not _usage_cache_int(value):
                return False
            if last_cum is not None and value < last_cum:
                return False
            last_cum = value
        # base 是「保留区之前那段的累计和」，cum 从最早一行累计且含被裁前缀，
        # 所以 base 不可能超过 cum[0]。旧版压缩把 base 累加过（第二次起重复
        # 计入前缀），这样的条目必须整份拒绝——否则左端落在保留区之前的区间
        # 求和会算出负数（真机面板显示过 -1293.5M）。
        if cum and pair["base"] > cum[0]:
            return False
    return True


def _usage_cache_day_key_ok(key):
    """日键必须是 "YYYY-MM-DD"：窗口与日桶的比较靠的就是这个形状。"""
    return (isinstance(key, str) and len(key) == 10 and key[4] == "-"
            and key[7] == "-")


def _usage_cache_days_ok(days, floor, bucket_ok):
    """按日分桶的表：{日键: 桶}，floor 是「对哪些天完整」的下界（日键或 None）。

    桶的形状由调用方给（snapshot / analytics / 时间序列各一套），日键与
    floor 的形状在这里统一校验：键不是日期的桶永远不会被窗口选中，但也说明
    这份状态不是我们写出来的，直接拒绝更干净。
    """
    if not isinstance(days, dict):
        return False
    if floor is not None and not _usage_cache_day_key_ok(floor):
        return False
    for key, bucket in days.items():
        if not _usage_cache_day_key_ok(key) or not bucket_ok(bucket):
            return False
    return True


def _usage_cache_series_bucket_ok(bucket):
    """时间序列桶：9 个数字；日桶还带 hours 子表（小时键是数字字符串）。

    小时键写成字符串是 JSON 往返决定的（JSON 的对象键只能是字符串），折叠
    侧因此从一开始就用 str(小时序号)，加载回来的状态与现折的状态形状一致。
    """
    if not isinstance(bucket, dict):
        return False
    for field in _USAGE_CACHE_SERIES_NUMBERS:
        if not _usage_cache_number(bucket.get(field)):
            return False
    hours = bucket.get("hours")
    if hours is None:
        return True
    if not isinstance(hours, dict):
        return False
    for hour, sub in hours.items():
        if not (isinstance(hour, str) and hour.isdigit()):
            return False
        if not isinstance(sub, dict) or "hours" in sub:
            return False
        for field in _USAGE_CACHE_SERIES_NUMBERS:
            if not _usage_cache_number(sub.get(field)):
                return False
    return True


def _usage_cache_series_minutes_ok(rows):
    """分钟环的落盘形状：[分钟, 8 个数字] 的严格升序列表。

    None 是合法的（旧版 checkpoint 没有分钟环这个键）：采用端把它当「环只
    从采用点之后才完整」处理（见 _usage_cache_adopt_series），所以这里放行
    而不是拒整份。行首必须是真整数（JSON 里 12345.0 不算）、严格递增——
    递增同时钉住「没有重复分钟」；其余 8 个必须是折叠累加出来的数字。
    """
    if rows is None:
        return True
    if not isinstance(rows, list):
        return False
    last = None
    for row in rows:
        if not isinstance(row, list) or len(row) != 1 + len(_SERIES_MINUTE_FIELDS):
            return False
        minute = row[0]
        if not _usage_cache_int(minute):
            return False
        if last is not None and minute <= last:
            return False
        last = minute
        for value in row[1:]:
            if not _usage_cache_number(value):
                return False
    return True


def _usage_cache_minutes_state(rows):
    """落盘行 -> 折叠时的字典桶（键是 epoch 分钟整数）。

    还原出的桶与冷折出来的逐字段同类型（int 的字段来自 JSON 的整数写法，
    credit 是浮点），所以「采用后再切片」与「冷折后切片」的输出逐字节一致。
    """
    out = {}
    for row in rows or []:
        key = int(row[0])
        bucket = _new_series_bucket(key * 60)
        for index, field in enumerate(_SERIES_MINUTE_FIELDS, start=1):
            bucket[field] = row[index]
        out[key] = bucket
    return out


def _usage_cache_credits_ok(credits):
    """credit 缓冲：{at, iso, model, account, credit, total_tokens} 的列表。"""
    if not isinstance(credits, list):
        return False
    for item in credits:
        if not isinstance(item, dict):
            return False
        for field in ("at", "credit", "total_tokens"):
            if not _usage_cache_number(item.get(field)):
                return False
        for field in ("iso", "model", "account"):
            if not isinstance(item.get(field), str):
                return False
    return True


def _usage_cache_hits_ok(hits, overflow):
    """闭窗口边沿账的落盘形状：[整秒 epoch] 列表 + 严格 bool 溢出标记。

    旧版 checkpoint 没有这两个键（entry.get 给 None）→ 拒：升级后第一次刷新
    重折一次（与 v3→v4「缺小时表的 snapshot/analytics 被拒、一次性重折」
    同一套处置）。账是「直到某刻没有行落在整秒上」的证明，缺了它闭窗口只能
    退回扫描（正确但慢），宁可重折一次把它补全。
    """
    if not isinstance(hits, list) or not isinstance(overflow, bool):
        return False
    for value in hits:
        if not _usage_cache_int(value):
            return False
    return True


def _usage_cache_take(kind, realm=None):
    """从 checkpoint 里取出一份状态（取过即删），没有合适的返回 None。

    snapshot/analytics 按 realm 匹配（JSON 的 null 表示不限 realm），
    by_account / remaining 只有一份。逐份取用：同一份不会被第二次采用，也就
    不存在「先取走、校验失败、下个 realm 又拿到一份脏状态」的路径。

    只解析、只取用调用方要的那一类文件：其余四类连文件都不打开。
    """
    entries = _usage_cache_kind_entries(kind)
    if not isinstance(entries, list):
        return None
    # 取走这一步也要在锁里：两个 realm 的首次刷新可能同时到这里，一个
    # 边遍历边 pop、另一个也在 pop，遍历就会跳过条目。跳过的后果只是这次
    # 不采用（安全方向），但少一次竞争就少一个要解释的路径。
    with _usage_cache_lock:
        for index, entry in enumerate(entries):
            if not isinstance(entry, dict):
                continue
            if kind not in ("by_account", "remaining"):
                # realm 字段必须显式存在且类型正确：realm=None 的折叠（全部
                # realm）和某个具体 realm 的折叠是两份不同的状态，缺字段的
                # 条目一旦被 None 匹配走，就会把「只折了一个 realm」的数字
                # 当成「全部 realm」报出去。
                entry_realm = entry.get("realm")
                if "realm" not in entry or not (entry_realm is None
                                                or isinstance(entry_realm, str)):
                    continue
                if entry_realm != realm:
                    continue
            entries.pop(index)
            return entry
    return None


def _usage_cache_position_ok(entry, log_key):
    """校验一份状态的 (offset, key, tail)；不合格返回 None。

    复用 #189 的两个原语而不是另造一套：offset 必须仍落在同一个文件的现有
    字节里（dev/ino 相同、当前 size >= offset），并且 offset 前的最后 64 字节
    与写下时一致——文件被替换、截短、copytruncate 后重写都会被这两条挡住。
    """
    offset = entry.get("offset")
    key = entry.get("key")
    tail_hex = entry.get("tail")
    if not _usage_cache_int(offset) or offset <= 0:
        return None
    if not isinstance(key, list) or len(key) != 3 or not all(
            _usage_cache_int(v) for v in key):
        return None
    if not isinstance(tail_hex, str):
        return None
    try:
        tail = bytes.fromhex(tail_hex)
    except ValueError:
        return None
    if not _log_offset_holds(tuple(key), log_key, offset):
        return None
    if _log_tail_signature(offset) != tail:
        return None
    return offset, tail


def _usage_cache_fingerprints_ok(entry, pricing_used_ok, realm_key):
    """「用到的价」与 realm 指纹必须和当前进程读到的一致。

    价格一侧由调用方用 _pricing_used_unchanged(entry["pricing"], pricing)
    判好（记录形状不符、价表读不出来、总开关翻转都在那里返回 False），这里
    只把它和 realm 一侧一起收口。任一侧取不到（None）都算不一致：取不到就
    意味着「折叠当时读了什么」已经无法确认，缓存不能当有效用。
    """
    if not pricing_used_ok or realm_key is None:
        return False
    return entry.get("realm_inputs") == realm_key


def _usage_cache_note_adopted(kind, offset):
    """采用成功后的节流初始化：刚加载的 offset 不算「有新字节要写」。

    不初始化的话，重启后第一次刷新会把「offset 从 0 涨到文件尾」当成一次大
    推进，白白重写一份内容相同的文件。
    """
    global _usage_cache_last_attempt
    with _usage_cache_lock:
        if offset > _usage_cache_progress.get(kind, 0):
            _usage_cache_progress[kind] = offset
        if offset > _usage_cache_checkpointed.get(kind, 0):
            _usage_cache_checkpointed[kind] = offset
        _usage_cache_last_attempt = time.time()


def _usage_cache_adopt_snapshot(r, state, log_key, pricing, realm):
    """把 checkpoint 里对应 realm 的 snapshot 折叠装进 state；成功返回 True。

    调用方持有 _usage_snap_state_lock，并且只在 state 全新时调用：采用只把
    offset/tail 往前挪，折叠逻辑一行不动。

    闭窗口的边沿账（hits/hits_overflow）是这一份的必填项：旧版 checkpoint
    没有这两个键，会被拒 → 升级后第一次刷新重折一次（一次性代价，与
    v3→v4 缺小时表时的处置一致）。
    """
    entry = _usage_cache_take("snapshot", r)
    if entry is None:
        return False
    position = _usage_cache_position_ok(entry, log_key)
    if position is None:
        return False
    if not _usage_cache_fingerprints_ok(
            entry, _pricing_used_unchanged(entry.get("pricing"), pricing),
            _realm_inputs_key(realm)):
        return False
    snap = entry.get("snap")
    if not _usage_cache_snapshot_ok(snap):
        return False
    days = entry.get("days")
    floor = entry.get("days_floor")
    if not _usage_cache_days_ok(days, floor, _usage_cache_snapshot_ok):
        return False
    hours_floor = entry.get("hours_floor_at")
    if hours_floor is not None and not _usage_cache_number(hours_floor):
        return False
    hits = entry.get("hits")
    overflow = entry.get("hits_overflow")
    if not _usage_cache_hits_ok(hits, overflow):
        return False
    offset, tail = position
    state.update({"snap": snap, "offset": offset, "tail": tail,
                  "days": days, "days_floor": floor,
                  "hours_floor_at": _hours_floor_on_adopt(days, hours_floor),
                  "hits": list(hits), "hits_overflow": overflow,
                  # 「用到的价」记录跟着状态一起装进来：后续增量折叠继续往
                  # 这份记录里加，下一次判废拿它比对当前价表。
                  "pricing": _copy_pricing_record(entry.get("pricing"))})
    _usage_cache_note_adopted("snapshot", offset)
    return True


def _usage_cache_adopt_by_account(state, log_key):
    """by_account 折叠的采用；这一份没有价格/realm 指纹（折叠不读它们）。"""
    entry = _usage_cache_take("by_account")
    if entry is None:
        return False
    position = _usage_cache_position_ok(entry, log_key)
    if position is None:
        return False
    buckets = entry.get("buckets")
    if not _usage_cache_buckets_ok(buckets):
        return False
    offset, tail = position
    state.update({"buckets": buckets, "offset": offset, "tail": tail})
    _usage_cache_note_adopted("by_account", offset)
    return True


def _usage_cache_adopt_remaining(state, log_key):
    """剩余用量折叠的采用；同样没有价格/realm 指纹（折叠不读它们）。

    事件日志的位置在这里单独校验：它很小，位置对不上就整份放弃——冷读一遍
    两个文件比「先采用一半、再发现对不上」简单得多。
    每个 pair 还要过一遍 _pair_repair：旧版写出的缓冲按日志顺序排列，`at`
    可能回退几毫秒，这里统一整理成按 `at` 稳定排序的规范形态（有序的文件
    上它就是一次 O(n) 的核对）。整理不了的（负增量=坏数据）整份放弃。
    """
    entry = _usage_cache_take("remaining")
    if entry is None or not _usage_cache_remaining_ok(entry):
        return False
    position = _usage_cache_position_ok(entry, log_key)
    if position is None:
        return False
    try:
        file_state = {"offset": entry["file"]["offset"],
                      "key": tuple(entry["file"]["key"]),
                      "tail": bytes.fromhex(entry["file"]["tail"])}
    except ValueError:
        return False
    if not _log_resume_ok(file_state, _usage_log_key(LIMIT_EVENTS_FILE),
                          LIMIT_EVENTS_FILE):
        return False
    offset, tail = position
    usage = {}
    for pair_key, pair in entry["usage"].items():
        uid, _, model = pair_key.partition("\x1f")
        buf = {"at": list(pair["at"]), "cum": list(pair["cum"]),
               "head": pair["head"], "base": pair["base"]}
        if not _pair_repair(buf):
            return False
        usage[(uid, model)] = buf
    state["log"] = {"offset": offset, "key": tuple(entry["key"]), "tail": tail}
    state["file"] = file_state
    state["usage"] = usage
    state["events"] = [dict(event) for event in entry["events"]]
    state["keys"] = {(event["account"], event["model"], float(event["reset"]))
                     for event in state["events"]}
    _usage_cache_note_adopted("remaining", offset)
    return True


def _usage_cache_adopt_analytics(realm, state, log_key, pricing, realm_inputs):
    """analytics all-time 折叠的采用；闭窗口的边沿账是必填项（见 snapshot 版）。"""
    entry = _usage_cache_take("analytics", realm)
    if entry is None:
        return False
    position = _usage_cache_position_ok(entry, log_key)
    if position is None:
        return False
    if not _usage_cache_fingerprints_ok(
            entry, _pricing_used_unchanged(entry.get("pricing"), pricing),
            _realm_inputs_key(realm_inputs)):
        return False
    maps = entry.get("maps")
    if not _usage_cache_analytics_ok(maps):
        return False
    days = entry.get("days")
    floor = entry.get("days_floor")
    if not _usage_cache_days_ok(days, floor, _usage_cache_analytics_day_ok):
        return False
    max_at = entry.get("max_at")
    if not _usage_cache_number(max_at):
        return False
    hits = entry.get("hits")
    overflow = entry.get("hits_overflow")
    if not _usage_cache_hits_ok(hits, overflow):
        return False
    offset, tail = position
    state.update({"maps": maps, "offset": offset, "tail": tail,
                  "days": days, "days_floor": floor, "max_at": max_at,
                  "hits": list(hits), "hits_overflow": overflow,
                  "pricing": _copy_pricing_record(entry.get("pricing"))})
    _usage_cache_note_adopted("analytics", offset)
    return True


def _usage_cache_adopt_series(r, state, log_key, realm_inputs):
    """时间序列状态的采用；这一份只有 realm 指纹（折叠不算价）。"""
    entry = _usage_cache_take("series", r)
    if entry is None:
        return False
    position = _usage_cache_position_ok(entry, log_key)
    if position is None:
        return False
    realm_key = _realm_inputs_key(realm_inputs)
    if realm_key is None or entry.get("realm_inputs") != realm_key:
        return False
    days = entry.get("days")
    floor = entry.get("days_floor")
    if not _usage_cache_days_ok(days, floor, _usage_cache_series_bucket_ok):
        return False
    max_at = entry.get("max_at")
    if not _usage_cache_number(max_at):
        return False
    credits = entry.get("credits")
    if not _usage_cache_credits_ok(credits):
        return False
    minutes = entry.get("minutes")
    if not _usage_cache_series_minutes_ok(minutes):
        return False
    minutes_from = entry.get("minutes_from")
    if minutes_from is not None and not _usage_cache_number(minutes_from):
        return False
    hits = entry.get("hits")
    overflow = entry.get("hits_overflow")
    if not _usage_cache_hits_ok(hits, overflow):
        return False
    if minutes is None:
        # 旧版 checkpoint（没有分钟环这个键）：这一份状态是能用的（日/小时桶
        # 与 credits 都在），只是分钟环要从采用点之后才开始折。把完整线记成
        # 采用时的 max_at——前缀里所有行的 at 都 <= 它，所以起点晚于它的窗口
        # 只碰得到之后折进来的行，环对它们是完整的；早于它的窗口一律退回
        # 整段扫描（见 _series_slice_minutes）。
        minutes = []
        minutes_from = max_at
    offset, tail = position
    state.update({"days": days, "days_floor": floor, "max_at": max_at,
                  "credits": credits, "offset": offset, "tail": tail,
                  "minutes": _usage_cache_minutes_state(minutes),
                  "minutes_from": minutes_from,
                  "hits": list(hits), "hits_overflow": overflow})
    _usage_cache_note_adopted("series", offset)
    return True


def _usage_cache_days_copy(days, copy_bucket):
    """日桶表的深拷贝（序列化在锁外做，桶必须和活状态脱钩）。"""
    return {key: copy_bucket(bucket) for key, bucket in (days or {}).items()}


def _copy_series_day(bucket):
    """一个时间序列日桶的深拷贝（含 hours 子表）。"""
    copy = dict(bucket)
    copy["hours"] = {h: dict(sub) for h, sub in (bucket.get("hours") or {}).items()}
    return copy


def _copy_snapshot_day(bucket):
    """一个窗口半日桶的深拷贝（含 hours 子表，见 _usage_hour_bucket）。"""
    copy = _copy_usage_snapshot(bucket)
    hours = bucket.get("hours")
    if hours is not None:
        copy["hours"] = {h: _copy_usage_snapshot(sub)
                         for h, sub in hours.items()}
    return copy


# 分钟环落盘行的字段顺序：[分钟, 这 8 个数字]（at 由分钟推出，省下每桶 10
# 字节；credit 在最后，与 _new_series_bucket 的键序一致）。
_SERIES_MINUTE_FIELDS = ("requests", "errors", "prompt_tokens",
                         "completion_tokens", "reasoning_tokens",
                         "cached_tokens", "total_tokens", "credit")


def _usage_cache_minutes_copy(minutes):
    """分钟环的落盘形态：按分钟升序的 [分钟, 8 个数字] 行。

    形状与内存里的字典桶不同（键换成行首的整数、字段名省掉），这是刻意的
    体积预算：环最多 361 个桶（6 小时），字典形状每个桶 ~150 字节、满环要
    50KB 级，位置数组把每个桶压到 40~50 字节（本机真实数据实测 52）。形状
    差异只存在于落盘/加载这一层：加载端（_usage_cache_minutes_state）还原
    成与冷折完全一样的字典桶，切片与输出的字节不受影响。排序写出而不是按
    插入顺序，是为了让文件内容与折叠顺序无关（冷折与「采用后再折」写出的
    这份字节一致）。状态里没有环（不该发生，但落盘不靠断言）时写 null：
    加载端按「环不完整」处理。
    """
    if minutes is None:
        return None
    rows = []
    for key in sorted(minutes or {}):
        bucket = minutes[key]
        rows.append([int(key)] + [bucket.get(field)
                                  for field in _SERIES_MINUTE_FIELDS])
    return rows


def _usage_cache_snapshot_entry(realm, state):
    """一份 snapshot 状态的 JSON 形态；不适合落盘时返回 None。

    只写「折叠成功过」的状态：错误路径会把 offset 归零，这里直接跳过——
    offset 为 0 的缓存没有任何加速作用，写进去只会让加载端白校验一遍。
    """
    offset = state.get("offset") or 0
    tail = state.get("tail")
    if offset <= 0 or not isinstance(tail, bytes) or state.get("snap") is None:
        return None
    # 「用到的价」记录（T15；见「用到的价」状态指纹一节）：形状不合格就不写。
    # 记录为 None（总开关关着 / 价表读不出来）时也跳过——那份折叠没读任何价，
    # 落盘没有任何加速作用（采纳端只认「记录也是 None」这一种一致）。
    record = state.get("pricing")
    if not _pricing_record_ok(record):
        return None
    realm_inputs = state.get("realm")
    realm_key = _realm_inputs_key(realm_inputs)
    if realm_key is None:
        return None
    if not _realm_fold_reproducible(realm_inputs[1]):
        # 这次折叠的归属冷进程复现不了（见 _realm_fold_reproducible）：写下去
        # 会被按新归属当成有效。宁可这次不写，等池就绪/重载后再写。
        return None
    return {"realm": realm, "offset": offset,
            "key": list(state["key"] or (0, 0, 0)), "tail": tail.hex(),
            # 记录的深拷贝在锁内做，序列化在锁外做（同下面几份深拷贝）。
            "pricing": _copy_pricing_record(record), "realm_inputs": realm_key,
            # 深拷贝在锁内做，序列化在锁外做（见 _usage_cache_collect）。
            "snap": _copy_usage_snapshot(state["snap"]),
            # 按日分桶的窗口半：与 all-time 半同一次扫描折出来的，位置也
            # 跟着同一条 offset/tail 校验，不另设一套。日桶里还挂着小时
            # 子表（T9 的小时层），深拷贝必须跟着一起做。
            "days": _usage_cache_days_copy(state.get("days"),
                                           _copy_snapshot_day),
            "days_floor": state.get("days_floor"),
            # 小时层的剪枝线（见 _hours_cover）：缺了它重启后的小时层要么
            # 不折（None 直接挡住 _usage_hour_bucket），要么服务了被剪过的
            # 天——两者都不对，所以它和日桶一起落盘。
            "hours_floor_at": state.get("hours_floor_at"),
            # 闭窗口的边沿账（见 _closed_window_hit_record）：直到某刻没有行
            # 落在整秒上的证明。旧版 checkpoint 没有这两个键，采用端因此
            # 拒整份、升级后一次性重折。
            "hits": list(state["hits"]),
            "hits_overflow": bool(state["hits_overflow"])}


def _usage_cache_by_account_entry(state):
    """by_account 折叠的 JSON 形态；同上，只写折叠成功过的。"""
    offset = state.get("offset") or 0
    tail = state.get("tail")
    buckets = state.get("buckets")
    if offset <= 0 or not isinstance(tail, bytes) or not isinstance(buckets, dict):
        return None
    return {"offset": offset, "key": list(state["key"] or (0, 0, 0)),
            "tail": tail.hex(),
            # 深拷贝：序列化在锁外做，桶里的 models 必须和活状态脱钩。
            "buckets": {k: dict(v, models=dict(v.get("models") or {}))
                        for k, v in buckets.items()}}


def _usage_cache_remaining_entry(state):
    """剩余用量折叠的 JSON 形态；同上，只写折叠成功过的。

    两份位置都要带上：usage.jsonl 的（加载端靠 _usage_cache_position_ok
    校验）和事件日志的（很小，加载端整份比对）。缓冲以 "uid\\x1fmodel" 为
    键——JSON 的对象键只能是字符串。
    """
    log = state.get("log") or {}
    file_slot = state.get("file") or {}
    events = state.get("events")
    usage = state.get("usage")
    offset = log.get("offset") or 0
    tail = log.get("tail")
    if offset <= 0 or not isinstance(tail, bytes):
        return None
    if not isinstance(events, list) or not isinstance(usage, dict):
        return None
    buffers = {}
    for (uid, model), pair in usage.items():
        if not isinstance(pair, dict):
            return None
        buffers["%s\x1f%s" % (uid, model)] = {
            "at": list(pair.get("at") or ()),
            "cum": list(pair.get("cum") or ()),
            "head": int(pair.get("head") or 0),
            "base": int(pair.get("base") or 0),
        }
    return {"offset": offset, "key": list(log.get("key") or (0, 0, 0)),
            "tail": tail.hex(),
            "file": {"offset": int(file_slot.get("offset") or 0),
                     "key": list(file_slot.get("key") or (0, 0, 0)),
                     "tail": (file_slot.get("tail") or b"").hex()},
            "events": [dict(event) for event in events],
            "usage": buffers}


def _usage_cache_analytics_entry(realm, state):
    """analytics all-time 折叠的 JSON 形态；同上。"""
    offset = state.get("offset") or 0
    tail = state.get("tail")
    if offset <= 0 or not isinstance(tail, bytes):
        return None
    record = state.get("pricing")
    if not _pricing_record_ok(record):
        return None          # 同 snapshot：「用到的价」记录不合格就不写
    realm_inputs = state.get("realm")
    realm_key = _realm_inputs_key(realm_inputs)
    if realm_key is None:
        return None
    if not _realm_fold_reproducible(realm_inputs[1]):
        return None          # 同 snapshot：这次折叠的归属冷进程复现不了
    return {"realm": realm, "offset": offset,
            "key": list(state["key"] or (0, 0, 0)), "tail": tail.hex(),
            "pricing": _copy_pricing_record(record), "realm_inputs": realm_key,
            "maps": _copy_analytics_all_time(state["maps"]),
            "days": _usage_cache_days_copy(state.get("days"),
                                           _copy_analytics_window_maps),
            "days_floor": state.get("days_floor"),
            # 折过的最大 at：小时层靠它判断 until 有没有切掉桶里的行。
            "max_at": state.get("max_at") or 0,
            # 闭窗口的边沿账（见 _closed_window_hit_record）：旧版 checkpoint
            # 没有这两个键，采用端因此拒整份、升级后一次性重折。
            "hits": list(state["hits"]),
            "hits_overflow": bool(state["hits_overflow"])}


def _usage_cache_series_entry(realm, state):
    """时间序列状态的 JSON 形态；同上，只写折叠成功过的。

    这一份最大（日桶 + 24 小时子桶 × 保留天数），所以深拷贝只拷桶本身，
    credit 缓冲是 dict 列表、直接浅拷外层。分钟环再压一层：落盘用位置数组
    而不是字典桶（见 _usage_cache_minutes_copy），满环 361 个桶也就 10KB
    量级（本机实测每桶 52 字节），字典形状会平白多出几十 KB。
    """
    offset = state.get("offset") or 0
    tail = state.get("tail")
    if offset <= 0 or not isinstance(tail, bytes):
        return None
    realm_key = _realm_inputs_key(state.get("realm"))
    if realm_key is None:
        return None
    return {"realm": realm, "offset": offset,
            "key": list(state["key"] or (0, 0, 0)), "tail": tail.hex(),
            "realm_inputs": realm_key,
            "days": _usage_cache_days_copy(state.get("days"), _copy_series_day),
            "days_floor": state.get("days_floor"),
            "max_at": state.get("max_at") or 0,
            "credits": [dict(item) for item in (state.get("credits") or [])],
            "minutes": _usage_cache_minutes_copy(state.get("minutes")),
            "minutes_from": state.get("minutes_from"),
            # 闭窗口的边沿账（见 _closed_window_hit_record）：旧版 checkpoint
            # 没有这两个键，采用端因此拒整份、升级后一次性重折。
            "hits": list(state["hits"]),
            "hits_overflow": bool(state["hits_overflow"])}


def _usage_cache_collect_kind(kind):
    """逐把取一类状态，做锁外可序列化的深拷贝；返回 (entries, offset) 或 None。

    每次只取一类：写盘路径按类分别调用，只重写「有推进的那一类」。状态各自
    加锁、各自拷贝，各类之间不强求同一个瞬间：加载端对每份分别校验 offset/
    tail/指纹，一份新一份旧也各自成立。锁是逐把拿、随即放开的，不存在嵌套
    ——写盘路径不能制造「snapshot -> analytics」这样的锁序边。

    entries 为空（状态都归零/不可复现）时返回 None：没得写就不写，等下一次
    折叠成功再落盘。offset 是该类各 realm 状态里的最远位置。
    """
    if kind == "snapshot":
        entries = []
        done = 0
        with _usage_snap_state_lock:
            for realm, state in _usage_snap_state.items():
                entry = _usage_cache_snapshot_entry(realm, state)
                if entry is not None:
                    entries.append(entry)
                    done = max(done, state["offset"])
        return (entries, done) if entries else None
    if kind == "by_account":
        with _byacct_state_lock:
            entry = _usage_cache_by_account_entry(_byacct_state)
            if entry is None:
                return None
            return [entry], _byacct_state["offset"]
    if kind == "analytics":
        entries = []
        done = 0
        with _analytics_state_lock:
            for realm, state in _analytics_state.items():
                entry = _usage_cache_analytics_entry(realm, state)
                if entry is not None:
                    entries.append(entry)
                    done = max(done, state["offset"])
        return (entries, done) if entries else None
    if kind == "series":
        entries = []
        done = 0
        with _series_state_lock:
            for realm, state in _series_state.items():
                entry = _usage_cache_series_entry(realm, state)
                if entry is not None:
                    entries.append(entry)
                    done = max(done, state["offset"])
        return (entries, done) if entries else None
    if kind == "remaining":
        with _remaining_state_lock:
            entry = _usage_cache_remaining_entry(_remaining_state)
            if entry is None:
                return None
            return [entry], entry["offset"]
    return None


def _usage_cache_write_kind(kind, entries):
    """原子写一类状态的 checkpoint：同目录临时文件 + os.replace。

    直接在目标文件上写，崩在中间会留下半份 JSON；加载端虽然能识别（解析失败
    按没有缓存处理），但一份完整的旧文件更省事。不 fsync：这是一份加速件，
    掉电后写坏或丢掉的，加载端一律当没有缓存，不值得为它多刷一次盘。

    不排序键：checkpoint 里 dict 的键顺序就是折叠时的插入顺序（先出现的先
    排），加载回来必须与冷启动逐字节一致——响应的 JSON 字节里键序是可见的，
    而冷启动折叠出的顺序正是这份插入顺序。排序只留给 _object_digest() 那种
    「只要内容一样就算一样」的指纹。

    盘上「本进程还没碰过」的 realm 条目原样保留：一类里按 realm 分条目，
    写盘只重写活跃 realm 的那些；不保留的话，重启后先折的 realm 触发一次写盘
    就会把别的 realm 的旧条目整份清掉，那些 realm 下次请求只能重折（真机
    实测：重启后先请求 realm=all 再请求 realm=intl，intl 重折 15s）。保留的
    条目仍要过加载端的逐条校验，位置/指纹对不上照样被拒——宁可多扫一次。
    """
    if kind in ("snapshot", "analytics", "series"):
        live_realms = {entry.get("realm") for entry in entries}
        with _usage_cache_lock:
            old = _usage_cache_read_kind(kind) or []
        keep = [e for e in old
                if isinstance(e, dict) and e.get("realm") not in live_realms]
        if keep:
            entries = list(entries) + keep
    payload = {"schema": _USAGE_CACHE_SCHEMA, "kind": kind,
               "written_at": time.time(), "entries": entries}
    path = _usage_cache_kind_path(kind)
    tmp = path + ".tmp"
    text = json.dumps(payload, ensure_ascii=False, separators=(",", ":"))
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
        os.replace(tmp, path)
    except OSError:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def _usage_cache_maybe_save(kind, offset):
    """一次折叠成功后，按节流阈值决定要不要把「有推进的类」落盘。

    调用点必须在状态锁之外：这里要逐把取状态，在状态锁里再取别的状态锁
    会构成锁序环（snapshot -> analytics 与反向同时存在）。任何异常都吞掉——
    落盘失败只损失一次加速，不能影响这次刷新。

    写盘按类分开（v4）：没推进的类连文件都不重写——旧布局里一份文件装五类，
    任何一类推进都要整份重写；现在单视图进程只重写自己那一类的几十 KB。

    写哪几类：字节阈值到点的类（progress - checkpointed >= min_bytes）里，写
    调用方那一类（它刚折完，状态最新）和真正推进过的类；计时器到点而没有任何
    类到字节阈值时，写所有推进过的类。字节阈值设成 0（测试/探针的强制写）时
    「到点」对所有折过的类都成立，过滤器把没推进过的那些留在原地——旧布局里
    它们的内容与盘上逐字节相同，重写只是写放大。
    """
    global _usage_cache_last_attempt
    if not _usage_cache_enabled():
        return
    try:
        # 串行化整段写盘：两个请求线程可能同时判定「该写」，而临时文件名是
        # 固定的（path + ".tmp"，和 save_runtime_override 一致），两个写者
        # 同时打开同一个临时文件会互相截断。等锁的一方在锁里会重新判定一遍，
        # 第一个写者已经把 checkpointed 推上去，于是它多半直接返回。
        with _usage_cache_write_lock:
            now = time.time()
            with _usage_cache_lock:
                if offset > _usage_cache_progress.get(kind, 0):
                    _usage_cache_progress[kind] = offset
                min_bytes = _usage_cache_min_bytes()
                min_seconds = _usage_cache_min_seconds()
                due_kinds = [k for k in _usage_cache_progress
                             if _usage_cache_progress.get(k, 0)
                             - _usage_cache_checkpointed.get(k, 0) >= min_bytes]
                # 计时器分支只在**确有推进**时才允许触发：进度没动时写出的
                # 是一份与上次逐字节相同的 checkpoint，纯写放大——面板开着
                # 就每 900s 白写一份 120KB（复现见套件 [7]）。空闲重写对
                # 「重启后免冷扫」没有任何贡献，只有日志增长才值得落盘。
                advanced = [k for k in _usage_cache_progress
                            if _usage_cache_progress.get(k, 0)
                            > _usage_cache_checkpointed.get(k, 0)]
                if not due_kinds and (not advanced
                                      or (now - _usage_cache_last_attempt)
                                      < min_seconds):
                    return
                # 记「尝试」而不是「成功」：只读文件系统上失败会一直成立，
                # 不退避的话每次刷新都去试写一遍。
                _usage_cache_last_attempt = now
                write_kinds = [k for k in due_kinds
                               if k == kind or k in advanced] or advanced
            # 某一类收集不出东西（状态归零/归属不可复现）就跳过它：其余类照常
            # 落盘，互不牵连——这是拆文件的意义所在。
            for advanced_kind in write_kinds:
                try:
                    collected = _usage_cache_collect_kind(advanced_kind)
                    if collected is None:
                        continue
                    entries, done = collected
                    _usage_cache_write_kind(advanced_kind, entries)
                except Exception as exc:
                    log("usage aggregate cache write failed (%s): %s"
                        % (advanced_kind, exc))
                    continue
                with _usage_cache_lock:
                    if done > _usage_cache_checkpointed.get(advanced_kind, 0):
                        _usage_cache_checkpointed[advanced_kind] = done
    except Exception as exc:
        log("usage aggregate cache write failed: %s" % exc)


def runtime_settings_view():
    """Current panel-visible settings (never returns the password or the key)."""
    key = API_KEY or ""
    if len(key) > 8:
        masked = key[:4] + "*" * 6 + key[-4:]
    else:
        masked = "*" * len(key)
    keys = []
    for entry in configured_keys():
        raw = entry.get("key") or ""
        keys.append({
            "id": entry.get("id") or "",
            "name": entry.get("name") or "",
            "realm": entry.get("realm") or "",
            "models": list(entry.get("models") or []),
            "enabled": entry.get("enabled", True) is not False,
            "masked": (raw[:4] + "*" * 6 + raw[-4:]) if len(raw) > 8 else "*" * len(raw),
            "source": entry.get("source") or "panel",
            "created_at": entry.get("created_at") or "",
        })
    # Deleted keys are read-only history: the secret is gone, so the panel can
    # only list them (name and dates) and must not offer a copy button.
    deleted_keys = []
    for entry in wb_settings.api_keys(ACCOUNTS_DIR, include_deleted=True):
        if not entry.get("deleted_at"):
            continue
        deleted_keys.append({
            "id": entry.get("id") or "",
            "name": entry.get("name") or "",
            "realm": entry.get("realm") or "",
            "created_at": entry.get("created_at") or "",
            "deleted_at": entry.get("deleted_at") or "",
        })
    return {
        "panel_password_is_default": wb_settings.panel_password_is_default(ACCOUNTS_DIR),
        "api_key_set": bool(key),
        "api_key_set_by_panel": API_KEY_FILE_SET,
        "api_key_masked": masked,
        "auth_required": auth_required(),
        "api_keys": keys,
        "deleted_api_keys": deleted_keys,
        "limits": wb_settings.limits_snapshot(ACCOUNTS_DIR),
        "reserve_credits": wb_settings.reserve_credits(ACCOUNTS_DIR),
        "daily_token_limit": wb_settings.daily_token_limit(ACCOUNTS_DIR),
        "daily_credit_limit": wb_settings.daily_credit_limit(ACCOUNTS_DIR),
        "model_daily_token_limit": wb_settings.model_daily_token_limit(ACCOUNTS_DIR),
        "pricing_refresh_minutes": wb_settings.pricing_refresh_minutes(ACCOUNTS_DIR),
        "credits_refresh_hours": wb_settings.credits_refresh_hours(ACCOUNTS_DIR),
        "section_max_height": wb_settings.section_max_height(ACCOUNTS_DIR),
        "ui_language": wb_settings.ui_language(ACCOUNTS_DIR),
        "pricing_variant_inherit": wb_settings.pricing_variant_inherit(ACCOUNTS_DIR),
        "pricing_enabled": wb_settings.pricing_enabled(ACCOUNTS_DIR),
        "auto_switch_product": wb_settings.auto_switch_product(ACCOUNTS_DIR),
        "daily_chat_web": wb_settings.daily_chat_web(ACCOUNTS_DIR),
        "daily_chat_model": wb_settings.daily_chat_model(ACCOUNTS_DIR),
        "daily_chat_model_default": wb_accounts.DAILY_CHAT_MODEL,
        "local_web_tools": wb_settings.local_web_tools(ACCOUNTS_DIR),
        "remaining_priority_enabled":
            wb_settings.remaining_priority_enabled(ACCOUNTS_DIR),
        "egress_backoff_enabled":
            wb_settings.egress_backoff_enabled(ACCOUNTS_DIR),
        "accounts_collapsed": wb_settings.accounts_collapsed(ACCOUNTS_DIR),
        "accounts_separate_tab": wb_settings.accounts_separate_tab(ACCOUNTS_DIR),
        "key_before_hidden": wb_settings.key_before_hidden(ACCOUNTS_DIR),
        "hidden_pages": wb_settings.hidden_pages(ACCOUNTS_DIR),
        "update_check_enabled": wb_settings.update_check_enabled(ACCOUNTS_DIR),
        "upstream": wb_settings.upstream_config(ACCOUNTS_DIR),
        "prompt": wb_settings.prompt_config(ACCOUNTS_DIR),
        "accounts_dir": ACCOUNTS_DIR,
        "usage_dir": USAGE_DIR,
        "settings_file": wb_settings.settings_path(ACCOUNTS_DIR),
        "version": "1.7.3",
    }
def current_account():
    """Account used for display purposes (health / usage summaries)."""
    return POOL.representative() if POOL else None
# ---------------------------------------------------------------------------
# Prefix-based session affinity (PATCHED-BY-OPS)
# ---------------------------------------------------------------------------
# 上游 prompt cache 是【账号级】的：只有同一个账号再次看到相同前缀才会命中。
# 实测证据（wk 实例 11 个号）：8 次完全相同的前缀请求被轮询分散到 8 个账号，
# 缓存率全部为 0%；而带上会话标识固定落到同一账号时，第 2 次起缓存率即 95.2%。
#
# sub2api / DSH 等客户端并不发送 X-Conversation-Id 之类的会话标识，
# 于是 hub 走纯轮询，同一对话每一轮都换账号，缓存必然归零。
#
# 这里在缺少显式会话键时，用【对话稳定前缀】派生亲和键：
# 取消息列表的前两条（system + 首条 user），它们在整段对话生命周期内不变，
# 因此同一对话的每一轮都会落到同一账号；而不同对话的首条 user 不同，
# 依旧会分散到各账号，负载均衡不受影响。
AFFINITY_BY_PREFIX = os.environ.get("WB_AFFINITY_BY_PREFIX", "1").lower() not in (
    "0", "false", "no", "off")
AFFINITY_DEBUG = os.environ.get("WB_AFFINITY_DEBUG", "0").lower() in (
    "1", "true", "yes", "on")
# 亲和的上限：对话超过这么多条消息后不再绑定账号。
#
# 为什么需要上限：亲和把整段对话钉在同一个账号上，而对话的上下文是单调
# 增长的，于是那一个账号要反复接收越来越大的请求体。实测（wk4 实例，
# 2026-10-08，333 个请求）请求体与上游断连率的关系：
#
#     msgs <150    82 个请求   断连率  0.0%
#     msgs 150-300 59 个请求   断连率  3.4%
#     msgs 300-400 64 个请求   断连率  7.8%
#     msgs 400-500 50 个请求   断连率 10.0%
#
# 断连（TimeoutError / RemoteDisconnected）会触发重试，把一个 8.8s 的请求
# 拖到 11.6s，首字延迟随之翻倍。超过阈值后放弃亲和，让这个对话重新参与
# 轮询：代价是它丢掉前缀缓存，收益是断连和重试消失。
#
# 阈值不能设得太低，否则会波及正常长度的对话（本实例 94% 的请求缓存命中率
# 来自亲和）。设 0 表示不限制，保持 1.6.x 的原有行为。
AFFINITY_MAX_MSGS = int(os.environ.get("WB_AFFINITY_MAX_MSGS", "400") or 0)
# 超限对话改成【页内轮转】而不是撒到整个池子。
#
# 为什么：整池轮转下每个账号要等「池子大小」轮才再见到这段对话，间隔一旦
# 超过上游缓存的存活期就整段漏。线上实测（2026-10-10，37706 条请求）：超限
# 对话占 59% 的 prompt token，却贡献了 64% 的漏掉量，签名是只命中约 24k 的
# 系统提示（账号见过同一客户端的系统提示、没见过这段历史）——典型的「落在
# 没预热过的账号上」。页内轮转把「再见到」的周期从「池子大小」轮缩到 size
# 轮，页里每个账号都留着前缀；同时超大请求体仍摊在 size 个账号上，不是压回
# 一个（那正是 400 条上限要解决的断连问题）。
#
# 取值：≥2 = 页大小（账号数）；1 = 和普通对话一样钉住（等于取消上限）；
# 0 = 保持 1.6.x 的整池轮转。页的划分见 wb_accounts.AccountPool._page_slice。
AFFINITY_PAGE_SIZE = int(os.environ.get("WB_AFFINITY_PAGE_SIZE", "3") or 0)
def affinity_route(messages):
    """This conversation's (session_key, page).

    `session_key` is the stable conversation key; `page` is the (index, size)
    of the pool page a conversation past AFFINITY_MAX_MSGS rotates over, or
    None for every other case (short conversations, the page switched off, and
    conversations whose client supplied its own session key). Both come from
    the same head hash, so every turn of a conversation agrees on its key and
    its page without any server-side state.
    """
    if not AFFINITY_BY_PREFIX:
        return None, None
    try:
        msgs = messages or []
        if not msgs:
            return None, None
        head = msgs[:2]
        blob = json.dumps(head, ensure_ascii=False, sort_keys=True).encode("utf-8")
        digest = hashlib.sha256(blob).hexdigest()[:16]
        key = "pfx-" + digest
        if AFFINITY_MAX_MSGS > 0 and len(msgs) > AFFINITY_MAX_MSGS:
            if AFFINITY_PAGE_SIZE > 1:
                page = (int(digest[:8], 16) % AFFINITY_PAGE_SIZE, AFFINITY_PAGE_SIZE)
                if AFFINITY_DEBUG:
                    log("affinity: %d msgs (> %d), page %d/%d"
                        % (len(msgs), AFFINITY_MAX_MSGS, page[0], page[1]))
                return key, page
            if AFFINITY_PAGE_SIZE == 1:
                # Pinned like any other conversation: the cap is switched off.
                return key, None
            if AFFINITY_DEBUG:
                log("affinity: skip %d msgs (> %d), letting the pool rotate"
                    % (len(msgs), AFFINITY_MAX_MSGS))
            return None, None
        return key, None
    except Exception:
        return None, None
def derive_affinity_key(messages):
    """Conversation key only; see affinity_route for the pool page."""
    return affinity_route(messages)[0]
def prompt_fingerprint(messages):
    """Privacy-safe fingerprint of the outgoing prompt.
    Cache hits need a byte-identical prefix, so these hashes answer "is my
    prefix stable / is my conversation continuous?" without storing any text.
    """
    try:
        def h(obj):
            blob = json.dumps(obj, ensure_ascii=False, sort_keys=True).encode("utf-8")
            return hashlib.sha256(blob).hexdigest()[:12]
        msgs = messages or []
        out = {"msgs_sha": h(msgs), "n_msgs": len(msgs)}
        if msgs:
            out["system_sha"] = h(msgs[0]) if msgs[0].get("role") == "system" else ""
            out["prefix_sha"] = h(msgs[:-1]) if len(msgs) > 1 else ""
        return out
    except Exception:
        return {}

LOG_BUFFER = deque(maxlen=2000)
_LOG_LOCK = threading.Lock()
_LOG_COUNTER = 0

def add_log_entry(msg, level=None, tag=None):
    global _LOG_COUNTER
    ts = time.strftime("%Y-%m-%d %H:%M:%S")
    t_short = time.strftime("%H:%M:%S")
    msg_str = str(msg).rstrip()
    if not level:
        lower = msg_str.lower()
        if any(k in lower for k in ("error", "exception", "failed", "11128", "11101", "11140", "traceback", "errno", "fatal")):
            level = "ERROR"
        elif any(k in lower for k in ("warn", "warning", "retry", "timeout")):
            level = "WARN"
        else:
            level = "INFO"
    if not tag:
        lower = msg_str.lower()
        if "chat:" in lower or "chat done" in lower or "/v1/chat" in lower or "/chat/completions" in lower or "responses" in lower:
            tag = "chat"
        elif "scheduler" in lower or "调度器" in lower:
            tag = "scheduler"
        elif "task" in lower or "任务" in lower or "打卡" in lower or "猫猫" in lower or "travel" in lower:
            tag = "tasks"
        elif "account" in lower or "账号" in lower or "pool" in lower or "imported" in lower:
            tag = "accounts"
        elif "model" in lower or "catalog" in lower or "模型" in lower:
            tag = "catalog"
        elif "auth" in lower or "token" in lower or "oauth" in lower:
            tag = "auth"
        elif "settings" in lower or "设置" in lower:
            tag = "settings"
        else:
            tag = "system"
    with _LOG_LOCK:
        _LOG_COUNTER += 1
        entry = {
            "id": _LOG_COUNTER,
            "ts": ts,
            "time": t_short,
            "level": level,
            "tag": tag,
            "msg": msg_str,
        }
        LOG_BUFFER.append(entry)
    return entry

def log(msg, level=None, tag=None):
    sys.stderr.write(f"[wb-proxy] {time.strftime('%H:%M:%S')} {msg}\n")
    sys.stderr.flush()
    add_log_entry(msg, level=level, tag=tag)

# 本地网路工具的多引擎轮换、冷却与失败分类写进看板日志（tag=webtools）
wb_webtools.set_logger(log)

def get_logs(limit=200, level="", tag="", search="", since_id=0):
    with _LOG_LOCK:
        items = list(LOG_BUFFER)
    if since_id > 0:
        items = [x for x in items if x["id"] > since_id]
    if level:
        items = [x for x in items if x["level"] == level.upper()]
    if tag:
        items = [x for x in items if x["tag"].lower() == tag.lower()]
    if search:
        s = search.lower()
        items = [x for x in items if s in x["msg"].lower() or s in x["tag"].lower()]
    total = len(items)
    if limit and limit > 0 and since_id == 0:
        items = items[-limit:]
    max_id = items[-1]["id"] if items else since_id
    return {"total": total, "logs": items, "max_id": max_id}

def clear_logs():
    with _LOG_LOCK:
        LOG_BUFFER.clear()

# ---------------------------------------------------------------------------
# upstream helpers
# ---------------------------------------------------------------------------
#: Auxiliary models the API advertises but that are not usable for chat.
#: "lite" backs internal helpers (title generation, compaction) and upstream
#: rejects it with 11102; the codewise/completion entries are text-completion
#: or IDE-inline models, not chat models.
# Exclude WorkBuddy virtual aliases / quick presets
VIRTUAL_ALIAS_MODELS = {
    "default-model",
    "fast-model",
    "balanced-model",
    "primary-model",
    "deep-model",
    # The domestic exit's auto-router entry: the picker shows it, but it is
    # not a model a client can pin, so it stays out of the advertised list.
    "auto",
}
NON_CHAT_MODELS = {"lite"} | VIRTUAL_ALIAS_MODELS
NON_CHAT_PREFIXES = ("codewise-", "completion-")
NON_CHAT_SUFFIXES = ("-image-alpha", "-image-alpha-edit", "-taco-completion")
def is_chat_model(mid):
    if not mid:
        return False
    if mid in NON_CHAT_MODELS:
        return False
    if mid.startswith(NON_CHAT_PREFIXES):
        return False
    if mid.endswith(NON_CHAT_SUFFIXES):
        return False
    return True
CN_UI_ORDER = [
    "hy4-preview-f",
    "hy3",
    "deepseek-v4.1-flash",
    "glm-5.3",
    "glm-5.3-flash",
    "glm-5.2",
    "glm-5.1",
    "glm-5v-turbo",
    "minimax-m3",
    "kimi-k3-1",
    "kimi-k2.8-preview",
    "kimi-k2.7",
    "kimi-k2.6",
    "deepseek-v4-pro",
]
INTL_UI_ORDER = [
    "hy4-preview-f",
    "hy3",
    "deepseek-v4.1-flash",
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
    "grok-4.7",
    "gemini-3.5-flash",
    "glm-5.3-flash",
    "glm-5.3",
    "glm-5.2",
    "kimi-k3",
    "kimi-k2.6",
    "kimi-k2.8-preview",
]
def merge_reasoning(base, live):
    """Field-level merge of one model's reasoning block.

    The live catalogue is the source of truth, but it does not always restate
    every field: the desktop endpoint sometimes gives a bare `effort` where the
    bundled table knows the level is selectable. A top-level update would drop
    `supportedEfforts`, and an `effort` without it reads as a pin - the model
    then looks unselectable in /v1/models and every request to it is reported at
    the wrong level. So the two blocks are merged field by field, and a block
    that ends up with `supportedEfforts` is selectable: its `effort` is the
    default restated, not a pin.
    """
    out = dict(base) if isinstance(base, dict) else {}
    if isinstance(live, dict):
        out.update(live)
    if out.get("supportedEfforts") and out.get("effort"):
        out.setdefault("defaultEffort", out["effort"])
        out.pop("effort", None)
    return out
def merge_catalog(primary, realm=None, extras=False):
    r = realm or CURRENT_REALM
    merged = {}
    # "all" is the union of both realms. The analytics dashboard lists every
    # model the gateway has served, so it must not drop the ones that only
    # one side's catalog knows about.
    if r == "all":
        source_static = list(wb_catalog.STATIC_INTL_MODELS) + list(wb_catalog.STATIC_CN_MODELS)
    else:
        source_static = getattr(wb_catalog, "STATIC_CN_MODELS" if r == "cn" else "STATIC_INTL_MODELS", wb_catalog.STATIC_MODELS)
    for item in source_static:
        mid = item.get("id")
        # First catalog wins for a shared id, so the intl entry is not
        # overwritten by its cn counterpart when both are merged.
        if mid and is_chat_model(mid) and mid not in merged:
            merged[mid] = dict(item)
    for mid, meta in primary or []:
        if not is_chat_model(mid):
            continue
        if meta:
            base = merged.get(mid) or {}
            base_reasoning = base.get("reasoning")
            base.update(meta)
            if isinstance(meta.get("reasoning"), dict):
                base["reasoning"] = merge_reasoning(base_reasoning, meta["reasoning"])
            merged[mid] = base
        elif mid not in merged:
            merged[mid] = {}
    order = CN_UI_ORDER if r == "cn" else INTL_UI_ORDER
    out = []
    if r == "all":
        order = list(INTL_UI_ORDER) + [m for m in CN_UI_ORDER if m not in INTL_UI_ORDER]
    seen = set()
    for mid in order:
        if mid in merged and mid not in seen:
            seen.add(mid)
            out.append((mid, merged[mid]))
    if extras:
        # A model the curated table has never heard of still ships when the
        # *live* catalogue lists it - that is how a newly added upstream model
        # reaches /v1/models without a release. The bundled snapshot alone is
        # not enough: it also carries legacy entries the picker may not show.
        for mid, _meta in primary or []:
            if mid in merged and mid not in seen:
                seen.add(mid)
                out.append((mid, merged[mid]))
    return out
_catalog_lock = threading.Lock()

_catalog_fallback_log = {}
def note_bundled_reasoning(realm, live, entries):
    """Say once when a model's reasoning controls come from the bundled table.

    The live catalogue is the source of truth and the snapshot is only the
    fallback, so values the snapshot alone carries can be no fresher than the
    snapshot. Two shapes put them there: a model the live catalogue gives no
    reasoning block at all, and one it gives a bare `effort` for where the
    snapshot knows the level is selectable. One line per change is enough.
    """
    live_meta = dict(live or [])

    def from_snapshot(mid, meta):
        reasoning = meta.get("reasoning") or {}
        if not reasoning:
            return False
        live_reasoning = (live_meta.get(mid) or {}).get("reasoning") or {}
        if not live_reasoning:
            return True
        return bool(reasoning.get("supportedEfforts")) \
            and not live_reasoning.get("supportedEfforts")

    missing = sorted(mid for mid, meta in entries if from_snapshot(mid, meta))
    if not missing:
        _catalog_fallback_log.pop(realm, None)
        return
    if _catalog_fallback_log.get(realm) == frozenset(missing):
        return
    _catalog_fallback_log[realm] = frozenset(missing)
    shown = ", ".join(missing[:6]) + (" ..." if len(missing) > 6 else "")
    log("catalog    : bundled table fills in reasoning controls the live "
        "catalogue omits for %d model(s): %s" % (len(missing), shown))
def fetch_models(realm=None):
    r = realm or CURRENT_REALM
    with _lock:
        c = _models_cache.get(r) or {"at": 0.0, "data": None}
        if c["data"] and time.time() - c["at"] < 300:
            return c["data"]
    # One upstream walk per realm even when several callers miss the cache at
    # the same moment: a batch of /v1/models requests must not turn into a
    # batch of upstream requests.
    with _catalog_lock:
        with _lock:
            c = _models_cache.get(r) or {"at": 0.0, "data": None}
            if c["data"] and time.time() - c["at"] < 300:
                return c["data"]
        live, extras = curated_live_sources(r)
        if not live and r in ("intl", "all"):
            # The narrow endpoint is not the desktop catalogue, so it keeps the
            # old whitelist behaviour: only names the order table knows.
            live = [(m, {}) for m in fetch_endpoint_models()]
            extras = False
        entries = merge_catalog(live, realm=r, extras=extras)
        note_bundled_reasoning(r, live, entries)
        with _lock:
            _models_cache[r] = {"at": time.time(), "data": entries}
        return entries
def model_entry(mid, meta):
    """Build a rich /v1/models entry from the desktop app catalog metadata.
    The OpenAI spec only names id/object/created/owned_by, so capability data is
    convention-driven. Several shapes are emitted at once so that different
    clients (OpenRouter-style, LobeChat-style, plain-flag readers) all find
    what they look for.
    """
    meta = meta or {}
    item = {
        "id": mid,
        "object": "model",
        "created": int(time.time()),
        "owned_by": "workbuddy",
    }
    name = meta.get("name")
    if name:
        item["name"] = name
    desc = meta.get("descriptionEn") or meta.get("descriptionZh")
    if desc:
        item["description"] = desc
    # ---- modality / capability ----
    # disabledMultimodal explicitly turns image input off; absent means allowed.
    vision = bool(meta.get("supportsImages")) and not meta.get("disabledMultimodal")
    tools = bool(meta.get("supportsToolCall"))
    thinks = bool(meta.get("supportsReasoning"))
    inputs = ["text"] + (["image"] if vision else [])
    # Capability flags under every spelling the common clients look for.
    # /v1/models has no standard for this, so each convention is emitted at
    # once rather than guessing which one a given client reads:
    #   capabilities.vision      generic
    #   supports_vision/images   LobeChat-style flat flags
    #   vision                   Cherry Studio / NextChat style
    #   abilities.vision         LobeChat
    #   multimodal               misc
    #   *_modalities             OpenRouter
    item["capabilities"] = {
        "vision": vision,
        "tool_calls": tools,
        "reasoning": thinks,
    }
    item["supports_vision"] = vision
    item["supports_images"] = vision
    item["supports_tool_calls"] = tools
    item["supports_reasoning"] = thinks
    item["vision"] = vision
    item["multimodal"] = vision
    item["abilities"] = {
        "vision": vision,
        "functionCall": tools,
        "function_call": tools,
        "reasoning": thinks,
    }
    item["input_modalities"] = inputs
    item["output_modalities"] = ["text"]
    item["modalities"] = {"input": inputs, "output": ["text"]}
    # OpenRouter-shaped block, read by several multi-provider clients.
    item["architecture"] = {
        "input_modalities": inputs,
        "output_modalities": ["text"],
        "modality": "+".join(inputs) + "->text",
    }
    # ---- limits ----
    # Four-level lookup: remote > built-in knowledge table > local model.json
    # cache > models.dev (asynchronously warmed; never blocks). context_length
    # always has a value (1M when unknown - a high estimate is safer than a
    # low one); max_output_tokens is omitted when unknown.
    ctx_value, out_value, _limit_source = wb_modelsdev.lookup(
        mid, meta.get("maxInputTokens"), meta.get("maxOutputTokens"),
        directory=ACCOUNTS_DIR)
    item["context_length"] = ctx_value
    item["max_input_tokens"] = ctx_value
    if out_value:
        item["max_output_tokens"] = out_value
        item["max_completion_tokens"] = out_value
    # Annotate the measured upstream clamp (from output_probes.json) without
    # overriding the model's spec value; the panel shows "钳制 N×" beside it.
    clamp = wb_probes.clamp_for(ACCOUNTS_DIR, mid)
    if clamp:
        item["output_clamp"] = clamp
        item["max_output_tokens_clamped"] = clamp
    ctx = (meta.get("contextWindow") or {}).get("supportedLengths")
    if ctx:
        item["context_windows"] = ctx
    # ---- reasoning controls ----
    reasoning = meta.get("reasoning") or {}
    efforts = reasoning.get("supportedEfforts")
    if efforts:
        item["reasoning_efforts"] = efforts
    if reasoning.get("effort"):
        item["reasoning_fixed_effort"] = reasoning["effort"]
    if reasoning.get("defaultEffort"):
        item["reasoning_default_effort"] = reasoning["defaultEffort"]
    if reasoning.get("canDisableThinking") is not None:
        item["reasoning_can_disable"] = reasoning["canDisableThinking"]
    if meta.get("onlyReasoning") is not None:
        item["always_reasoning"] = bool(meta.get("onlyReasoning"))
    # ---- misc ----
    if meta.get("credits"):
        item["credits"] = meta["credits"]
    if meta.get("vendor"):
        item["vendor"] = meta["vendor"]
    if meta.get("temperature") is not None:
        item["temperature"] = meta["temperature"]
    if meta.get("top_p") is not None:
        item["top_p"] = meta["top_p"]
    if meta.get("isDefault"):
        item["is_default"] = True
    tags = [t for t in (meta.get("tags") or []) if isinstance(t, str) and not t.startswith("badge:")]
    if tags:
        item["tags"] = tags
    return item
def read_product_config_models(realm=None):
    """Read the desktop app's cached catalog: [(id, meta), ...].

    "all" reads both apps when they are installed, so the combined view gets
    each side's metadata instead of only the domestic one.
    """
    r = realm or CURRENT_REALM
    if r == "all":
        out = _read_product_config_dir(".workbuddy-ai")
        seen = set(mid for mid, _ in out)
        for mid, meta in _read_product_config_dir(".workbuddy"):
            if mid not in seen:
                out.append((mid, meta))
        return out
    return _read_product_config_dir(".workbuddy-ai" if r == "intl" else ".workbuddy")


def _read_product_config_dir(cache_dir):
    home = os.path.expanduser("~")
    p = os.path.join(home, cache_dir, "cache", "acc-product-config-v3.json")
    try:
        with open(p, encoding="utf-8") as fh:
            cfg = json.load(fh)
    except Exception as exc:
        return []
    def find(node):
        if isinstance(node, dict):
            models = node.get("models")
            if isinstance(models, list) and models and isinstance(models[0], dict) and models[0].get("id"):
                return models
            for value in node.values():
                hit = find(value)
                if hit:
                    return hit
        return None
    models = find(cfg) or []
    out = []
    for m in models:
        mid = m.get("id")
        if isinstance(mid, str) and mid:
            out.append((mid, m))
    return out
#: The desktop client's own product-config endpoint. The cache file that
#: read_product_config_models() reads is this response written to disk, so
#: calling it directly is what lets a machine without the desktop app
#: (Docker, NAS, a headless server) advertise the live catalogue - live
#: multipliers included - instead of the narrower endpoint or the bundled
#: snapshot.
REMOTE_CONFIG_PATH = "/v3/config"

#: Suffixes that mark a variant of a name the catalogue already carries: the
#: regional build (deepseek-v4.1-flash-sg) and the experimental one (hy3-x).
#: Measured on both exits: the plain name is the free (x0.00) one and the
#: variant is the paid one, so the plain name is what gets advertised.
VARIANT_SUFFIXES = ("-sg", "-x")


def remote_config_headers(account, realm, ua=None):
    """Headers for the product-config call.

    The UA decides which catalogue comes back and only the desktop UA returns
    the full list (an unknown one is a hard 400, code 12403), so this uses a
    realm's fixed desktop UA rather than the account's current identity.
    """
    cfg = wb_accounts.get_realm_config(realm)
    return {
        "Accept": "application/json, text/plain, */*",
        "User-Agent": ua or cfg["chat_ua"],
        "Origin": cfg["origin"],
        "Referer": cfg["origin"] + "/",
        "Authorization": "Bearer " + account.access_token,
        "X-User-Id": account.uid,
    }


def _agent_model_lists(payload):
    """Every agent's bare-string model list, cli-named agents first.

    The catalogue the picker shows rides in agents[].models. The endpoint
    answers with it under a "data" key while the desktop cache file is the
    same document written to disk without that envelope, so both are read.
    """
    roots = [payload]
    data = payload.get("data")
    if isinstance(data, dict):
        roots.append(data)
    cli, other = [], []
    for root in roots:
        agents = root.get("agents")
        if isinstance(agents, dict):
            entries = list(agents.items())
        elif isinstance(agents, list):
            entries = [((entry.get("name") if isinstance(entry, dict) else None),
                        entry) for entry in agents]
        else:
            continue
        for name, entry in entries:
            if not isinstance(entry, dict):
                continue
            models = entry.get("models")
            if not (isinstance(models, list) and models
                    and isinstance(models[0], str)):
                continue
            ids = [str(m).strip() for m in models if isinstance(m, str)]
            ids = [m for m in ids if m]
            if not ids:
                continue
            (cli if str(name or "").strip().lower() == "cli" else other).append(ids)
    return cli, other


def parse_remote_catalog(payload):
    """(ids, meta) from a /v3/config response, or None when it carries none.

    The picker's list rides in agents[].models as bare ids - under "data" in
    the endpoint's answer, at the top level in the desktop cache file. The
    per-model metadata (credits, limits, copy) lives in a separate models
    array. An unusable credential answers HTTP 200 with an *empty* list, so
    an empty catalogue is reported as None and the caller falls back instead
    of publishing "this exit has no models".
    """
    if not isinstance(payload, dict):
        return None
    cli_lists, other_lists = _agent_model_lists(payload)
    pool = cli_lists or other_lists
    best = max(pool, key=len) if pool else None
    ids, seen = [], set()
    for mid in best or []:
        if mid not in seen:
            seen.add(mid)
            ids.append(mid)
    if not ids:
        return None

    meta = {}

    def walk(node):
        if isinstance(node, dict):
            models = node.get("models")
            if isinstance(models, list) and models \
                and isinstance(models[0], dict) and models[0].get("id"):
                for item in models:
                    mid = str(item.get("id") or "").strip()
                    if mid:
                        meta.setdefault(mid, item)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    walk(payload)
    return ids, meta


def snapshot_credits():
    """id -> credits from the bundled catalogue (both realms, intl first).

    Used to answer "is there a free sibling?" for a variant the remote lists
    but whose sibling it no longer does: the free hy4-preview-f, for example,
    is what the cn picker keeps while the remote only names the paid one.
    """
    out = {}
    for source in (getattr(wb_catalog, "STATIC_INTL_MODELS", []),
                           getattr(wb_catalog, "STATIC_CN_MODELS", [])):
        for item in source or []:
            mid = str(item.get("id") or "").strip()
            if mid:
                out.setdefault(mid, str(item.get("credits") or "").strip().lower())
    return out


def curate_remote_catalog(realm, ids, meta=None):
    """Trim a remote catalogue to the models the picker should offer.

      - virtual aliases (default-model ... auto) are not models;
      - "-sg" / "-x" builds are the paid variant of a name the list already
        carries;
      - when a free ("x0.00") sibling exists, the free one is the one the
        picker shows, so the paid sibling is dropped;
      - everything else keeps its upstream order. Names the upstream does not
        list at all stay available through the curated order tables and the
        bundled snapshot, which merge_catalog() keeps.
    """
    credits = snapshot_credits()
    for mid, item in (meta or {}).items():
        if isinstance(item, dict):
            credits[mid] = str(item.get("credits") or "").strip().lower()
    order = CN_UI_ORDER if realm == "cn" else INTL_UI_ORDER
    known = set(ids) | set(credits) | set(order)

    def free(mid):
        return credits.get(mid) in ("x0.00", "x0", "0", "0.00")

    out = []
    for mid in ids:
        if not is_chat_model(mid):
            continue
        if mid.endswith(VARIANT_SUFFIXES):
            continue
        if mid.endswith("-f"):
            base = mid[:-2]
            if base in known and free(base) and not free(mid):
                continue
        elif (mid + "-f") in known and free(mid + "-f") and not free(mid):
            continue
        out.append(mid)
    return out


def fetch_remote_product_config(realm):
    """(ids, meta) from the realm's own product-config endpoint, or None.

    At most two 10s attempts bound the wait: one per desktop UA, because the
    endpoint sits behind the WAF where a dropped connection is normal, and
    every caller has a fallback (the desktop cache file, the narrow model
    endpoint, the bundled snapshot).
    """
    if realm not in ("intl", "cn") or POOL is None:
        return None
    account = POOL.representative(realm=realm)
    if account is None or not account.access_token:
        log("remote catalog: no usable %s account, skipping" % realm)
        return None
    cfg = wb_accounts.get_realm_config(realm)
    url = cfg["chat_upstream"] + REMOTE_CONFIG_PATH
    # The chat UA is the desktop identity the rest of the gateway uses; the
    # plain app UA is the second try, for a build that answers only to it.
    uas = [cfg["chat_ua"]]
    if cfg.get("billing_ua") and cfg["billing_ua"] != cfg["chat_ua"]:
        uas.append(cfg["billing_ua"])
    last = None
    for ua in uas:
        headers = remote_config_headers(account, realm, ua)
        req = urllib.request.Request(url, headers=headers, method="GET")
        try:
            with wb_accounts.urlopen(req, timeout=10, proxy=account.proxy) as resp:
                payload = json.loads(resp.read().decode("utf-8", "replace"))
        except Exception as exc:
            last = exc
            continue
        parsed = parse_remote_catalog(payload)
        if parsed:
            return parsed
        last = "empty catalogue"
    log("remote catalog: %s fetch failed (%s)" % (realm, last))
    return None


def product_config_path(realm):
    """The desktop cache file for a realm (the intl app writes its own)."""
    home = os.path.expanduser("~")
    cache_dir = ".workbuddy-ai" if realm == "intl" else ".workbuddy"
    return os.path.join(home, cache_dir, "cache", "acc-product-config-v3.json")


def read_cached_remote_catalog(realm):
    """(ids, meta) from the desktop cache file, parsed like the remote."""
    if realm == "all":
        first = read_cached_remote_catalog("intl")
        second = read_cached_remote_catalog("cn")
        if not first:
            return second
        if not second:
            return first
        ids = list(first[0]) + [m for m in second[0] if m not in set(first[0])]
        meta = dict(second[1])
        meta.update(first[1])
        return ids, meta
    try:
        with open(product_config_path(realm), encoding="utf-8") as fh:
            payload = json.load(fh)
    except Exception:
        return None
    return parse_remote_catalog(payload)


def curated_live_sources(realm):
    """(entries, extras) for the realm's live catalogue, already curated.

    Remote first, then the desktop cache file - the cache is this very
    response written to disk, so both go through the same parser and the same
    rules. `extras` says the entries came from the desktop catalogue, whose
    membership may add a model the curated tables have never seen; the legacy
    readers keep the old whitelist behaviour.
    """
    remote = None
    try:
        remote = fetch_remote_product_config(realm)
    except Exception as exc:
        log("remote catalog: %s failed (%s)" % (realm, exc))
    source = remote or read_cached_remote_catalog(realm)
    if source:
        ids, meta = source
        return ([(mid, meta.get(mid) or {})
                for mid in curate_remote_catalog(realm, ids, meta)], True)
    legacy = read_product_config_models(realm=realm)
    if legacy:
        ids = [mid for mid, _ in legacy]
        meta = dict((mid, m) for mid, m in legacy if isinstance(m, dict))
        keep = set(curate_remote_catalog(realm, ids, meta))
        return ([(mid, m) for mid, m in legacy if mid in keep], False)
    return [], False


def fetch_endpoint_models():
    account = POOL.pick(realm="intl") if POOL else None
    if account is None:
        log("model discovery skipped: no usable account")
        cached = _models_cache.get("intl", {}).get("data")
        return [m for m, _ in (cached or [])]
    req = urllib.request.Request(UPSTREAM + MODELS_PATH, method="GET", headers=account.headers())
    try:
        with wb_accounts.urlopen(req, timeout=30, proxy=account.proxy) as resp:
            payload = json.loads(resp.read().decode("utf-8"))
    except Exception as exc:
        log(f"model discovery failed: {exc}")
        cached = _models_cache.get("intl", {}).get("data")
        return [m for m, _ in (cached or [])]
    ids, seen = [], set()
    for agent in (payload.get("data") or {}).get("agents") or []:
        for mid in agent.get("models") or []:
            if mid not in seen:
                seen.add(mid)
                ids.append(mid)
    return ids
def strip_data_prefix(line):
    line = line.strip()
    # SSE comment / heartbeat / keepalive / empty line
    if not line or line.startswith(":"):
        return ""
    while line.startswith("data:"):
        line = line[5:].strip()
    # Handle possible "data: : heartbeat"
    if not line or line.startswith(":"):
        return ""
    return line
def clean_chunk(raw):
    """Drop the empty noise fields the WorkBuddy gateway pads deltas with."""
    try:
        obj = json.loads(raw)
    except Exception:
        return raw
    changed = False
    if isinstance(obj.get("usage"), dict):
        before = dict(obj["usage"])
        normalize_usage_cache_aliases(obj["usage"])
        if obj["usage"] != before:
            changed = True
    for choice in obj.get("choices") or []:
        if isinstance(choice, dict) and choice.get("finish_reason") == "":
            choice["finish_reason"] = None
            changed = True
        delta = choice.get("delta")
        if not isinstance(delta, dict):
            continue
        # PATCHED-BY-OPS: 原判断 `if not delta.get("function_call")` 对
        # {"name":"","arguments":""} 为假（非空 dict 是真值），空占位删不掉。
        # 改为显式检查：name 与 arguments 均空才视为占位噪音。
        fc = delta.get("function_call")
        if fc is not None:
            fc_empty = False
            if isinstance(fc, dict):
                fc_empty = not fc.get("name")
            else:
                fc_empty = not fc
            if fc_empty:
                delta.pop("function_call", None)
                changed = True
        if isinstance(delta.get("tool_calls"), list) and not delta["tool_calls"]:
            delta.pop("tool_calls")
            changed = True
        for key in NOISE_KEYS:
            if key in delta and not delta.get(key):
                delta.pop(key)
                changed = True
        if not delta and not choice.get("finish_reason"):
            return ""
    return json.dumps(obj, ensure_ascii=False) if changed else raw
def _strip_empty_fc(obj):
    """PATCHED-BY-OPS: 递归剔除空 function_call 占位（Responses/chat 通用）。"""
    changed = False
    if isinstance(obj, dict):
        fc = obj.get("function_call")
        if isinstance(fc, dict) and not fc.get("name"):
            obj.pop("function_call", None)
            changed = True
        tc = obj.get("tool_calls")
        if isinstance(tc, list) and not tc:
            obj.pop("tool_calls", None)
            changed = True
        for v in list(obj.values()):
            if _strip_empty_fc(v):
                changed = True
    elif isinstance(obj, list):
        for v in obj:
            if _strip_empty_fc(v):
                changed = True
    return changed
def clean_responses_frame(frame):
    """PATCHED-BY-OPS: 清洗 Responses SSE 帧（bytes）。
    输入 b'event: x\ndata: {...}\n\n'；只改写 data: 行的 JSON，
    event: 行原样保留。解析失败原样返回（不破坏未知格式）。
    """
    if not frame:
        return frame
    try:
        text = frame.decode("utf-8")
    except Exception:
        return frame
    out, changed = [], False
    for line in text.splitlines():
        st = line.strip()
        if st.startswith("data:"):
            payload = st[5:].strip()
            if payload and payload != "[DONE]":
                try:
                    obj = json.loads(payload)
                    if _strip_empty_fc(obj):
                        line = "data: " + json.dumps(obj, ensure_ascii=False)
                        changed = True
                except Exception:
                    pass
        out.append(line)
    return ("\n".join(out) + "\n\n").encode("utf-8") if changed else frame
def normalize_roles(messages):
    """Map role names the upstream rejects onto ones it accepts.
    WorkBuddy only knows system / user / assistant / tool. OpenAI's newer
    "developer" role (used by the Codex CLI and current SDKs) is the same thing
    as "system", but sending it verbatim fails with code 11128.
    """
    out = []
    for m in messages or []:
        if not isinstance(m, dict):
            out.append(m)
            continue
        item = m
        if m.get("role") == "developer":
            item = dict(m)
            item["role"] = "system"
        out.append(item)
    return out
# ---------------------------------------------------------------------------
# Fingerprint Sanitization (immunizes against Codex / Claude Code WAF patterns)
# ---------------------------------------------------------------------------
SANITIZE_FEATURES = (
    "x-anthropic-billing-header",
    "cc_entrypoint=",
    "You are Claude Code",
    "Main branch (",
    "You are a coding agent running in the Codex CLI",
    "github.com/anthropics/",
    "11128",
)
SANITIZE_REWRITES = (
    ("You are Claude Code, Anthropic's official CLI for Claude",
     "You are Claude Code, Anthropic's official CLI tool for Claude"),
    ("Main branch (you will usually use this for PRs)",
     "Default branch (you will usually use this for PRs)"),
    ("You are a coding agent running in the Codex CLI, a terminal-based coding assistant.",
     "You are a coding agent running in the Codex CLI tool, a terminal-based coding assistant."),
    ("To give feedback, users should report the issue at https://github.com/anthropics/claude-code/issues",
     "To provide feedback, users should report the issue at https://github.com/anthropics/claude-code/issues"),
    ("11128", "11-128"),
)
SANITIZE_HDR_RE = re.compile(r"(?i)x-anthropic-billing-header:[^;\r\n]*;?\s*")
SANITIZE_BARE_HDR_RE = re.compile(r"(?i)x-anthropic-billing-header")
SANITIZE_KV_RE = re.compile(r"(?i)\bcc_[a-z0-9_]+=[^;\r\n]*;?\s*")
# WorkBuddy upstream returns 11128 ("Illegal API invocation from an unapproved
# channel") when this exact OmO identity fingerprint appears as a contiguous
# substring in a system message. A/B tests show the match is case-insensitive,
# survives surrounding prefix/suffix text, and stops matching when the phrase
# structure is changed. Rewrite only this confirmed fingerprint, leaving the
# agent identity and behaviour intact while dropping the framework attribution.
SANITIZE_OMO_JUNIOR_RE = re.compile(
    r"Sisyphus-Junior - Focused executor from OhMyOpenCode", re.IGNORECASE
)
# 指纹判定的字面量快路径（见 has_fingerprint）：原先对每段文本跑两个 (?i) 正则，
# 4KB 输入实测 319µs，其中正则 137+188µs、几个 `in` 只占 9µs；64KB 文本要
# 6.2ms，而 sanitize_messages 会给每个文本段都过一遍——build_upstream_body
# (168KB) 的耗时约 95% 都耗在这里（13.60ms -> 2.14ms）。
# 两个模式都是纯字面量，换成小写副本上的 `in` 之后是纯 C 速度的单遍扫描。
#
# 等价性：re.IGNORECASE 的 Unicode 语义与 str.lower() 只有三处差异（CPython
# 3.13 全码点实测，其余所有带大小写的码点两法完全一致）：
#   'İ'(U+0130) 与 'ı'(U+0131) 能匹配字面量 i，'ſ'(U+017F) 能匹配 s，
# 而这三个字符的 lower() 并不落到 i/s。所以先把它们按 SANITIZE_REI_FOLD
# 归一化成基字母、再小写，副本上的 `in` 与 (?i) search 完全等价（多字符模式
# 同样成立；全码点替换 + 定向用例 + 随机串对拍零差异，见 tests/_test_hotpath_savings.py）。
SANITIZE_REI_FOLD = str.maketrans({0x130: "i", 0x131: "i", 0x17F: "s"})
# 两个小写字面量直接从正则派生，保证与正则永远同源：手抄第二份迟早漂移，
# 派生出来的值不存在这个问题。pattern 里的 (?i) 是内联旗标，剥掉后就是
# 字面量本体。
SANITIZE_BARE_HDR_LOWER = SANITIZE_BARE_HDR_RE.pattern
if SANITIZE_BARE_HDR_LOWER.startswith("(?i)"):
    SANITIZE_BARE_HDR_LOWER = SANITIZE_BARE_HDR_LOWER[4:]
SANITIZE_OMO_JUNIOR_LOWER = SANITIZE_OMO_JUNIOR_RE.pattern.lower()
def has_fingerprint(text):
    if not isinstance(text, str) or not text:
        return False
    for f in SANITIZE_FEATURES:
        if f in text:
            return True
    # 三个特例字符几乎从不出现：先做三次 C 速度探测，命中才走归一化副本，
    # 常规文本只付一次 lower()。
    if "\u0130" in text or "\u0131" in text or "\u017f" in text:
        low = text.translate(SANITIZE_REI_FOLD).lower()
    else:
        low = text.lower()
    return SANITIZE_BARE_HDR_LOWER in low or SANITIZE_OMO_JUNIOR_LOWER in low
def sanitize_text(text):
    if not isinstance(text, str) or not text:
        return text
    if not has_fingerprint(text):
        return text
    # Keep the rewrite deliberately narrow: do not globally remove
    # "OhMyOpenCode" or "Sisyphus-Junior", because either token alone is
    # accepted by the upstream. Only the confirmed contiguous fingerprint is
    # neutralized.
    text = SANITIZE_OMO_JUNIOR_RE.sub("Sisyphus-Junior - Focused executor", text)
    for old, new in SANITIZE_REWRITES:
        text = text.replace(old, new)
    text = SANITIZE_HDR_RE.sub("", text)
    if "cc_" in text:
        prev = ""
        while prev != text:
            prev = text
            text = SANITIZE_KV_RE.sub("", text)
    text = SANITIZE_BARE_HDR_RE.sub("x-anthropic-billing-hdr", text)
    return text.strip()
def sanitize_content(content):
    if isinstance(content, str):
        return sanitize_text(content)
    if isinstance(content, list):
        out = []
        for part in content:
            if isinstance(part, dict) and part.get("type") == "text" and "text" in part:
                p = dict(part)
                p["text"] = sanitize_text(p["text"])
                out.append(p)
            else:
                out.append(part)
        return out
    return content
def sanitize_tool_calls(tool_calls):
    if not isinstance(tool_calls, list):
        return tool_calls
    out = []
    for tc in tool_calls:
        if isinstance(tc, dict):
            item = dict(tc)
            fn = item.get("function")
            if isinstance(fn, dict) and isinstance(fn.get("arguments"), str):
                fn = dict(fn)
                fn["arguments"] = sanitize_text(fn["arguments"])
                item["function"] = fn
            out.append(item)
        else:
            out.append(tc)
    return out
def sanitize_messages(messages):
    out = []
    for m in messages or []:
        if isinstance(m, dict):
            item = dict(m)
            if "content" in item:
                item["content"] = sanitize_content(item["content"])
            if isinstance(item.get("reasoning_content"), str):
                item["reasoning_content"] = sanitize_text(item["reasoning_content"])
            if "tool_calls" in item:
                item["tool_calls"] = sanitize_tool_calls(item["tool_calls"])
            out.append(item)
        else:
            out.append(m)
    return out


# ---------------------------------------------------------------------------
# Tool-call pairing repair
# ---------------------------------------------------------------------------
def _empty_content(value):
    """True when an assistant message carries no content at all.

    None / "" / [] all mean "no content": clients disagree on which empty
    shape they emit, and a batch split into adjacent assistant messages must
    merge for either shape (a missed [] was the original 11148 case).
    """
    if value is None:
        return True
    if isinstance(value, str):
        return value == ""
    if isinstance(value, list):
        return len(value) == 0
    return False


def _merge_reasoning_content(dst, src):
    """Fold src's reasoning_content into dst, newline-joined when both exist."""
    rc = src.get("reasoning_content")
    if not isinstance(rc, str) or not rc:
        return
    prev = dst.get("reasoning_content")
    if isinstance(prev, str) and prev:
        dst["reasoning_content"] = prev + "\n" + rc
    else:
        dst["reasoning_content"] = rc


def merge_adjacent_tool_calls(messages):
    """Merge back-to-back assistant tool_calls messages into one.

    Some OpenAI-compatible agent clients replay a parallel batch as several
    adjacent assistant messages, each carrying one tool_call. DeepSeek-family
    upstreams answer 400 / code 11148 (tool_call_sequence_broken) for that
    shape and retire the conversation, because every retry replays the same
    history and switching accounts cannot help. Merging the declarations
    restores the shape the upstream accepts.

    Conditions are strict, so no semantics are invented:
      - the messages must be adjacent;
      - the trailing message must have empty content (None/""/[]);
      - the leading message must already be an assistant with tool_calls.

    A second form folds a plain assistant's string content into a preceding
    tool_calls assistant that has no content of its own (the same split batch
    replayed in the other order). reasoning_content is preserved either way,
    because DeepSeek multi-turn thinking requires it back.
    """
    if not isinstance(messages, list) or len(messages) < 2:
        return messages, False
    out = []
    changed = False
    for m in messages:
        if not isinstance(m, dict):
            out.append(m)
            continue
        if m.get("role") == "assistant" and out:
            prev = out[-1] if isinstance(out[-1], dict) else None
            if prev is not None and prev.get("role") == "assistant":
                tcs = m.get("tool_calls")
                prev_calls = prev.get("tool_calls")
                # Form 1: this message declares tool calls and has no content.
                if (isinstance(tcs, list) and tcs
                        and _empty_content(m.get("content"))
                        and isinstance(prev_calls, list) and prev_calls):
                    prev["tool_calls"] = prev_calls + tcs
                    _merge_reasoning_content(prev, m)
                    changed = True
                    continue
                # Form 2: plain string content folds into a preceding
                # content-less tool_calls assistant.
                if ("tool_calls" not in m
                        and isinstance(m.get("content"), str) and m["content"]
                        and isinstance(prev_calls, list) and prev_calls
                        and _empty_content(prev.get("content"))):
                    prev["content"] = m["content"]
                    _merge_reasoning_content(prev, m)
                    changed = True
                    continue
        out.append(m)
    if not changed:
        return messages, False
    return out, True


def repack_tool_result_blocks(messages):
    """Keep a tool_calls batch and its results adjacent.

    The upstream requires the role:"tool" results to follow the assistant
    message that requested them with nothing in between. Codex's
    image_resize_notice, for one, arrives as a developer message right after a
    tool output; with parallel calls it lands between two results, the pairing
    reads as broken and the upstream rejects the whole request (code 11148),
    retiring the conversation. This only reorders: same results, same relative
    order, the intruders moved behind the batch.
    """
    if not isinstance(messages, list) or len(messages) < 3:
        return messages, False
    out = []
    changed = False
    i = 0
    while i < len(messages):
        m = messages[i]
        if not isinstance(m, dict) or m.get("role") != "assistant":
            out.append(m)
            i += 1
            continue
        calls = m.get("tool_calls")
        if not isinstance(calls, list) or not calls:
            out.append(m)
            i += 1
            continue
        want = set()
        for tc in calls:
            if isinstance(tc, dict):
                tid = tc.get("id")
                if isinstance(tid, str) and tid:
                    want.add(tid)
        out.append(m)
        i += 1
        results = []
        between = []
        saw_non_tool = False
        while i < len(messages):
            mm = messages[i]
            if not isinstance(mm, dict):
                break
            role = mm.get("role")
            if role == "tool":
                tid = mm.get("tool_call_id")
                if not (isinstance(tid, str) and tid in want):
                    break
                results.append(mm)
                if saw_non_tool:
                    changed = True
                i += 1
                continue
            if not results:
                break
            # A following assistant.tool_calls opens the next batch: it must go
            # back to the outer loop, or its own results never get repacked.
            if role == "assistant" and isinstance(mm.get("tool_calls"), list) \
                    and mm["tool_calls"]:
                break
            between.append(mm)
            saw_non_tool = True
            i += 1
        out.extend(results)
        out.extend(between)
    if not changed:
        return messages, False
    return out, True


def cleanup_orphan_tool_calls(messages):
    """Drop tool calls that have no result, and results that have no call.

    A failed tool call (bad arguments, timeout, unknown tool) leaves the client
    with an assistant tool_calls entry it can never answer: the result message
    is never written, yet the entry rides along with the history on every later
    turn and the upstream rejects each one (code 11148), so a single failed
    call can retire a whole conversation. Both sides are trimmed against the
    same set of ids, so no half-pairing can survive the repair.
    """
    if not isinstance(messages, list) or not messages:
        return messages, False
    call_ids = set()
    result_ids = set()
    for m in messages:
        if not isinstance(m, dict):
            continue
        role = m.get("role")
        if role == "tool":
            tid = m.get("tool_call_id")
            if isinstance(tid, str) and tid:
                result_ids.add(tid)
        elif role == "assistant":
            calls = m.get("tool_calls")
            if isinstance(calls, list):
                for tc in calls:
                    if isinstance(tc, dict):
                        tid = tc.get("id")
                        if isinstance(tid, str) and tid:
                            call_ids.add(tid)
    if not call_ids and not result_ids:
        return messages, False
    keep = call_ids & result_ids
    changed = False
    for m in messages:
        if not isinstance(m, dict) or m.get("role") != "assistant":
            continue
        calls = m.get("tool_calls")
        if not isinstance(calls, list) or not calls:
            continue
        kept = [tc for tc in calls
                if isinstance(tc, dict) and isinstance(tc.get("id"), str)
                and tc["id"] in keep]
        if len(kept) == len(calls):
            continue
        changed = True
        if kept:
            m["tool_calls"] = kept
        else:
            m.pop("tool_calls", None)
    out = []
    for m in messages:
        if isinstance(m, dict) and m.get("role") == "tool":
            tid = m.get("tool_call_id")
            if not (isinstance(tid, str) and tid in keep):
                changed = True
                continue
        out.append(m)
    if not changed:
        return messages, False
    return out, True


def is_truncated_arguments(raw):
    """True when a non-empty tool-arguments string is not valid JSON.

    A stream cut off by max_tokens or a dropped connection leaves the last
    tool call with half-written JSON. An empty/whitespace string is a legal
    no-argument tool, and any parseable JSON (including null/scalars/arrays)
    is the model's own output for the client to validate - only non-empty
    unparsable strings count as truncation.
    """
    if not isinstance(raw, str):
        return False
    trimmed = raw.strip()
    if not trimmed:
        return False
    try:
        json.loads(trimmed)
        return False
    except Exception:
        return True


def drop_truncated_tool_calls(calls):
    """Return the tool calls whose arguments are not half-written JSON."""
    if not isinstance(calls, list):
        return calls
    kept = []
    for call in calls:
        if not isinstance(call, dict):
            kept.append(call)
            continue
        fn = call.get("function")
        if isinstance(fn, dict) and is_truncated_arguments(fn.get("arguments")):
            continue
        kept.append(call)
    return kept


# ---------------------------------------------------------------------------
# DeepSeek Multi-turn Consistency: reasoning_content backfill
# ---------------------------------------------------------------------------
# Upstream (code 11155 "the reasoning content from the previous turn must be
# passed back in thinking mode") requires every assistant message to carry a
# `reasoning_content` string while thinking is on. Two halves gate the fix,
# mirroring the official client's ReasoningContentBackfillRule:
#   - thinkingEnabled: deepseek + thinking enabled -> always backfill, even
#     when a third-party client dropped reasoning entirely (this was the bug:
#     only the hasTrace half existed, so zero-trace histories were forwarded
#     untouched and rejected).
#   - hasTrace: any existing reasoning trace -> backfill regardless of the
#     thinking flag.
# Upstream also validates len(reasoning) > 0, so an empty placeholder is not
# enough on its own: `reasoning` is mirrored with a non-empty value.
def backfill_reasoning_content(messages, model, thinking_enabled=None):
    if not model or not str(model).lower().startswith("deepseek"):
        return messages
    if thinking_enabled is None:
        thinking_enabled = False
    has_trace = False
    for m in messages:
        if not isinstance(m, dict):
            continue
        reasoning = m.get("reasoning")
        if isinstance(reasoning, str) and reasoning:
            has_trace = True
            break
        if "reasoning_content" in m:
            has_trace = True
            break
    if not thinking_enabled and not has_trace:
        return messages
    out = []
    for m in messages:
        if isinstance(m, dict) and m.get("role") == "assistant":
            item = dict(m)
            rc = item.get("reasoning_content")
            if not isinstance(rc, str):
                # Non-string (null/number/absent) counts as missing, matching
                # the official `typeof !== "string"` check.
                legacy = item.get("reasoning")
                rc = legacy if isinstance(legacy, str) else ""
                item["reasoning_content"] = rc
            # Mirror onto `reasoning` with a non-empty value: upstream rejects
            # an empty/absent reasoning, while a whitespace placeholder passes
            # its length check and carries no model-visible semantics.
            existing = item.get("reasoning")
            if not (isinstance(existing, str) and existing):
                item["reasoning"] = rc if rc else " "
            out.append(item)
        else:
            out.append(m)
    return out
# ---------------------------------------------------------------------------
# Tool & Tool Choice Normalization (avoids code 11101 on object tool_choice)
# ---------------------------------------------------------------------------
def normalize_tool_choice(obj):
    if "tool_choice" not in obj:
        return
    tc = obj["tool_choice"]
    if isinstance(tc, str):
        val = tc.strip().lower()
        if val == "none":
            # 这里曾经把 tools/functions 一起删掉，那正是 Agent 死循环的成因：
            # 工具声明没了，模型拿不到函数签名、又没有结构化工具通道，却仍被要求
            # 完成任务，于是把调用降级成 DSML / 伪 JSON 文本塞进 content
            # （tool_calls 为空、finish_reason=stop）。客户端解析不到调用只能再
            # 追问一轮，模型又重复一遍 "I'll do it"，上下文每轮 +2 条消息、token
            # 线性膨胀，直到撑爆窗口或用户手动断开。
            #
            # tool_choice="none" 的语义是「本轮不许调用工具」，这层意思由
            # tool_choice 字段本身表达就够了，不需要抹掉能力声明。
            # 上游把 tool_choice 声明为 string（发对象会 11101），所以保持字符串
            # 原样透传，同时保留 tools。
            #
            # 取舍：实测本上游并不真正遵守 tool_choice="none"（保留 tools 后它
            # 仍返回 tool_calls）。但对比两条路 —— 删 tools 会让模型输出不可解析
            # 的文本、Agent 原地空转；留 tools 则走正常 tool_calls 通道，客户端能
            # 正常执行与回填 —— 后者明显更好。确实需要禁止调用时，客户端不传
            # tools 即可。
            obj["tool_choice"] = "none"
        return
    if isinstance(tc, dict):
        typ = (tc.get("type") or "").strip().lower()
        if typ == "none":
            # 同上：保留 tools 声明。上游只认字符串，对象形式必须降级成
            # "none"，否则 11101。
            obj["tool_choice"] = "none"
        elif typ in ("auto", "required"):
            obj["tool_choice"] = typ
        elif typ == "function":
            name = (tc.get("function") or {}).get("name") or tc.get("name") or ""
            obj["tool_choice"] = name.strip() or "auto"
        else:
            obj.pop("tool_choice", None)
    else:
        obj.pop("tool_choice", None)
def normalize_tools(obj):
    tools = obj.get("tools")
    if not tools or not isinstance(tools, list):
        return
    norm = []
    for t in tools:
        if not isinstance(t, dict):
            continue
        # Wrap top-level name tool definition into Chat Completions function schema
        if "name" in t and "function" not in t and t.get("type") == "function":
            fn = {
                "name": t.get("name") or "",
                "description": t.get("description") or "",
                "parameters": t.get("parameters") or {},
            }
            if "strict" in t:
                fn["strict"] = t["strict"]
            norm.append({"type": "function", "function": fn})
        else:
            norm.append(t)
    obj["tools"] = norm
# ---------------------------------------------------------------------------
# Stop Sequence Normalization (avoids code 11101 on a string stop)
# ---------------------------------------------------------------------------
def normalize_stop(obj):
    # OpenAI 把 stop 声明成 string | array，客户端传单个字符串完全合法；本上游的
    # Go 端却把该字段定死为 []string，收到字符串直接 400：
    #   Unmarshal chat params failed with error: json: cannot unmarshal string
    #   into Go struct field Request.stop of type []string   (code 11101)
    # 实测（deepseek-v4.1-flash，同一条 prompt，同一个上游账号）：
    #   "stop": "END"     -> 400 / code 11101
    #   "stop": ["END"]   -> 200
    # 归一化成数组后语义完全等价：OpenAI 里单字符串就是单元素数组，模型侧看到的
    # 停止序列一模一样。空字符串 / 空数组 / null / 非法类型一律删字段——它们的实际
    # 含义都是「没有停止序列」，而空值本身在上游同样会被判参数不合法。
    if "stop" not in obj:
        return
    val = obj["stop"]
    if isinstance(val, str):
        val = [val] if val else []
    elif isinstance(val, (list, tuple)):
        val = [s for s in val if isinstance(s, str) and s]
    else:
        val = []
    if val:
        obj["stop"] = val
    else:
        obj.pop("stop", None)
# ---------------------------------------------------------------------------
# DeepSeek DSML Tool Calls Fallback Parser
# ---------------------------------------------------------------------------
TAG_START = r"<[^>]*DSML[^>]*"
DSML_CALLS_RE = re.compile(TAG_START + r"calls>(.*?)</[^>]*DSML[^>]*calls>", re.DOTALL)
DSML_INVOKE_RE = re.compile(TAG_START + r"invoke\s+name=[\x22\x27]([^\x22\x27]+)[\x22\x27]>(.*?)</[^>]*invoke>", re.DOTALL)
DSML_PARAM_RE = re.compile(TAG_START + r"parameter\s+name=[\x22\x27]([^\x22\x27]+)[\x22\x27][^>]*>(.*?)</[^>]*parameter>", re.DOTALL)
def parse_dsml_tool_calls(text):
    if not text or "DSML" not in text:
        return None, text
    match = DSML_CALLS_RE.search(text)
    if not match:
        return None, text
    calls_block = match.group(1)
    tool_calls = []
    for inv_match in DSML_INVOKE_RE.finditer(calls_block):
        func_name = inv_match.group(1)
        params_block = inv_match.group(2)
        params = {}
        for p_match in DSML_PARAM_RE.finditer(params_block):
            p_name = p_match.group(1)
            p_val = p_match.group(2).strip()
            params[p_name] = p_val
        tool_calls.append({
            "id": _new_id("call_"),
            "name": func_name,
            "arguments": json.dumps(params, ensure_ascii=False),
        })
    clean = (text[:match.start()].strip() + " " + text[match.end():].strip()).strip()
    return tool_calls, clean
# ---------------------------------------------------------------------------
# Printed tool calls on the Chat Completions path
# ---------------------------------------------------------------------------
# The Responses path has parsed DSML text since the tool_choice="none" fix, but
# /v1/chat/completions -- the route the desktop client actually uses -- still
# forwarded upstream `content` verbatim. When the model printed its call instead
# of emitting structured tool_calls, the client saw plain text, found no call,
# and the turn just ended: the markup stayed in the transcript and nothing ran.
# Reported by the user as "it keeps stopping by itself".
#
# Two spellings show up in the wild:
#   1. the full-width DSML form the suite already pins:
#      <||DSML|| calls><||DSML|| invoke name="run_code"> ... </||DSML|| invoke>
#   2. a hybrid the model mixes on its own -- a plain opening tag with DSML
#      closing tags, and the first parameter's close tag swallowed whole:
#      <run_code><parameter name="code">...</parameter>
#      <parameter name="description">...</|DSML| parameter>
#      </run_code></invoke></|DSML| parameter></|DSML| invoke></|DSML| calls>
#
# parse_dsml_tool_calls() above only matches (1): it wants a DSML opening tag
# for the calls block, which (2) never has. The parser below is driven by the
# <parameter> elements instead, because those are spelled the same way in both
# shapes and a parameter name is the one thing a call cannot do without.
LEAK_PARAM_RE = re.compile(
    r"<\s*(?:\|\s*DSML\s*\|\s*)?(?P<tag>parameter|param|arg|argument)\b"
    r"(?P<attrs>[^>]*)>"
    r"(?P<value>.*?)"
    r"</\s*(?:\|\s*DSML\s*\|\s*)?(?:parameter|param|arg|argument)\s*>",
    re.DOTALL | re.IGNORECASE,
)
LEAK_TAG_RE = re.compile(
    r"<\s*/?\s*(?:\|\s*DSML\s*\|\s*)?"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)(?P<attrs>[^>]*)>"
)
LEAK_NAME_ATTR_RE = re.compile(r"""name\s*=\s*["']([^"']*)["']""")
LEAK_WRAPPER_TAGS = {"calls", "tool_calls", "function_calls", "function_call"}
LEAK_PARAM_TAGS = {"parameter", "param", "arg", "argument"}
# First characters of every tag the stream filter has to hold text back for.
LEAK_TAG_PREFIXES = (
    "<run_code", "<run_shell", "<invoke", "<parameter", "<param", "<arg",
    "<argument", "<calls", "<tool_calls", "<function_calls", "<function_call",
    "<think", "<thinking",
    "</run_code", "</run_shell", "</invoke", "</parameter", "</param",
    "</arg", "</argument", "</calls", "</tool_calls", "</function_calls",
    "</function_call", "</think", "</thinking",
    "<|dsml|", "</|dsml|",
)


def _normalize_dsml_bars(text):
    """Fold the full-width spelling of the DSML token back to ASCII bars."""
    return re.sub(r"\uff5c+\s*DSML\s*\uff5c+", "|DSML|", text)


def _strip_spans(text, spans):
    """Remove every [start, end) span, then collapse the leftover blanks."""
    if not spans:
        return text
    kept = []
    last = 0
    for start, end in sorted(spans):
        if start < last:
            continue
        kept.append(text[last:start])
        last = end
    kept.append(text[last:])
    return "".join(kept).strip()


def parse_leaked_tool_calls(text):
    """Recover a tool call the model printed instead of emitting.

    Returns (calls, clean_text) with calls in the same shape as
    parse_dsml_tool_calls(), or (None, text) when nothing was recognised.
    """
    if not text or "<" not in text:
        return None, text
    # Fold the full-width spelling first so both shapes go through one walk.
    original = text
    text = _normalize_dsml_bars(text)
    found = []
    for m in LEAK_PARAM_RE.finditer(text):
        attr = LEAK_NAME_ATTR_RE.search(m.group("attrs") or "")
        if not attr or not attr.group(1):
            continue
        found.append((m.start(), attr.group(1), m.group("value").strip(), m.span()))
    if not found:
        return None, original
    tags = list(LEAK_TAG_RE.finditer(text))
    calls = []
    for start, pname, value, span in found:
        owner = None
        for tag in tags:
            if tag.start() >= start:
                break
            if tag.group(0).lstrip().startswith("</"):
                continue
            tname = tag.group("name").lower()
            if tname in LEAK_WRAPPER_TAGS or tname in LEAK_PARAM_TAGS:
                continue
            owner = tag
        name = ""
        if owner is not None:
            attr = LEAK_NAME_ATTR_RE.search(owner.group("attrs") or "")
            name = (attr.group(1) if attr else owner.group("name")).strip()
        if not name:
            continue
        key = owner.start() if owner is not None else None
        if calls and calls[-1]["open"] == key:
            calls[-1]["params"][pname] = value
            calls[-1]["spans"].append(span)
        else:
            calls.append({"name": name, "params": {pname: value},
                          "open": key, "spans": [span]})
    if not calls:
        return None, original
    # Everything from the call's opening tag to the last of its parameters is
    # markup rather than prose, so the whole region goes away in one piece.
    regions = []
    for call in calls:
        first = call["open"]
        if first is None:
            first = min(s for s, _e in call["spans"])
        last = max(e for _s, e in call["spans"])
        # Swallow the closing tags that trail the last parameter.
        while True:
            nxt = last
            while nxt < len(text) and text[nxt] in " \t\r\n":
                nxt += 1
            m = LEAK_TAG_RE.match(text, nxt)
            if not m or not m.group(0).lstrip().startswith("</"):
                break
            last = m.end()
        regions.append((first, last))
    clean = _strip_spans(text, regions)
    # Wrappers and stray tags the region walk did not cover (<|DSML| calls>,
    # <think>, ...) are markup too; drop any tag carrying the DSML token.
    leftover = []
    for m in LEAK_TAG_RE.finditer(clean):
        if "DSML" in _normalize_dsml_bars(m.group(0)).upper():
            leftover.append(m.span())
        elif m.group("name").lower() in LEAK_WRAPPER_TAGS:
            leftover.append(m.span())
    clean = _strip_spans(clean, leftover)
    return [{
        "id": _new_id("call_"),
        "name": call["name"],
        "arguments": json.dumps(call["params"], ensure_ascii=False),
    } for call in calls], clean


def repair_leaked_tool_calls(message):
    """Move a printed call out of `content` and into `tool_calls`.

    Only fires when the message carries no structured call: a real tool_calls
    entry always wins, so a model that emits both is left alone. Returns True
    when the message changed.
    """
    if not isinstance(message, dict):
        return False
    existing = message.get("tool_calls")
    if isinstance(existing, list) and existing:
        return False
    content = message.get("content")
    if not isinstance(content, str) or "<" not in content:
        return False
    calls, clean = parse_leaked_tool_calls(content)
    if not calls:
        calls, clean = parse_dsml_tool_calls(content)
    if not calls:
        return False
    message["content"] = clean
    message["tool_calls"] = [{
        "id": call["id"],
        "type": "function",
        "function": {"name": call["name"], "arguments": call["arguments"]},
    } for call in calls]
    return True


def could_be_leaked_call(tail):
    """True while `tail` (starting at "<") may still grow into printed markup."""
    low = _normalize_dsml_bars(tail).lower()
    m = LEAK_TAG_RE.match(low)
    if m and any(m.group(0).startswith(p) for p in LEAK_TAG_PREFIXES):
        return True
    if len(low) > 96 or ">" in low:
        return False
    return any(p.startswith(low) or low.startswith(p) for p in LEAK_TAG_PREFIXES)


class LeakedToolCallStreamFilter:
    """Hold back assistant text that may be a printed tool call.

    /v1/chat/completions forwards text the moment it arrives, so a call the
    model printed instead of emitting would already be on the wire before the
    stream ended and anything could parse it. Buffer the text instead: release
    it as soon as it is clearly prose, convert whatever is left at the end.

    feed(text) returns a list of ("text", str) and ("calls", [call, ...]).
    """

    MAX_HOLD = 262144

    def __init__(self):
        self.buf = ""

    def _release_all(self):
        out, self.buf = self.buf, ""
        return [("text", out)] if out else []

    def feed(self, text):
        if not text:
            return []
        self.buf += text
        for idx in range(len(self.buf)):
            if self.buf[idx] != "<":
                continue
            if not could_be_leaked_call(self.buf[idx:]):
                continue
            if idx:
                head, self.buf = self.buf[:idx], self.buf[idx:]
                return [("text", head)]
            break
        else:
            return self._release_all()
        if len(self.buf) > self.MAX_HOLD:
            return self._release_all()
        return []

    def flush(self):
        if not self.buf:
            return []
        buf, self.buf = self.buf, ""
        calls, clean = parse_leaked_tool_calls(buf)
        if not calls:
            calls, clean = parse_dsml_tool_calls(buf)
        if not calls:
            return [("text", buf)]
        out = []
        if clean:
            out.append(("text", clean))
        out.append(("calls", calls))
        return out


def chat_chunk(model, delta, finish=None, chunk_id="chatcmpl-wb"):
    """One Chat Completions streaming chunk."""
    return json.dumps({
        "id": chunk_id,
        "object": "chat.completion.chunk",
        "created": int(time.time()),
        "model": model,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    }, ensure_ascii=False)


def chat_text_chunk(model, text, chunk_id="chatcmpl-wb"):
    return chat_chunk(model, {"content": text}, None, chunk_id)


def chat_tool_call_chunks(model, calls, chunk_id="chatcmpl-wb"):
    """Deltas that hand recovered calls over as structured tool calls."""
    out = []
    for i, call in enumerate(calls):
        out.append(chat_chunk(model, {"tool_calls": [{
            "index": i,
            "id": call["id"],
            "type": "function",
            "function": {"name": call["name"], "arguments": call["arguments"]},
        }]}, None, chunk_id))
    return out


def chat_finish_chunk(model, reason, chunk_id="chatcmpl-wb"):
    return chat_chunk(model, {}, reason, chunk_id)


def translate_max_completion_tokens(obj):
    alias = obj.pop("max_completion_tokens", None)
    if alias is None:
        return
    if "max_tokens" in obj:
        return
    try:
        val = int(alias)
        if val > 0:
            obj["max_tokens"] = val
    except (TypeError, ValueError):
        pass
# ---------------------------------------------------------------------------
# 模型封锁表
#
# 背景：客户端除了使用者的对话，还会自己发背景请求（记忆整理、自动复核等）。
# 这些请求不经过模型选单，而是直接使用目录上的模型 ID，因此可能在使用者
# 没有实际操作时，用付费模型消耗额度。
#
# 对策（选用）：把要拒绝的模型填进 ALLOWED_MODELS / BANNED_SUBSTRING /
#               EXTRA_BANNED；命中的请求在本机直接回 400，完全不碰上游。
#               预设全部为空 = 不封锁任何模型，行为与原版相同。
#
# 调整方式：
#   要放行某个模型 -> 加进 ALLOWED_MODELS 或 ALLOWED_PREFIXES
#   要连非 gpt 的模型一起挡 -> 加进 EXTRA_BANNED
# ---------------------------------------------------------------------------

# 允许放行的模型（你要用的）
ALLOWED_MODELS = {
    # 预设不封锁任何模型；填入模型 id 即可只放行这些
}

# 允许前缀：涵盖 -high / -preview / [1M] 等变体
ALLOWED_PREFIXES = ()

# 封锁字串：模型名里含这个就拒绝
BANNED_SUBSTRING = ""

# 额外封锁的内部模型（不在 gpt- 前缀内，但也会烧点）
EXTRA_BANNED = set()


def is_model_banned(model):
    """True 表示这个模型名不该被送去上游。

    规则：ALLOWED_MODELS / ALLOWED_PREFIXES 命中就放行；其余只要命中
    BANNED_SUBSTRING 或 EXTRA_BANNED 就拒绝，没命中则照常送往上游。
    三个设定预设都是空的，所以预设不封锁任何模型。
    """
    if not model:
        return False
    m = str(model).strip().lower()
    # 白名单优先（含 -high / -preview / [1M] 这类变体）
    if m in ALLOWED_MODELS:
        return False
    if any(m.startswith(a) for a in ALLOWED_PREFIXES):
        return False
    # 命中封锁字串就拒绝
    if BANNED_SUBSTRING and BANNED_SUBSTRING in m:
        return True
    # 其他已知会烧点的内部模型
    if m in EXTRA_BANNED:
        return True
    return False


# ---------------------------------------------------------------------------
# 背景请求拦截
#
# Codex App 除了使用者的对话，还会自己发背景请求（记忆整理、环境建议、自动复核…）。
# 这些请求不经过模型选单，所以单靠模型白名单挡不住 —— 它们可能直接用目录上
# 的付费模型（例如 gpt-6-astra 这类），在使用者没有实际操作时照样消耗额度。
#
# Codex 会在 client_metadata 里带 x-codex-turn-metadata，内容像：
#   {"request_kind":"memory","thread_source":"memory_consolidation",
#    "turn_trigger":"memory_consolidation"}
# 这里就靠这个标记判断：命中背景关键字 -> 本地直接拒绝，不碰上游、不扣点。
# ---------------------------------------------------------------------------

# 要不要拦截背景请求（False = 全部放行，维持原行为）
BLOCK_BACKGROUND_REQUESTS = False

# 命中任一关键字就视为背景请求（不分大小写、子字串比对）
BACKGROUND_TRIGGER_KEYWORDS = (
    "memory_consolidation",
    "memory-write",
    "memory_write",
    "memorywriting",
    "ambient",
    "suggestion",
    "auto_review",
    "auto-review",
    "autoreview",
    "title",
    "compaction",
    "compact",
    "summariz",
)


# Thread sources that belong to a job the client started on its own. A
# compaction request carries one of these when the client triggered it, and the
# user's own thread when the operator pressed "compact the context" - so the
# source has to be read before the keyword list, where "compaction" matches
# both and would otherwise refuse the button.
BACKGROUND_THREAD_SOURCES = (
    "memory_consolidation",
    "memory",
    "ambient",
    "suggestion",
    "auto_review",
    "autoreview",
    "title",
)


def turn_metadata_fields(payload):
    """Flatten the request_kind / turn_trigger / thread_source hints we get.

    Codex sends them either as plain client_metadata keys or as a JSON string
    under a metadata key of its own, so both shapes are read. Returns {} when
    the payload carries none of them.
    """
    if not isinstance(payload, dict):
        return {}
    meta = payload.get("client_metadata")
    if not isinstance(meta, dict):
        return {}

    # 收集所有可能的来源/触发栏位
    fields = {}
    for key, value in meta.items():
        if isinstance(value, str) and value.strip().startswith("{"):
            try:
                inner = json.loads(value)
            except Exception:
                inner = None
            if isinstance(inner, dict):
                for k in ("request_kind", "turn_trigger", "thread_source", "kind", "trigger", "source"):
                    if k in inner:
                        fields[k] = inner[k]
        if key in ("request_kind", "turn_trigger", "thread_source"):
            fields[key] = value
    return fields


def is_compaction_request(payload):
    """True for the operator's own "compact the context" request.

    request_kind=compaction carries the same word as the background keyword, but
    this request is one the user asked for: the client sends it on the user's
    thread, while a compaction the client started by itself names the job that
    started it. Refusing this one takes the context-compaction button away.
    """
    fields = turn_metadata_fields(payload)
    kind = str(fields.get("request_kind") or "").strip().lower()
    if "compact" not in kind:
        return False
    source = str(fields.get("thread_source") or "").strip().lower()
    return source not in BACKGROUND_THREAD_SOURCES


def background_request_reason(payload):
    """若这是 Codex 自己发的背景请求，回传说明字串；否则回传 ""。

    只看 client_metadata，不碰讯息内容。
    """
    fields = turn_metadata_fields(payload)
    if not fields:
        return ""

    if is_compaction_request(payload):
        return ""

    blob = " ".join(str(v) for v in fields.values()).lower()
    for kw in BACKGROUND_TRIGGER_KEYWORDS:
        if kw in blob:
            return "%s=%s" % (
                ",".join(sorted(fields.keys())),
                ",".join(str(fields[k]) for k in sorted(fields)),
            )
    return ""


def client_effort_of(body):
    """The effort the request itself carries, under either client spelling.

    build_upstream_body() reads both "reasoning_effort" and "reasoningEffort",
    and only fills in the model default for the models it injects for - so a
    camelCase request keeps the camelCase key, and a request to a model the
    catalog pins to one level carries no effort at all.
    """
    if not isinstance(body, dict):
        return None
    return body.get("reasoning_effort") or body.get("reasoningEffort")


def upstream_effort_of(body, model=None):
    """The reasoning effort a request actually runs at, or None when unknown.

    Resolved the way the upstream will apply it:

      - a model the catalog pins to one level (reasoning.effort, no
        supportedEfforts) always runs there: the picker offers no choice for it,
        so a value the request carries anyway does not change the answer;
      - a request that switched thinking off, or asked for "none", ran without
        reasoning and nothing below overrides that;
      - otherwise the client's own value wins, under either spelling;
      - otherwise the model's declared defaultEffort applies.

    Reading the body alone is not enough for the last two: the gateway only
    writes an effort into the body for the models it injects for, so a plain
    request to a pinned model would otherwise be reported as "no effort".
    """
    given = client_effort_of(body)
    if model:
        fixed = model_fixed_effort(model)
        if fixed:
            return fixed
    thinking = (body or {}).get("thinking") if isinstance(body, dict) else None
    if isinstance(thinking, dict) and \
            str(thinking.get("type") or "").strip().lower() == "disabled":
        return "none"
    if str(given or "").strip().lower() == "none":
        return "none"
    if given:
        return given
    return model_default_effort(model) if model else None


def background_request_message(reason):
    return ("這是客戶端自己發的背景請求（%s），本機代理已擋下，"
            "避免在沒有實際操作時消耗上游額度。"
            "要放行請把 wb_proxy.py 的 BLOCK_BACKGROUND_REQUESTS 改成 False。"
            % reason)


def banned_model_message(model):
    allowed = "、".join(sorted(ALLOWED_MODELS))
    return ("模型 %s 已被本機代理封鎖（依 ALLOWED_MODELS / BANNED_SUBSTRING 設定）。"
            "目前允許：%s。要放行請編輯 wb_proxy.py 的 ALLOWED_MODELS。"
            % (model, allowed))


def key_model_message(entry, model):
    """Explain a per-key model restriction the same way the global ban does."""
    name = (entry or {}).get("name") or "未命名"
    allowed = "、".join((entry or {}).get("models") or []) or "-"
    asked = str(model or "").strip() or "(未指定模型)"
    return ("API Key「%s」的模型限制不允許 %s。該 Key 目前允許：%s。"
            "請在看板「設置」頁修改這個 Key 的模型限制，或改用允許該模型的 Key。"
            % (name, asked, allowed))


# Process-memory degradation window: content-blocked passthrough/append
# traffic switches to the minimal neutral prompt until the next 00:00 CST
# (panel degrade.go). Restarts clear it, which is fine - the next rejection
# re-triggers it.
PROMPT_DEGRADE = wb_prompt.DegradeGate()


def prompt_mode_config():
    """Prompt mode settings, fail-open to passthrough on any read error."""
    try:
        cfg = wb_settings.prompt_config(ACCOUNTS_DIR)
    except Exception:
        cfg = None
    if not isinstance(cfg, dict):
        cfg = {"mode": "passthrough", "file": ""}
    return cfg


def apply_prompt_mode(messages):
    """Apply the configured prompt mode to one request's messages.

    passthrough is the legacy behaviour (client system prompts ride through);
    custom/append are opt-in. While the degrade window is active, passthrough
    and append switch to the minimal neutral prompt; custom never degrades.
    An unreadable prompt file fails open to passthrough rather than blocking.
    """
    cfg = prompt_mode_config()
    mode = cfg.get("mode") or "passthrough"
    degraded = PROMPT_DEGRADE.active()
    if mode in ("custom", "append"):
        try:
            text = wb_prompt.load_prompt(mode, cfg.get("file"))
        except Exception:
            text = ""
        if text:
            return wb_prompt.apply_mode(messages, mode, text, degraded=degraded)
        return messages
    if degraded:
        return wb_prompt.apply_mode(messages, mode, "", degraded=True)
    return messages


def build_upstream_body(payload):
    model = payload.get("model") or ""
    # Resolve the effective thinking state before the backfill below: while
    # thinking is on, the upstream requires reasoning_content on every
    # assistant message, whether or not the client kept a reasoning trace.
    thinking = payload.get("thinking")
    thinking_type = ""
    if isinstance(thinking, dict):
        thinking_type = str(thinking.get("type") or "").strip().lower()
    effort = payload.get("reasoning_effort") or payload.get("reasoningEffort")
    thinking_enabled = False
    if str(model).lower().startswith("deepseek"):
        if thinking_type == "enabled":
            thinking_enabled = True
        elif thinking_type != "disabled" and str(effort or "").strip().lower() != "none":
            thinking_enabled = True
    messages = normalize_roles(payload.get("messages") or [])
    # Prompt mode runs before sanitize/backfill so the gateway prompt is the
    # one the upstream sees, with the client's fingerprint-y system text gone.
    messages = apply_prompt_mode(messages)
    messages = sanitize_messages(messages)
    messages = backfill_reasoning_content(
        messages, model, thinking_enabled=thinking_enabled
    )
    if not messages or (messages[0].get("role") != "system"):
        messages = [{"role": "system", "content": SYSTEM_PROMPT}] + messages
    body = dict(payload)
    # Private request markers ride along on the chat body for the Responses
    # path (the namespace map, the local-web-tools flag). They are not part of
    # the upstream protocol, so drop them here rather than trusting the
    # upstream to ignore unknown keys.
    for _marker in [k for k in body if str(k).startswith("_")]:
        body.pop(_marker, None)
    # dict(payload) 会把原始模型名一起带过去，所以别名要在这里覆盖回去
    body["model"] = model
    body["messages"] = messages
    # Repair tool-call pairing before the body leaves: a call whose result never
    # came back, results split from their batch by an interleaved message, or a
    # parallel batch split into adjacent assistant messages makes the upstream
    # reject every later turn of that conversation. Order matters: merge the
    # split declarations first, so repack sees one complete batch.
    repaired, _merged = merge_adjacent_tool_calls(body["messages"])
    repaired, _repacked = repack_tool_result_blocks(repaired)
    repaired, _cleaned = cleanup_orphan_tool_calls(repaired)
    body["messages"] = repaired
    translate_max_completion_tokens(body)
    normalize_tool_choice(body)
    normalize_tools(body)
    normalize_stop(body)
    if "max_tokens" not in body:
        default_max = model_default_max_output_tokens(model)
        if default_max:
            body["max_tokens"] = default_max
    # Thinking injection for DeepSeek models.
    #
    # thinking.type=enabled on its own does not switch the reasoning trace on:
    # the upstream still answers without one unless an effort level rides along.
    # Measured against the live upstream on deepseek-v4.1-flash, same prompt:
    #   enabled + no effort   -> reasoning_tokens 0,  reasoning_content len 0
    #   reasoning_effort=high -> reasoning_tokens 37, reasoning_content len 117
    # The client's own choice always wins; an effort level is only filled in
    # when it left the field out, and never for a request that opted out.
    if str(model).lower().startswith("deepseek"):
        thinking = body.get("thinking")
        opted_out = isinstance(thinking, dict) and \
            str(thinking.get("type") or "").strip().lower() == "disabled"
        effort = body.get("reasoning_effort") or body.get("reasoningEffort")
        if not opted_out and str(effort or "").strip().lower() != "none":
            if "thinking" not in body:
                body["thinking"] = {"type": "enabled"}
            if not effort:
                body["reasoning_effort"] = model_default_effort(model) or "high"
    body["stream"] = True
    if "stream_options" not in body:
        body["stream_options"] = {"include_usage": True}
    return body


def model_catalog_meta(model):
    """The catalog metadata dict for a model, or {}.

    Read from the same merged catalog that /v1/models advertises.
    Deliberately side-effect free: it reads the already-populated model cache
    and the shipped static tables only. Calling fetch_models() here would let a
    cold cache trigger an upstream discovery round-trip from inside request
    handling, turning one chat call into a network fetch.
    """
    if not model:
        return {}
    try:
        realm = detect_model_realm(model) or CURRENT_REALM
        entries = (_models_cache.get(realm) or {}).get("data")
        if not entries:
            name = "STATIC_CN_MODELS" if realm == "cn" else "STATIC_INTL_MODELS"
            table = getattr(wb_catalog, name, None) or wb_catalog.STATIC_MODELS
            entries = [(m.get("id"), m) for m in table if isinstance(m, dict)]
        m_lower = str(model).strip().lower()
        for mid, meta in entries:
            if str(mid).strip().lower() == m_lower:
                return meta or {}
    except Exception as exc:
        log("catalog meta lookup failed for '%s': %s" % (model, exc))
    return {}


def model_reasoning_meta(model):
    """The catalog's reasoning block for a model, or {}."""
    return model_catalog_meta(model).get("reasoning") or {}


def model_default_max_output_tokens(model):
    """The max output tokens the catalog declares for a model, or None."""
    val = model_catalog_meta(model).get("maxOutputTokens")
    if val is not None:
        try:
            val = int(val)
            if val > 0:
                return val
        except (TypeError, ValueError):
            pass
    return None


def model_default_effort(model):
    """The effort the catalog applies when the client asks for none, or None."""
    effort = model_reasoning_meta(model).get("defaultEffort")
    return effort.strip() if isinstance(effort, str) and effort.strip() else None


def model_fixed_effort(model):
    """The effort the catalog pins a model to, or None when it is selectable.

    reasoning.effort without supportedEfforts means the model always runs at
    that level: /v1/models advertises it as reasoning_fixed_effort and the
    picker offers no choice for it, so an effort the request carries anyway does
    not change what ran. A model that declares supportedEfforts is selectable
    whatever else its block carries - an `effort` beside it is the default, not
    a pin.
    """
    reasoning = model_reasoning_meta(model)
    if reasoning.get("supportedEfforts"):
        return None
    effort = reasoning.get("effort")
    return effort.strip() if isinstance(effort, str) and effort.strip() else None


def prompt_cache_key_enabled():
    """Whether to inject prompt_cache_key into outbound requests.

    Off by default. The upstream turns out to cache repeated prefixes on its
    own: with an identical ~8k-token prefix sent twice, the second call already
    reports prompt_cache_hit_tokens=9600 and the same credit with or without
    this field, on both exits (www.workbuddy.ai and copilot.tencent.com) and on
    both a free and a billed model. Injecting it changed neither the hit rate
    nor the charge, so it is left as an opt-in for experimenting rather than
    added to every request.
    """
    return os.environ.get("WB_PROMPT_CACHE_KEY", "0").strip().lower() in (
        "1", "true", "yes", "on")


def inject_prompt_cache_key(body, uid, conversation):
    """Add the upstream prompt_cache_key so its prefix cache can be reused.

    Kept for experimentation only: measurement on this upstream showed the
    prefix cache working without it (see prompt_cache_key_enabled). The key
    still carries the account uid, because the upstream cache is scoped per
    account -- a key shared between accounts would let one account's request
    read another's cached prefix, so this must never be made a fixed string.

    Priority matches the client's intent: an explicit prompt_cache_key is never
    overwritten; then the body's own conversation id; then the session key the
    gateway resolved (header or conversation-prefix derived).
    """
    if not isinstance(body, dict):
        return body
    existing = body.get("prompt_cache_key")
    if isinstance(existing, str) and existing.strip():
        return body
    conv = conversation if isinstance(conversation, str) else ""
    for field in ("conversation_id", "conversationId"):
        value = body.get(field)
        if isinstance(value, str) and value.strip():
            conv = value.strip()
            break
    uid8 = (uid or "")[:8] or "-"
    seed = "%s|%s" % (uid or "", conv)
    digest = hashlib.sha256(seed.encode("utf-8")).hexdigest()[:32]
    out = dict(body)
    out["prompt_cache_key"] = "wb2a-%s-%s" % (uid8, digest)
    return out
class ContentRejected(Exception):
    """Upstream content review rejected this request (403 / code 11140).

    Not an account problem: another credential gets the same 403 for the same
    content, so it is passed straight through instead of cooling the pool.
    """

    def __init__(self, http_error=None, detail=""):
        self.http_error = http_error
        self.detail = detail or ""
        super().__init__("upstream rejected the request content (403)")

    @property
    def code(self):
        return 403


class RateLimited(Exception):
    """Upstream throttled this model (429 / code 6004). Distinct from a dead
    pool: the credential is fine, only the model is cooling down for a while."""

    def __init__(self, http_error=None, detail="", wait=60, message=""):
        self.http_error = http_error
        self.detail = detail or ""
        self.wait = max(1, int(wait or 60))
        # 429s answered without an upstream call (the pool is parked by the
        # daily token guard) carry their own text instead of the upstream
        # wording.
        self.message = message or ""
        super().__init__("upstream rate limit: %s" % (self.detail[:200] or "429"))


def gateway_hint(status, message):
    """A gateway-side note that sits beside the raw upstream message.

    The hint never replaces the upstream wording (clients keep parsing the
    same envelope); it only explains what the gateway could classify, so a
    user staring at "400 Bad Request" learns whether the fix is a smaller
    context, a different model, a re-login or simply waiting. An empty string
    means "nothing to add" - no hint is invented for unknown failures.

    Covers the shapes the hub can classify from the upstream body/status:
    11133 model_param_invalid, 11135 invalid_image_data, rate limits, content
    review, exhausted credits, dead sessions, missing models, prompt-length
    rejections, WAF blocks and the local no-usable-account case.
    """
    text = str(message or "")
    lower = text.lower()
    if status == 503 and "concurrent chat limit" in lower:
        return "gateway is busy at its concurrency limit; retry shortly"
    if ("11133" in lower or "model_param_invalid" in lower
            or "invalid request parameters" in lower):
        return ("request parameters were rejected by the model provider; "
                "check message format and model capabilities")
    if ("11135" in lower or "invalid_image_data" in lower
            or "replace the image" in lower):
        return ("image data rejected by upstream; use a real/valid image, "
                "may need a new conversation")
    if "no usable account" in lower or "no healthy account" in lower:
        return "no healthy account available in pool; check /status or retry later"
    if ("context length" in lower or "context_length" in lower
            or "prompt too long" in lower or "too many tokens" in lower
            or "maximum context" in lower):
        return ("request context exceeds the model's limit; reduce "
                "history/message size")
    if status == 429 or "rate limit" in lower or "frequency limit" in lower:
        return "rate limited by upstream; retry after reset"
    if status == 402 or ("insufficient" in lower and "credit" in lower):
        return ("account credits exhausted at upstream; waiting for daily "
                "check-in to restore")
    if "session" in lower and ("not found" in lower or "expired" in lower):
        return ("account session expired at upstream; the account is disabled "
                "until re-login")
    if "content" in lower and ("reject" in lower or "policy" in lower):
        return ("request content was rejected by content policy; adjust the "
                "prompt and retry")
    if ("no such model" in lower or "model not found" in lower
            or "unsupported model" in lower):
        return ("upstream has no such model on this backend; switch model or "
                "retry on another account")
    if status == 403 and "waf" in lower:
        return "upstream WAF blocked the gateway; retry after the block window"
    return ""


# ---------------------------------------------------------------------------
# 出站身分自动切换
#
# 官方有三套身分（workbuddy / vscode / cli），端点与配额通道各不相同，
# 对照表见 wb_identity._ENDPOINTS。
#
# 某模型在某条通道被限流（429 / code 6004）时，换成另一套身分通常还能继续
# 用 —— 那是另一条配额线。每轮最多切 MAX_PRODUCT_SWITCHES 次，避免来回弹跳。
#
# 身分会写进凭证档并在重启后读回（issue #76）：面板手动切换当下就落盘，
# 这里的自动切换则在下一次任何 save() 时一并写入。
#
# 这个开关交给面板设定决定（issue #67），预设关闭：自动切换会吃掉重试预算，
# 也会把帐号留在操作者没主动选过的身分上，要用的话自己开。
# ---------------------------------------------------------------------------

MAX_PRODUCT_SWITCHES = 4
_SWITCH_LOG = {}

#: 出口级退避在单次请求里最多睡这么久（见 wb_egress）：退避阶梯可以到十几秒，
#: 但把一个客户端请求按住十几秒是不可接受的，所以只睡这个上限；共享计数器仍然
#: 让并发请求一起变慢（这才是「一次短暂的全局退避」）。
EGRESS_SLEEP_MAX = 5.0


def auto_switch_product_enabled():
    """Whether a 429 may rotate the outbound identity (panel setting, off by default)."""
    return wb_settings.auto_switch_product(ACCOUNTS_DIR)


def _switch_count(account, model):
    """这一轮已经切过几次（60 秒内的切换算同一轮）。"""
    entry = _SWITCH_LOG.get((account.uid, model))
    if not entry:
        return 0
    count, last = entry
    if time.time() - last > 60:
        return 0
    return count


def _try_switch_product(account, model):
    """429 时换身分重试。回传 True 表示已切换、可以重试。

    同一请求内最多切 MAX_PRODUCT_SWITCHES 次：
      cli -> workbuddy -> cli -> workbuddy
    四次都不行就放弃，让呼叫端回报真正的 429。
    """
    count = _switch_count(account, model)
    if count >= MAX_PRODUCT_SWITCHES:
        return False
    current = getattr(account, "product", wb_identity.PRODUCT_DESKTOP)
    if current == wb_identity.PRODUCT_DESKTOP:
        target = wb_identity.PRODUCT_VSCODE
    elif current == wb_identity.PRODUCT_VSCODE:
        target = wb_identity.PRODUCT_CLI
    else:
        target = wb_identity.PRODUCT_DESKTOP
    try:
        changed = account.set_product(target)
    except Exception as exc:
        log("product switch failed: %s" % exc, level="WARN")
        return False
    if not changed:
        return False
    _SWITCH_LOG[(account.uid, model)] = (count + 1, time.time())
    if len(_SWITCH_LOG) > 500:
        _SWITCH_LOG.clear()
    log("account %s: %s 被限流，自動切換身分 -> %s (第 %d/%d 次)"
        % (account.uid[:8], current, target, count + 1, MAX_PRODUCT_SWITCHES),
        level="WARN")
    return True


def reset_switch_counter(account, model):
    """成功之后归零，下一次请求重新享有 4 次切换额度。"""
    _SWITCH_LOG.pop((account.uid, model), None)


def retry_after_seconds(model, realm):
    """Shortest wait until any account of this realm can serve `model` again."""
    if not POOL:
        return 60
    waits = [a.throttle_wait(model=model) for a in POOL.accounts
             if a.realm == realm and a.enabled and a.access_token]
    active = [w for w in waits if w > 0]
    return int(min(active)) if active else 60


def realm_model_throttled(realm, model):
    """True when accounts exist and are healthy but all are cooling this model.

    Only a *model* cooldown counts. A plain account cooldown usually comes from
    a transient network error, and reporting that as "rate limited" told
    clients to back off from a model that was never throttled.
    """
    if not POOL:
        return (False, 0)
    existing = [a for a in POOL.accounts
                if a.realm == realm and a.enabled and a.access_token]
    if not existing:
        return (False, 0)
    waits = []
    for a in existing:
        wait = getattr(a, "model_cooldowns", {}).get(model, 0.0) - time.time()
        waits.append(max(0.0, wait))
    if waits and all(w > 0 for w in waits):
        return (True, int(min(waits)))
    return (False, 0)


def is_transient(exc):
    """Network-level flakiness that deserves a retry, not a cooldown.

    Upstream occasionally drops a TLS handshake mid-stream
    (SSL: UNEXPECTED_EOF_WHILE_READING / Remote end closed connection).
    Treating that as a dead account took the only intl account offline for 60s
    and turned one hiccup into a 502 storm.
    """
    t = ("%s %s" % (type(exc).__name__, exc)).lower()
    markers = (
        "ssl", "unexpected_eof", "eof occurred", "remote end closed",
        "connection reset", "connection aborted", "connectionreseterror",
        "connectionabortederror", "timed out", "timeout", "temporarily unavailable",
        "bad gateway", "502", "503", "504", "incompleteread",
    )
    return any(m in t for m in markers)

def exc_detail(exc, limit=180):
    """异常的可读原因（类型 + 底层文本），而不只是类型名。

    这些行原先只插值 `type(exc).__name__`，于是现场只剩一串 `URLError`：事后
    分不清那一下到底是 SSL 握手中断、连接被重置还是读超时，而这三者指向完全
    不同的根因——上游边缘在丢连接 / 连接被重置 / 连接池复用了已死的连接。
    URLError 的真实原因挂在 `.reason` 上（`str(exc)` 会包一层 `<urlopen error
    ...>`，`.reason` 才是干净的内层文本），所以优先取 `.reason`，空则退回
    `str(exc)`，再空则退回类型名，保证这一格永远有内容。
    """
    reason = getattr(exc, "reason", None)
    text = str(reason) if reason is not None else str(exc)
    text = (text or "").strip()
    if not text:
        return type(exc).__name__
    return ("%s: %s" % (type(exc).__name__, text))[:limit]

# The shapes the upstream uses for a rate limit. A body naming one is a soft
# limit; it still says nothing about scope, so the caller decides that from the
# reset clock (one model), from credential wording (the account), or - when
# neither is present - from repetition, and then only on one model.
SOFT_LIMIT_MARKERS = ("usage exceeds frequency limit", "too many requests",
                      "请求过于频繁")
SOFT_LIMIT_CODES = (14003,)     # intl, observed on a real 429

# Wording that scopes a 429 to the credential itself. These are matched as whole
# words or explicit phrases, never as bare substrings: a 429 body carries
# metadata such as "accountId", "account_id" or "credentialId", and a substring
# test reads those field names as a scope statement and hands the credential the
# 10m -> 2h ladder. Provisional: no captured body has shown yet what a genuine
# account-level 429 says, so the list stays short and fails towards the narrow
# model window.
ACCOUNT_SCOPE_PHRASES = (
    "account-level", "account level", "account-wide", "account scope",
    "account quota", "account rate limit", "account limit",
    "per account", "per-account", "this account", "your account",
    "the account", "an account",
    "credential-level", "credential level", "credential scope",
    "credential limit", "per credential", "this credential",
    "the credential", "your credential",
    "账号级", "账户级", "该账号", "此账号", "该账户", "此账户",
    "账号限流", "账户限流", "凭证级", "该凭证",
)


def credential_scope_phrase(detail):
    """The scope phrase the body uses, or None when it names no scope.

    ASCII phrases are anchored on word boundaries, so an identifier like
    `accountId` cannot satisfy one. The Chinese phrases are plain substrings:
    they have no word boundary to anchor to and cannot occur inside an
    identifier.
    """
    lowered = (detail or "").lower()
    for phrase in ACCOUNT_SCOPE_PHRASES:
        if phrase.isascii():
            if re.search(r"\b%s\b" % re.escape(phrase), lowered):
                return phrase
        elif phrase in lowered:
            return phrase
    return None


def rate_limit_code(detail):
    """The `code` field of a JSON 429 body, or None when it is not JSON."""
    try:
        obj = json.loads(detail)
    except Exception:
        return None
    return obj.get("code") if isinstance(obj, dict) else None


def rate_limit_is_soft_shape(detail):
    """True when a 429 body names a rate limit, whatever its scope.

    Both shapes the incident environment produced are covered: the bare "usage
    exceeds frequency limit" body (229 of the 263 recorded 429s) and the intl
    code 14003 body ("too many requests"). Recognising them is what lets the
    caller answer with a window on one model rather than on the credential.
    """
    text = (detail or "").lower()
    if any(marker in text for marker in SOFT_LIMIT_MARKERS):
        return True
    return rate_limit_code(detail) in SOFT_LIMIT_CODES


# 上游「这个模型的窗口额度用满」的应答码。6004 是唯一带重置墙钟的额度判定，
# 也是用户口径里「达到限额」的唯一标志：别的 429（14003、裸频率限制文本、
# 账号级软限）即使碰巧带了个时钟，也不是额度用满的证据。
CAP_LIMIT_CODE = 6004


def rate_limit_is_cap(detail):
    """True only for the upstream's quota-cap answer (code 6004).

    Tolerant on purpose: the field has only ever been seen as an integer, but
    a string form must not silently drop a real cap. Anything unparsable is
    not a cap, so the caller's persistent-disable path fails closed.
    """
    try:
        return int(rate_limit_code(detail)) == CAP_LIMIT_CODE
    except (TypeError, ValueError):
        return False


def rate_limit_is_account_level(detail, reset_at):
    """True only with positive evidence that the 429 is credential-scoped.

    The model-scoped form names a reset wall clock (code 6004). Treating the
    *absence* of one as account scope made every 429 without a timestamp - which
    includes the most common body the upstream sends - buy the whole credential
    a 600s cooldown doubling to 7200s, so a handful of them left every account of
    a realm cooling at once while a panel test kept reporting the credential
    healthy. Absence is evidence of nothing: the credential is parked only when
    the body itself says the limit is on the credential; everything else gets a
    window on one model, decided by the caller.

    Provisional: the phrase list is short because no captured body has shown yet
    what a genuine account-level 429 looks like, so it is meant to be re-checked
    against one rather than grown speculatively.
    """
    if reset_at is not None:
        return False
    return credential_scope_phrase(detail) is not None


#: 上游风控限制（HTTP 403, code 11140「账号存在异常行为，已被限制使用」）
#: 的账号冷却时长：限制按小时计，2 小时后自然重试，上游解封即自愈——比
#: 透传给客户端、账号继续被选用好，也比 402 那种「停到本地 04:00」轻。
RESTRICTED_403_COOLDOWN = 7200.0

#: 识别风控限制 403 的正文特征。内容审核的 403 走降级提示词那条路，二者必须
#: 分开：对被限制的账号换提示词没有意义。code 匹配用正则（requestId 是 hex，
#: 裸字符串 "11140" 可能撞上），措辞只收实测的限制用语，不猜。
_RESTRICTED_403_CODE_RE = re.compile(r'"code"\s*:\s*"?11140"?')
_RESTRICTION_403_MARKERS = ("unusual activity", "账号存在异常行为", "已被限制")


def restriction_is_account_level(detail):
    """True when a 403 body says the ACCOUNT is restricted, not the content.

    实测正文：{"code":11140,"msg":"request illegal",...,"displayMsg":{"en":
    "Your account has been restricted due to unusual activity","zh":"账号存在
    异常行为，已被限制使用"}}。en 在前 zh 在后，短截断也保得住 code 与 en；
    拿不全的正文宁可漏判回内容审核路径，也不误判。
    """
    if not detail:
        return False
    if _RESTRICTED_403_CODE_RE.search(detail):
        return True
    lowered = detail.lower()
    return any(marker in lowered for marker in _RESTRICTION_403_MARKERS)


def parse_rate_limit_reset(detail):
    """Pull the reset time out of an upstream 429 body, if it names one.

    Upstream answers code 6004 with a reset wall clock, but the wording is
    per-realm: the intl form is "... your usage will reset at
    2026-09-19 18:29:03 UTC+8 ...", the cn form is "... 将在
    2026-10-09 14:44:59 UTC+8 重置 ...". Matching only the English form left
    every cn 429 without a reset time, so it read as an account-level soft
    limit and cooled the whole credential instead of parking just the
    throttled model. Returns an epoch or None. Kept tolerant on purpose: an
    unparseable body must not break the request path.
    """
    if not detail:
        return None
    m = re.search(r"(?:reset at|将在)\s*"
                  r"(\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2})", detail)
    if not m:
        return None
    stamp = m.group(1).replace("T", " ")
    tz = re.search(r"UTC([+-]\d{1,2})(?::?(\d{2}))?", detail)
    offset = 0
    if tz:
        hours = int(tz.group(1))
        minutes = int(tz.group(2) or 0)
        offset = hours * 3600 + (minutes * 60 if hours >= 0 else -minutes * 60)
    try:
        base = time.mktime(time.strptime(stamp, "%Y-%m-%d %H:%M:%S")) - time.timezone
        return base - offset
    except Exception:
        return None


def upstream_timeouts():
    """(header, idle) seconds for the chat upstream socket.

    header covers connect/TLS/first byte; idle bounds each mid-stream
    read, so active data keeps the connection alive while a silent
    stream fails out and releases its in-flight lease. There is no
    total-duration cap, matching the panel project's semantics.
    """
    try:
        cfg = wb_settings.upstream_config(ACCOUNTS_DIR)
    except Exception:
        cfg = None
    header = 120.0
    idle = 300.0
    if isinstance(cfg, dict):
        try:
            header = float(cfg.get("header_timeout_seconds") or header)
        except (TypeError, ValueError):
            pass
        try:
            idle = float(cfg.get("idle_timeout_seconds") or idle)
        except (TypeError, ValueError):
            pass
    return max(1.0, header), max(1.0, idle)


def _apply_stream_idle_timeout(response, seconds):
    """Switch the upstream socket to the mid-stream idle timeout.

    urllib's timeout covers the initial request; the same socket is then
    read for the whole SSE stream. Setting the socket timeout to the idle
    value bounds each read instead of the whole stream. Best-effort: an
    unknown socket shape keeps the header timeout.
    """
    try:
        seconds = float(seconds)
    except (TypeError, ValueError):
        return False
    if seconds <= 0:
        return False
    fp = getattr(response, "fp", None)
    raw = getattr(fp, "raw", None)
    sock = getattr(raw, "_sock", None)
    if sock is None:
        sock = getattr(fp, "_sock", None)
    if sock is None:
        return False
    try:
        sock.settimeout(seconds)
        return True
    except Exception:
        return False


def open_upstream(payload, session_key=None, target_realm=None, prebuilt_body=None):
    # Refresh the daily guards before picking. The scan underneath is
    # incremental and TTL-cached, so this is a stat() plus a cached dict on
    # the hot path, and an account parked by any of the guards is skipped
    # like any other unusable one. One call covers all three guards: they
    # share the same fold, and calling them separately paid for it three
    # times on every request.
    apply_daily_guards()
    # 剩余用量优先调度（开关默认关）：把「估计剩余接近耗尽」的 (账号, 模型)
    # 权重推给池，让它们先接单。只在开关打开时读剩余估算（载荷有短 TTL，热路径
    # 上是缓存命中）；关着时推空表，把上一次可能留下的权重清掉，下一个请求就
    # 回到与从前逐字节一致的纯轮询。
    if POOL is not None:
        POOL.apply_remaining_weights(
            remaining_schedule_weights() if remaining_priority_enabled() else None)
    realm = target_realm or detect_model_realm(payload.get("model")) or CURRENT_REALM
    model = str(payload.get("model") or "")
    # 复用调用方已建好的 body：/v1/chat/completions 在进这里之前已经 build 过
    # 一次（那次的产物只用于一行日志和 prompt_fingerprint），此前这里会为同一
    # 个 payload 再建一遍，大请求实测白付 ~13.6ms/请求。403 降级重试时仍按
    # payload 重建（degrade 会切换 prompt 模式，必须拿到新 body）。
    upstream_body = (prebuilt_body if prebuilt_body is not None
                     else build_upstream_body(payload))
    # PATCHED-BY-OPS: 客户端未提供会话标识时，用对话稳定前缀兜底。
    # 位置放在 build_upstream_body 之后，保证键与真正发往上游的消息一致
    # （该函数可能在最前面插入 SYSTEM_PROMPT）。affinity_page 只在超限对话上
    # 有值（页内轮转），客户端自带会话键的请求不参与。
    affinity_page = None
    if not session_key:
        session_key, affinity_page = affinity_route(upstream_body.get("messages"))
        if session_key and AFFINITY_DEBUG:
            log("affinity: derived %s for %d msgs"
                % (session_key, len(upstream_body.get("messages") or [])))
    total = max(1, POOL.count_ready(realm, model=model)) if POOL else 1
    tried = set()
    # 这一轮请求已经记过账的账号：重试轮（transient_hits 后 tried.clear() 整池
    # 重轮）会把同一个账号再挑一遍，若每次都记，一次请求就能给同一个账号记上
    # 好几笔 fails/degrade——池子越空、重轮越频繁，越容易把一个刚抖了一下的
    # 账号直接推过熔断阈值。一次请求对一个账号最多记一次。
    charged = set()

    def charge_account(account, message, unknown):
        """给账号记一笔失败，一次请求最多记一笔（见上面的 charged）。"""
        if account.uid in charged:
            return False
        charged.add(account.uid)
        if unknown:
            account.note_unknown_failure(message)
        else:
            account.note_failure(message)
        return True

    last_error = None
    last_uid = None
    last_429 = None
    last_429_detail = ""
    last_403_detail = ""
    transient_hits = 0
    degraded_retried = False
    # Read once per request, not per attempt: this is a panel setting, and a
    # settings read on every retry would be pure overhead.
    auto_switch = auto_switch_product_enabled()
    # Panel parity: header/idle timeouts are read once per request, not
    # once per retry, so a settings read never lands in the retry loop.
    header_timeout, idle_timeout = upstream_timeouts()
    # Same treatment for the egress-level backoff switch (issue #271): one read
    # per request, and when it is off the retry loop below is byte-for-byte the
    # old path.
    egress_backoff = wb_settings.egress_backoff_enabled(ACCOUNTS_DIR)
    # 同一段出口在一次请求里只记一次：重试轮（tried.clear() 之后整池重轮）会把
    # 同一个账号 / 同一段出口再挑一遍，若每次都记，一次请求就能把共享计数器推高
    # 好几步——那正是 issue #271 里「一次请求给同一账号记好几笔」的另一半。已经
    # 记过的那一段沿用同一段退避，不重复记。
    egress_wait = {}

    def charge_egress(url, proxy):
        key = wb_upstream_pool.egress_key(url, proxy)
        if key not in egress_wait:
            egress_wait[key] = wb_egress.note_transient(key)
        return egress_wait[key]

    max_attempts = max(2, total) + 1 + (MAX_PRODUCT_SWITCHES if auto_switch else 0)
    # 每次失败都把【出错的那个账号】从这段对话的历史里降级（demote），而不是
    # 忘掉整段对话：重选时优先回到还留着这段前缀的暖号（见 SessionAffinity），
    # 一次大 prompt 的冷启动比重选一次账号贵得多。
    for _attempt in range(max_attempts):
        account = POOL.pick_for_session(realm=realm, session_key=session_key,
                                        exclude=tried, model=model,
                                        page=affinity_page) if POOL else None
        if account is None:
            if transient_hits and _attempt < max_attempts - 1:
                tried.clear()
                time.sleep(min(1.5 * transient_hits, 3.0))
                continue
            break
        if account.realm != realm:
            if session_key and POOL: POOL.affinity.demote(session_key, account.uid)
            continue
        tried.add(account.uid)
        last_uid = account.uid
        cfg = wb_accounts.get_realm_config(account.realm)
        chat_url = account.chat_base_url() + CHAT_PATH
        # The cache key is account scoped, so it is rebuilt per candidate rather
        # than once up front. Opt-in only: measurement showed the upstream
        # caches prefixes without it (see prompt_cache_key_enabled).
        if prompt_cache_key_enabled():
            attempt_body = inject_prompt_cache_key(upstream_body, account.uid, session_key)
        else:
            attempt_body = upstream_body
        attempt_data = json.dumps(attempt_body, ensure_ascii=False).encode("utf-8")
        req = urllib.request.Request(chat_url, data=attempt_data, method="POST",
                                     headers=account.headers(purpose="chat"))
        try:
            resp = wb_accounts.urlopen(req, timeout=header_timeout,
                                       proxy=account.proxy)
            _apply_stream_idle_timeout(resp, idle_timeout)
            account.note_success(model=model)
            if egress_backoff:
                # This outbound path just served a request, so its transient
                # streak is over (same rule as the account's note_success).
                wb_egress.note_success(
                    wb_upstream_pool.egress_key(chat_url, account.proxy))
            reset_switch_counter(account, model)
            # The third element is the reasoning effort this request ran at: the
            # body is rebuilt per attempt, but the effort is a property of the
            # model and the request, and the callers record it on the usage row.
            return resp, account, upstream_effort_of(upstream_body, model)
        except urllib.error.HTTPError as exc:
            if exc.code == 429:
                try:
                    detail = exc.read(600).decode("utf-8", "replace")
                except Exception:
                    detail = ""
                reset_at = parse_rate_limit_reset(detail)
                if rate_limit_is_account_level(detail, reset_at):
                    wait = account.note_soft_rate("HTTP 429 (account soft rate)")
                    log("account %s soft-rate limited (%r), cooling %.0fs (streak %d)"
                        % (account.uid[:8], credential_scope_phrase(detail), wait,
                           account.soft_streak))
                    if session_key and POOL:
                        POOL.affinity.demote(session_key, account.uid)
                    last_error = exc
                    last_429 = exc
                    last_429_detail = detail
                    continue
                # No reset clock, so nothing here says whether the limit is on
                # the credential or on one model. For a recognised soft-limit
                # shape the repetition is the evidence: the window goes on this
                # model and grows on the short ladder (60s up to 10m) instead of
                # the credential's 600s -> 2h one. An unrecognised body gets the
                # same 60s on one model, with no streak at all.
                wait = max(1.0, reset_at - time.time()) if reset_at else 60.0
                if reset_at is None and rate_limit_is_soft_shape(detail):
                    wait = account.note_unscoped_rate(model)
                    log("account %s: unscoped 429, parking '%s' for %.0fs "
                        "(unscoped streak %d)"
                        % (account.uid[:8], model, wait, account.unscoped_streak))
                # Model-scoped: only this model is throttled for this account,
                # so sibling models stay serviceable on the same credential.
                account.note_error("HTTP 429 (model throttled)", model=model, until=reset_at,
                                   cooldown=wait, detail=detail)
                if reset_at:
                    # A cap with a reset clock is the one observable moment the
                    # window budget can be measured from; the usage rows alone
                    # lose it (only the last account of a retry batch keeps its
                    # attribution). See note_limit_event().
                    note_limit_event(account, model, reset_at)
                    # 「达到限额」的唯一标志是上游返回 6004（用户口径）：别的
                    # 429 即使带重置墙钟也不构成额度判定，不记 cap、不写盘。
                    # 记录范围（免费模型）由 note_model_cap 把守——范围外的
                    # 模型返回 False，重启后自动放开，不参与持久化禁用。
                    if rate_limit_is_cap(detail):
                        try:
                            if account.note_model_cap(model, reset_at):
                                _save_account_caps(account, model, reset_at)
                        except Exception as exc:
                            log("account %s: model cap not persisted: %s"
                                % (account.uid[:8], exc))
                if auto_switch and _try_switch_product(account, model):
                    # 换了身分就等于换了一条配额线：要把它从「已试过」拿掉，
                    # 并清掉刚刚记下的模型冷却，否则下一轮回圈会找不到帐号。
                    # keep_caps：6004 的禁用（仍在恢复期内）跨身分成立，不能
                    # 被这次清理带走——真机实测它会让 6004 之后账号立刻又可用。
                    tried.discard(account.uid)
                    try:
                        account.clear_error(model=model, keep_caps=True)
                    except Exception:
                        pass
                    if session_key and POOL:
                        POOL.affinity.demote(session_key, account.uid)
                    continue
                log("account %s throttled on '%s' (429), retry in %ds"
                    % (account.uid[:8], model, int(wait)))
                if session_key and POOL:
                    POOL.affinity.demote(session_key, account.uid)
                last_error = exc
                last_429 = exc
                last_429_detail = detail
                continue
            if exc.code == 403:
                try:
                    detail = exc.read(600).decode("utf-8", "replace")
                except Exception:
                    detail = ""
                # 先分流：上游风控限制（code 11140「账号存在异常行为，已被限制
                # 使用」）是账号级的坏，降级提示词救不了，透传给客户端只会让
                # 一个被限制的账号把对话一次次打死。冷却这个账号、换下一个继
                # 续；全都试完还是不行才把原因交给客户端。
                if restriction_is_account_level(detail):
                    account.note_error("HTTP 403 (account restricted)",
                                       cooldown=RESTRICTED_403_COOLDOWN,
                                       single_account=(total <= 1), detail=detail)
                    log("account %s restricted by upstream (403 11140), "
                        "cooling %.0fs" % (account.uid[:8], RESTRICTED_403_COOLDOWN))
                    if session_key and POOL:
                        POOL.affinity.demote(session_key, account.uid)
                    last_error = exc
                    last_403_detail = detail
                    continue
                # Panel degrade.go: a content rejection in passthrough/append
                # mode is usually a system-prompt fingerprint false positive.
                # Switch to the minimal neutral prompt until next 00:00 CST
                # and retry this turn once; custom mode opted out.
                if (not degraded_retried
                        and prompt_mode_config().get("mode") in ("passthrough", "append")):
                    PROMPT_DEGRADE.trigger()
                    degraded_retried = True
                    upstream_body = build_upstream_body(payload)
                    tried.discard(account.uid)
                    log("upstream 403 (content review) -> degraded prompt retry")
                    continue
                log("upstream 403 for '%s' (content review), passing through" % model)
                if session_key and POOL:
                    POOL.affinity.demote(session_key, account.uid)
                last_error = exc
                last_403_detail = detail
                break
            if exc.code == 402:
                try:
                    detail = exc.read(400).decode("utf-8", "replace")
                except Exception:
                    detail = ""
                account.note_balance_cooled(detail or "HTTP 402 (insufficient credits)")
                log("account %s out of credits (402), parked until 04:00"
                    % account.uid[:8])
                if session_key and POOL:
                    POOL.affinity.demote(session_key, account.uid)
                last_error = exc
                continue
            if exc.code == 401:
                log("account %s rejected (HTTP 401), rotating" % account.uid[:8])
                if session_key and POOL:
                    POOL.affinity.demote(session_key, account.uid)
                account.note_error("HTTP 401",
                                   cooldown=60,
                                   single_account=(total <= 1))
                last_error = exc
                continue
            if exc.code >= 500:
                # 一切服务端 5xx（含上游自定义的 550 这类）都按瞬时故障轮转，
                # 不在已知清单里的也不能把错误原样甩回客户端——换号是默认，
                # 全试完才交出去。
                transient_hits += 1
                if egress_backoff and exc.code in (502, 503, 504):
                    # Gateway-side blips (502/503/504) say the path is unhappy,
                    # not the credential: charge the egress and back off for
                    # seconds (issue #271). 500 stays on the account - it can be
                    # a real fault tied to this credential and needs another look
                    # before it is moved off the breaker.
                    wait = charge_egress(chat_url, account.proxy)
                    log("upstream %s for '%s', backing off the egress %.1fs"
                        % (exc.code, model, wait))
                    if session_key and POOL:
                        POOL.affinity.demote(session_key, account.uid)
                    last_error = exc
                    time.sleep(min(wait, EGRESS_SLEEP_MAX))
                    continue
                charge_account(account, "HTTP %d" % exc.code, unknown=False)
                log("upstream %s for '%s', retrying (fails=%d)"
                    % (exc.code, model, account.fails))
                if session_key and POOL:
                    POOL.affinity.demote(session_key, account.uid)
                last_error = exc
                continue
            raise
        except Exception as exc:
            if session_key and POOL:
                POOL.affinity.demote(session_key, account.uid)
            if is_transient(exc):
                transient_hits += 1
                if egress_backoff:
                    # A transport blip is the path's problem, not the account's:
                    # charge the egress and back off for seconds (issue #271).
                    # The account's fails/degrade counters are left alone, so a
                    # few seconds of flakiness no longer parks healthy accounts.
                    wait = charge_egress(chat_url, account.proxy)
                    log("account %s: upstream connection hiccup for '%s' (%s), "
                        "backing off the egress %.1fs"
                        % (account.uid[:8], model, exc_detail(exc), wait))
                    last_error = exc
                    time.sleep(min(wait, EGRESS_SLEEP_MAX))
                    continue
                charge_account(account, "connection: %s" % exc_detail(exc, 120),
                               unknown=True)
                # 带上 UID：这一行同时是一次扣分（连续 3 次就把账号熔断 30
                # 分钟），不写清楚是谁挨的这一下，事后只能看着一串 hiccup 猜
                # 是哪几个账号被关掉、池子为什么突然空了。
                log("account %s: upstream connection hiccup for '%s' (%s), "
                    "retrying (degrade=%d)"
                    % (account.uid[:8], model, exc_detail(exc), account.degrade_count))
                last_error = exc
                time.sleep(min(0.6 * transient_hits, 2.0))
                continue
            charge_account(account, str(exc)[:120], unknown=True)
            account.note_error(str(exc)[:120], cooldown=60, single_account=(total <= 1))
            last_error = exc
            continue
    if last_error is not None:
        # Carry the account that produced the failure out to the caller, so
        # the error row can name it even though the local variable that would
        # have held it was never assigned in the caller.
        try:
            last_error.account_uid = last_uid
        except Exception:
            pass
        if last_429 is not None:
            exc = RateLimited(last_429, last_429_detail,
                              wait=retry_after_seconds(model, realm))
            exc.account_uid = last_uid
            raise exc
        if last_403_detail:
            exc = ContentRejected(last_error, last_403_detail)
            exc.account_uid = last_uid
            raise exc
        raise last_error
    throttled, wait = realm_model_throttled(realm, model)
    if throttled:
        raise RateLimited(None, "usage exceeds frequency limit", wait=wait)
    enabled = [a for a in POOL.accounts
               if a.realm == realm and a.enabled and a.access_token] if POOL else []
    if enabled and all(a.daily_limit_blocked() for a in enabled):
        reason = ("every usable account reached today's token limit (%s per "
                  "account); the pool resumes after local midnight"
                  % wb_settings.daily_token_limit(ACCOUNTS_DIR, realm))
        raise RateLimited(None, reason,
                          wait=seconds_until_local_midnight(), message=reason)
    if enabled and model and all(a.credit_limit_blocked(model) for a in enabled):
        reason = ("every usable account reached today's credit limit (%s per "
                  "account); paid models resume after local midnight, free "
                  "models keep working"
                  % wb_settings.daily_credit_limit(ACCOUNTS_DIR, realm))
        raise RateLimited(None, reason,
                          wait=seconds_until_local_midnight(), message=reason)
    if enabled and model and all(a.model_token_limit_blocked(model) for a in enabled):
        reason = ("every usable account reached today's token limit for %s "
                  "(%s per account); the model resumes after local midnight"
                  % (model, wb_settings.model_daily_token_limit(ACCOUNTS_DIR, realm)))
        raise RateLimited(None, reason,
                          wait=seconds_until_local_midnight(), message=reason)
    raise RuntimeError("no usable account for realm '%s': all are disabled, cooling down, "
                       "expired or parked by the daily token limit%s"
                       % (realm, pool_unavailable_detail(realm, model)))

def pool_unavailable_detail(realm, model=None):
    """把该区域每个账号不可用的原因拼成一句，附在「没有可用账号」后面。

    这句话以前只列了四种可能（停用 / 冷却 / 过期 / 日限额），而 v1.6.16 之后
    账号还会被熔断与降权窗口挡住——恰好这两种一个字都没提。于是生产上出现过
    一场很难解释的报错：上游成片丢连接，几个账号各连踩 3 次触发 30 分钟熔断，
    池子瞬间空掉、请求 11ms 就回 503，而看板上那些账号仍然显示「可用」，
    排查的人只能对着「明明有 9 个账号」的池子猜。

    这里不改变判定，只把判定结果说出来：每个不可用账号「八位 UID + 原因」。
    """
    if POOL is None:
        return ""
    accounts = [a for a in POOL.accounts if a.realm == realm]
    if not accounts:
        return "；该区域没有任何账号"
    reasons = []
    for account in accounts:
        try:
            reason = account.unavailable_reason(model=model)
        except Exception:
            reason = ""
        if reason:
            reasons.append("%s %s" % ((account.uid or "?")[:8], reason))
    if not reasons:
        return ""
    return "；各账号：%s" % "，".join(reasons)

def extract_session_key(headers, payload):
    key = (
        headers.get("X-Conversation-Id") or
        headers.get("Conversation-Id") or
        headers.get("X-Session-Id") or
        headers.get("Session-Id") or
        payload.get("conversation_id") or
        payload.get("session_id") or
        (payload.get("metadata") or {}).get("conversation_id")
    )
    if key:
        return str(key).strip()
    return None

# CJK 计数走单遍 C 扫描：先 sub 掉 CJK 再按长度差计数，与原来的逐字符 Python
# 循环逐值相等（64KB ASCII 实测 18.7ms -> 1.25ms）。别换 findall，实测更慢。
# 调用点：aggregate_stream、_chat_stream_response 的 usage 兜底、
# /v1/messages/count_tokens。
CJK_RE = re.compile(r"[\u4e00-\u9fff\u3400-\u4dbf]")
def estimate_tokens(text):
    if not text:
        return 0
    if not isinstance(text, str):
        text = str(text)
    cjk = len(text) - len(CJK_RE.sub("", text))
    other = len(text) - cjk
    return cjk + max(1, int(other / 3.6)) if text else 0

# 每次向流式响应要多少字节。64 KiB 远大于一条 SSE 事件（典型几百字节），
# 又远小于 http.client 单次 read 的上限，按它切块能把「一行一次 syscall」
# 摊成「一块一次」，同时不会让首字节延迟变差（见 iter_sse_lines）。
SSE_READ_BLOCK = 64 * 1024

def iter_sse_lines(source, block_size=SSE_READ_BLOCK, before_read=None):
    """按块读取流式响应并逐行产出，把逐行 read 的 syscall 开销摊掉。

    直接 `for line in resp` 迭代 http.client.HTTPResponse 时，每一行都要走
    一次 read() syscall，而每次 syscall 都要释放并重新抢回 GIL。一轮流式回复
    动辄几千条事件，于是进程大部分时间花在二十来个 handler 线程之间来回交接
    GIL 上，而不是在干活。4 vCPU 真机实测（v1.7.2，15~20 req/s）：10 秒内
    106 214 次 futex，占全部 syscall 时间的 71%，其中约三分之二来自这条逐行
    读取路径。

    read1() 每次最多一次底层 syscall，所以按块取回来再在内存里切行，能把
    syscall 数降一到两个数量级。延迟不变：read1() 一有字节就返回，完整的行
    立刻产出；只有跨块的那一行需要等下一块。

    产出的行**带换行符**，与直接迭代 response 的形状完全一致，调用方一行都
    不用改。没有 read1 的对象（列表、文本模式的文件……）退回普通迭代。

    `before_read` 在每个**可能阻塞的 read1 之前**调一次，调用方可以在这里把
    攒下的写出掉：一个读取块里的事件因此合成一次 write，而下一个字节一到就
    立刻产出——延迟与逐块写完全一样，只是不再一个 chunk 一次 sendall。
    """
    read1 = getattr(source, "read1", None)
    if read1 is None:
        for line in source:
            if before_read is not None:
                before_read()
            yield line
        return
    buf = b""
    while True:
        if before_read is not None:
            before_read()
        chunk = read1(block_size)
        if not chunk:
            break
        buf += chunk
        if b"\n" not in buf:
            continue
        head, _, buf = buf.rpartition(b"\n")
        for line in head.split(b"\n"):
            yield line + b"\n"
    if buf:
        # 上游没以换行收尾时，最后一行照旧要交出去（迭代 response 也是这个形状）。
        yield buf

def aggregate_stream(raw_iter, model, resp_id):
    """Fold an SSE stream into one non-streaming chat.completion object."""
    content, reasoning, finish = [], [], "stop"
    tool_calls_map = {}
    usage = None
    started = time.time()
    first_chunk_at = None
    saw_done = False
    for line in iter_sse_lines(raw_iter):
        data = strip_data_prefix(line.decode("utf-8", "replace"))
        if not data or data == "[DONE]":
            if data == "[DONE]":
                saw_done = True
            continue
        try:
            chunk = json.loads(data)
        except Exception:
            continue
        if first_chunk_at is None:
            first_chunk_at = time.time()
        if chunk.get("id"):
            resp_id = chunk["id"]
        if chunk.get("model"):
            model = chunk["model"]
        u = chunk.get("usage")
        if u:
            if usage is None or (u.get("total_tokens") or 0) >= (usage.get("total_tokens") or 0):
                usage = u
        for choice in chunk.get("choices") or []:
            delta = choice.get("delta") or {}
            if delta.get("content"):
                content.append(delta["content"])
            if delta.get("reasoning_content"):
                reasoning.append(delta["reasoning_content"])
            for tc in delta.get("tool_calls") or []:
                idx = tc.get("index")
                if idx is None:
                    idx = len(tool_calls_map)
                fn = tc.get("function") or {}
                call_id = tc.get("id")
                fn_name = fn.get("name") or ""
                fn_args = fn.get("arguments") or ""
                if idx not in tool_calls_map:
                    tool_calls_map[idx] = {
                        "id": call_id or _new_id("call_"),
                        "type": tc.get("type") or "function",
                        "function": {
                            "name": fn_name,
                            "arguments": fn_args,
                        }
                    }
                else:
                    entry = tool_calls_map[idx]
                    if call_id:
                        entry["id"] = call_id
                    if fn_name:
                        entry["function"]["name"] = (entry["function"]["name"] or "") + fn_name
                    if fn_args:
                        entry["function"]["arguments"] = (entry["function"]["arguments"] or "") + fn_args
            fc = delta.get("function_call")
            # PATCH2-BY-OPS: 上游会在流末尾发 function_call:{"name":"","arguments":""}
            # 占位。原判断对空 dict 成立，会凭空生成 tool_call 并伪造 id，
            # 导致 finish_reason 被改成 "tool_calls"（参数全空）→ 严格客户端死等。
            # 故：只要 name 为空即视为无效占位直接跳过。
            fc_is_empty = (not isinstance(fc, dict)) or (not fc.get("name"))
            if fc and isinstance(fc, dict) and not fc_is_empty:
                idx = 0
                if idx not in tool_calls_map:
                    tool_calls_map[idx] = {
                        "id": _new_id("call_"),
                        "type": "function",
                        "function": {
                            "name": fc.get("name") or "",
                            "arguments": fc.get("arguments") or "",
                        }
                    }
                else:
                    entry = tool_calls_map[idx]
                    if fc.get("name") and not entry["function"]["name"]:
                        entry["function"]["name"] = fc["name"]
                    if fc.get("arguments"):
                        entry["function"]["arguments"] += fc["arguments"]
            if choice.get("finish_reason"):
                finish = choice["finish_reason"]
    message = {"role": "assistant", "content": "".join(content)}
    if reasoning:
        message["reasoning_content"] = "".join(reasoning)
    # A call the model printed instead of emitting arrives as `content`. The
    # Responses path has parsed that shape for a while; this one did not, so the
    # client saw prose, found no call, and ended the turn with nothing to run.
    if not tool_calls_map and repair_leaked_tool_calls(message):
        finish = "tool_calls"
    # PATCH2-BY-OPS: 二次防御——剔除「无函数名且无参数」的空 tool_call。
    # 即使上游以 tool_calls 数组形式发空占位，也不会泄漏给客户端。
    if tool_calls_map:
        tool_calls_map = {
            k: v for k, v in tool_calls_map.items()
            if (v.get("function") or {}).get("name")
        }
    # A stream cut off by max_tokens / a dropped connection leaves the last
    # tool call with half-written JSON. Drop those calls instead of handing
    # the client unparsable arguments; only non-empty unparsable strings
    # count as truncated, so no-argument tools survive.
    if tool_calls_map and (finish == "length" or not saw_done):
        tool_calls_map = {
            k: v for k, v in tool_calls_map.items()
            if not is_truncated_arguments((v.get("function") or {}).get("arguments"))
        }
    if tool_calls_map:
        ordered_tcs = [tool_calls_map[k] for k in sorted(tool_calls_map.keys())]
        message["tool_calls"] = ordered_tcs
        if finish in ("stop", None):
            finish = "tool_calls"
    elif message.get("tool_calls"):
        # A call recovered from `content` counts as a real one, and it also
        # overrides a "length" the upstream reported for the same turn: the call
        # came through whole, so the turn did not really run out of room.
        finish = "tool_calls"
    elif finish == "tool_calls":
        # 占位被全部过滤掉，无实际工具调用，降级为正常结束，防止客户端无限挂起等待
        finish = "stop"
    if usage is None or (usage.get("total_tokens") or 0) == 0:
        full_c = "".join(content)
        full_r = "".join(reasoning)
        if full_c or full_r:
            comp = estimate_tokens(full_c) + estimate_tokens(full_r)
            prompt_est = max(1, comp // 2)
            usage = {
                "prompt_tokens": prompt_est,
                "completion_tokens": comp,
                "total_tokens": prompt_est + comp,
                "completion_tokens_details": {"reasoning_tokens": estimate_tokens(full_r)},
                "prompt_tokens_details": {"cached_tokens": 0},
            }
    out = {
        "id": resp_id or "chatcmpl-wb",
        "object": "chat.completion",
        "created": int(time.time()),
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
    }
    if usage:
        normalize_usage_cache_aliases(usage)
        out["usage"] = usage
    out["elapsed_ms"] = int((time.time() - started) * 1000)
    out["first_chunk_at"] = first_chunk_at
    return out
# ---------------------------------------------------------------------------
# Responses API (/v1/responses) <-> Chat Completions translation
# ---------------------------------------------------------------------------
#
# Kelivo and other clients can speak OpenAI's newer Responses API. The upstream
# gateway only speaks Chat Completions, so those requests are translated down,
# and the reply is translated back up into Responses objects / SSE events.
def local_ip_addresses():
    """Every non-loopback IPv4 address this machine answers on."""
    found = []
    try:
        for info in socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET):
            ip = info[4][0]
            if ip not in found and not ip.startswith("127."):
                found.append(ip)
    except Exception:
        pass
    if not found:
        try:
            probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            probe.connect(("8.8.8.8", 80))
            found.append(probe.getsockname()[0])
            probe.close()
        except Exception:
            pass
    return found
def _new_id(prefix):
    return prefix + uuid.uuid4().hex
def _flatten_content(content):
    """Flatten Responses-style content into text, or OpenAI vision parts."""
    if content is None:
        return ""
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return str(content)
    texts, parts = [], []
    for piece in content:
        if isinstance(piece, str):
            texts.append(piece)
            parts.append({"type": "text", "text": piece})
            continue
        if not isinstance(piece, dict):
            continue
        ptype = piece.get("type") or ""
        if ptype in ("input_text", "output_text", "text", "summary_text"):
            t = piece.get("text") or ""
            texts.append(t)
            parts.append({"type": "text", "text": t})
        elif ptype in ("input_image", "image_url", "image") or "image_url" in piece:
            url = piece.get("image_url") or piece.get("url")
            if isinstance(url, dict):
                url = url.get("url")
            if not url and piece.get("data"):
                mime = piece.get("mimeType") or piece.get("mime_type") or "image/png"
                url = f"data:{mime};base64," + piece["data"]
            if url:
                parts.append({"type": "image_url", "image_url": {"url": url}})
    if any(p.get("type") == "image_url" for p in parts):
        return parts          # multimodal: keep structured parts
    return chr(10).join(t for t in texts if t)


# ---------------------------------------------------------------------------
# Responses API "custom" (freeform) tools
#
# Some clients - most notably Codex 0.15x - declare their file-editing tool as a
# *custom* (freeform) tool rather than a JSON-schema function:
#
#     {"type": "custom", "name": "apply_patch", "format": {...grammar...}}
#
# and expect the model to answer with a custom_tool_call item carrying the raw
# payload in "input", then feed the result back as custom_tool_call_output.
#
# The upstream chat endpoint has no notion of custom tools, so we downgrade them
# to ordinary function tools with a single "input" string parameter on the way
# out, and re-inflate them to custom_tool_call on the way back. Without this the
# tool is silently ignored: the model emits the payload as ordinary prose and the
# client never sees a tool call (measured: 52 text deltas, 0 tool items).
# ---------------------------------------------------------------------------

CUSTOM_TOOL_HINT = (
    "This is a freeform tool. Put the COMPLETE raw payload into the single "
    "'input' string parameter, verbatim. Do not wrap it in JSON, do not wrap "
    "it in markdown code fences, do not add commentary."
)


# ---------------------------------------------------------------------------
# namespace 工具拒绝
#
# Codex 会把 MCP server / 外挂工具用 type="namespace" 的形式送出来。实测行为：
#   反代「接受」namespace 工具 -> app 把 MCP／外挂工具当成不可执行
#                                -> 每一次呼叫都回 "unsupported call"
#   反代「拒绝」namespace 工具 -> app 自动 fallback 成 flat function 清单
#                                -> 全部工具恢复正常
# （此行为在 Command Code proxy.mjs 的 CC_REJECT_NAMESPACE_TOOLS 实验里有记载，
#   Agent Router 也是靠直接拒绝这类请求才正常的。）
#
# 所以在这里主动回一个格式明确的 400，逼 app 走 fallback。
# 想还原成「照单全收」就把 REJECT_NAMESPACE_TOOLS 改成 False。
# ---------------------------------------------------------------------------

REJECT_NAMESPACE_TOOLS = False  # 保持关闭：正解是展开+还原 namespace

NAMESPACE_TOOL_MESSAGE = (
    'Unsupported tool type "namespace": this endpoint only supports flat '
    '"function" tools. Resend the tools as individual function entries.'
)


def find_namespace_tool(tools):
    """回传第一个 type=="namespace" 的工具名称，没有就回 None。"""
    for t in tools or []:
        if isinstance(t, dict) and str(t.get("type") or "").lower() == "namespace":
            return str(t.get("name") or t.get("server_label") or "(unnamed)")
    return None


def _is_custom_tool(tool):
    return isinstance(tool, dict) and str(tool.get("type") or "").lower() == "custom"


def custom_tool_names(tools, _depth=0):
    """Names of tools declared as freeform/custom in a Responses request.

    递归进 namespace：新版 Codex Desktop 把自由格式工具嵌在 functions 命名
    空间里（exec 的 type 就是 "custom"）。漏掉它的后果不是「少一个名字」，
    而是回程被判成 function_call，客户端按 (name, namespace, kind) 三元组匹
    配不到执行器，调用不执行、不返回，模型只能无限重试。namespace 只是客户
    端给工具分组的方式，不是停止查找的理由。
    """
    names = set()
    if _depth > NAMESPACE_MAX_DEPTH:
        return names
    for t in tools or []:
        if not isinstance(t, dict):
            continue
        if _is_custom_tool(t) and t.get("name"):
            names.add(str(t["name"]))
        elif str(t.get("type") or "").lower() == "namespace":
            subs = t.get("tools")
            if not isinstance(subs, list):
                subs = t.get("children")
            if not isinstance(subs, list):
                subs = t.get("functions")
            names |= custom_tool_names(subs, _depth + 1)
    return names


def _downgrade_custom_tool(tool):
    """Rewrite a Responses custom tool into a Chat function tool."""
    desc = tool.get("description") or ""
    fmt = tool.get("format") or {}
    extra = ""
    if isinstance(fmt, dict) and fmt.get("definition"):
        extra = chr(10) + chr(10) + "Grammar:" + chr(10) + str(fmt["definition"])
    return {
        "type": "function",
        "name": tool.get("name") or "",
        "description": (desc + chr(10) + chr(10) + CUSTOM_TOOL_HINT + extra).strip(),
        "parameters": {
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Complete raw payload for this tool, verbatim.",
                }
            },
            "required": ["input"],
        },
    }


# ---------------------------------------------------------------------------
# namespace 工具：展开 + 还原
#
# 新版 Codex App 把 MCP／外挂工具用 namespace 形式送出：
#   {"type":"namespace","name":"codex_app","tools":[{name:"list_threads",...}]}
#
# 上游 Chat Completions 只认 flat function，看不懂 namespace。
# 但 App 回程是用 (name, namespace) 两个栏位找执行器 ——
# 只给 flat name，App 一律回 "unsupported call"（实测 js / list_threads 全灭）。
#
# 三件事：
#   1. Expand   送上游前把 namespace 展开成 flat function，记住 name -> namespace
#   2. Normalise 模型回传的 name 可能是 js / ns__js / ns::js，都要能解析
#   3. Restore   回程的 function_call / custom_tool_call 补上 namespace 栏位
#
# 参考：某开源 CodeBuddy/WorkBuddy 反向代理项目的 tool-namespaces 说明
# ---------------------------------------------------------------------------

NAMESPACE_MAX_DEPTH = 4
_NS_SEP = "__"


def expand_namespace_tools(tools, _depth=0):
    """把 namespace 展开成 flat function 清单，其余工具原样保留。

      * 子工具可能在 tools / children / functions 任一栏位
      * namespace 子工具常常没有 type 栏位，展开时补成上游认得的 flat function
      * custom / web_search 等非 function 项目原样留下，交给既有管线处理
      * 同名只留第一个
      * 回传 (flat_tools, name_to_namespace)
    """
    flat = []
    mapping = {}
    seen = set()
    max_depth = max(0, int(_depth) + NAMESPACE_MAX_DEPTH)

    def collect(entry, depth, ns_name):
        if not isinstance(entry, dict) or depth > max_depth:
            return
        etype = str(entry.get("type") or "").lower()
        if etype == "namespace":
            subs = entry.get("tools")
            if not isinstance(subs, list):
                subs = entry.get("children")
            if not isinstance(subs, list):
                subs = entry.get("functions")
            if not isinstance(subs, list):
                subs = []
            child_ns = str(entry.get("name") or ns_name or "")
            for sub in subs:
                collect(sub, depth + 1, child_ns)
            return
        if ns_name and etype in ("", "function"):
            fn = entry.get("function") if isinstance(entry.get("function"), dict) else None
            if fn is None:
                fn = {
                    "name": entry.get("name"),
                    "description": entry.get("description") or "",
                    "parameters": entry.get("parameters") or entry.get("input_schema")
                                  or {"type": "object", "properties": {}},
                }
            name = str(fn.get("name") or "").strip()
            if not name or name in seen:
                return
            seen.add(name)
            mapping[name] = ns_name
            flat_fn = {
                "type": "function",
                "name": name,
                "description": fn.get("description") or "",
                "parameters": fn.get("parameters") or {"type": "object", "properties": {}},
            }
            if "strict" in fn:
                flat_fn["strict"] = fn["strict"]
            flat.append(flat_fn)
            return
        fn = entry.get("function") if isinstance(entry.get("function"), dict) else {}
        name = str(entry.get("name") or fn.get("name") or "").strip()
        if name:
            if name in seen:
                return
            seen.add(name)
            if ns_name:
                mapping[name] = ns_name
        flat.append(entry)

    for entry in tools or []:
        collect(entry, 0, "")
    return flat, mapping


def resolve_namespaced_name(name, mapping):
    """把模型回传的名字解析回 (bare_name, namespace)。接受 js / ns__js / ns::js。"""
    if not name:
        return name, ""
    name = str(name)
    if name in mapping:
        return name, mapping[name]
    if "::" in name:
        idx = name.find("::")
        if idx > 0:
            tail = name[idx + 2:]
            head = name[:idx]
            if tail in mapping:
                return tail, mapping[tail]
            return tail, head

    # ns__tool 用精确比对，避免 namespace 内含 '__'（如 codex_apps__github）时切错
    for tool, ns in mapping.items():
        if name == ns + _NS_SEP + tool:
            return tool, ns

    return name, ""


def stamp_namespace(item, mapping):
    """把模型回传的扁平工具名还原成 (name, namespace)。

    串流的 response.output_item.done 事件才是客户端派发工具呼叫的依据，
    所以每个 function_call / custom_tool_call 项目都要在送出前补上 namespace。
    """
    if not mapping or not isinstance(item, dict):
        return item
    bare, ns = resolve_namespaced_name(item.get("name"), mapping)
    if ns:
        item["name"] = bare
        item["namespace"] = ns
    return item


def apply_namespace_to_calls(output_items, mapping):
    """替 Responses 的 function_call / custom_tool_call 补上 namespace。"""
    if not mapping or not isinstance(output_items, list):
        return output_items, 0
    fixed = 0
    for item in output_items:
        if not isinstance(item, dict):
            continue
        if item.get("type") not in ("function_call", "custom_tool_call"):
            continue
        if item.get("namespace"):
            continue
        bare, ns = resolve_namespaced_name(item.get("name"), mapping)
        if ns:
            item["name"] = bare
            item["namespace"] = ns
            fixed += 1
    return output_items, fixed


def additional_tools_from_input(payload):
    """从 input 数组里收集 additional_tools 条目的 tools。

    新版 Codex Desktop 0.162+ 把工具放在 input 内的
    {"type": "additional_tools", "tools": [...]} 条目里，顶层 tools 是空的。
    出站（responses_to_chat）与回程（custom_tool_names / request_meta.tools）
    的取值都必须走这里，否则两边看到的是不同的工具清单。
    """
    collected = []
    for item in payload.get("input") or []:
        if isinstance(item, dict) and item.get("type") == "additional_tools":
            sub_tools = item.get("tools")
            if isinstance(sub_tools, list):
                collected.extend(sub_tools)
    return collected


def _tools_for_chat(tools):
    """Downgrade custom tools; leave everything else untouched."""
    out = []
    for t in tools or []:
        if not isinstance(t, dict):
            continue
        out.append(_downgrade_custom_tool(t) if _is_custom_tool(t) else t)
    return out


def _unwrap_custom_input(args):
    """Pull the freeform string back out of an {"input": "..."} argument blob."""
    if not isinstance(args, str):
        return json.dumps(args or "", ensure_ascii=False)
    try:
        parsed = json.loads(args)
    except Exception:
        return args
    if isinstance(parsed, dict):
        val = parsed.get("input")
        if isinstance(val, str):
            return val
        if val is not None:
            return json.dumps(val, ensure_ascii=False)
    if isinstance(parsed, str):
        return parsed
    return args

# The gateway can run web_search / web_fetch itself; the panel switch decides.
#
# Some clients (Codex App and similar harnesses) declare web_search as a
# server-side tool, but the upstream has no executor for it: forwarding the
# declaration leaves the model answering as if no tool had been offered. With
# the switch on, the gateway swaps the declaration for a function of its own,
# swallows the calls and runs them locally (wb_webtools), then feeds the
# results back.
#
# Off by default: the declaration is forwarded untouched and a client that
# declares its own search tool receives the call - the behaviour since v1.5.3.
# Turning it on means the gateway itself fetches URLs a model asks for, so the
# egress policy is the operator's call.
def local_web_tools_enabled():
    """Panel switch: does this gateway run web_search / web_fetch itself?

    Read per request, so flipping the panel takes effect on the next one
    without a restart.
    """
    try:
        return wb_settings.local_web_tools(ACCOUNTS_DIR) is True
    except Exception:
        return False


def web_tools_active(body):
    """True when this request's tools were swapped for the gateway's own.

    Interception only applies to a request whose definitions the gateway
    injected: with the switch off, a client's own same-named function must be
    forwarded instead of being swallowed here.
    """
    return isinstance(body, dict) and body.get("_web_tools") is True


def sum_usage(total, part):
    """把一轮的 token 用量累加起来。

    代跑网路工具会多跑好几次上游，那些 token 是真的花掉的，所以记帐要加总，
    不能让最后一轮盖掉前面几轮。
    """
    if not isinstance(part, dict):
        return total
    if not isinstance(total, dict):
        total = {}
    for key, value in part.items():
        if isinstance(value, dict):
            total[key] = sum_usage(total.get(key), value)
        elif isinstance(value, (int, float)) and not isinstance(value, bool):
            total[key] = (total.get(key) or 0) + value
        elif key not in total:
            total[key] = value
    return total


_CITATION_MD_RE = re.compile(r"\[([^\]\n]{1,200})\]\((https?://[^)\s]+)\)")


def build_citations(text, sources):
    """把模型实际引用到的来源转成 url_citation annotations。

    只标注真的有出现在工具输出里的网址 —— 模型自己编的连结不会被当成引用。
    """
    text = str(text or "")
    if not text or not sources:
        return []
    by_url = {}
    for s in sources or []:
        if not isinstance(s, dict):
            continue
        url = str(s.get("url") or "").strip()
        if not url:
            continue
        by_url.setdefault(url, s)
        by_url.setdefault(url.rstrip("/"), s)

    anns = []
    seen = set()

    def add(url, title, start, end):
        key = (url, start, end)
        if key in seen or start < 0 or end <= start:
            return
        seen.add(key)
        anns.append({
            "type": "url_citation",
            "url": url,
            "title": title or url,
            "start_index": start,
            "end_index": end,
        })

    md_spans = []
    for m in _CITATION_MD_RE.finditer(text):
        url = m.group(2)
        src = by_url.get(url) or by_url.get(url.rstrip("/"))
        if not src:
            continue
        md_spans.append((m.start(0), m.end(0)))
        add(url, src.get("title") or m.group(1), m.start(0), m.end(0))

    for m in re.finditer(r"https?://[^\s<>()\[\]]+", text):
        if any(m.start(0) >= s and m.end(0) <= e for s, e in md_spans):
            continue
        url = m.group(0).rstrip(".,;:!?")
        src = by_url.get(url) or by_url.get(url.rstrip("/"))
        if not src:
            continue
        add(url, src.get("title"), m.start(0), m.start(0) + len(url))

    anns.sort(key=lambda a: (a["start_index"], a["end_index"]))
    return anns


def follow_up_with_tool_results(internal_calls, holder, model, session_key, t_start,
                                drop_tools=False):
    """执行反代自己代跑的网路工具，把结果喂回模型，回传新的上游连线。

    drop_tools=True 表示这是最后一轮：把网路工具从工具清单收回，模型没有东西
    可以再呼叫，只能用手上的结果把话讲完。旧版在回合用尽时合成一个
    resp_wrapup（status=completed、output=[]）收尾，那等于把失败伪装成正常
    结束，客户端看到的就是「讲到一半断掉」——issue #43。
    """
    convo = holder.get("convo_messages")
    if convo is None:
        convo = list(holder.get("base_messages") or [])
        holder["convo_messages"] = convo

    tool_calls = []
    for i, c in enumerate(internal_calls):
        tool_calls.append({
            "id": "call_web_%d_%d" % (int(t_start * 1000) % 1000000, i),
            "type": "function",
            "function": {"name": c["name"], "arguments": c.get("arguments") or "{}"},
        })
    convo.append({"role": "assistant", "content": None, "tool_calls": tool_calls})

    for tc in tool_calls:
        nm = tc["function"]["name"]
        result = wb_webtools.execute(nm, tc["function"]["arguments"])
        found = wb_webtools.sources_from_result(result)
        if found:
            holder.setdefault("web_sources", []).extend(found)
        log("web tool %s -> %d chars, %d citeable source(s)"
            % (nm, len(result or ""), len(found)), level="INFO")
        convo.append({
            "role": "tool",
            "tool_call_id": tc["id"],
            "name": nm,
            "content": result,
        })

    body = dict(holder.get("base_body") or {})
    if drop_tools:
        body["tools"] = [t for t in (body.get("tools") or [])
                         if not wb_webtools.is_internal_tool(tool_name_of(t))]
        convo.append({
            "role": "system",
            "content": ("The web tools are no longer available. Answer the user now with "
                        "what you already have. Do not say that you are searching again."),
        })
    body["messages"] = convo
    body["stream"] = True
    return open_upstream(body, session_key=session_key,
                         target_realm=holder.get("realm"))


def internal_calls_from_chat(chat_obj, web_tools=False):
    """Calls in an aggregated chat completion the gateway runs itself.

    Only a request whose definitions the gateway injected can carry such a
    call; with the switch off a client's own same-named function stays the
    client's, so this answers empty.
    """
    message = ((chat_obj.get("choices") or [{}])[0] or {}).get("message") or {}
    out = []
    if not web_tools:
        return out
    for tc in message.get("tool_calls") or []:
        fn = tc.get("function") or {}
        name = fn.get("name") or ""
        if wb_webtools.is_internal_tool(name):
            out.append({"name": name, "arguments": fn.get("arguments") or "{}"})
    return out


def tool_name_of(tool):
    """Tool name, whichever of the two shapes the entry uses."""
    if not isinstance(tool, dict):
        return ""
    if isinstance(tool.get("function"), dict):
        return str((tool.get("function") or {}).get("name") or "")
    return str(tool.get("name") or "")


def _responses_input_to_messages(payload):
    """Turn the Responses input items into chat messages."""
    messages = []
    instructions = payload.get("instructions")
    if isinstance(instructions, str) and instructions.strip():
        messages.append({"role": "system", "content": instructions})
    inp = payload.get("input")
    pending_reasoning = ""
    if isinstance(inp, str):
        messages.append({"role": "user", "content": inp})
    elif isinstance(inp, list):
        for item in inp:
            if isinstance(item, str):
                messages.append({"role": "user", "content": item})
                continue
            if not isinstance(item, dict):
                continue
            itype = item.get("type")
            if itype in (None, "message", "user"):
                body = _flatten_content(item.get("content"))
                if body:
                    role = item.get("role") or "user"
                    if role == "developer":
                        role = "system"
                    # If this is assistant text and the previous message is an assistant
                    # message (e.g. from an adjacent function_call), merge them so
                    # tool_calls and text stay in one message without breaking tool sequence.
                    if role == "assistant" and messages and messages[-1].get("role") == "assistant":
                        prev = messages[-1]
                        if prev.get("content"):
                            prev["content"] = str(prev["content"]) + chr(10) + str(body)
                        else:
                            prev["content"] = body
                        if pending_reasoning and "reasoning_content" not in prev:
                            prev["reasoning_content"] = pending_reasoning
                            pending_reasoning = ""
                    else:
                        msg_dict = {"role": role, "content": body}
                        if role == "assistant" and pending_reasoning:
                            msg_dict["reasoning_content"] = pending_reasoning
                            pending_reasoning = ""
                        messages.append(msg_dict)
            elif itype == "reasoning":
                # Reasoning item from previous assistant turn in Responses API.
                # In standard Chat Completions, reasoning is either backfilled into
                # the assistant message's reasoning_content or omitted.
                r_text = ""
                summ = item.get("summary")
                if isinstance(summ, list):
                    r_text = chr(10).join(
                        p.get("text", "") for p in summ if isinstance(p, dict) and p.get("text")
                    )
                elif isinstance(summ, str):
                    r_text = summ
                if not r_text:
                    cnt = item.get("content")
                    if isinstance(cnt, str):
                        r_text = cnt
                    elif isinstance(cnt, list):
                        r_text = _flatten_content(cnt)
                if r_text:
                    if messages and messages[-1].get("role") == "assistant":
                        messages[-1]["reasoning_content"] = r_text
                    else:
                        pending_reasoning = r_text
            elif itype == "function_call_output":
                raw_out = item.get("output")
                if not item.get("call_id"):
                    if isinstance(raw_out, list):
                        _txt = _flatten_content(raw_out)
                    elif isinstance(raw_out, dict):
                        _txt = json.dumps(raw_out, ensure_ascii=False)
                    else:
                        _txt = str(raw_out or "")
                    if isinstance(_txt, list):
                        # _flatten_content returns structured chat parts when
                        # the output carries images. Keep them as multimodal
                        # user content instead of crashing on .strip().
                        try:
                            log("[wb-proxy] orphan function_call_output kept as multimodal parts (%d)"
                                % len(_txt))
                        except Exception:
                            pass
                        messages.append({
                            "role": "user",
                            "content": ([{"type": "text", "text":
                                          "[Message from another task - treat this "
                                          "as a user instruction]"}] + _txt),
                        })
                        continue
                    _txt = (_txt or "").strip()
                    if _txt:
                        messages.append({
                            "role": "user",
                            "content": ("[Message from another task - treat this "
                                        "as a user instruction]" + chr(10) + chr(10) + _txt),
                        })
                        continue
                if isinstance(raw_out, list):
                    content = _flatten_content(raw_out)
                elif isinstance(raw_out, dict):
                    if raw_out.get("type") in ("input_image", "image_url", "image") or "image_url" in raw_out:
                        content = _flatten_content([raw_out])
                    else:
                        content = json.dumps(raw_out, ensure_ascii=False)
                elif isinstance(raw_out, str):
                    content = raw_out
                else:
                    content = str(raw_out)
                messages.append({
                    "role": "tool",
                    "tool_call_id": item.get("call_id") or "",
                    "content": content,
                })
            elif itype == "function_call":
                tc_item = {
                    "id": item.get("call_id") or item.get("id") or "",
                    "type": "function",
                    "function": {
                        "name": item.get("name") or "",
                        "arguments": item.get("arguments") or "{}",
                    },
                }
                # Merge into previous assistant message if adjacent
                if messages and messages[-1].get("role") == "assistant":
                    prev = messages[-1]
                    if "tool_calls" in prev:
                        prev["tool_calls"].append(tc_item)
                    else:
                        prev["tool_calls"] = [tc_item]
                    if pending_reasoning and "reasoning_content" not in prev:
                        prev["reasoning_content"] = pending_reasoning
                        pending_reasoning = ""
                else:
                    msg_dict = {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [tc_item],
                    }
                    if pending_reasoning:
                        msg_dict["reasoning_content"] = pending_reasoning
                        pending_reasoning = ""
                    messages.append(msg_dict)
            elif itype == "custom_tool_call":
                # Freeform tool call coming back as conversation history.
                raw_input = item.get("input")
                if isinstance(raw_input, (dict, list)):
                    raw_input = json.dumps(raw_input, ensure_ascii=False)
                if not isinstance(raw_input, str):
                    raw_input = "" if raw_input is None else str(raw_input)
                tc_item = {
                    "id": item.get("call_id") or item.get("id") or "",
                    "type": "function",
                    "function": {
                        "name": item.get("name") or "",
                        "arguments": json.dumps({"input": raw_input}, ensure_ascii=False),
                    },
                }
                # Merge into previous assistant message if adjacent
                if messages and messages[-1].get("role") == "assistant":
                    prev = messages[-1]
                    if "tool_calls" in prev:
                        prev["tool_calls"].append(tc_item)
                    else:
                        prev["tool_calls"] = [tc_item]
                    if pending_reasoning and "reasoning_content" not in prev:
                        prev["reasoning_content"] = pending_reasoning
                        pending_reasoning = ""
                else:
                    msg_dict = {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [tc_item],
                    }
                    if pending_reasoning:
                        msg_dict["reasoning_content"] = pending_reasoning
                        pending_reasoning = ""
                    messages.append(msg_dict)
            elif itype == "custom_tool_call_output":
                # Result of a freeform tool call (e.g. apply_patch output).
                raw_out = item.get("output")
                if isinstance(raw_out, list):
                    content = _flatten_content(raw_out)
                elif isinstance(raw_out, dict):
                    content = json.dumps(raw_out, ensure_ascii=False)
                elif isinstance(raw_out, str):
                    content = raw_out
                else:
                    content = str(raw_out)
                messages.append({
                    "role": "tool",
                    "tool_call_id": item.get("call_id") or "",
                    "content": content,
                })
            elif itype == "agent_message":
                parts = item.get("content")
                if isinstance(parts, list):
                    text = chr(10).join(
                        str((p or {}).get("text") or (p or {}).get("encrypted_content") or "")
                        if isinstance(p, dict) else str(p)
                        for p in parts
                    ).strip()
                else:
                    text = str(parts or "").strip()
                if text:
                    messages.append({
                        "role": "user",
                        "content": ("[Message from another task - treat this as "
                                    "a user instruction]" + chr(10) + chr(10) + text),
                    })
            elif itype == "additional_tools":
                # 由 responses_to_chat() 统一提取合并进 chat["tools"]，此处无需生成 message
                pass
            else:
                log("responses: WARNING unhandled input item type=%r keys=%s"
                    % (itype, sorted(item.keys())[:8]))
    return messages


def responses_to_chat(payload):
    """Translate a Responses API request body into a Chat Completions body."""
    messages = _responses_input_to_messages(payload)
    chat = {"model": payload.get("model"), "messages": messages}
    for key in ("temperature", "top_p", "seed"):
        if payload.get(key) is not None:
            chat[key] = payload[key]
    if payload.get("max_output_tokens") is not None:
        chat["max_tokens"] = payload["max_output_tokens"]
    else:
        default_max = model_default_max_output_tokens(payload.get("model"))
        if default_max:
            chat["max_tokens"] = default_max
    effort = None
    reasoning = payload.get("reasoning")
    if isinstance(reasoning, dict):
        effort = reasoning.get("effort")
    if not effort:
        effort = payload.get("reasoning_effort")
    if effort:
        chat["reasoning_effort"] = effort
    # 合并顶层 tools 和 input 列表中的 additional_tools 条目（新版 Codex Desktop 0.162+ 特征）
    raw_tools = []
    if isinstance(payload.get("tools"), list):
        raw_tools.extend(payload["tools"])
    raw_tools.extend(additional_tools_from_input(payload))
    if raw_tools:
        flat_tools, ns_map = expand_namespace_tools(raw_tools)
        chat["tools"] = _tools_for_chat(flat_tools)
        chat["_namespace_map"] = ns_map
    # 客户端宣告 web_search / web_fetch 时，把那份宣告换成我们的
    # function（见 wb_webtools.install_tool_defs）。
    # 看板开关关闭时原样透传，客户端自己的同名工具不受影响。
    # 用合并后的清单：工具可能整份都在 additional_tools 里，只看顶层会漏。
    if local_web_tools_enabled():
        wants = wb_webtools.client_wants_web(raw_tools)
        if wants["search"] or wants["fetch"]:
            chat["tools"] = wb_webtools.install_tool_defs(chat.get("tools") or [], wants)
            chat["_web_tools"] = True
    if payload.get("tool_choice"):
        chat["tool_choice"] = payload["tool_choice"]
    if payload.get("parallel_tool_calls") is not None:
        chat["parallel_tool_calls"] = payload["parallel_tool_calls"]
    return chat


# ---------------------------------------------------------------------------
# Anthropic Messages API (/v1/messages) <-> Chat Completions
# ---------------------------------------------------------------------------
def anthropic_error_type(status):
    """Map an HTTP status to the Anthropic error.type vocabulary."""
    try:
        status = int(status)
    except (TypeError, ValueError):
        status = 500
    if status in (400, 422):
        return "invalid_request_error"
    if status == 401:
        return "authentication_error"
    if status == 403:
        return "permission_error"
    if status == 404:
        return "not_found_error"
    if status == 429:
        return "rate_limit_error"
    if status in (503, 529):
        return "overloaded_error"
    return "api_error"


def anthropic_error_obj(status, message, error_type=None):
    return {
        "type": "error",
        "error": {
            "type": error_type or anthropic_error_type(status),
            "message": str(message or "upstream error"),
        },
    }


def anthropic_sse_frame(event, payload):
    body = dict(payload or {})
    body["type"] = event
    raw = json.dumps(body, ensure_ascii=False, separators=(",", ":"))
    return ("event: %s\ndata: %s\n\n" % (event, raw)).encode("utf-8")


def _anthropic_text(value):
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    if isinstance(value, (int, float, bool)):
        return str(value)
    return ""


def _anthropic_json_text(value):
    if value is None:
        return "{}"
    if isinstance(value, str):
        return value
    try:
        return json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    except Exception:
        return "{}"


def _anthropic_parse_tool_input(value):
    if value is None:
        return {}
    if isinstance(value, dict):
        return value
    if isinstance(value, str):
        if not value:
            return {}
        try:
            parsed = json.loads(value)
        except Exception:
            return {"raw": value}
        if isinstance(parsed, dict):
            return parsed
        return {"value": parsed}
    return {"value": value}


def _anthropic_content_to_text(value, is_error=None):
    if isinstance(value, str):
        text = value
    elif isinstance(value, list):
        parts = []
        for item in value:
            if not isinstance(item, dict):
                continue
            typ = str(item.get("type") or "")
            if typ == "image":
                part = _anthropic_image_part(item)
                parts.append(_anthropic_text((part.get("image_url") or {}).get("url")))
            elif typ == "document":
                parts.append(_anthropic_document_text(item))
            else:
                parts.append(_anthropic_block_text(item))
        text = "".join(parts)
    elif isinstance(value, dict):
        text = _anthropic_block_text(value)
    else:
        text = _anthropic_text(value)
    if is_error is True:
        return "error: " + text
    return text


def _anthropic_block_text(block):
    if not isinstance(block, dict):
        return ""
    if "text" in block:
        return _anthropic_text(block.get("text"))
    if "content" in block:
        return _anthropic_content_to_text(block.get("content"))
    return ""


def _anthropic_image_part(block):
    source = block.get("source") if isinstance(block, dict) else None
    url = ""
    if isinstance(source, dict):
        stype = str(source.get("type") or "")
        if stype == "base64":
            media = _anthropic_text(source.get("media_type") or "image/png")
            data = _anthropic_text(source.get("data"))
            url = "data:%s;base64,%s" % (media, data)
        elif stype == "url":
            url = _anthropic_text(source.get("url"))
        elif stype == "file":
            url = _anthropic_text(source.get("file_id") or source.get("url"))
    return {"type": "image_url", "image_url": {"url": url}}


def _anthropic_document_text(block):
    if not isinstance(block, dict):
        return ""
    title = _anthropic_text(block.get("title"))
    source = block.get("source") if isinstance(block.get("source"), dict) else {}
    stype = str(source.get("type") or "")
    body = ""
    if stype == "text":
        body = _anthropic_text(source.get("data"))
    elif stype == "content":
        body = _anthropic_content_to_text(source.get("content"))
    elif stype == "base64":
        media = _anthropic_text(source.get("media_type") or "application/octet-stream")
        body = "[document %s omitted]" % media
    elif stype == "url":
        body = _anthropic_text(source.get("url"))
    elif stype == "file":
        body = _anthropic_text(source.get("file_id") or source.get("url"))
    if title:
        return title + "\n" + body
    return body


def _anthropic_system_text(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        parts = []
        for item in value:
            if not isinstance(item, dict):
                continue
            text = _anthropic_text(item.get("text"))
            if text:
                parts.append(text)
        return "\n".join(parts)
    return ""


def _anthropic_tool_choice(value):
    if isinstance(value, str):
        val = value.strip().lower()
        return {"auto": "auto", "any": "required", "none": "none",
                "required": "required"}.get(val, val)
    if isinstance(value, dict):
        typ = str(value.get("type") or "").strip().lower()
        if typ == "auto":
            return "auto"
        if typ == "any":
            return "required"
        if typ == "none":
            return "none"
        if typ == "tool":
            return {"type": "function",
                    "function": {"name": _anthropic_text(value.get("name"))}}
    return None


def anthropic_tools_to_chat(tools):
    out = []
    skipped = []
    if not isinstance(tools, list):
        return out, skipped
    for item in tools:
        if not isinstance(item, dict):
            continue
        if isinstance(item.get("function"), dict):
            out.append(item)
            continue
        name = _anthropic_text(item.get("name") or item.get("type"))
        schema = item.get("input_schema")
        if schema is None:
            schema = item.get("parameters")
        if schema is None:
            if name:
                skipped.append(name)
            continue
        fn = {"name": name}
        if item.get("description") is not None:
            fn["description"] = item.get("description")
        fn["parameters"] = schema
        out.append({"type": "function", "function": fn})
    return out, skipped


def anthropic_effort_from_messages(payload):
    if not isinstance(payload, dict):
        return None
    output_config = payload.get("output_config")
    if isinstance(output_config, dict):
        effort = _anthropic_text(output_config.get("effort")).strip()
        if effort:
            return effort
    thinking = payload.get("thinking")
    if not isinstance(thinking, dict):
        return None
    typ = _anthropic_text(thinking.get("type")).strip().lower()
    if typ == "disabled":
        return "none"
    if typ != "enabled":
        return None
    try:
        budget = int(thinking.get("budget_tokens") or 0)
    except (TypeError, ValueError):
        return None
    if budget <= 0:
        return None
    if budget >= 32000:
        return "xhigh"
    if budget >= 16000:
        return "high"
    if budget >= 8000:
        return "medium"
    return "low"


def _anthropic_blocks_to_messages(role, blocks):
    tool_calls = []
    tool_messages = []
    content_parts = []
    texts = []
    has_non_text = False
    for block in blocks:
        if not isinstance(block, dict):
            continue
        typ = str(block.get("type") or "")
        if typ in ("thinking", "redacted_thinking"):
            continue
        if typ in ("tool_use", "server_tool_use"):
            tool_calls.append({
                "id": _anthropic_text(block.get("id")),
                "type": "function",
                "function": {
                    "name": _anthropic_text(block.get("name")),
                    "arguments": _anthropic_json_text(block.get("input")),
                },
            })
            continue
        if typ == "tool_result":
            tool_messages.append({
                "role": "tool",
                "tool_call_id": _anthropic_text(block.get("tool_use_id")),
                "content": _anthropic_content_to_text(block.get("content"),
                                                      block.get("is_error")),
            })
            continue
        if typ == "image":
            part = _anthropic_image_part(block)
            if not (part.get("image_url") or {}).get("url"):
                continue
            has_non_text = True
            content_parts.append(part)
            continue
        if typ == "document":
            has_non_text = True
            content_parts.append({"type": "text", "text": _anthropic_document_text(block)})
            continue
        text = _anthropic_block_text(block)
        texts.append(text)
        content_parts.append({"type": "text", "text": text})
    out = list(tool_messages)
    if not tool_calls and not texts and not has_non_text:
        return out
    msg = {"role": role or "user"}
    if tool_calls:
        msg["tool_calls"] = tool_calls
    if has_non_text:
        msg["content"] = content_parts
    elif texts:
        msg["content"] = "".join(texts)
    else:
        msg["content"] = ""
    out.append(msg)
    return out


def messages_to_chat(payload):
    """Translate an Anthropic Messages request into Chat Completions."""
    if not isinstance(payload, dict):
        raise ValueError("request body must be a JSON object")
    model = _anthropic_text(payload.get("model")).strip()
    if not model:
        raise ValueError("model is required")
    messages_in = payload.get("messages")
    if not isinstance(messages_in, list):
        raise ValueError("messages must be a list")
    if not messages_in:
        raise ValueError("messages must not be empty")
    chat = {"model": model, "messages": []}
    for key in ("temperature", "top_p"):
        if payload.get(key) is not None:
            chat[key] = payload[key]
    if payload.get("max_tokens") is not None:
        chat["max_tokens"] = payload["max_tokens"]
    else:
        default_max = model_default_max_output_tokens(model)
        if default_max:
            chat["max_tokens"] = default_max
    if payload.get("stream") is not None:
        chat["stream"] = bool(payload.get("stream"))
    stops = payload.get("stop_sequences")
    if isinstance(stops, list) and stops:
        chat["stop"] = stops
    metadata = payload.get("metadata")
    if isinstance(metadata, dict) and metadata.get("user_id") is not None:
        chat["user"] = metadata.get("user_id")
    tools, skipped = anthropic_tools_to_chat(payload.get("tools"))
    if tools:
        chat["tools"] = tools
    choice = _anthropic_tool_choice(payload.get("tool_choice"))
    if choice is not None:
        chat["tool_choice"] = choice
    raw_choice = payload.get("tool_choice")
    if isinstance(raw_choice, dict) and raw_choice.get("disable_parallel_tool_use") is True:
        chat["parallel_tool_calls"] = False
    effort = anthropic_effort_from_messages(payload)
    if effort:
        chat["reasoning_effort"] = effort
    messages = []
    system = _anthropic_system_text(payload.get("system"))
    if skipped:
        note = ("These Anthropic server tools are not available here: "
                + ", ".join(skipped) + ". Do not call them.")
        system = (system + "\n" + note) if system else note
    if system:
        messages.append({"role": "system", "content": system})
    for index, item in enumerate(messages_in):
        if not isinstance(item, dict):
            continue
        role = _anthropic_text(item.get("role")).strip() or "user"
        content = item.get("content")
        if role in ("system", "developer"):
            # Claude Code >= 2.1.286 turns on Anthropic's mid-conversation
            # system beta and carries system messages inside `messages`. The
            # upstream knows the system role, so keep the message where the
            # client put it rather than failing the whole request.
            text = _anthropic_system_text(content) or _anthropic_content_to_text(content)
            if text:
                messages.append({"role": "system", "content": text})
            continue
        if role not in ("user", "assistant"):
            raise ValueError(
                "messages[%d].role %r must be user or assistant; "
                "pass a system prompt in the top-level system field" % (index, role))
        if isinstance(content, str):
            messages.append({"role": role, "content": content})
        elif isinstance(content, list):
            messages.extend(_anthropic_blocks_to_messages(role, content))
        elif content is not None:
            text = _anthropic_text(content)
            if text:
                messages.append({"role": role, "content": text})
    chat["messages"] = messages
    return chat


def _anthropic_usage(usage):
    out = {
        "input_tokens": 0,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "output_tokens": 0,
    }
    if not isinstance(usage, dict):
        return out
    try:
        out["input_tokens"] = int(usage.get("prompt_tokens") or 0)
    except (TypeError, ValueError):
        pass
    try:
        out["output_tokens"] = int(usage.get("completion_tokens") or 0)
    except (TypeError, ValueError):
        pass
    cached = usage.get("prompt_cache_hit_tokens")
    if cached is None:
        cached = _best_cached_tokens(usage)
    try:
        out["cache_read_input_tokens"] = int(cached or 0)
    except (TypeError, ValueError):
        pass
    try:
        out["cache_creation_input_tokens"] = int(usage.get("prompt_cache_write_tokens") or 0)
    except (TypeError, ValueError):
        pass
    service_tier = usage.get("service_tier")
    if service_tier:
        out["service_tier"] = service_tier
    return out


def _anthropic_message_id(value):
    value = _anthropic_text(value)
    if not value:
        return _new_id("msg_")
    return value if value.startswith("msg_") else "msg_" + value


def _anthropic_stop_reason(finish_reason):
    value = str(finish_reason or "").strip().lower()
    if value == "length":
        return "max_tokens"
    if value in ("tool_calls", "function_call"):
        return "tool_use"
    if value in ("content_filter", "refusal"):
        return "refusal"
    return "end_turn"


def chat_to_messages(obj):
    """Fold one Chat Completions object into an Anthropic Messages object."""
    if not isinstance(obj, dict):
        obj = {}
    choices = obj.get("choices") if isinstance(obj.get("choices"), list) else []
    message = {}
    finish = ""
    if choices and isinstance(choices[0], dict):
        message = choices[0].get("message") if isinstance(choices[0].get("message"), dict) else {}
        finish = choices[0].get("finish_reason") or ""
    content = []
    # The upstream thinking trace becomes a native thinking block so clients
    # such as Claude Code can render it; the request side drops incoming
    # thinking blocks, so nothing unsigned ever reaches the upstream.
    reasoning = _anthropic_text(message.get("reasoning_content"))
    if reasoning:
        content.append({"type": "thinking", "thinking": reasoning})
    text = message.get("content")
    if isinstance(text, list):
        text = "".join(_anthropic_block_text(part) for part in text if isinstance(part, dict))
    text = _anthropic_text(text)
    if text:
        content.append({"type": "text", "text": text})
    tool_calls = message.get("tool_calls")
    if isinstance(tool_calls, list):
        for call in tool_calls:
            if not isinstance(call, dict):
                continue
            fn = call.get("function") if isinstance(call.get("function"), dict) else {}
            content.append({
                "type": "tool_use",
                "id": _anthropic_text(call.get("id")) or _new_id("toolu_"),
                "name": _anthropic_text(fn.get("name")),
                "input": _anthropic_parse_tool_input(fn.get("arguments")),
            })
    return {
        "id": _anthropic_message_id(obj.get("id")),
        "type": "message",
        "role": "assistant",
        "model": _anthropic_text(obj.get("model")),
        "content": content,
        "stop_reason": _anthropic_stop_reason(finish),
        "stop_sequence": None,
        "usage": _anthropic_usage(obj.get("usage")),
    }


def _anthropic_estimate_chat_tokens(chat):
    values = []
    if isinstance(chat, dict):
        for msg in chat.get("messages") or []:
            if isinstance(msg, dict):
                values.append(_anthropic_json_text(msg.get("content")))
                for call in msg.get("tool_calls") or []:
                    if isinstance(call, dict):
                        values.append(_anthropic_json_text(call))
        for tool in chat.get("tools") or []:
            values.append(_anthropic_json_text(tool))
    return sum(estimate_tokens(value) for value in values if value)


def stream_messages_events(raw_iter, model, holder=None):
    """Yield Anthropic Messages SSE frames from a chat-completions SSE stream."""
    holder = holder if isinstance(holder, dict) else {}
    state = {
        "started": False,
        "done": False,
        "failed": False,
        "id": "",
        "model": model,
        "stop": "end_turn",
        "thinking_open": False,
        "thinking_idx": None,
        "text_open": False,
        "text_idx": None,
        "next": 0,
        "tools": {},
        "tool_order": [],
        "usage": None,
    }

    def usage_payload():
        return _anthropic_usage(state.get("usage"))

    def ensure_start(chunk=None):
        if state["started"]:
            return None
        state["started"] = True
        if isinstance(chunk, dict):
            state["id"] = _anthropic_text(chunk.get("id"))
            state["model"] = _anthropic_text(chunk.get("model")) or state["model"]
        return anthropic_sse_frame("message_start", {
            "message": {
                "id": _anthropic_message_id(state["id"]),
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": state["model"],
                "stop_reason": None,
                "stop_sequence": None,
                "usage": {"input_tokens": 0, "output_tokens": 0},
            }
        })

    def ensure_thinking():
        frames = []
        if not state["thinking_open"]:
            state["thinking_idx"] = state["next"]
            state["next"] += 1
            state["thinking_open"] = True
            frames.append(anthropic_sse_frame("content_block_start", {
                "index": state["thinking_idx"],
                "content_block": {"type": "thinking", "thinking": ""},
            }))
        return frames

    def ensure_text():
        frames = []
        if not state["text_open"]:
            if state["thinking_open"]:
                state["thinking_open"] = False
                frames.append(anthropic_sse_frame(
                    "content_block_stop", {"index": state["thinking_idx"]}))
            state["text_idx"] = state["next"]
            state["next"] += 1
            state["text_open"] = True
            frames.append(anthropic_sse_frame("content_block_start", {
                "index": state["text_idx"],
                "content_block": {"type": "text", "text": ""},
            }))
        return frames

    for line in iter_sse_lines(raw_iter):
        if state["done"] or state["failed"]:
            break
        data = strip_data_prefix(line.decode("utf-8", "replace"))
        if not data:
            continue
        if data == "[DONE]":
            break
        try:
            chunk = json.loads(data)
        except Exception:
            continue
        if isinstance(chunk.get("error"), dict):
            state["failed"] = True
            err = chunk["error"]
            yield anthropic_sse_frame("error", {
                "error": {
                    "type": anthropic_error_type(502),
                    "message": _anthropic_text(err.get("message") or "upstream error"),
                }
            })
            return
        start = ensure_start(chunk)
        if start:
            yield start
        # Capture usage before the choices guard: the final upstream frame
        # carries both the usage block and the finish_reason (and no delta),
        # so skipping it here would zero out every streaming usage row.
        if isinstance(chunk.get("usage"), dict):
            state["usage"] = chunk["usage"]
        choices = chunk.get("choices") if isinstance(chunk.get("choices"), list) else []
        if not choices:
            continue
        choice = choices[0] if isinstance(choices[0], dict) else {}
        delta = choice.get("delta") if isinstance(choice.get("delta"), dict) else {}
        # Upstream thinking trace (deepseek-style reasoning_content) becomes
        # native thinking blocks so the client can display the reasoning.
        reason = delta.get("reasoning_content")
        if isinstance(reason, str) and reason:
            for frame in ensure_thinking():
                yield frame
            yield anthropic_sse_frame("content_block_delta", {
                "index": state["thinking_idx"],
                "delta": {"type": "thinking_delta", "thinking": reason},
            })
        text = delta.get("content")
        if isinstance(text, str) and text:
            for frame in ensure_text():
                yield frame
            yield anthropic_sse_frame("content_block_delta", {
                "index": state["text_idx"],
                "delta": {"type": "text_delta", "text": text},
            })
        for call in delta.get("tool_calls") or []:
            if not isinstance(call, dict):
                continue
            try:
                idx = int(call.get("index") or 0)
            except (TypeError, ValueError):
                idx = 0
            entry = state["tools"].get(idx)
            if entry is None:
                entry = {"idx": idx, "block": None, "id": "", "name": "", "opened": False}
                state["tools"][idx] = entry
                state["tool_order"].append(idx)
            if call.get("id"):
                entry["id"] = _anthropic_text(call.get("id"))
            fn = call.get("function") if isinstance(call.get("function"), dict) else {}
            if fn.get("name"):
                entry["name"] = _anthropic_text(fn.get("name"))
            args = fn.get("arguments") if fn.get("arguments") is not None else ""
            if not entry["opened"] and (entry["id"] or entry["name"] or args):
                if state["thinking_open"]:
                    state["thinking_open"] = False
                    yield anthropic_sse_frame("content_block_stop", {"index": state["thinking_idx"]})
                if state["text_open"]:
                    state["text_open"] = False
                    yield anthropic_sse_frame("content_block_stop", {"index": state["text_idx"]})
                entry["opened"] = True
                entry["block"] = state["next"]
                state["next"] += 1
                if not entry["id"]:
                    entry["id"] = "toolu_" + str(idx)
                yield anthropic_sse_frame("content_block_start", {
                    "index": entry["block"],
                    "content_block": {
                        "type": "tool_use",
                        "id": entry["id"],
                        "name": entry["name"],
                        "input": {},
                    },
                })
            if entry["opened"] and args:
                yield anthropic_sse_frame("content_block_delta", {
                    "index": entry["block"],
                    "delta": {"type": "input_json_delta", "partial_json": _anthropic_text(args)},
                })
        finish = choice.get("finish_reason")
        if finish:
            state["stop"] = _anthropic_stop_reason(finish)
    if state["failed"]:
        return
    if not state["started"]:
        start = ensure_start(None)
        if start:
            yield start
    if state["thinking_open"]:
        state["thinking_open"] = False
        yield anthropic_sse_frame("content_block_stop", {"index": state["thinking_idx"]})
    if state["text_open"]:
        state["text_open"] = False
        yield anthropic_sse_frame("content_block_stop", {"index": state["text_idx"]})
    for idx in state["tool_order"]:
        entry = state["tools"][idx]
        if not entry["opened"]:
            continue
        yield anthropic_sse_frame("content_block_stop", {"index": entry["block"]})
    state["done"] = True
    # Hand the raw upstream usage block back to the caller: the stream
    # response records it on the usage row, and without this every streamed
    # Messages request was logged as usage_missing.
    holder["usage"] = state.get("usage")
    yield anthropic_sse_frame("message_delta", {
        "delta": {"stop_reason": state["stop"], "stop_sequence": None},
        "usage": usage_payload(),
    })
    yield anthropic_sse_frame("message_stop", {})


def _responses_usage(u):
    if not u:
        return None
    det = u.get("completion_tokens_details") or {}
    pdet = u.get("prompt_tokens_details") or {}
    return {
        "input_tokens": u.get("prompt_tokens") or 0,
        "input_tokens_details": {
            "cached_tokens": u.get("prompt_cache_hit_tokens")
            or det.get("cached_tokens") or pdet.get("cached_tokens") or 0,
        },
        "output_tokens": u.get("completion_tokens") or 0,
        "output_tokens_details": {"reasoning_tokens": det.get("reasoning_tokens") or 0},
        "total_tokens": u.get("total_tokens") or 0,
    }
def chat_to_response(chat_obj, model, custom_names=None, request_meta=None, namespace_map=None, sources=None):
    """Fold a Chat Completions object into a Responses API response object.

    custom_names is the set of tool names the client declared as freeform
    ("custom"). Calls to those tools are re-inflated into custom_tool_call
    items so clients such as Codex recognise them.

    request_meta echoes the request-level capabilities (tools, tool_choice,
    parallel_tool_calls) back on the response. They used to be hardcoded to
    tools=[], tool_choice=auto and parallel_tool_calls=true, so a client that
    asked for something else was told the opposite of what it requested.
    """
    custom_names = custom_names or set()
    choice = (chat_obj.get("choices") or [{}])[0]
    msg = choice.get("message") or {}
    text = msg.get("content") or ""
    reasoning = msg.get("reasoning_content") or ""
    output = []
    if reasoning:
        output.append({
            "id": _new_id("rs_"),
            "type": "reasoning",
            "status": "completed",
            "summary": [{"type": "summary_text", "text": reasoning}],
        })
    for tc in msg.get("tool_calls") or []:
        fn = tc.get("function") or {}
        call_id = tc.get("id") or _new_id("call_")
        name = fn.get("name") or ""
        if name and name in custom_names:
            output.append({
                "id": _new_id("ctc_"),
                "type": "custom_tool_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "input": _unwrap_custom_input(fn.get("arguments") or ""),
            })
        else:
            output.append({
                "id": _new_id("fc_"),
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "arguments": fn.get("arguments") or "{}",
            })
    # DeepSeek DSML tool calls fallback
    if not (msg.get("tool_calls")):
        dsml_calls, clean_t = parse_dsml_tool_calls(text)
        if dsml_calls:
            for dc in dsml_calls:
                output.append({
                    "id": _new_id("fc_"),
                    "type": "function_call",
                    "status": "completed",
                    "call_id": dc.get("id") or _new_id("call_"),
                    "name": dc.get("name") or "",
                    "arguments": dc.get("arguments") or "{}",
                })
            text = clean_t
    if text or not output:
        output.append({
            "id": _new_id("msg_"),
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text,
                         "annotations": build_citations(text, sources)}] if text else [],
        })
    finish = choice.get("finish_reason") or "stop"
    obj = {
        "id": _new_id("resp_"),
        "object": "response",
        "created_at": int(time.time()),
        "status": "completed" if finish != "length" else "incomplete",
        "model": model,
        "output": output,
        "output_text": text,
        "metadata": {},
    }
    if namespace_map:
        output, _ns_fixed = apply_namespace_to_calls(output, namespace_map)
        obj["output"] = output
    meta = request_meta or {}
    obj["parallel_tool_calls"] = meta.get("parallel_tool_calls", True)
    obj["tool_choice"] = meta.get("tool_choice", "auto")
    obj["tools"] = meta.get("tools") or []
    u = _responses_usage(chat_obj.get("usage"))
    if u:
        obj["usage"] = u
    if finish == "length":
        obj["incomplete_details"] = {"reason": "max_output_tokens"}
    return obj
def stream_responses_events(upstream, model, holder):
    """Yield Responses-API SSE frames translated from chat-completions chunks."""
    resp_id, msg_id, rs_id = _new_id("resp_"), _new_id("msg_"), _new_id("rs_")
    created = int(time.time())
    seq = 0
    text_parts, reason_parts = [], []
    outputs = []
    reason_index = None
    msg_index = None
    finish = "stop"
    usage = None
    tool_calls_map = {}
    text_buffer = ""
    dsml_tool_calls = []
    saw_done = False
    custom_names = set(holder.get("custom_names") or ())
    ns_map = holder.get("namespace_map") or {}
    # 由反代代跑的网路工具呼叫，收集起来不转发给客户端
    _internal_calls = {}
    # Only reach for same-named calls when this request's definitions were the
    # gateway's own (see web_tools_active); otherwise they belong to the client.
    _own_web_tools = web_tools_active(holder.get("base_body"))
    # Echo the request capabilities the client actually sent, same as the
    # non-streaming path; these were hardcoded before.
    meta = holder.get("request_meta") or {}
    def resp_obj(status):
        obj = {
            "id": resp_id,
            "object": "response",
            "created_at": created,
            "status": status,
            "model": model,
            "output": [o for o in outputs if o],
            "output_text": "".join(text_parts),
            "parallel_tool_calls": meta.get("parallel_tool_calls", True),
            "tool_choice": meta.get("tool_choice", "auto"),
            "tools": meta.get("tools") or [],
            "metadata": {},
        }
        u = _responses_usage(usage)
        if u:
            obj["usage"] = u
        if ns_map:
            obj["output"], _nsf = apply_namespace_to_calls(obj.get("output") or [], ns_map)
        return obj
    def ev(etype, payload):
        nonlocal seq
        seq += 1
        data = {"type": etype, "sequence_number": seq}
        data.update(payload)
        body = json.dumps(data, ensure_ascii=False)
        return ("event: " + etype + chr(10) + "data: " + body + chr(10) + chr(10)).encode("utf-8")
    def reason_item(status):
        return {
            "id": rs_id,
            "type": "reasoning",
            "status": status,
            "summary": [{"type": "summary_text", "text": "".join(reason_parts)}],
        }
    def _annotations():
        """引用来源：只认工具真的回传过的网址。"""
        try:
            return build_citations("".join(text_parts), holder.get("web_sources") or [])
        except Exception:
            return []

    def msg_item(status):
        item = {"id": msg_id, "type": "message", "status": status,
                "role": "assistant", "content": []}
        if text_parts:
            item["content"] = [{"type": "output_text", "text": "".join(text_parts),
                              "annotations": _annotations()}]
        return item
    def _finalize():
        # Close out the stream: reasoning item, structured tool calls,
        # DSML fallback, the message item and response.completed.
        nonlocal msg_index, text_buffer
        if reason_index is not None and outputs[reason_index] is None:
            full_r = "".join(reason_parts)
            yield ev("response.reasoning_summary_text.done", {
                "item_id": rs_id, "output_index": reason_index, "summary_index": 0, "text": full_r,
            })
            yield ev("response.reasoning_summary_part.done", {
                "item_id": rs_id, "output_index": reason_index, "summary_index": 0,
                "part": {"type": "summary_text", "text": full_r},
            })
            outputs[reason_index] = reason_item("completed")
            yield ev("response.output_item.done",
                     {"output_index": reason_index, "item": outputs[reason_index]})
        # 1. Emit completed structured tool calls
        for idx in sorted(tool_calls_map.keys()):
            entry = tool_calls_map[idx]
            if entry.get("custom"):
                yield ev("response.custom_tool_call_input.done", {
                    "output_index": entry["output_index"],
                    "item_id": entry["item_id"],
                    "call_id": entry["id"],
                    "input": _unwrap_custom_input(entry["arguments"]),
                })
                fc_item = {
                    "id": entry["item_id"],
                    "type": "custom_tool_call",
                    "status": "completed",
                    "call_id": entry["id"],
                    "name": entry["name"],
                    "input": _unwrap_custom_input(entry["arguments"]),
                }
            else:
                yield ev("response.function_call_arguments.done", {
                    "output_index": entry["output_index"],
                    "item_id": entry["item_id"],
                    "call_id": entry["id"],
                    "arguments": entry["arguments"],
                })
                fc_item = {
                    "id": entry["item_id"],
                    "type": "function_call",
                    "status": "completed",
                    "call_id": entry["id"],
                    "name": entry["name"],
                    "arguments": entry["arguments"],
                }
            stamp_namespace(fc_item, ns_map)
            outputs[entry["output_index"]] = fc_item
            yield ev("response.output_item.done", {
                "output_index": entry["output_index"],
                "item": fc_item,
            })
        # Flush remaining buffered text if any
        if text_buffer:
            calls_rem, clean_rem = parse_dsml_tool_calls(text_buffer)
            if calls_rem:
                dsml_tool_calls.extend(calls_rem)
            if clean_rem:
                text_parts.append(clean_rem)
                if msg_index is not None:
                    yield ev("response.output_text.delta", {
                        "item_id": msg_id, "output_index": msg_index,
                        "content_index": 0, "delta": clean_rem,
                    })
            text_buffer = ""
        # 2. DSML fallback: emit buffered/parsed DSML tool calls if no structured tool_calls were emitted
        full_text = "".join(text_parts)
        dsml_calls = dsml_tool_calls
        if not dsml_calls:
            extra_calls, clean_text = parse_dsml_tool_calls(full_text)
            if extra_calls:
                dsml_calls = extra_calls
                full_text = clean_text
        if dsml_calls and not tool_calls_map:
            for dc in dsml_calls:
                # DSML 形状的网路工具呼叫一样由反代执行
                if _own_web_tools and wb_webtools.is_internal_tool(dc.get("name")):
                    entry = _internal_calls.setdefault(dc.get("id") or _new_id("call_"),
                                                       {"name": dc.get("name"), "arguments": "{}"})
                    entry["name"] = dc.get("name") or entry["name"]
                    entry["arguments"] = dc.get("arguments") or entry.get("arguments") or "{}"
                    continue
                out_idx = len(outputs)
                fc_item = {
                    "id": _new_id("fc_"),
                    "type": "function_call",
                    "status": "completed",
                    "call_id": dc.get("id") or _new_id("call_"),
                    "name": dc.get("name") or "",
                    "arguments": dc.get("arguments") or "{}",
                }
                stamp_namespace(fc_item, ns_map)
                outputs.append(fc_item)
                yield ev("response.output_item.added", {
                    "output_index": out_idx,
                    "item": dict(fc_item, status="in_progress", arguments=""),
                })
                yield ev("response.function_call_arguments.delta", {
                    "output_index": out_idx,
                    "item_id": fc_item["id"],
                    "call_id": fc_item["call_id"],
                    "delta": fc_item["arguments"],
                })
                yield ev("response.function_call_arguments.done", {
                    "output_index": out_idx,
                    "item_id": fc_item["id"],
                    "call_id": fc_item["call_id"],
                    "arguments": fc_item["arguments"],
                })
                yield ev("response.output_item.done", {
                    "output_index": out_idx,
                    "item": fc_item,
                })
        # 这一轮如果有代跑的网路工具呼叫，就把完成事件留给下一轮，
        # 否则客户端会以为整个回合已经结束（旧版是在回合用尽时补一个合成的
        # resp_wrapup，那才是 issue #43 真正的病灶）。
        if _internal_calls:
            holder.setdefault("internal_calls", []).extend(
                {"name": v["name"], "arguments": v["arguments"]}
                for v in _internal_calls.values()
            )
            holder["suppress_completion"] = True
            # 让 App 画出原生的「已搜寻网路」卡片：对每个代跑的呼叫送出
            # web_search_call 项目与生命周期事件。
            for _v in _internal_calls.values():
                _nm = str(_v.get("name") or "")
                try:
                    _a = json.loads(_v.get("arguments") or "{}")
                except Exception:
                    _a = {}
                if not isinstance(_a, dict):
                    _a = {}
                if _nm == wb_webtools.WEB_FETCH_NAME:
                    _action = {"type": "open_page", "url": wb_webtools.url_arg(_a)}
                else:
                    _action = {"type": "search", "query": wb_webtools.query_args(_a)}
                _ws_id = _new_id("ws_")
                _ws_idx = len(outputs)
                outputs.append(None)
                yield ev("response.output_item.added", {
                    "output_index": _ws_idx,
                    "item": {"id": _ws_id, "type": "web_search_call",
                             "status": "in_progress"},
                })
                yield ev("response.web_search_call.in_progress", {
                    "output_index": _ws_idx, "item_id": _ws_id,
                })
                yield ev("response.web_search_call.searching", {
                    "output_index": _ws_idx, "item_id": _ws_id,
                })
                _ws_item = {"id": _ws_id, "type": "web_search_call", "status": "completed"}
                if _action.get("query") or _action.get("url"):
                    _ws_item["action"] = _action
                outputs[_ws_idx] = _ws_item
                yield ev("response.output_item.done", {
                    "output_index": _ws_idx, "item": _ws_item,
                })
                yield ev("response.web_search_call.completed", {
                    "output_index": _ws_idx, "item_id": _ws_id,
                })
        # 3. Emit message item only if text was emitted OR no other output item exists
        has_other_items = any(o for o in outputs if o)
        if msg_index is not None or full_text or not has_other_items:
            if msg_index is None:
                msg_index = len(outputs)
                outputs.append(None)
                yield ev("response.output_item.added", {
                    "output_index": msg_index,
                    "item": {"id": msg_id, "type": "message", "status": "in_progress",
                             "role": "assistant", "content": []},
                })
                yield ev("response.content_part.added", {
                    "item_id": msg_id, "output_index": msg_index, "content_index": 0,
                    "part": {"type": "output_text", "text": "", "annotations": _annotations()},
                })
            yield ev("response.output_text.done", {
                "item_id": msg_id, "output_index": msg_index, "content_index": 0, "text": full_text,
            })
            yield ev("response.content_part.done", {
                "item_id": msg_id, "output_index": msg_index, "content_index": 0,
                "part": {"type": "output_text", "text": full_text,
                         "annotations": _annotations()},
            })
            outputs[msg_index] = msg_item("completed")
            yield ev("response.output_item.done", {"output_index": msg_index, "item": outputs[msg_index]})
        nonlocal usage
        if usage is None or (usage.get("total_tokens") or 0) == 0:
            out_txt = "".join(text_parts)
            rs_txt = "".join(reason_parts)
            if out_txt or rs_txt:
                comp = estimate_tokens(out_txt) + estimate_tokens(rs_txt)
                prompt_est = max(1, estimate_tokens(str(meta.get("input") or "")))
                usage = {
                    "prompt_tokens": prompt_est,
                    "completion_tokens": comp,
                    "total_tokens": prompt_est + comp,
                    "completion_tokens_details": {"reasoning_tokens": estimate_tokens(rs_txt)},
                    "prompt_tokens_details": {"cached_tokens": 0},
                }
                holder["usage"] = usage
        status = ("completed" if (finish != "length" and not dropped_truncated)
                  else "incomplete")
        final = resp_obj(status)
        if finish == "length":
            final["incomplete_details"] = {"reason": "max_output_tokens"}
        if not holder.get("suppress_completion"):
            yield ev("response.completed", {"response": final})

    # 只有第一轮开场。第二轮以后再送一次 response.created，客户端会
    # 看到同一则回应被开了两次。
    if not holder.get("suppress_lifecycle"):
        yield ev("response.created", {"response": resp_obj("in_progress")})
        yield ev("response.in_progress", {"response": resp_obj("in_progress")})
    for raw in upstream:
        data = strip_data_prefix(raw.decode("utf-8", "replace"))
        if not data or data == "[DONE]":
            if data == "[DONE]":
                saw_done = True
            continue
        try:
            chunk = json.loads(data)
        except Exception:
            continue
        u = chunk.get("usage")
        if u:
            if usage is None or (u.get("total_tokens") or 0) >= (usage.get("total_tokens") or 0):
                usage = u
                holder["usage"] = usage
        for choice in chunk.get("choices") or []:
            delta = choice.get("delta") or {}
            piece = delta.get("reasoning_content")
            if piece:
                if reason_index is None:
                    reason_index = len(outputs)
                    outputs.append(None)
                    yield ev("response.output_item.added",
                             {"output_index": reason_index, "item": reason_item("in_progress")})
                    yield ev("response.reasoning_summary_part.added", {
                        "item_id": rs_id, "output_index": reason_index, "summary_index": 0,
                        "part": {"type": "summary_text", "text": ""},
                    })
                reason_parts.append(piece)
                yield ev("response.reasoning_summary_text.delta", {
                    "item_id": rs_id, "output_index": reason_index,
                    "summary_index": 0, "delta": piece,
                })
            for tc in delta.get("tool_calls") or []:
                idx = tc.get("index", 0)
                fn = tc.get("function") or {}
                fn_name = fn.get("name") or ""
                fn_args = fn.get("arguments") or ""
                call_id = tc.get("id") or ""
                # web_search / web_fetch 由反代执行，不转发给客户端
                if idx in _internal_calls or (
                        _own_web_tools and fn_name
                        and wb_webtools.is_internal_tool(fn_name)):
                    entry = _internal_calls.setdefault(idx, {"name": fn_name, "arguments": ""})
                    if fn_name:
                        entry["name"] = fn_name
                    if fn_args:
                        entry["arguments"] += fn_args
                    continue
                if idx not in tool_calls_map:
                    out_idx = len(outputs)
                    outputs.append(None)
                    c_id = call_id or _new_id("call_")
                    is_custom = bool(fn_name) and fn_name in custom_names
                    entry = {
                        "output_index": out_idx,
                        "id": c_id,
                        "name": fn_name,
                        "arguments": fn_args,
                        "custom": is_custom,
                        "item_id": _new_id("ctc_" if is_custom else "fc_"),
                    }
                    tool_calls_map[idx] = entry
                    item = {
                        "id": entry["item_id"],
                        "status": "in_progress",
                        "call_id": c_id,
                        "name": fn_name,
                    }
                    if is_custom:
                        item["type"] = "custom_tool_call"
                        item["input"] = ""
                    else:
                        item["type"] = "function_call"
                        item["arguments"] = ""
                    # namespace 必须在 output_item.added 就带上（照 CiderCC-UwU
                    # proxy.mjs openItem 的做法）。事后才补只会改到 done，
                    # 客户端早就从 added 事件派发过了。
                    stamp_namespace(item, ns_map)
                    yield ev("response.output_item.added", {
                        "output_index": out_idx,
                        "item": item,
                    })
                else:
                    entry = tool_calls_map[idx]
                    if fn_name and not entry["name"]:
                        entry["name"] = fn_name
                        if fn_name in custom_names:
                            entry["custom"] = True
                    if fn_args:
                        entry["arguments"] += fn_args
                        if entry.get("custom"):
                            yield ev("response.custom_tool_call_input.delta", {
                                "output_index": entry["output_index"],
                                "item_id": entry["item_id"],
                                "call_id": entry["id"],
                                "delta": fn_args,
                            })
                        else:
                            yield ev("response.function_call_arguments.delta", {
                                "output_index": entry["output_index"],
                                "item_id": entry["item_id"],
                                "call_id": entry["id"],
                                "delta": fn_args,
                            })
            piece = delta.get("content")
            if piece:
                if msg_index is None:
                    if reason_index is not None:
                        full_r = "".join(reason_parts)
                        yield ev("response.reasoning_summary_text.done", {
                            "item_id": rs_id, "output_index": reason_index,
                            "summary_index": 0, "text": full_r,
                        })
                        yield ev("response.reasoning_summary_part.done", {
                            "item_id": rs_id, "output_index": reason_index, "summary_index": 0,
                            "part": {"type": "summary_text", "text": full_r},
                        })
                        outputs[reason_index] = reason_item("completed")
                        yield ev("response.output_item.done",
                                 {"output_index": reason_index, "item": outputs[reason_index]})
                    msg_index = len(outputs)
                    outputs.append(None)
                    yield ev("response.output_item.added", {
                        "output_index": msg_index,
                        "item": {"id": msg_id, "type": "message", "status": "in_progress",
                                 "role": "assistant", "content": []},
                    })
                    yield ev("response.content_part.added", {
                        "item_id": msg_id, "output_index": msg_index, "content_index": 0,
                        "part": {"type": "output_text", "text": "", "annotations": _annotations()},
                    })
                # DSML tool call buffering: do not stream raw DSML tags to client
                text_buffer += piece
                while text_buffer:
                    idx = text_buffer.find("<")
                    if idx == -1:
                        text_parts.append(text_buffer)
                        yield ev("response.output_text.delta", {
                            "item_id": msg_id, "output_index": msg_index,
                            "content_index": 0, "delta": text_buffer,
                        })
                        text_buffer = ""
                        break
                    m = DSML_CALLS_RE.search(text_buffer)
                    if m and m.start() == idx:
                        if idx > 0:
                            lead = text_buffer[:idx]
                            text_parts.append(lead)
                            yield ev("response.output_text.delta", {
                                "item_id": msg_id, "output_index": msg_index,
                                "content_index": 0, "delta": lead,
                            })
                        calls_found, _ = parse_dsml_tool_calls(m.group(0))
                        if calls_found:
                            dsml_tool_calls.extend(calls_found)
                        text_buffer = text_buffer[m.end():]
                        continue
                    cand = text_buffer[idx:idx+30]
                    is_cand = ("DSML" in cand) or (len(cand) < 10 and not any(c in cand for c in (" ", "\t", "\n", ">")))
                    if is_cand:
                        lead = text_buffer[:idx]
                        text_parts.append(lead)
                        yield ev("response.output_text.delta", {
                            "item_id": msg_id, "output_index": msg_index,
                            "content_index": 0, "delta": lead,
                        })
                        text_buffer = text_buffer[idx:]
                        break
                    else:
                        next_lt = text_buffer[idx+1:].find("<")
                        if next_lt != -1:
                            flush_len = idx + 1 + next_lt
                            lead = text_buffer[:flush_len]
                            text_parts.append(lead)
                            yield ev("response.output_text.delta", {
                                "item_id": msg_id, "output_index": msg_index,
                                "content_index": 0, "delta": lead,
                            })
                            text_buffer = text_buffer[flush_len:]
                        else:
                            text_parts.append(text_buffer)
                            yield ev("response.output_text.delta", {
                                "item_id": msg_id, "output_index": msg_index,
                                "content_index": 0, "delta": text_buffer,
                            })
                            text_buffer = ""
                            break
            if choice.get("finish_reason"):
                finish = choice["finish_reason"]
    # Truncated streams (max_tokens, dropped connection) can leave tool-call
    # arguments as half-written JSON. Do not close those calls out as
    # completed: drop them before finalize so the client never receives a
    # function_call_arguments.done / output_item.done with unparsable
    # arguments. Complete calls in the same batch are kept.
    dropped_truncated = 0
    if tool_calls_map and (finish == "length" or not saw_done):
        for idx in sorted(tool_calls_map.keys()):
            entry = tool_calls_map[idx]
            if is_truncated_arguments(entry.get("arguments")):
                tool_calls_map.pop(idx, None)
                dropped_truncated += 1
    yield from _finalize()

# ---------------------------------------------------------------------------
# HTTP layer
# ---------------------------------------------------------------------------
def _if_none_match_hit(header_value, etag):
    """If-None-Match 头是否命中给定的 ETag。

    RFC 7232 §3.2：字段值是一个逗号分隔的 entity-tag 列表，且 If-None-Match
    用弱比较——W/ 前缀忽略，所以 W/"x" 与 "x" 等同；"*" 匹配任何已存在的表示。
    """
    for token in header_value.split(","):
        token = token.strip()
        if token == "*":
            return True
        if token.startswith("W/"):
            token = token[2:].strip()
        if token and token == etag:
            return True
    return False


def _cache_etag(kind, key, built_at):
    """给带 TTL 缓存的只读接口派生一个强校验符（ETag）。

    由 (接口名, 缓存键, 缓存条目的构建时刻) 三者派生，**不是**对响应字节做哈希：

      - 构建时刻在两次重建之间是恒定的，所以"数字没变"的轮询回 304；缓存一重建
        它必然变化，客户端立刻拿到新数据。哈希响应字节恰好相反——payload 里有
        time.time() 派生的字段（/usage 的 started/since、/usage/timeseries 的
        桶边界与 until），同一份缓存被序列化两次都不逐字节相同，tag 会永远不命中，
        每次轮询照样重传 10~100KB，还白搭一次全量序列化 + 哈希的 CPU。
      - (键, 时刻) 都是现成的，一次 sha1 只哈希几十字节的键文本；哈希整个响应体
        则要先把 JSON 序列化出来，在 5 秒一轮的热路径上反而更贵。

    键文本进摘要而不是直接拼进 tag：缓存键里含调用方给的 realm，它可能带引号 /
    反斜杠（`?realm=a"b` 一路原样进键），拼出来会发出一个畸形的 ETag 头；摘要
    则永远是合法的 etagc 字符。摘要只取 16 个十六进制位：它只需要在"同一实例的
    少量缓存条目"之间区分，不是安全边界。
    """
    stamp = "%x" % int(built_at * 1_000_000)
    digest = hashlib.sha1(("%s\x1f%s" % (kind, key)).encode("utf-8")).hexdigest()[:16]
    return '"%s-%s"' % (stamp, digest)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    # 关掉 Nagle（对连接设 TCP_NODELAY）：响应头和响应体是两次独立 write，Nagle
    # 会把第二次小写攒在手里等第一次的 ACK，碰上客户端 delayed ACK 就是每个小响应
    # 白付 ~40ms（路由器实测同一 keep-alive 连接的连续小响应：中位数 50.0ms ->
    # 1.41ms；面板几乎所有 <64KB 的 JSON 都中招，Tailscale 常驻连接上每笔都付
    # 一次）。StreamRequestHandler.setup() 原生支持这个开关，werkzeug/uvicorn
    # 同做法，一行生效。
    disable_nagle_algorithm = True
    # Which configured API key the caller used, set by _key_ok(). Its bound
    # realm decides the upstream exit for this request alone.
    key_entry = None
    # 本次 _json() 响应要附带的校验符（由 _json_cached() 设置）。放成实例属性
    # 而不是参数，是因为测试里大量 Handler 桩只覆写了 _json(code, obj) 这两个
    # 位置参数——加参数会让它们全部报错，而属性默认 None 对它们完全无感。
    _json_etag = None
    # The stdlib default caps the request line at 64KB and answers an opaque
    # bare "414 Request-URI Too Long" for anything longer. Raise it and reply in
    # the normal JSON error shape so an over-long URL is diagnosable.
    max_request_line = 1024 * 1024
    def handle_one_request(self):
        # Reset per-request auth state. HTTP/1.1 keeps the connection alive, so
        # one Handler instance serves many requests; a request that authenticates
        # via the panel token never reassigns key_entry, and without this reset
        # it inherited the realm binding of whatever API key used the connection
        # before it - sending that request to the wrong upstream exit.
        self.key_entry = None
        # 校验符同理：留着上一轮的 tag，下一次 _json() 就会把别的请求的 ETag
        # 发出去（keep-alive 上同一个 Handler 实例服务一整条连接）。
        self._json_etag = None
        # Body-tracking state must also start clean for every request, otherwise
        # a later drain would skip a body that has not been read yet.
        self._body_consumed = False
        try:
            self.raw_requestline = self.rfile.readline(self.max_request_line + 1)
        except (ConnectionResetError, BrokenPipeError, ConnectionAbortedError):
            self.close_connection = True
            return
        except Exception:
            self.close_connection = True
            return
        if len(self.raw_requestline) > self.max_request_line:
            self.requestline = ''
            self.request_version = ''
            self.command = ''
            # The cap is enforced by reading at most max_request_line + 1
            # bytes, so the rest of the oversized line is still in the socket.
            # Replying and then closing with unread data pending makes the OS
            # send an RST, which discards the buffered reply - the client sees
            # a reset and no error at all. Drain a bounded amount first so the
            # 414 actually arrives.
            self._drain_oversized_request_line()
            try:
                self._error(414, "request line too long (limit %d bytes); "
                                 "put long content in the POST body, not the URL"
                            % self.max_request_line, "invalid_request_error")
            except Exception:
                pass
            self.close_connection = True
            return
        if not self.raw_requestline:
            self.close_connection = True
            return
        if not self.parse_request():
            return
        mname = 'do_' + self.command
        if not hasattr(self, mname):
            self.send_error(501, "Unsupported method (%r)" % self.command)
            return
        getattr(self, mname)()
        self.wfile.flush()
    def handle(self):
        try:
            super().handle()
        except (ConnectionResetError, BrokenPipeError, ConnectionAbortedError):
            pass
    def finish(self):
        try:
            super().finish()
        except (ConnectionResetError, BrokenPipeError, ConnectionAbortedError):
            pass
    server_version = "wb-proxy/1.7.3"
    def log_message(self, fmt, *args):
        # 静默过滤前端看板高频定时心跳的正常 200 GET 请求（/logs、/usage、/accounts 轮询等）
        # 避免自增死循环刷屏与日志污染。遇 4xx/5xx 异常或所有非 GET 业务操作依然如实记录。
        try:
            status_code = int(args[1]) if len(args) > 1 and str(args[1]).isdigit() else 200
            if status_code < 400 and getattr(self, "command", "GET") == "GET":
                req_path = (getattr(self, "path", None) or (args[0] if args else "")).split("?")[0]
                quiet_prefixes = (
                    "/logs", "/usage", "/accounts", "/scheduler",
                    "/health", "/panel/status", "/realm", "/favicon.ico"
                )
                if any(req_path == p or req_path.startswith(p + "/") for p in quiet_prefixes):
                    return
        except Exception:
            pass
        log(fmt % args)
    def _json(self, code, obj):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        # 只有 _json_cached() 覆盖的只读接口才有校验符；其余调用点保持原样，
        # 一个头都不多发。
        etag = self._json_etag
        if etag is not None:
            # no-cache 而不是 no-store：允许浏览器存一份，但每次使用前必须回源
            # 校验。配合 ETag，校验命中就是一次 304（无 body），而不是重传
            # 10~100KB 的 JSON；no-store 会让浏览器连存都不存，条件请求无从谈起。
            self.send_header("Cache-Control", "no-cache")
            self.send_header("ETag", etag)
        # self.path is unset when parse_request() never ran (an over-long
        # request line is rejected before it), so fall back to "".
        if cors_origin_allowed(getattr(self, "path", "") or ""):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)
        # Flush here rather than relying on the caller: with HTTP/1.1
        # keep-alive the client blocks until the response is actually on the
        # wire, and an error reply only flushed at the end of the handler looks
        # like a hung request.
        try:
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            self.close_connection = True
    def _json_cached(self, code, data_fn, etag_fn):
        """带 TTL 缓存的只读接口：_json() + 条件请求（命中 If-None-Match 回 304）。

        data_fn() 取数、etag_fn() 取"当前缓存条目的校验符"，两者在这里配对调用，
        而且是**取数前后各取一次戳**：只有两次相同，才敢把这个 tag 发出去。这个
        相等就是"这份响应体确实出自该 tag 所描述的那条缓存条目"的证明——

          - 两次读取之间若发生了一次重建，戳就会变，于是这一次不发校验符，走普通
            200；客户端下一轮就重新同步（下一轮取数前取到的就是新条目的戳，前后
            一致，tag 生效）。代价是每次重建后多一次全量响应，换来的是绝不会发出
            一个不描述本次响应体的 tag。
          - 反过来（先取数、后取戳）才有真问题：拿到的是新条目的戳配旧条目的数据，
            客户端会带着这个 tag 回来，命中 304，于是把过期数字一直显示到下一次
            重建为止。

        两个可调用对象都无参，参数由调用方用闭包带上；顺序由这里保证，调用方不
        必（也不该）自己拼这个顺序。任何解析/取值异常都退化成"没有校验符"的普通
        200，绝不让响应路径崩掉。
        """
        try:
            before = etag_fn()
        except Exception:
            before = None
        data = data_fn()
        try:
            after = etag_fn()
        except Exception:
            after = None
        etag = after if (after is not None and after == before) else None
        if etag is not None:
            try:
                inm = self.headers.get("If-None-Match")
            except Exception:
                # 没有 headers 对象（测试桩 / 未走 parse_request）时按不匹配处理。
                inm = None
            if inm is not None and _if_none_match_hit(inm, etag):
                # 304 不带 body，也不带 Content-Length：RFC 7230 §3.3.3 规定 304
                # 在空行处结束，再报一个全量长度反而会诱使客户端 / 代理去等一个
                # 永远不会来的 body。校验符和 Cache-Control 必须原样重发
                # （RFC 7232 §4.1），否则缓存会丢掉状态、下次又整份重取。
                self.send_response(304)
                self.send_header("Cache-Control", "no-cache")
                self.send_header("ETag", etag)
                self.end_headers()
                return
            self._json_etag = etag
        try:
            return self._json(code, data)
        finally:
            # 无论 _json() 是否抛异常，都不能把这次请求的 tag 留给下一次。
            self._json_etag = None
    def _discard_body(self):
        """Drain the request body so the connection stays in sync.

        A POST rejected before its body is read (401, 404, a panel route) leaves
        the payload sitting in the socket. On a keep-alive connection the next
        request then starts by parsing that leftover JSON as the request line,
        which surfaces as a bogus "414 Request-URI Too Long" - with an empty
        request line in the log - on an otherwise healthy connection.

        Handles both Content-Length and Transfer-Encoding: chunked, since
        clients switch to the latter for large bodies.
        """
        if getattr(self, "_body_consumed", False):
            # The handler already read the body (e.g. an error raised after
            # _read_payload). Reading Content-Length bytes again would block
            # until the client gives up, turning an instant reply into a hang.
            return
        # No parsed request means no headers object and nothing buffered to
        # drain: the over-long request line is rejected before parse_request()
        # ever runs. Reading self.headers here would raise out of _error() and
        # leave the client with no reply at all.
        headers = getattr(self, "headers", None)
        if headers is None:
            return
        transfer_encoding = (headers.get("Transfer-Encoding") or "").lower()
        try:
            if "chunked" in transfer_encoding:
                self._drain_chunked_body()
                return
            length = int(headers.get("Content-Length") or 0)
        except Exception:
            length = 0
        if length <= 0:
            return
        if length > MAX_PAYLOAD_BYTES:
            # The client announced a body we refuse (413). Reading it would
            # block until it finishes sending gigabytes, so close instead and
            # let it see the reply plus the disconnect.
            self.close_connection = True
            return
        remaining = length
        try:
            while remaining > 0:
                chunk = self.rfile.read(min(remaining, 65536))
                if not chunk:
                    break
                remaining -= len(chunk)
        except Exception:
            # A short read means the peer went away; nothing left to align.
            pass
    def _drain_chunked_body(self):
        """Consume a chunked body (terminated by a zero-length chunk)."""
        try:
            while True:
                line = self.rfile.readline(65536)
                if not line:
                    return
                size_field = line.split(b";", 1)[0].strip()
                if not size_field:
                    continue
                size = int(size_field, 16)
                if size == 0:
                    # Optional trailers, then the final blank line.
                    while True:
                        trailer = self.rfile.readline(65536)
                        if not trailer or trailer in (b"\r\n", b"\n"):
                            return
                remaining = size
                while remaining > 0:
                    data = self.rfile.read(min(remaining, 65536))
                    if not data:
                        return
                    remaining -= len(data)
                self.rfile.read(2)  # trailing CRLF after each chunk
        except Exception:
            self.close_connection = True
    # How much of an over-long request line to read before giving up. The peer
    # is already misbehaving; this only needs to be enough that a normal client
    # (which sent one line and is waiting for an answer) sees the reply.
    OVERSIZED_DRAIN_LIMIT = 8 * 1024 * 1024

    def _drain_oversized_request_line(self):
        """Consume the rest of a too-long request line, within a budget.

        Without this the reply is lost to an RST (see the caller). The newline
        ends the line; past the budget the peer is clearly not going to stop,
        so give up and let the connection close.
        """
        budget = self.OVERSIZED_DRAIN_LIMIT
        try:
            while budget > 0:
                chunk = self.rfile.readline(min(budget, 65536))
                if not chunk:
                    return
                budget -= len(chunk)
                if chunk.endswith(b"\n"):
                    return
        except Exception:
            pass

    def _handle_expect_continue(self):
        """Answer 'Expect: 100-continue' before deciding to reject a body.

        Clients that send this header wait for the interim response before
        transmitting a large payload. Rejecting outright (or draining first)
        made both sides wait on each other until the socket timed out.
        """
        # No parsed request means no headers object; there is no interim
        # response to send, and touching self.headers here would raise out of
        # the error reply the caller is trying to produce.
        headers = getattr(self, "headers", None)
        if headers is None:
            return
        expect = (headers.get("Expect") or "").lower()
        if "100-continue" not in expect:
            return
        try:
            self.send_response_only(100)
            self.end_headers()
            self.wfile.flush()
        except Exception:
            pass
    def _error(self, code, message, err_type="server_error"):
        # Every early rejection funnels through here, so draining the body in
        # one place covers all of them. Unblock any client still waiting on
        # "Expect: 100-continue" first, otherwise it never sends the body and
        # the drain below waits for data that will never arrive.
        self._handle_expect_continue()
        self._discard_body()
        error = {"message": message, "type": err_type, "code": code}
        hint = gateway_hint(code, message)
        if hint:
            error["gateway_hint"] = hint
        self._json(code, {"error": error})
    def _anthropic_error(self, code, message, err_type=None):
        """Reply with the Anthropic JSON error envelope, not OpenAI's."""
        self._handle_expect_continue()
        self._discard_body()
        return self._json(code, anthropic_error_obj(code, message, err_type))
    def _rate_limited(self, exc):
        """429 with Retry-After, so clients back off instead of hammering.

        The upstream body names the reset time; when it does not, fall back to
        the shortest model cooldown we know about.
        """
        wait = max(1, int(getattr(exc, "wait", 60) or 60))
        # A 429 raised without an upstream call (the pool is parked by the
        # daily token guard) carries its own text; everything else keeps the
        # upstream wording.
        custom = getattr(exc, "message", "")
        text = custom or (
            "upstream rate limit reached for this model; retry in %ds" % wait)
        # The upstream detail only decorates the upstream wording; a local
        # message would only repeat itself.
        detail = ""
        if exc.detail and not custom:
            detail = " - " + exc.detail[:200]
        # 429 can be answered before the body is read (the model cooldown is
        # checked on the way in), so drain it exactly like _error does.
        self._handle_expect_continue()
        self._discard_body()
        body = json.dumps({
            "error": {
                "message": text + detail,
                "type": "rate_limit_error",
                "code": 429,
                "retry_after": wait,
            }
        }, ensure_ascii=False).encode("utf-8")
        self.send_response(429)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Retry-After", str(wait))
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)
    def _anthropic_rate_limited(self, exc):
        """429 with Retry-After in the Anthropic error envelope."""
        wait = max(1, int(getattr(exc, "wait", 60) or 60))
        text = (getattr(exc, "message", "") or
                "upstream rate limit reached for this model; retry in %ds" % wait)
        if getattr(exc, "detail", "") and not getattr(exc, "message", ""):
            text += " - " + str(exc.detail)[:200]
        self._handle_expect_continue()
        self._discard_body()
        body = json.dumps(anthropic_error_obj(429, text, "rate_limit_error"),
                          ensure_ascii=False).encode("utf-8")
        self.send_response(429)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Retry-After", str(wait))
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)

    def _download(self, filename, obj):
        """Send a JSON document as a browser download.
        Content-Disposition is quoted because the filename is generated from
        user-controlled parts (the realm filter) and could otherwise break the
        header or allow a response-splitting attempt.
        """
        body = json.dumps(obj, ensure_ascii=False, indent=2).encode("utf-8")
        safe = re.sub(r'[^A-Za-z0-9._-]', "_", str(filename))[:120] or "export.json"
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Disposition", 'attachment; filename="%s"' % safe)
        self.send_header("Cache-Control", "no-store")
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)
    def _supplied_key(self):
        """The key the caller presented.

        Accepts the spellings clients actually send: the Authorization header
        with or without the "Bearer" scheme, the x-api-key / api-key headers
        used by several OpenAI-compatible clients, and the ?key= query the
        dashboard falls back to when it cannot set headers.
        """
        # The auth scheme is case-insensitive per RFC 7235, so "bearer sk-x"
        # and "BEARER sk-x" must strip just like "Bearer sk-x". The old
        # removeprefix("Bearer ") left the scheme attached for other casings
        # and the whole "bearer sk-x" string was then compared as a key.
        header = (self.headers.get("Authorization") or "").strip()
        supplied = ""
        if header:
            scheme, _, value = header.partition(" ")
            if scheme.lower() == "bearer":
                supplied = value.strip()
            else:
                supplied = header
            # Tolerate a quoted credential, which some SDKs add.
            if len(supplied) >= 2 and supplied[0] == supplied[-1] and supplied[0] in "\"'":
                supplied = supplied[1:-1].strip()
        if supplied:
            return supplied
        for name in ("x-api-key", "api-key", "x-auth-token"):
            value = (self.headers.get(name) or "").strip()
            if value:
                return value
        # Browsers cannot set headers on a top-level navigation, so accept the
        # key as a query parameter too - the dashboard uses this when opened
        # from another device.
        try:
            query = parse_qs(urlparse(self.path).query)
            for name in ("key", "api_key", "api-key"):
                value = (query.get(name) or [""])[0].strip()
                if value:
                    return value
            return ""
        except Exception:
            return ""
    def _key_ok(self):
        """True when the request carries a right key (or no key is needed)."""
        # An authenticated panel session also unlocks the management APIs,
        # so the browser never has to keep the API key in localStorage.
        if self._panel_ok():
            return True
        self.key_entry = identify_key(self._supplied_key())
        if self.key_entry:
            return True
        if not auth_required():
            return True
        return False
    def _key_realm(self):
        """Realm bound to the key this request used, or "" when unbound."""
        return (self.key_entry or {}).get("realm") or ""
    def _key_id(self):
        """Settings id of the key that paid for this request, or None.

        None covers a panel session and a deployment that runs without any key
        configured - both are real, and the usage log keeps them apart from
        rows written before the key field existed.
        """
        return (self.key_entry or {}).get("id") or None
    def _cross_realm_error(self, model, realm):
        """Explain a model/exit mismatch instead of letting upstream reject it.
        Sending gpt-6-astra to the domestic exit (or deepseek-v4-pro to the
        international one) earns an opaque 403 from upstream, so catch it here
        and say which key is bound where.
        """
        if not realm or not model:
            return ""
        owner = exclusive_realm(model)
        if not owner or owner == realm:
            return ""
        name = (self.key_entry or {}).get("name") or "当前 Key"
        served = "国内版" if owner == "cn" else "国际版"
        used = "国内版" if realm == "cn" else "国际版"
        return ("模型 %s 只在%s提供，但「%s」绑定的是%s出口。"
                "请改用对应出口的 Key，或把该 Key 的出口改为「跟随面板切换」。"
                % (model, served, name, used))
    def _banned_model_error(self, model):
        """被封锁的模型直接报错，不碰上游、不扣任何点数。"""
        if not is_model_banned(model):
            return ""
        return banned_model_message(model)

    def _key_model_error(self, model):
        """Per-key model restriction: reject before the request reaches upstream.

        A key that lists no models stays unrestricted, so this is a no-op
        unless the operator asked for a limit.
        """
        entry = self.key_entry
        if not entry:
            return ""
        if wb_settings.key_allows_model(entry, model):
            return ""
        return key_model_message(entry, model)
    def _request_realm(self, explicit=None):
        """Pick the upstream exit for this request.
        Priority: an explicit ?realm= argument, then the realm bound to the
        API key, then the X-Realm header / ?realm= query, and finally the
        global switch. Returning None lets open_upstream() fall back to
        model-based detection.
        """
        if explicit:
            return explicit
        bound = self._key_realm()
        if bound:
            return bound
        header = self.headers.get("X-Realm")
        if header:
            return header
        try:
            return parse_qs(urlparse(self.path).query).get("realm", [None])[0]
        except Exception:
            return None
    def _authorized(self):
        if self._key_ok():
            return True
        # Say how a key must be presented, so a key that merely looks identical
        # (masked copy, trailing whitespace) is diagnosable straight from the
        # client error. Deliberately does not echo key names or values.
        hint = ("send it as 'Authorization: Bearer <key>' or '?key=<key>'; "
                "copy the value from the panel's 设置 page")
        try:
            if not any(k.get("enabled") for k in configured_keys()) and not API_KEY:
                hint = ("no key is configured - open the dashboard and add one, "
                        "or restart with --api-key")
        except Exception:
            pass
        self._error(401, "invalid api key - " + hint, "invalid_request_error")
        return False
    # ---- web panel access ----
    def _panel_token(self):
        """Session token from the X-Panel-Token header.
        Deliberately header-only: a token in the query string leaks through
        browser history, the Referer header and any reverse-proxy access log.
        """
        token = (self.headers.get("X-Panel-Token") or "").strip()
        return token
    def _panel_ok(self):
        return PANEL.valid(self._panel_token())
    @staticmethod
    def _is_panel_route(path):
        """Management endpoints shown in the web panel.
        Model listings stay reachable with the API key alone so that plain
        OpenAI clients can keep discovering models.
        """
        if path.startswith("/accounts"):
            return True
        if path.startswith("/usage") or path.startswith("/v1/usage"):
            return True
        if path.startswith("/activity"):
            return True
        if path.startswith("/tasks") or path.startswith("/scheduler"):
            return True
        if path.startswith("/settings"):
            return True
        if path.startswith("/updates"):
            return True
        if path.startswith("/logs"):
            return True
        # The pricing table is the panel's own view of the estimate, not part
        # of the OpenAI-compatible surface: its writes (/pricing/refresh,
        # /pricing/mapping) were already panel-only, and the read side returns
        # the same management state plus resolved project-local file paths.
        if path.startswith("/pricing"):
            return True
        if path.startswith("/agents"):
            return True
        return False
    def do_OPTIONS(self):
        self.send_response(204)
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Headers", "*")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Content-Length", "0")
        self.end_headers()
    def do_GET(self):
        parsed = urlparse(self.path)
        path = parsed.path
        query = parse_qs(parsed.query)
        # The standalone configurator, for clients on machines that do not run
        # the gateway. Served before the panel gate and deliberately without a
        # session: the machines that need it are exactly the ones that cannot
        # pass the loopback check, and the body is public source code (the
        # same wb_agents.py shipped in the repository, no secrets inside).
        if path == "/agents/cli":
            return self._get_agents_cli()
        if self._is_panel_route(path) and not self._panel_ok():
            return self._error(401, "panel password required", "invalid_request_error")
        if path in ("/", "/dashboard", "/ui"):
            return self._get_dashboard()
        if path == "/panel/status":
            return self._get_panel_status()
        if path == "/health":
            return self._get_health()
        if path == "/realm":
            return self._get_realm()
        if path in ("/v1/models", "/models"):
            return self._get_v1_models()
        if path in ("/usage", "/v1/usage"):
            return self._get_v1_usage(query)
        if path == "/usage/recent":
            return self._get_usage_recent(query)
        if path == "/accounts/credits/detail":
            return self._get_account_credits_detail(query)
        if path == "/accounts/credits":
            return self._get_accounts_credits()
        if path == "/accounts/credits/grants":
            return self._get_account_credits_grants()
        if path == "/accounts":
            return self._get_accounts(query)
        if path == "/accounts/export":
            return self._get_accounts_export(query)
        if path == "/accounts/login/poll":
            return self._get_accounts_login_poll(query)
        if path == "/usage/analytics":
            return self._get_usage_analytics(query)
        if path == "/usage/by-account":
            return self._get_usage_by_account()
        if path == "/usage/remaining":
            return self._get_usage_remaining()
        if path == "/usage/perf":
            return self._get_usage_perf(query)
        if path == "/usage/timeseries":
            return self._get_usage_timeseries(query)
        if path == "/activity/history":
            return self._get_activity_history(query)
        if path == "/tasks":
            return self._get_tasks(query)
        if path == "/scheduler":
            return self._get_scheduler()
        if path == "/pricing":
            return self._get_pricing()
        if path == "/settings":
            return self._get_settings()
        if path == "/updates":
            return self._get_updates()
        if path == "/proxy/slots":
            if not self._panel_ok():
                return self._error(
                    401, "panel password required", "invalid_request_error"
                )
            return self._json(200, {"slots": proxy_slots_view()})
        if path == "/logs":
            return self._get_logs(query)
        if path == "/logs/export":
            return self._get_logs_export()
        if path == "/settings/reveal":
            return self._get_settings_reveal(query)
        if path == "/agents":
            return self._get_agents()
        if path == "/agents/available":
            # 看板启动时用这个廉价判定决定要不要显示入口：不 import
            # wb_agents、不做任何探测（issue #246）。
            return self._json(200, {"enabled": self._agents_client_allowed()})
        return self._error(404, "not found", "invalid_request_error")
    def _get_dashboard(self):
        return self._dashboard()

    def _get_panel_status(self):
        # Answer without a token: the dashboard needs to know whether to
        # show the login screen before it can hold a session.
        info = {
            "panel_password_required": True,
            "panel_password_is_default": wb_settings.panel_password_is_default(ACCOUNTS_DIR),
            "authenticated": self._panel_ok(),
        }
        # Whether a key exists is not a secret; its value never leaves the
        # process, and the settings endpoint only reports a masked form.
        info["api_key_set"] = bool(API_KEY)
        return self._json(200, info)

    def _get_health(self):
        # Always answer (the launcher uses this to detect a running copy),
        # but only expose account identity to an authorised caller.
        rep = current_account()
        info = {
            "ok": True,
            # Report the realm actually in use; this used to be the
            # literal "intl" and drifted from the panel switch.
            "realm": CURRENT_REALM,
            "accounts": len(POOL.accounts) if POOL else 0,
            "accounts_ready": POOL.count_ready() if POOL else 0,
            "api_key_required": auth_required(),
        }
        if self._key_ok():
            info.update({
                "uid": rep.uid if rep else None,
                "domain": rep.domain if rep else None,
                "issuer": wb_accounts.jwt_issuer(rep.access_token) if rep else None,
                "credential_file": os.path.basename(rep.path) if rep and rep.path else None,
                "expires_at": rep.expires_at if rep else None,
            })
        return self._json(200, info)
    # Accept the conventional /v1 prefix and the bare path, because clients
    # differ in whether they append "/v1" themselves.

    def _get_realm(self):
        return self._json(200, {"current": CURRENT_REALM, "options": ["intl", "cn"]})

    def _get_v1_models(self):
        if not self._authorized():
            return
        req_realm = self._request_realm() or CURRENT_REALM
        try:
            entries = fetch_models(realm=req_realm)
        except Exception as exc:
            return self._error(502, str(exc))
        # Level 4 is best effort: warm the local cache in the background at
        # most once per cooldown; offline deployments just keep the fallback.
        try:
            wb_modelsdev.refresh_async(ACCOUNTS_DIR, log=log)
        except Exception:
            pass
        data = [model_entry(mid, meta) for mid, meta in entries]
        return self._json(200, {"object": "list", "data": data, "realm": req_realm or CURRENT_REALM})

    def _get_v1_usage(self, query):
        if not self._authorized():
            return
        req_realm = query.get('realm', [None])[0] or self.headers.get('X-Realm') or CURRENT_REALM
        req_range, req_since, req_until = range_query(query)
        # 取数与取校验符必须看到同一套参数（否则 tag 落到另一条缓存条目上），
        # 但"先取戳、再取数、再取一次戳"的配对顺序由 _json_cached 保证。
        return self._json_cached(
            200,
            lambda: usage_snapshot(realm=req_realm, range=req_range,
                                   since=req_since, until=req_until),
            lambda: usage_snapshot_etag(realm=req_realm, range=req_range,
                                        since=req_since, until=req_until))

    def _get_usage_recent(self, query):
        if not self._authorized():
            return
        try:
            limit = max(1, min(1000, int((query.get("limit") or ["100"])[0])))
        except ValueError:
            limit = 100
        try:
            page = max(1, int((query.get("page") or ["1"])[0]))
        except ValueError:
            page = 1
        req_realm = query.get('realm', [None])[0] or self.headers.get('X-Realm') or CURRENT_REALM
        return self._json(200, recent_usage(limit, realm=req_realm, page=page))

    def _get_activity_history(self, query):
        """账号每日活动的结构化历史（issue #34）。只读，供面板的签到记录用。

        筛选条件写错时返回 400 而不是静默忽略：调用方拿着一个被忽略的
        result=failed 会以为「这几天没有失败」，实际上它拿回的是全部结果。
        """
        if not self._authorized():
            return

        def first(name):
            values = query.get(name) or [""]
            return (values[0] if values else "") or ""

        try:
            payload = wb_activity.query(range_key=first("range") or None,
                                        uid=first("uid"),
                                        task=first("task"),
                                        result=first("result"),
                                        limit=first("limit") or None)
        except ValueError as exc:
            return self._error(400, str(exc), "invalid_request_error")
        return self._json(200, payload)

    def _get_accounts_credits(self):
        if not self._authorized():
            return
        # Refresh credits for all accounts
        for a in (POOL.accounts if POOL else []):
            a.fetch_credits()
        return self._json(200, {"accounts": account_views()})

    def _get_account_credits_grants(self):
        """积分获取历史：只读账号积分快照，不发上游请求。

        要更新的数字先走「一键刷新积分」（POST /accounts/credits），这里只把
        最近一次快照里的积分包摊平成一张表；载荷带上快照时刻，面板据此提醒
        数据有多旧。
        """
        if not self._authorized():
            return
        return self._json(200, credit_grants())

    def _get_account_credits_detail(self, query):
        if not self._authorized():
            return
        uid = (query.get("uid") or [""])[0]
        refresh = (query.get("refresh") or ["0"])[0] in ("1", "true", "yes")
        if not POOL:
            return self._json(200, {"ok": False, "error": "账号池未初始化"})
        account = POOL.get(uid) if uid else None
        if not account:
            if POOL.accounts:
                account = POOL.accounts[0]
            else:
                return self._json(200, {"ok": False, "error": "未找到指定账号"})

        if refresh or not account.credits or not account.credits.get("packages"):
            res = account.fetch_credits()
            if not res.get("ok"):
                return self._json(200, {
                    "ok": False,
                    "uid": account.uid,
                    "nickname": account.nickname,
                    "realm": account.realm,
                    "credits": account.credits,
                    "error": res.get("error", "获取积分明细失败")
                })

        return self._json(200, {
            "ok": True,
            "uid": account.uid,
            "nickname": account.nickname,
            "realm": account.realm,
            "credits": account.credits
        })

    def _get_accounts(self, query):
        if not self._authorized():
            return
        # Fold the usage log before building the view, so the 日限额 badge and
        # the parked count describe right now instead of the last request.
        apply_daily_guards()
        return self._json(200, {
            "accounts": account_views(realm=query.get('realm', [None])[0] or CURRENT_REALM),
            "storage": ACCOUNTS_DIR,
            "usable": POOL.count_ready() if POOL else 0,
        })

    def _get_accounts_export(self, query):
        if not self._authorized():
            return
        # ?download=1 makes the browser save it as a file; without it the
        # document is returned inline so the dashboard can show a summary.
        # ?uid= narrows it to specific accounts (repeatable, comma-joined),
        # which is how the per-row "export" button works.
        realm = (query.get("realm") or [None])[0] or None
        if realm not in ("intl", "cn"):
            realm = None
        include_secrets = (query.get("secrets") or ["1"])[0] not in ("0", "false", "no")
        uids = []
        for raw in query.get("uid") or []:
            uids.extend(part.strip() for part in str(raw).split(",") if part.strip())
        if uids:
            known = {a.uid for a in (POOL.accounts if POOL else [])}
            missing = [u for u in uids if u not in known]
            if missing:
                return self._error(404, "no such account: %s" % ", ".join(missing[:5]),
                                   "invalid_request_error")
        doc = wb_accounts.build_export_document(
            POOL.accounts if POOL else [],
            realm=realm,
            include_secrets=include_secrets,
            uids=uids or None,
        )
        if (query.get("download") or ["0"])[0] in ("1", "true", "yes"):
            stamp = time.strftime("%Y%m%d-%H%M%S")
            if len(uids) == 1:
                # Name a single-account export after the account, so a
                # folder of them stays readable.
                label = uids[0][:8]
            else:
                label = realm + "-" if realm else ""
            name = "workbuddy-accounts-%s%s.json" % (label, stamp)
            return self._download(name, doc)
        return self._json(200, doc)

    def _get_accounts_login_poll(self, query):
        if not self._authorized():
            return
        state = (query.get("state") or [""])[0]
        return self._json(200, POOL.poll_login(state))

    def _get_usage_analytics(self, query):
        if not self._authorized():
            return
        req_realm = query.get("realm", [None])[0] or None
        req_range, req_since, req_until = range_query(query)
        return self._json_cached(
            200,
            lambda: compute_usage_analytics(realm=req_realm, range=req_range,
                                            since=req_since, until=req_until),
            lambda: usage_analytics_etag(realm=req_realm, range=req_range,
                                         since=req_since, until=req_until))

    def _get_usage_by_account(self):
        if not self._authorized():
            return
        return self._json_cached(200,
                                 lambda: {"accounts": usage_by_account()},
                                 usage_by_account_etag)

    def _get_usage_remaining(self):
        if not self._authorized():
            return
        # 载荷挂短 TTL（WB_REMAINING_TTL，默认 15 秒）+ ETag：面板 5 秒一轮的
        # 轮询在 TTL 内命中同一条缓存 → 校验符不变 → 304，重建与重传都省掉。
        return self._json_cached(200, remaining_usage, remaining_usage_etag)

    def _get_usage_timeseries(self, query):
        if not self._authorized():
            return
        req_realm = (query.get('realm', [None])[0]
                     or self.headers.get('X-Realm') or CURRENT_REALM)
        req_range, req_since, req_until = range_query(query)
        bucket = (query.get("bucket") or [None])[0]
        try:
            bucket_seconds = int(bucket) if bucket else None
        except (TypeError, ValueError):
            bucket_seconds = None
        return self._json_cached(
            200,
            lambda: usage_timeseries(realm=req_realm, range=req_range,
                                     since=req_since, until=req_until,
                                     bucket_seconds=bucket_seconds),
            lambda: usage_timeseries_etag(realm=req_realm, range=req_range,
                                          since=req_since, until=req_until,
                                          bucket_seconds=bucket_seconds))

    def _get_usage_perf(self, query):
        if not self._authorized():
            return
        try:
            sample = max(10, min(20000, int((query.get("sample") or ["5000"])[0])))
        except ValueError:
            sample = 5000
        req_realm = query.get('realm', [None])[0] or self.headers.get('X-Realm') or CURRENT_REALM
        req_range, req_since, req_until = range_query(query)
        return self._json_cached(
            200,
            lambda: perf_stats(sample, realm=req_realm, range=req_range,
                               since=req_since, until=req_until),
            lambda: perf_stats_etag(sample, realm=req_realm, range=req_range,
                                    since=req_since, until=req_until))

    def _get_tasks(self, query):
        if not self._authorized():
            return
        cn_accounts = [a for a in (POOL.accounts if POOL else []) if a.realm == "cn" and a.enabled]
        if not cn_accounts:
            return self._json(200, {"tasks": [], "summary": {}, "accounts": [], "msg": "未找到可用的国内版账号"})
        uid = (query.get("uid") or [None])[0]
        acc = None
        if uid and uid != "all":
            target = POOL.get(uid) if POOL else None
            if target and target.realm == "cn":
                acc = target
        if not acc:
            acc = cn_accounts[0]
        tasks, summary = growth_snapshot(acc)
        acct_list = [{"uid": a.uid, "nickname": a.nickname or a.uid[:8]} for a in cn_accounts]
        return self._json(200, {
            "tasks": tasks,
            "summary": summary,
            "account": acc.public(),
            "accounts": acct_list,
        })

    def _get_scheduler(self):
        if not self._authorized():
            return
        return self._json(200, SCHEDULER.status() if SCHEDULER else {"enabled": False, "msg": "未运行"})

    def _get_pricing(self):
        if not self._authorized():
            return
        if PRICING:
            return self._json(200, PRICING.status())
        return self._json(200, {
            "interval_minutes": 0.0, "enabled": False, "running": False,
            "master_enabled": wb_settings.pricing_enabled(ACCOUNTS_DIR),
            "policies": 0, "models": 0, "current": {}, "logs": [],
            "gaps": [], "gap_summary": {"total": 0, "or_missing": 0,
                                        "variant_unmatched": 0, "aliases": 0,
                                        "variants_enabled":
                                            wb_settings.pricing_variant_inherit(ACCOUNTS_DIR)},
            "variant_inherit": wb_settings.pricing_variant_inherit(ACCOUNTS_DIR),
            "overrides_file": wb_pricing.overrides_path(),
            "policies_file": wb_pricing.policies_path(),
            "timeline": wb_pricing.timeline_path(), "msg": "未运行",
        })

    def _get_settings(self):
        if not self._authorized():
            return
        return self._json(200, runtime_settings_view())

    def _get_updates(self):
        """What the running build is, and whether a newer stable release exists.

        Read-only and cheap: it reports the last check's outcome, never runs
        one. The panel's "check now" is the POST below.
        """
        if not self._authorized():
            return
        if UPDATES:
            return self._json(200, UPDATES.status())
        return self._json(200, {
            "current_version": running_version(),
            "latest_version": None, "update_available": False,
            "enabled": wb_settings.update_check_enabled(ACCOUNTS_DIR),
            "checking": False, "last_attempt": None, "last_success": None,
            "last_error": "", "release_url": "", "published_at": "",
            "msg": "更新检查未运行",
        })

    def _get_logs(self, query):
        if not self._authorized():
            return
        try:
            limit = int(query.get("limit", ["200"])[0])
        except (ValueError, TypeError):
            limit = 200
        level = query.get("level", [""])[0]
        tag = query.get("tag", [""])[0]
        search = query.get("search", [""])[0]
        try:
            since_id = int(query.get("since_id", ["0"])[0])
        except (ValueError, TypeError):
            since_id = 0
        return self._json(200, get_logs(limit=limit, level=level, tag=tag, search=search, since_id=since_id))

    def _get_logs_export(self):
        if not self._authorized():
            return
        log_data = get_logs(limit=5000)
        lines = [f"[{item['ts']}] [{item['level']}] [{item['tag']}] {item['msg']}" for item in log_data["logs"]]
        text_content = "\n".join(lines).encode("utf-8")
        filename = f"wb-proxy-{time.strftime('%Y%m%d-%H%M%S')}.log"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Disposition", f'attachment; filename="{filename}"')
        self.send_header("Content-Length", str(len(text_content)))
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(text_content)
        return

    def _get_settings_reveal(self, query):
        # The panel only ever draws masked keys, so copying one needs an
        # explicit request. Panel session required, API key is not enough.
        if not self._panel_ok():
            return self._error(401, "panel password required", "invalid_request_error")
        wanted = (query.get("id") or [""])[0]
        for entry in configured_keys():
            if entry.get("id") == wanted:
                return self._json(200, {"id": wanted, "key": entry.get("key") or ""})
        return self._error(404, "no such key", "invalid_request_error")

    def _dashboard(self):
        try:
            with open(DASHBOARD_HTML, "rb") as fh:
                body = fh.read()
        except Exception as exc:
            return self._error(500, f"dashboard.html unavailable: {exc}")
        # The static file carries a placeholder; replace it with the instance
        # default so the first paint already uses the right language.
        language = wb_settings.ui_language(ACCOUNTS_DIR)
        body = body.replace(
            b'data-ui-language="__WB_UI_LANGUAGE__"',
            ('data-ui-language="%s"' % language).encode("utf-8"),
        )
        # 面板首页是纯静态资源（服务端逐字节原样发出、不含任何机密），所以可以
        # 允许浏览器存储副本、每次打开再回源校验：校验命中回 304（无 body），
        # 省下整页（约 470KB）的重复下载，手机 / Tailscale 远程访问体感最明显。
        #
        # 校验符必须覆盖**所有**影响响应体的输入，不只是文件本身：body 还取决于
        # 上面注入的 ui_language——用户改一次界面语言，文件没动、body 却变了；
        # 若 tag 只看文件，客户端带旧 tag 回来会拿到 304 + 旧语言的页面。所以 tag
        # 由 (文件 mtime_ns, size, language) 三者派生：前两个代表文件字节（本文件
        # 只在应用更新时被整体替换、从不原地修改），第三个就是本次实际注入的值，
        # 三者组合变化 ⇔ 响应字节变化。mtime 用纳秒精度，同一秒内的两次替换也能
        # 得到不同 tag。
        etag = None
        try:
            st = os.stat(DASHBOARD_HTML)
            # 强校验符（不带 W/ 前缀）：响应是「文件字节 + 本次语言」的精确副本，
            # tag 变 ⇔ 字节变。
            etag = '"%x-%x-%s"' % (st.st_mtime_ns, st.st_size, language)
        except Exception:
            # stat 取不到（或时间戳无法表示）不是致命错误：退化为一律按普通
            # 200 处理，只是这一次没有条件请求支持，绝不让面板页本身打不开。
            etag = None
        # 只认 If-None-Match，不发送、也不理会 If-Modified-Since。Last-Modified
        # 只能描述文件的 mtime，而响应体还取决于语言：日期无法表达这个输入，
        # 一旦发布出去，偏好日期的客户端就会拿它校验，语言一变就拿到过期的
        # 304。ETag 覆盖全部输入、单靠它就足够完备，所以干脆不提供日期——
        # 不发布它，客户端就没有用它的理由（RFC 7232 §2.2 里 Last-Modified 只是
        # SHOULD，响应体并非单一文件、没有可一致表达的修改日期，只发 ETag 完备）。
        if etag is not None:
            inm = self.headers.get("If-None-Match")
            if inm is not None and _if_none_match_hit(inm, etag):
                # 304 不带 body：浏览器手里已有一份，一个字节都不用再传。也不带
                # Content-Length：304 按 RFC 7230 §3.3.3 在空行处结束，再报全量
                # 长度反而会诱使客户端 / 代理等待一个永远不会来的 body。校验符和
                # Cache-Control 必须原样重发（RFC 7232 §4.1），否则缓存会丢掉状态。
                self.send_response(304)
                self.send_header("Cache-Control", "no-cache")
                self.send_header("ETag", etag)
                self.end_headers()
                return
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        # no-cache 而不是 no-store：允许存储，但每次使用前必须回源校验；配合
        # 上面的校验符，校验命中的代价是一次 304 而不是整页约 470KB。不加
        # max-age：没有它浏览器每次打开都会校验，应用更新后新版页面立即生效。
        self.send_header("Cache-Control", "no-cache")
        if etag is not None:
            self.send_header("ETag", etag)
        self.end_headers()
        self.wfile.write(body)
    def _read_chunked_body(self, max_bytes=MAX_PAYLOAD_BYTES):
        """Decode a Transfer-Encoding: chunked body into bytes.

        Some OpenAI-compatible clients stream large requests with chunked
        encoding instead of a Content-Length. Reading only Content-Length saw an
        empty body and answered 400 invalid JSON.
        """
        chunks = []
        total = 0
        while True:
            line = self.rfile.readline(65536)
            if not line:
                break
            size_field = line.split(b";", 1)[0].strip()
            if not size_field:
                continue
            try:
                size = int(size_field, 16)
            except ValueError:
                raise BadJSON()
            if size == 0:
                # Consume optional trailers up to the terminating blank line.
                while True:
                    trailer = self.rfile.readline(65536)
                    if not trailer or trailer in (b"\r\n", b"\n"):
                        break
                break
            total += size
            if total > max_bytes:
                # Keep draining so the connection stays aligned, then refuse.
                self._drain_chunked_body()
                raise BodyTooLarge(total)
            remaining = size
            while remaining > 0:
                data = self.rfile.read(min(remaining, 65536))
                if not data:
                    raise BadJSON()
                chunks.append(data)
                remaining -= len(data)
            self.rfile.read(2)  # CRLF after the chunk data
        self._body_consumed = True
        return b"".join(chunks)
    def _read_payload(self, max_bytes=MAX_PAYLOAD_BYTES, allow_list=False):
        """Parse the request body into a dict (or a list when allow_list).
        Raises BodyTooLarge / BadJSON so every caller handles both cases the
        same way instead of each remembering to check for None.
        """
        transfer_encoding = (self.headers.get("Transfer-Encoding") or "").lower()
        try:
            if "chunked" in transfer_encoding:
                raw_bytes = self._read_chunked_body(max_bytes=max_bytes)
                data = json.loads(raw_bytes.decode("utf-8", "replace") or "{}")
                if isinstance(data, dict):
                    return data
                if allow_list and isinstance(data, list):
                    return data
                return {}
            length = int(self.headers.get("Content-Length") or 0)
        except (BodyTooLarge, BadJSON):
            raise
        except Exception:
            raise BadJSON()
        if length > max_bytes:
            raise BodyTooLarge(length)
        if length < 0:
            raise BadJSON()
        try:
            raw = self.rfile.read(length).decode("utf-8") if length else "{}"
            # Mark the body as taken so a later error reply does not try to
            # drain the same bytes again (that read would block forever).
            self._body_consumed = True
            data = json.loads(raw or "{}")
        except Exception:
            raise BadJSON()
        if isinstance(data, dict):
            return data
        if allow_list and isinstance(data, list):
            # The account-import endpoint accepts a bare array of accounts,
            # which is the most natural shape for a hand-written file.
            return data
        return {}
    def _payload_or_error(self, allow_list=False):
        """Read the body, replying with the right error and returning None."""
        try:
            return self._read_payload(allow_list=allow_list)
        except BodyTooLarge as exc:
            message = ("payload too large (%d bytes > %d limit)"
                       % (exc.length, MAX_PAYLOAD_BYTES))
            if self._anthropic_route():
                self._anthropic_error(413, message, "invalid_request_error")
            else:
                self._error(413, message, "invalid_request_error")
            return None
        except BadJSON:
            if self._anthropic_route():
                self._anthropic_error(400, "invalid JSON body", "invalid_request_error")
            else:
                self._error(400, "invalid JSON body", "invalid_request_error")
            return None
    def _anthropic_route(self):
        """True when the request path is one of the Anthropic Messages routes.

        Errors raised before the route dispatcher runs (body read, size cap)
        have to answer in the Anthropic envelope, or a Claude Code client sees
        an OpenAI-shaped error it cannot parse.
        """
        return self.path.split("?")[0] in (
            "/v1/messages", "/messages",
            "/v1/messages/count_tokens", "/messages/count_tokens",
        )
    def _handle_settings_save(self):
        """Persist panel-managed settings from the web settings tab."""
        payload = self._payload_or_error()
        if payload is None:
            return
        reply = {}
        if "api_keys" in payload:
            raw = payload.get("api_keys")
            if not isinstance(raw, list):
                return self._error(400, "api_keys must be a list", "invalid_request_error")
            # The panel only ever shows a masked key, so a blank value means
            # "keep what is stored" for that row rather than "clear it".
            existing = {entry.get("id"): entry for entry in configured_keys()}
            # A submission that carries an explicit delete list is an upsert:
            # only the ids it names are retired. That is what stops a stale or
            # incomplete list (a second tab, a save racing the post-save
            # reload) from wiping a key the user never removed. An older panel
            # sends no such field and keeps the replace-by-omission contract.
            delete_ids = payload.get("deleted_api_key_ids")
            upsert = isinstance(delete_ids, list)
            cleaned = []
            for item in raw:
                if not isinstance(item, dict):
                    return self._error(400, "each api key must be an object",
                                       "invalid_request_error")
                entry_id = str(item.get("id") or "").strip()
                stored = existing.get(entry_id) or {}
                value = str(item.get("key") or "").strip()
                if not value and entry_id and entry_id in existing:
                    value = existing[entry_id].get("key") or ""
                # A new row keeps an empty id here; wb_settings mints a random
                # one on write. Deriving it from the row's position reused ids
                # of rows deleted earlier, and two rows sharing an id made
                # /settings/reveal answer with the wrong key.
                if value and len(value) < 4:
                    return self._error(400, "api key must be at least 4 characters",
                                       "invalid_request_error")
                if not value:
                    return self._error(400, "a key entry is empty - fill it in or remove the row",
                                       "invalid_request_error")
                # A field the submission omits keeps whatever is stored, the
                # same rule `models` already follows: an older or partial
                # client must not silently clear a key's exit binding, its
                # name, or its disabled state.
                if "realm" in item:
                    realm = str(item.get("realm") or "").strip().lower()
                else:
                    realm = str(stored.get("realm") or "").strip().lower()
                if realm not in ("", "intl", "cn"):
                    return self._error(400, "realm must be intl, cn or empty",
                                       "invalid_request_error")
                # An older cached panel does not know this field at all, so a
                # row that omits it keeps whatever is stored instead of
                # silently dropping the restriction.
                if "models" in item:
                    models = item.get("models")
                else:
                    models = stored.get("models")
                if "name" in item:
                    name = str(item.get("name") or "").strip()
                else:
                    name = str(stored.get("name") or "").strip()
                if "enabled" in item:
                    enabled = item.get("enabled", True) is not False
                else:
                    enabled = stored.get("enabled", True) is not False
                created_at = item.get("created_at") or (stored.get("created_at") if entry_id in existing else None) or time.strftime("%Y/%m/%d %H:%M")
                cleaned.append({
                    "id": entry_id,
                    "name": name,
                    "key": value,
                    "realm": realm,
                    "models": models,
                    "enabled": enabled,
                    "created_at": created_at,
                })
            wb_settings.set_api_keys(
                ACCOUNTS_DIR, cleaned,
                delete_ids=(delete_ids if upsert else None),
            )
            reply["api_keys_saved"] = len(cleaned)
        if "auth_disabled" in payload:
            wb_settings.set_auth_disabled(ACCOUNTS_DIR, payload.get("auth_disabled"))
            reply["auth_disabled"] = bool(payload.get("auth_disabled"))

        # The four guards can arrive grouped under "limits" (the panel's own
        # shape) or as the flat top-level keys older clients still send, which
        # are read as the global default. Both land in the same grouped store,
        # so there is one source of truth no matter who writes it.
        limits_payload = payload.get("limits")
        if limits_payload is not None and not isinstance(limits_payload, dict):
            return self._error(400, "limits must be an object",
                               "invalid_request_error")
        touched = set()
        for key in wb_settings.LIMIT_KEYS:
            grouped = (limits_payload or {}).get(key)
            if isinstance(grouped, dict):
                scopes = {scope: grouped[scope]
                          for scope in wb_settings.LIMIT_SCOPES
                          if scope in grouped}
            elif key in payload:
                scopes = {"global": payload.get(key)}
            else:
                continue
            for scope, raw in scopes.items():
                if scope == "global":
                    if isinstance(raw, bool) or raw is None:
                        return self._error(400, "%s must be a whole number" % key,
                                           "invalid_request_error")
                    try:
                        number = int(raw)
                    except (TypeError, ValueError):
                        return self._error(400, "%s must be a whole number" % key,
                                           "invalid_request_error")
                    if number < 0:
                        return self._error(400, "%s cannot be negative" % key,
                                           "invalid_request_error")
                else:
                    # Blank clears the override back to "inherit the global".
                    if raw is None or raw == "":
                        number = None
                    else:
                        if isinstance(raw, bool):
                            return self._error(
                                400, "%s must be a whole number" % key,
                                "invalid_request_error")
                        try:
                            number = int(raw)
                        except (TypeError, ValueError):
                            return self._error(
                                400, "%s must be a whole number" % key,
                                "invalid_request_error")
                        if number < 0:
                            return self._error(
                                400, "%s cannot be negative" % key,
                                "invalid_request_error")
                wb_settings.set_limit(ACCOUNTS_DIR, key, scope, number)
                touched.add(key)
            reply[key] = wb_settings.limit_value(ACCOUNTS_DIR, key)
        if limits_payload is not None:
            reply["limits"] = wb_settings.limits_snapshot(ACCOUNTS_DIR)
        if touched:
            if POOL and "reserve_credits" in touched:
                POOL.apply_reserve_credits()
            if any(k in touched for k in ("daily_token_limit", "daily_credit_limit",
                                          "model_daily_token_limit")):
                # One pass refreshes all three: a limit edit is rare, and the
                # guards publish from the same fold anyway.
                apply_daily_guards(refresh=True)
            if "expiring_window_days" in touched and POOL:
                POOL.apply_expiring_window()

        if "pricing_enabled" in payload:
            raw = payload.get("pricing_enabled")
            if not isinstance(raw, bool):
                return self._error(400, "pricing_enabled must be true or false",
                                   "invalid_request_error")
            wb_settings.set_pricing_enabled(ACCOUNTS_DIR, raw)
            reply["pricing_enabled"] = raw
            if PRICING:
                PRICING.wake()
        if "pricing_refresh_minutes" in payload or "pricing_refresh_hours" in payload:
            # The interval is in minutes. The old field name is still accepted
            # (x60) so a panel page cached from the previous build cannot set
            # the wrong unit; it is answered under the new name.
            field = "pricing_refresh_minutes" \
                if "pricing_refresh_minutes" in payload else "pricing_refresh_hours"
            raw = payload.get(field)
            if isinstance(raw, bool) or raw is None:
                return self._error(400, "%s must be a number" % field,
                                   "invalid_request_error")
            try:
                minutes = float(raw)
            except (TypeError, ValueError):
                return self._error(400, "%s must be a number" % field,
                                   "invalid_request_error")
            if field == "pricing_refresh_hours":
                minutes *= 60.0
            if minutes < 0:
                return self._error(400, "pricing_refresh_minutes cannot be negative",
                                   "invalid_request_error")
            stored = wb_settings.set_pricing_refresh_minutes(ACCOUNTS_DIR, minutes)
            if PRICING:
                # A running wait picks the new interval up on the spot.
                PRICING.set_interval(stored)
            reply["pricing_refresh_minutes"] = stored
        if "credits_refresh_hours" in payload:
            # How stale a credit balance may get before the background refresher
            # updates it. Zero turns the refresher off.
            raw = payload.get("credits_refresh_hours")
            if isinstance(raw, bool) or raw is None:
                return self._error(400, "credits_refresh_hours must be a number",
                                   "invalid_request_error")
            try:
                hours = float(raw)
            except (TypeError, ValueError):
                return self._error(400, "credits_refresh_hours must be a number",
                                   "invalid_request_error")
            if hours < 0:
                return self._error(400, "credits_refresh_hours cannot be negative",
                                   "invalid_request_error")
            reply["credits_refresh_hours"] = \
                wb_settings.set_credits_refresh_hours(ACCOUNTS_DIR, hours)
            if CREDITS_REFRESHER:
                CREDITS_REFRESHER.wake()
        if "section_max_height" in payload:
            # Panel window height cap in pixels; 0 turns the cap off. bool is an
            # int subclass and must be rejected explicitly - true would
            # otherwise store 1 and shrink every window to a sliver.
            raw = payload.get("section_max_height")
            if isinstance(raw, bool) or raw is None:
                return self._error(400, "section_max_height must be a whole number",
                                   "invalid_request_error")
            try:
                number = int(raw)
            except (TypeError, ValueError, OverflowError):
                # OverflowError: a JSON payload may carry Infinity (1e400),
                # which int() refuses - answer 400, not a 500.
                return self._error(400, "section_max_height must be a whole number",
                                   "invalid_request_error")
            # A fractional count is refused rather than truncated: int(-0.5) is
            # 0, which would slip past the negative check and silently turn the
            # cap off.
            if float(raw) != number:
                return self._error(400, "section_max_height must be a whole number",
                                   "invalid_request_error")
            if number < 0:
                return self._error(400, "section_max_height cannot be negative",
                                   "invalid_request_error")
            reply["section_max_height"] = \
                wb_settings.set_section_max_height(ACCOUNTS_DIR, number)
        if "ui_language" in payload:
            # Instance-wide default language. The dashboard overrides this per
            # browser with localStorage; this value is the fallback when no
            # browser-local preference exists.
            raw = payload.get("ui_language")
            if not isinstance(raw, str) or raw not in ("zh", "zh-Hant", "en"):
                return self._error(400, "ui_language must be zh, zh-Hant or en",
                                   "invalid_request_error")
            reply["ui_language"] = wb_settings.set_ui_language(ACCOUNTS_DIR, raw)
        if "pricing_variant_inherit" in payload:
            # Strictly a JSON boolean, like the other switches: "false" as a
            # string would be truthy and silently keep the feature on.
            raw = payload.get("pricing_variant_inherit")
            if not isinstance(raw, bool):
                return self._error(400, "pricing_variant_inherit must be true or false",
                                   "invalid_request_error")
            previous = wb_settings.pricing_variant_inherit(ACCOUNTS_DIR)
            wb_settings.set_pricing_variant_inherit(ACCOUNTS_DIR, raw)
            reply["pricing_variant_inherit"] = raw
            if PRICING and previous != raw:
                # The switch only takes effect on the next fetch - a refresh
                # drops (off) or adds (on) the suffix-inherited assignments -
                # so kick one off rather than waiting out the interval.
                threading.Thread(target=PRICING.run_once, daemon=True,
                                 name="price-refresh-inherit").start()
                reply["pricing_refresh_started"] = True
        if "auto_switch_product" in payload:
            # Strictly a JSON boolean: a string like "false" would be truthy and
            # silently switch the feature on, which is the one thing an operator
            # turning it off must not get.
            raw = payload.get("auto_switch_product")
            if not isinstance(raw, bool):
                return self._error(400, "auto_switch_product must be true or false",
                                   "invalid_request_error")
            wb_settings.set_auto_switch_product(ACCOUNTS_DIR, raw)
            reply["auto_switch_product"] = raw
        if "daily_chat_web" in payload:
            raw = payload.get("daily_chat_web")
            if not isinstance(raw, bool):
                return self._error(400, "daily_chat_web must be true or false",
                                   "invalid_request_error")
            wb_settings.set_daily_chat_web(ACCOUNTS_DIR, raw)
            reply["daily_chat_web"] = raw
        if "daily_chat_model" in payload:
            raw = payload.get("daily_chat_model") or ""
            # Empty clears the override back to the built-in id. Anything else
            # has to look like a model id; a stray value here would be sent
            # upstream on every daily run.
            if not isinstance(raw, str) or len(raw) > 64 or \
                    (raw and not re.match(r"^[A-Za-z0-9._:-]+$", raw)):
                return self._error(400, "daily_chat_model must be a model id or empty",
                                   "invalid_request_error")
            wb_settings.set_daily_chat_model(ACCOUNTS_DIR, raw)
            reply["daily_chat_model"] = raw
        if "local_web_tools" in payload:
            raw = payload.get("local_web_tools")
            if not isinstance(raw, bool):
                return self._error(400, "local_web_tools must be true or false",
                                   "invalid_request_error")
            wb_settings.set_local_web_tools(ACCOUNTS_DIR, raw)
            reply["local_web_tools"] = raw
        if "remaining_priority_enabled" in payload:
            raw = payload.get("remaining_priority_enabled")
            if not isinstance(raw, bool):
                return self._error(400, "remaining_priority_enabled must be true or false",
                                   "invalid_request_error")
            wb_settings.set_remaining_priority_enabled(ACCOUNTS_DIR, raw)
            reply["remaining_priority_enabled"] = raw
        if "egress_backoff_enabled" in payload:
            # 出口级瞬时失败退避（issue #271）：与 remaining_priority_enabled
            # 同一形状——严格 JSON 布尔，字符串 "false" 会是真值。
            raw = payload.get("egress_backoff_enabled")
            if not isinstance(raw, bool):
                return self._error(400, "egress_backoff_enabled must be true or false",
                                   "invalid_request_error")
            wb_settings.set_egress_backoff_enabled(ACCOUNTS_DIR, raw)
            reply["egress_backoff_enabled"] = raw
            if not raw and POOL is not None:
                # 关掉就立刻清掉池里上一次推的权重，不等下一个请求；打开则
                # 由下一个请求照常推（此刻就算推也要先折一次剩余估算，没必要
                # 为一个开关多付这一趟）。
                POOL.apply_remaining_weights(None)
        if "accounts_collapsed" in payload:
            # A disclosure state, and the only thing this branch may touch: the
            # submission carries just this key, so the settings it does not name
            # survive the write.
            raw = payload.get("accounts_collapsed")
            if not isinstance(raw, bool):
                return self._error(400, "accounts_collapsed must be true or false",
                                   "invalid_request_error")
            wb_settings.set_accounts_collapsed(ACCOUNTS_DIR, raw)
            reply["accounts_collapsed"] = raw
        if "accounts_separate_tab" in payload:
            # Same shape again: strictly a JSON boolean, because "false" as a
            # string would be truthy and silently split the nav.
            raw = payload.get("accounts_separate_tab")
            if not isinstance(raw, bool):
                return self._error(400, "accounts_separate_tab must be true or false",
                                   "invalid_request_error")
            wb_settings.set_accounts_separate_tab(ACCOUNTS_DIR, raw)
            reply["accounts_separate_tab"] = raw
        if "key_before_hidden" in payload:
            # Same shape as accounts_collapsed: one disclosure state per
            # submission, and strictly a JSON boolean, because "false" as a
            # string would be truthy and silently fold the row away.
            raw = payload.get("key_before_hidden")
            if not isinstance(raw, bool):
                return self._error(400, "key_before_hidden must be true or false",
                                   "invalid_request_error")
            wb_settings.set_key_before_hidden(ACCOUNTS_DIR, raw)
            reply["key_before_hidden"] = raw
        if "hidden_pages" in payload:
            # A list of page keys, and the only thing this branch may touch: the
            # submission carries just this key, so the settings it does not name
            # survive the write. A bare string would iterate character by
            # character and hide nothing but garbage, so anything that is not a
            # list of strings is rejected outright rather than normalised away.
            raw = payload.get("hidden_pages")
            if not isinstance(raw, list) or any(not isinstance(k, str) for k in raw):
                return self._error(400, "hidden_pages must be a list of page keys",
                                   "invalid_request_error")
            # Reply with the normalised list, not the raw one: keys the request
            # spelled wrong are dropped on the way in, so the panel echoes back
            # what was actually stored.
            reply["hidden_pages"] = wb_settings.set_hidden_pages(ACCOUNTS_DIR, raw)
        if "update_check_enabled" in payload:
            # Strictly a JSON boolean, like the switches above: "false" as a
            # string would be truthy and silently start the daily GitHub call.
            raw = payload.get("update_check_enabled")
            if not isinstance(raw, bool):
                return self._error(400, "update_check_enabled must be true or false",
                                   "invalid_request_error")
            wb_settings.set_update_check_enabled(ACCOUNTS_DIR, raw)
            reply["update_check_enabled"] = raw
            if UPDATES and raw:
                # Turning it on should not wait out the rest of the poll sleep.
                UPDATES.wake()
        if "upstream" in payload:
            raw = payload.get("upstream")
            if not isinstance(raw, dict):
                return self._error(400, "upstream must be an object",
                                   "invalid_request_error")
            try:
                patch = wb_settings.validate_upstream_patch(raw)
            except ValueError as exc:
                return self._error(400, str(exc), "invalid_request_error")
            wb_settings.set_upstream_config(ACCOUNTS_DIR, patch)
            reply["upstream"] = wb_settings.upstream_config(ACCOUNTS_DIR)
        if "prompt" in payload:
            raw = payload.get("prompt")
            if not isinstance(raw, dict):
                return self._error(400, "prompt must be an object",
                                   "invalid_request_error")
            try:
                patch = wb_settings.validate_prompt_patch(raw)
            except ValueError as exc:
                return self._error(400, str(exc), "invalid_request_error")
            wb_settings.set_prompt_config(ACCOUNTS_DIR, patch)
            reply["prompt"] = wb_settings.prompt_config(ACCOUNTS_DIR)
        new_key = payload.get("api_key")
        if new_key is not None:
            new_key = str(new_key).strip()
            if new_key and len(new_key) < 4:
                return self._error(400, "api key must be at least 4 characters",
                                   "invalid_request_error")
            global API_KEY, API_KEY_FILE_SET
            wb_settings.set_api_key(ACCOUNTS_DIR, new_key)
            API_KEY = new_key
            API_KEY_FILE_SET = True
            reply["api_key_set"] = bool(new_key)
        if payload.get("restart_scheduler"):
            if SCHEDULER:
                SCHEDULER.stop()
                SCHEDULER.start()
            reply["scheduler"] = "restarted"
        reply.update(runtime_settings_view())
        return self._json(200, reply)

    def _handle_proxy_slots(self, path, payload):
        """Proxy-slot management (panel-authenticated)."""
        if path == "/proxy/slots":
            return self._json(200, {"slots": proxy_slots_view()})
        if path == "/proxy/slots/save":
            raw = payload.get("slots")
            if not isinstance(raw, list):
                return self._error(400, "slots must be a list", "invalid_request_error")
            cleaned = []
            for item in raw:
                if not isinstance(item, dict):
                    continue
                # Normalized by the same helper the store uses, so the probed
                # exit fields the panel echoes back survive a save instead of
                # being dropped by a second, hand-written field list here.
                entry = wb_settings._clean_slot_entry(item)
                if entry is not None:
                    cleaned.append(entry)
            saved = wb_settings.set_proxy_slots(ACCOUNTS_DIR, cleaned)
            if POOL:
                # A slot may have been removed: unbind anyone still naming it
                # before recomputing, so a stale id cannot survive.
                dropped = wb_settings.drop_missing_bindings(POOL, saved)
                POOL.apply_proxy_slots(saved)
                if dropped:
                    log("proxy slots: unbound %d account(s) from removed slots"
                        % dropped)
            log("proxy slots saved: %d slot(s)" % len(saved))
            return self._json(200, {"slots": proxy_slots_view()})
        if path == "/proxy/slots/test":
            slot_id = str(payload.get("id") or "").strip()
            slot = wb_settings.find_proxy_slot(ACCOUNTS_DIR, slot_id)
            if slot is None:
                return self._error(404, "no such proxy slot")
            probe = probe_proxy_intel(slot["url"])
            reply = dict(probe)
            reply["id"] = slot_id
            reply["slot"] = None
            if probe["ok"]:
                # Remember what the exit turned out to be, so the panel shows it
                # without probing again. `ok` covers the IP probe only: when the
                # geo lookup came back with nothing, the exit info already stored
                # is kept rather than overwritten with blanks - a blip at the
                # lookup must not lose what the last good probe learned.
                updated_fields = {
                    "ip": probe["exit_ip"],
                    "probed_at": int(time.time()),
                }
                if probe.get("intel_ok"):
                    updated_fields.update({
                        "country": probe["country"],
                        "country_code": probe["country_code"],
                        "ip_type": probe["ip_type"],
                        "isp": probe["isp"],
                        "asn": probe["asn"],
                    })
                else:
                    log("proxy slots: %s probed %s but the geo lookup returned "
                        "nothing; keeping the exit info already stored"
                        % (slot_id, probe["exit_ip"] or "-"))
                # The name is deliberately left alone. An empty name is how the
                # store marks a slot as auto-named ("label this slot by its
                # exit"), and slot_label() derives that label from the country
                # and kind it shows - writing the derived text into the name
                # would freeze it to the exit it happened to have at the time.
                updated = wb_settings.update_proxy_slot(
                    ACCOUNTS_DIR,
                    slot_id,
                    updated_fields,
                )
                reply["slot"] = updated
                reply["name"] = (updated or {}).get("name", "")
            return self._json(200, reply)
        if path == "/proxy/discover":
            return self._json(200, {"candidates": discover_proxy_slots()})
        return self._error(404, "not found", "invalid_request_error")

    # ---- one-click agent integration (wb_agents) ----

    def _agents_client_allowed(self):
        """本请求的来源地址能不能用一键配置（判定见模块级 agents_client_allowed）。"""
        return agents_client_allowed(getattr(self, "client_address", None))

    def _agents_base_url_hint(self):
        """Best guess at the URL a local client should point at.

        Derived from the bound socket; a 0.0.0.0 bind is unreachable for a
        client, so it is reported as 127.0.0.1 instead.
        """
        host, port = "", 0
        try:
            host, port = self.server.server_address[:2]
        except Exception:
            pass
        host = str(host or "").strip() or "127.0.0.1"
        if host in ("0.0.0.0", "::", ""):
            host = "127.0.0.1"
        return "http://%s:%d/v1" % (host, int(port or 0))

    def _agents_models(self):
        """Flat model list for the picker; bundled intl+cn catalog, deduped.

        Live sources stay out of this path on purpose: the panel must answer
        even when the upstream is down, so a failure here is simply a shorter
        list, never an error.
        """
        models = []
        seen = set()
        try:
            try:
                entries = fetch_models()
            except Exception:
                entries = []
            if entries:
                sources = ((mid, meta) for mid, meta in entries)
            else:
                sources = (
                    (item.get("id"), item)
                    for item in (list(getattr(wb_catalog, "STATIC_INTL_MODELS", []))
                                 + list(getattr(wb_catalog, "STATIC_CN_MODELS", [])))
                    if isinstance(item, dict)
                )
            for mid, meta in sources:
                mid = str(mid or "").strip()
                if not mid or mid in seen:
                    continue
                seen.add(mid)
                meta = meta if isinstance(meta, dict) else {}
                entry = {"id": mid}
                ctx = meta.get("maxInputTokens")
                window = meta.get("contextWindow")
                if isinstance(window, dict) and window.get("defaultLength"):
                    ctx = window.get("defaultLength")
                if not ctx and isinstance(window, dict):
                    lengths = window.get("supportedLengths") or []
                    ctx = lengths[-1] if lengths else None
                if ctx:
                    try:
                        entry["context_window"] = int(ctx)
                    except (TypeError, ValueError):
                        pass
                out = meta.get("maxOutputTokens")
                if out:
                    try:
                        entry["max_output"] = int(out)
                    except (TypeError, ValueError):
                        pass
                name = meta.get("name")
                if name:
                    entry["name"] = name
                models.append(entry)
        except Exception:
            pass
        return models

    def _get_agents_cli(self):
        """Serve wb_agents.py so a remote machine can fetch the configurator.

        No panel session and no loopback check on purpose: the callers are
        other machines, which is the whole point of the CLI (it writes the
        config files on the machine it runs on). The file carries no secrets
        - it is byte-identical to the copy in the repository.
        """
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "wb_agents.py")
        try:
            with open(path, "rb") as fh:
                body = fh.read()
        except OSError as exc:
            return self._error(500, "wb_agents.py unavailable: %s" % exc)
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Disposition",
                         'inline; filename="wb_agents.py"')
        self.send_header("Content-Length", str(len(body)))
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)
        return

    def _get_agents(self):
        if not self._agents_client_allowed():
            # 远程看板（服务端 / Docker / OpenWrt）：一键配置改的是网关所在
            # 机器的配置目录，到不了用户自己的电脑，直接按不可用返回。
            return self._json(200, {
                "enabled": False,
                "reason": "agent config is only available from the machine running the gateway",
            })
        wb_agents = agents_module()
        keys = []
        try:
            for entry in configured_keys():
                if entry.get("enabled"):
                    keys.append({
                        "id": entry.get("id"),
                        "name": entry.get("name") or "",
                        "enabled": True,
                    })
        except Exception:
            keys = []
        return self._json(200, {
            "enabled": True,
            "clients": list(wb_agents.overview(ACCOUNTS_DIR).values()),
            "models": self._agents_models(),
            "keys": keys,
            "global_key_set": bool(API_KEY),
            "auth_required": auth_required(),
            "gateway": {"base_url": self._agents_base_url_hint()},
        })

    def _agents_resolve_key(self, payload, warnings):
        """Pick the gateway key to hand to the client, per the payload.

        Returns the key string, or None when the request should fail. The
        failure message is appended to `warnings` only for the soft-fallback
        case; hard failures raise via the caller's 400 mapping.
        """
        key_id = str(payload.get("key_id") or "").strip()
        if key_id == "__global":
            return API_KEY or None
        keys = configured_keys()
        if key_id:
            for entry in keys:
                if entry.get("id") == key_id:
                    return entry.get("key") or None
            raise wb_agents.AgentConfigError(
                "no configured key with id %r" % key_id)
        # Default: the global key when set, else the first enabled panel key.
        if API_KEY:
            return API_KEY
        for entry in keys:
            if entry.get("enabled"):
                return entry.get("key") or None
        return None

    def _handle_agents_apply(self, payload):
        wb_agents = agents_module()
        client_id = str(payload.get("client") or payload.get("client_id") or "").strip()
        if not client_id:
            return self._error(400, "client is required", "invalid_request_error")
        base_url = str(payload.get("base_url") or "").strip()
        if not re.match(r"^https?://", base_url):
            return self._error(
                400, "base_url must start with http:// or https://",
                "invalid_request_error")
        model = str(payload.get("model") or "").strip() or None
        models = payload.get("models")
        if not models:
            # Fall back to the gateway's discovered catalog so clients that embed
            # model definitions (OpenCode, DSH, Crush, ZCode) receive the full
            # model list even when the front-end omitted the field.
            models = self._agents_models()
        if not isinstance(models, list):
            return self._error(400, "models must be a list",
                               "invalid_request_error")
        # A runaway picker must not turn into a megabyte config file.
        models = models[:80]
        cleaned_models = []
        for item in models:
            if isinstance(item, dict) and item.get("id"):
                cleaned_models.append(item)
            elif isinstance(item, str) and item.strip():
                cleaned_models.append({"id": item.strip()})
        warnings = []
        try:
            api_key = self._agents_resolve_key(payload, warnings)
        except wb_agents.AgentConfigError as exc:
            return self._error(400, str(exc), "invalid_request_error")
        if not api_key:
            if auth_required():
                return self._error(
                    400, "no gateway key available - 请先配置 API Key",
                    "invalid_request_error")
            api_key = "wb-local"
            warnings.append("gateway has no key configured; wrote placeholder "
                            "'wb-local' (auth is off, so any value works)")
        try:
            result = wb_agents.integrate(
                ACCOUNTS_DIR, client_id, base_url, api_key,
                model=model, models=cleaned_models)
        except wb_agents.AgentConfigError as exc:
            return self._error(400, str(exc), "invalid_request_error")
        except Exception as exc:
            return self._error(500, "agents apply failed: %s" % exc)
        result["warnings"] = warnings
        log("agents apply: client=%s base_url=%s model=%s files=%d"
            % (client_id, base_url, model or "-", len(result.get("files") or [])),
            tag="agents")
        return self._json(200, result)

    def _handle_agents_restore(self, payload):
        wb_agents = agents_module()
        client_id = str(payload.get("client") or payload.get("client_id") or "").strip()
        if not client_id:
            return self._error(400, "client is required", "invalid_request_error")
        try:
            result = wb_agents.restore(ACCOUNTS_DIR, client_id)
        except wb_agents.AgentConfigError as exc:
            return self._error(400, str(exc), "invalid_request_error")
        except Exception as exc:
            return self._error(500, "agents restore failed: %s" % exc)
        log("agents restore: client=%s files=%d"
            % (client_id, len(result.get("restored") or [])), tag="agents")
        return self._json(200, result)

    def _handle_panel(self, path):
        """Panel login, logout and the settings screen (password + API key)."""
        payload = self._payload_or_error()
        if payload is None:
            return
        if path == "/panel/login":
            peer_ip = self.client_address[0] if hasattr(self, "client_address") and self.client_address else "127.0.0.1"
            # 限流键不能直接取对端地址：反代后对端是代理本身，见 login_rate_limit_key
            client_ip = login_rate_limit_key(peer_ip, getattr(self, "headers", None))
            now = time.time()
            with _login_lock:
                _prune_login_attempts(now)
                attempts = [t for t in _login_attempts.get(client_ip, []) if now - t < 60]
                _login_attempts[client_ip] = attempts
                if len(attempts) >= 5:
                    wait_sec = int(60 - (now - attempts[0]))
                    return self._error(429, f"too many login attempts, please wait {max(1, wait_sec)}s", "rate_limit_error")
            password = str(payload.get("password") or "")
            if not wb_settings.verify_panel_password(ACCOUNTS_DIR, password):
                with _login_lock:
                    _login_attempts.setdefault(client_ip, []).append(now)
                # Small backoff delay to mitigate automated brute force
                time.sleep(0.5)
                return self._error(401, "invalid panel password", "invalid_request_error")
            with _login_lock:
                _login_attempts.pop(client_ip, None)
            token = PANEL.create()
            return self._json(200, {
                "ok": True,
                "token": token,
                "using_default_password": wb_settings.panel_password_is_default(ACCOUNTS_DIR),
            })
        if path == "/panel/logout":
            PANEL.revoke(self._panel_token())
            return self._json(200, {"ok": True})
        # Everything past this point requires an authenticated panel session.
        if not self._panel_ok():
            return self._error(401, "panel password required", "invalid_request_error")
        if path == "/panel/password":
            current = str(payload.get("current") or "")
            new = str(payload.get("new") or "")
            if not wb_settings.verify_panel_password(ACCOUNTS_DIR, current):
                return self._error(401, "current password is wrong", "invalid_request_error")
            if len(new) < 4:
                return self._error(400, "new password must be at least 4 characters", "invalid_request_error")
            wb_settings.set_panel_password(ACCOUNTS_DIR, new)
            if new != wb_settings.DEFAULT_PANEL_PASSWORD:
                # Rotating the password invalidates every other browser session.
                PANEL.revoke_all()
            token = PANEL.create()
            return self._json(200, {"ok": True, "token": token})
        return self._error(404, "not found", "invalid_request_error")
    def _handle_accounts(self, path, payload):
        """Account-management endpoints (dashboard uses these)."""
        if POOL is None:
            return self._error(503, "account pool unavailable")
        if path == "/accounts/import" and isinstance(payload, list):
            # A bare array is only meaningful for import; wrap it so the rest
            # of this handler can keep assuming a dict.
            payload = {"data": payload}
        if not isinstance(payload, dict):
            return self._error(400, "expected a JSON object", "invalid_request_error")
        if path in ("/accounts/credits", "/accounts/credits/fetch"):
            return self._route_accounts_credits_fetch(payload)
        if path == "/accounts/credits/detail":
            return self._route_account_credits_detail(payload)
        if path == "/tasks/run":
            return self._route_tasks_run(payload)
        if path == "/tasks/travel":
            return self._route_tasks_travel(payload)
        if path == "/tasks/streak-bonus":
            return self._route_tasks_streak_bonus(payload)
        if path == "/scheduler/trigger":
            return self._route_scheduler_trigger(payload)
        if path == "/scheduler/toggle":
            return self._route_scheduler_toggle(payload)
        if path == "/pricing/refresh":
            return self._route_pricing_refresh(payload)
        if path == "/logs/clear":
            return self._route_logs_clear(payload)
        if path == "/realm":
            return self._route_realm(payload)
        if path == "/accounts/checkin":
            return self._route_accounts_checkin(payload)
        if path == "/accounts/daily-chat":
            return self._route_accounts_daily_chat(payload)
        if path == "/accounts/daily-chat-web":
            return self._route_accounts_daily_chat_web(payload)
        if path == "/accounts/login/start":
            return self._route_accounts_login_start(payload)
        if path == "/accounts/login/cancel":
            return self._route_accounts_login_cancel(payload)
        if path == "/accounts/import/desktop":
            return self._route_accounts_import_desktop(payload)
        if path == "/accounts/refresh":
            return self._route_accounts_refresh(payload)
        if path == "/accounts/sync-profile":
            return self._route_accounts_sync_profile(payload)
        if path == "/accounts/test":
            return self._route_accounts_test(payload)
        if path == "/accounts/set":
            return self._route_accounts_set(payload)
        if path == "/accounts/product":
            return self._route_accounts_product(payload)
        if path == "/accounts/set-all":
            return self._route_accounts_set_all(payload)
        if path == "/accounts/delete":
            return self._route_accounts_delete(payload)
        if path == "/accounts/import":
            return self._route_accounts_import(payload)
        return self._error(404, "unknown account endpoint", "invalid_request_error")
    def _route_accounts_product(self, payload):
        """切换出站身分（cli <-> workbuddy），并即时回传结果。

        官方有两套身分、两条配额线。某条满了可以切到另一条继续用。
        """
        target = str(payload.get("product") or "").strip().lower()
        uid = payload.get("uid")
        realm = payload.get("realm")

        if target not in wb_identity.VALID_PRODUCTS:
            return self._error(400, "product must be 'workbuddy', 'vscode', or 'cli'",
                               "invalid_request_error")

        if uid:
            targets = [POOL.get(uid)]
        elif realm and realm != "all":
            targets = [a for a in POOL.accounts if a.realm == realm]
        else:
            targets = list(POOL.accounts)

        changed = []
        for account in targets:
            if account is None:
                continue
            try:
                if account.set_product(target):
                    # 立刻落盘：set_product() 只改记忆体，而面板上这一下是操作者
                    # 的明确选择，不能等到别的路径（refresh / 签到 / 查积分）刚好
                    # 存档才生效——切完就重启容器的人会白白丢掉这次切换。
                    try:
                        account.save(ACCOUNTS_DIR)
                    except Exception as exc:
                        log("product save failed for %s: %s" % (account.uid[:8], exc),
                            level="WARN")
                    changed.append(account.uid[:8])
                    log("account %s: 面板手動切換身分 -> %s"
                        % (account.uid[:8], target), level="INFO")
            except Exception as exc:
                log("product switch failed for %s: %s" % (account.uid[:8], exc),
                    level="WARN")

        return self._json(200, {
            "ok": True,
            "product": target,
            "changed": changed,
            "accounts": account_views(),
        })

    def _route_accounts_credits_fetch(self, payload):
        uid = payload.get("uid")
        realm = payload.get("realm")
        if uid:
            targets = [POOL.get(uid)]
        elif realm and realm != "all":
            targets = [a for a in POOL.accounts if a.realm == realm]
        else:
            targets = list(POOL.accounts)
        results = []
        for account in targets:
            if account is None:
                continue
            res = account.fetch_credits()
            results.append({"uid": account.uid, "ok": res.get("ok", False),
                            "credits": account.credits, "error": res.get("error", "")})
        return self._json(200, {"results": results, "accounts": account_views()})

    def _route_account_credits_detail(self, payload):
        if not self._authorized():
            return
        uid = payload.get("uid")
        refresh = payload.get("refresh", True)
        if not POOL:
            return self._json(200, {"ok": False, "error": "账号池未初始化"})
        account = POOL.get(uid) if uid else None
        if not account:
            return self._json(200, {"ok": False, "error": "未找到指定账号"})

        if refresh or not account.credits or not account.credits.get("packages"):
            res = account.fetch_credits()
            if not res.get("ok"):
                return self._json(200, {
                    "ok": False,
                    "uid": account.uid,
                    "nickname": account.nickname,
                    "realm": account.realm,
                    "credits": account.credits,
                    "error": res.get("error", "获取积分明细失败")
                })

        return self._json(200, {
            "ok": True,
            "uid": account.uid,
            "nickname": account.nickname,
            "realm": account.realm,
            "credits": account.credits
        })

    def _route_tasks_run(self, payload):
        if not POOL:
            return self._json(200, {"ok": False, "msg": "账号池不可用"})
        invalidate_tasks_cache()          # 跑完任务状态就变了，别再端旧快照
        uid = payload.get("uid")
        if uid and uid != "all":
            target = POOL.get(uid)
            if not target or target.realm != "cn":
                return self._json(200, {"ok": False, "msg": "未找到指定的国内版账号"})
            targets = [target]
        else:
            targets = [a for a in POOL.accounts if a.realm == "cn" and a.enabled]
        if not targets:
            return self._json(200, {"ok": False, "msg": "未找到已启用的国内版账号"})
        from wb_tasks import run_growth_tasks
        combined_logs = []
        total_credit = 0
        for i, acc in enumerate(targets):
            uid_str = acc.uid[:8] if acc.uid else "?"
            nick = acc.nickname or uid_str
            combined_logs.append(f"====== 正在为账号 [{nick} ({acc.uid})] 执行全自动成长任务 ({i+1}/{len(targets)}) ======")
            res = run_growth_tasks(acc, gap=1.0)
            # run_growth_tasks() reports its total as "earned_credit";
            # reading the old "credit_added" name silently summed zeros
            # and the dashboard always showed "+0 积分".
            total_credit += res.get("earned_credit") or 0
            for l in res.get("logs") or []:
                combined_logs.append(f"  {l}")
            if i < len(targets) - 1:
                time.sleep(1.5)
        combined_logs.append(f"====== 全部 {len(targets)} 个账号任务执行完毕，累计新增积分: +{total_credit} ======")
        add_log_entry(f"[任务福利] 批量成长任务执行完成，共 {len(targets)} 个账号，新增积分 +{total_credit}", tag="tasks")
        return self._json(200, {
            "ok": True,
            "credit_added": total_credit,
            "logs": combined_logs,
            "accounts_count": len(targets)
        })

    def _route_tasks_streak_bonus(self, payload):
        if not POOL:
            return self._json(200, {"ok": False, "msg": "账号池不可用"})
        invalidate_tasks_cache()
        uid = payload.get("uid")
        if uid and uid != "all":
            target = POOL.get(uid)
            if not target or target.realm != "cn":
                return self._json(200, {"ok": False, "msg": "未找到指定的国内版账号"})
            targets = [target]
        else:
            targets = [a for a in POOL.accounts if a.realm == "cn" and a.enabled]
        if not targets:
            return self._json(200, {"ok": False, "msg": "未找到已启用的国内版账号"})
        from wb_tasks import run_streak_bonus
        results = []
        for i, acc in enumerate(targets):
            uid_str = acc.uid[:8] if acc.uid else "?"
            nick = acc.nickname or uid_str
            res = run_streak_bonus(acc)
            results.append(f"{nick}: {res.get('msg')}")
            add_log_entry(f"[任务福利] 账号 [{nick}] 连登管家与抽奖: {res.get('msg')}", tag="tasks")
            if i < len(targets) - 1:
                time.sleep(1.0)
        return self._json(200, {"ok": True, "msg": chr(10).join(results)})

    def _route_tasks_travel(self, payload):
        if not POOL:
            return self._json(200, {"ok": False, "msg": "账号池不可用"})
        invalidate_tasks_cache()          # 同上：旅行会改任务/体力状态
        uid = payload.get("uid")
        if uid and uid != "all":
            target = POOL.get(uid)
            if not target or target.realm != "cn":
                return self._json(200, {"ok": False, "msg": "未找到指定的国内版账号"})
            targets = [target]
        else:
            targets = [a for a in POOL.accounts if a.realm == "cn" and a.enabled]
        if not targets:
            return self._json(200, {"ok": False, "msg": "未找到已启用的国内版账号"})
        from wb_tasks import do_cat_travel
        results = []
        for i, acc in enumerate(targets):
            uid_str = acc.uid[:8] if acc.uid else "?"
            nick = acc.nickname or uid_str
            res = do_cat_travel(acc)
            results.append({
                "uid": acc.uid,
                "nickname": nick,
                "action": res.get("action"),
                "msg": res.get("msg") or "",
                # do_cat_travel() returns the amount as "credit".
                "reward_credit": res.get("credit", 0)
            })
            if i < len(targets) - 1:
                time.sleep(1.0)
        summary_msg = chr(10).join([f"{r['nickname']}: {r['msg']}" for r in results])
        for r in results:
            add_log_entry(f"[任务福利] 账号 [{r['nickname']}] 猫猫旅行: {r['msg']}", tag="tasks")
        return self._json(200, {
            "ok": True,
            "results": results,
            "msg": summary_msg,
            "accounts_count": len(targets)
        })

    def _route_update_check(self):
        """Run one release check now, on the operator's explicit request.

        Inline rather than "started, poll /updates": it is a single bounded
        GitHub request, and the answer is what the button is for. `manual=True`
        ignores both the daily switch and the 24h window.
        """
        if not UPDATES:
            return self._json(200, {"ok": False, "msg": "更新检查未运行"})
        status = UPDATES.check(manual=True)
        status["ok"] = True
        return self._json(200, status)

    def _route_scheduler_trigger(self, payload):
        if SCHEDULER:
            return self._json(200, SCHEDULER.trigger_now())
        return self._json(200, {"ok": False, "msg": "调度器未初始化"})

    def _route_scheduler_toggle(self, payload):
        if SCHEDULER:
            SCHEDULER.enabled = not SCHEDULER.enabled
            SCHEDULER.log(f"用户切换调度器状态为: {'启用' if SCHEDULER.enabled else '暂停'}")
            return self._json(200, SCHEDULER.status())
        return self._json(200, {"ok": False, "msg": "调度器未初始化"})

    def _route_pricing_refresh(self, payload):
        # Fetching takes tens of seconds, so it runs on its own thread and the
        # panel polls /pricing for the outcome.
        if not PRICING:
            return self._json(200, {"ok": False, "msg": "价格刷新未运行"})
        if not wb_settings.pricing_enabled(ACCOUNTS_DIR):
            return self._json(200, {"ok": False, "msg": "价估算已关闭，请先启用"})
        threading.Thread(target=PRICING.run_once, daemon=True,
                         name="price-refresh-manual").start()
        out = PRICING.status()
        out["ok"] = True
        out["msg"] = "已开始抓取，稍候刷新查看结果"
        return self._json(200, out)

    def _route_logs_clear(self, payload):
        clear_logs()
        return self._json(200, {"ok": True})

    def _route_pricing_mapping(self, payload):
        """面板手填一条「hub 模型 → OpenRouter id」映射。

        写进 usage/pricing-overrides.json（运行期覆盖，不改源码里的
        OVERRIDES，也不随镜像升级丢失），随后立刻触发一次取价，让这条映射
        在几秒内生效。or_id 为空表示删除该映射。写入只认形状像 OpenRouter
        id 的值，并在快照里有该条目时予以确认（没有也接受，但要如实说明，
        因为上游随时可能刚上架而本地清单还没刷新）。
        """
        if PRICING is None:
            return self._json(200, {"ok": False, "msg": "价格刷新未运行"})
        if not wb_settings.pricing_enabled(ACCOUNTS_DIR):
            return self._json(200, {"ok": False, "msg": "价估算已关闭，请先启用"})
        model = str(payload.get("model") or "").strip()
        or_id = str(payload.get("or_id") or "").strip()
        if not model:
            return self._error(400, "model is required", "invalid_request_error")
        if or_id and not re.match(r"^[A-Za-z0-9._\-]+/[A-Za-z0-9._\-:]+$", or_id):
            return self._error(400, "or_id must look like 'vendor/model'",
                               "invalid_request_error")
        or_models, _by_norm = wb_pricing.live_index()
        known = bool(or_id) and or_id in (or_models or {})
        wb_pricing.save_runtime_override(model, or_id)
        PRICING.invalidate_gaps()
        threading.Thread(target=PRICING.run_once, daemon=True,
                         name="price-refresh-mapping").start()
        if not or_id:
            msg = "已删除 %s 的手填映射，下次取价起按自动匹配" % model
        elif known:
            msg = "已记录 %s → %s，正在重新取价" % (model, or_id)
        else:
            msg = ("已记录 %s → %s；本次快照里还没看到该条目，"
                   "取价后仍可能显示未定价" % (model, or_id))
        add_log_entry("[定价] 面板手填映射：%s → %s%s"
                      % (model, or_id or "(清除)",
                         "" if (known or not or_id) else "（快照暂无此条目）"),
                      tag="pricing")
        return self._json(200, {"ok": True, "model": model, "or_id": or_id,
                                "known": known, "msg": msg})

    def _route_realm(self, payload):
        # Changing the exit affects every key that is not realm-bound, so
        # it is an admin action: the panel session is required. GET /realm
        # stays open to API keys because it only reports the current exit.
        if not self._panel_ok():
            return self._error(403, "changing the upstream exit requires the "
                                    "panel session, not an API key",
                               "invalid_request_error")
        new_realm = payload.get("realm")
        if new_realm in ("intl", "cn"):
            save_persisted_realm(new_realm)
        return self._json(200, {"ok": True, "current": CURRENT_REALM, "persisted": True})

    def _route_accounts_checkin(self, payload):
        uid = payload.get("uid")
        invalidate_tasks_cache()          # 签到会改连续打卡状态，成长任务快照随之作废
        targets = [POOL.get(uid)] if uid else [a for a in (POOL.accounts if POOL else []) if a.realm == "cn"]
        results = []
        for account in targets:
            if account is None:
                continue
            res = account.checkin(trigger="manual")
            nick = account.nickname or account.uid[:8]
            add_log_entry(f"[任务福利] 账号 [{nick}] 每日签到: {res.get('msg') if res.get('ok') else res.get('error')}",
                          level="INFO" if res.get("ok") else "WARN", tag="tasks")
            results.append({"uid": account.uid, "nickname": account.nickname, **res})
        return self._json(200, {"results": results, "accounts": account_views()})

    def _route_accounts_daily_chat(self, payload):
        uid = payload.get("uid")
        if uid:
            targets = [POOL.get(uid)]
        else:
            targets = [a for a in POOL.accounts if a.realm == "intl" and a.enabled]
        results = []
        for account in targets:
            if account is None:
                continue
            res = account.daily_chat(trigger="manual")
            # 与网页通道打卡那条一样把结果写进运行日志：msg 里带着网页通道是
            # completed 还是失败原因，面板上光看 toast 的「成功」看不出来。
            nick = account.nickname or account.uid[:8]
            log(f"[任务福利] 账号 [{nick}] 每日活跃打卡: {res.get('msg') if res.get('ok') else res.get('error')}",
                level="INFO" if res.get("ok") else "WARN", tag="tasks")
            results.append({"uid": account.uid, "nickname": account.nickname, **res})
        return self._json(200, {"results": results, "accounts": account_views()})

    def _route_accounts_daily_chat_web(self, payload):
        """网页通道打卡：只建网页端会话，不发桌面端那条轻量对话。

        手动触发用。刻意不写 lastDailyChat——那是「今天已经打过卡」的闸门，
        手动补一次不该让定时巡检跳过当天的正常流程。
        """
        uid = payload.get("uid")
        if uid:
            targets = [POOL.get(uid)]
        else:
            targets = [a for a in POOL.accounts if a.realm == "intl" and a.enabled]
        results = []
        for account in targets:
            if account is None:
                continue
            res = account.daily_chat_web(trigger="manual")
            nick = account.nickname or account.uid[:8]
            log(f"[任务福利] 账号 [{nick}] 网页通道打卡: {res.get('conversation') if res.get('ok') else res.get('error')}",
                level="INFO" if res.get("ok") else "WARN", tag="tasks")
            results.append({"uid": account.uid, "nickname": account.nickname, **res})
        return self._json(200, {"results": results, "accounts": account_views()})

    def _route_accounts_login_start(self, payload):
        platform = payload.get("platform") or "CLI"
        target_realm = payload.get("realm") or CURRENT_REALM
        try:
            started = POOL.start_login(realm=target_realm, platform=platform)
        except Exception as exc:
            return self._error(502, "could not start login: %s" % exc)
        log("oauth login started (realm=%s, platform=%s, state=%s)" % (target_realm, platform, started["state"][:8]))
        return self._json(200, started)

    def _route_accounts_login_cancel(self, payload):
        state = payload.get("state") or ""
        return self._json(200, {"cancelled": POOL.cancel_login(state)})

    def _route_accounts_import_desktop(self, payload):
        # Two ways to call this:
        #   {}                     -> scan only (read-only, nothing imported)
        #   {"path": "..."}        -> import that credential
        #   {"all": true}          -> import everything the scan found
        #   {"recoverKey": true}   -> recover the at-rest key from the desktop client
        target_path = payload.get("path")
        if payload.get("recoverKey"):
            try:
                key = wb_accounts.desktop_atrest_key(
                    force=bool(payload.get("force")), log=lambda m: log(m, tag="accounts"))
            except Exception as exc:
                log("at-rest key recovery failed: %s" % exc, level="WARN", tag="accounts")
                return self._json(200, {"ok": False, "msg": str(exc),
                                        "atrest": wb_atrest.status()})
            return self._json(200, {
                "ok": True,
                "keyId": wb_atrest.derive_key_id(key),
                "msg": "已从桌面客户端进程回收密钥（只留在内存里）",
                "atrest": wb_atrest.status(),
            })
        if payload.get("forgetKey"):
            wb_atrest.forget_key()
            return self._json(200, {"ok": True, "atrest": wb_atrest.status()})
        if target_path:
            realm = payload.get("realm")
            try:
                account = POOL.import_desktop_credential(
                    path=target_path, realm=realm, source="desktop-app")
            except Exception as exc:
                return self._error(400, "import failed: %s" % exc)
            log("imported %s from %s (user confirmed)" % (account.uid[:8], os.path.basename(target_path)))
            return self._json(200, {
                "imported": [account.public()],
                "accounts": account_views(),
            })
        if payload.get("all"):
            imported = import_desktop_accounts(payload.get("realm"))
            return self._json(200, {
                "imported": [a.public() for a in imported],
                "accounts": account_views(),
            })
        return self._json(200, {
            "detected": desktop_credential_scan(),
            "accounts": account_views(),
            "pool_uids": [a.uid for a in POOL.accounts],
            "atrest": wb_atrest.status(),
        })

    def _route_accounts_refresh(self, payload):
        uid = payload.get("uid")
        targets = [POOL.get(uid)] if uid else list(POOL.accounts)
        results = []
        for account in targets:
            if account is None:
                continue
            ok = account.refresh()
            account.save(ACCOUNTS_DIR)
            results.append({"uid": account.uid, "ok": ok, "error": account.last_error})
        return self._json(200, {"results": results})

    def _route_accounts_sync_profile(self, payload):
        """Manual nickname refresh from the web console.

        Opt-in by nature: only runs when the operator asks for it. The
        upstream response carries phone numbers and other personal fields;
        wb_accounts.fetch_account_profile parses only uid/nickname.
        """
        uid = str(payload.get("uid") or "").strip()
        realm = payload.get("realm")
        if uid:
            targets = [POOL.get(uid)]
        elif realm and realm != "all":
            targets = [a for a in POOL.accounts if a.realm == realm]
        else:
            targets = list(POOL.accounts)
        updated = []
        failed = []
        for account in targets:
            if account is None:
                continue
            try:
                nickname = account.sync_nickname()
            except Exception as exc:
                failed.append({"uid": account.uid, "error": str(exc)[:200]})
                log("profile sync failed for %s: %s" % (account.uid[:8], exc),
                    level="WARN")
                continue
            updated.append({"uid": account.uid, "nickname": nickname})
            log("account %s: nickname synced from the web console"
                % account.uid[:8], level="INFO")
        return self._json(200, {"updated": updated, "failed": failed,
                                "accounts": account_views()})

    def _route_accounts_test(self, payload):
        uid = payload.get("uid")
        if not uid:
            return self._error(400, "uid required")
        account = POOL.get(uid)
        if not account:
            return self._error(404, "no such account")
        test_model = payload.get("model") or "deepseek-v4.1-flash"
        cfg = wb_accounts.get_realm_config(account.realm)
        chat_url = account.chat_base_url() + CHAT_PATH
        test_body = {
            "model": test_model,
            "messages": [{"role": "user", "content": "hi"}],
            "stream": False,
        }
        forwarded = build_upstream_body(test_body)
        body = json.dumps(forwarded, ensure_ascii=False).encode("utf-8")
        req = urllib.request.Request(
            chat_url, data=body, method="POST",
            headers=account.headers(purpose="chat")
        )
        t0 = time.time()
        try:
            with wb_accounts.urlopen(req, timeout=30, proxy=account.proxy) as resp:
                chat_obj = aggregate_stream(resp, test_model, None)
                wall_ms = int((time.time() - t0) * 1000)
                choices = chat_obj.get("choices") or []
                msg = (choices[0].get("message") or {}) if choices else {}
                reply_text = (msg.get("content") or msg.get("reasoning_content") or "OK").strip()
                if len(reply_text) > 80:
                    reply_text = reply_text[:77] + "..."
                # A served request, so it takes the semantic success path. That
                # clears the account-level streak/breaker/degrade counters, which
                # clear_error() never touched: a green panel test used to leave
                # soft_streak standing, and the next 429 resumed the ladder from
                # it. Only the tested model's cooldown is dropped here - sibling
                # models keep theirs, and a 402 balance park keeps its own lift
                # path (revive_balance_cooldown on a credits refresh).
                account.note_success(model=test_model)
                log(f"account test: uid={account.uid[:8]} model={test_model} wall={wall_ms}ms ok=True", tag="accounts")
                return self._json(200, {
                    "ok": True,
                    "uid": account.uid,
                    "model": test_model,
                    "elapsed_ms": wall_ms,
                    "reply": reply_text,
                })
        except urllib.error.HTTPError as exc:
            wall_ms = int((time.time() - t0) * 1000)
            detail = exc.read(400).decode("utf-8", "replace")
            # The label keeps its 80-char cut; `detail` rides along untouched so the
            # panel can show the whole upstream body on hover.
            account.note_error(f"HTTP {exc.code}: {detail[:80]}", cooldown=60, detail=detail)
            log(f"account test: uid={account.uid[:8]} model={test_model} wall={wall_ms}ms error={exc.code}", level="WARN", tag="accounts")
            return self._json(200, {
                "ok": False,
                "uid": account.uid,
                "status": exc.code,
                "error": f"HTTP {exc.code}: {detail[:150]}",
                "elapsed_ms": wall_ms,
            })
        except Exception as exc:
            wall_ms = int((time.time() - t0) * 1000)
            account.note_error(str(exc)[:80], cooldown=60)
            log(f"account test: uid={account.uid[:8]} model={test_model} wall={wall_ms}ms exc={exc}", level="WARN", tag="accounts")
            return self._json(200, {
                "ok": False,
                "uid": account.uid,
                "status": 500,
                "error": str(exc),
                "elapsed_ms": wall_ms,
            })

    def _route_accounts_set(self, payload):
        uid = payload.get("uid")
        if not uid:
            return self._error(400, "uid required")
        # Each field is applied on its own so a caller can change one thing
        # without restating the others; at least one must be present.
        updated = None
        if "proxySlot" in payload:
            updated = POOL.set_proxy_slot(uid, payload.get("proxySlot"))
            if updated is None:
                return self._error(404, "no such account")
            log("account %s proxy slot set to %s"
                % (uid[:8], updated.get("proxySlot") or "(direct)"))
        if "proxy" in payload:
            updated = POOL.set_proxy(uid, payload.get("proxy"))
            if updated is None:
                return self._error(404, "no such account")
            log("account %s proxy set to %s"
                % (uid[:8], updated.get("proxy") or "(direct)"))
        if "enabled" in payload:
            updated = POOL.set_enabled(uid, bool(payload.get("enabled")))
            if updated is None:
                return self._error(404, "no such account")
            log("account %s %s"
                % (uid[:8], "enabled" if payload.get("enabled") else "disabled"))
        if "weight" in payload:
            try:
                weight = int(payload.get("weight"))
            except (TypeError, ValueError):
                return self._error(400, "weight must be an integer between 1 and 100")
            if not 1 <= weight <= 100:
                return self._error(400, "weight out of range (1-100)")
            updated = POOL.set_weight(uid, weight)
            if updated is None:
                return self._error(404, "no such account")
            log("account %s weight set to %d" % (uid[:8], weight))
        if "note" in payload:
            updated = POOL.set_note(uid, payload.get("note"))
            if updated is None:
                return self._error(404, "no such account")
            log("account %s note set (%d chars)"
                % (uid[:8], len(updated.get("note") or "")))
        if updated is None:
            return self._error(
                400,
                "nothing to update: pass 'enabled', 'proxy', 'proxySlot', 'weight' or 'note'"
            )
        return self._json(200, {"account": updated})

    def _route_accounts_set_all(self, payload):
        POOL.set_all_enabled(bool(payload.get("enabled")))
        return self._json(200, {"accounts": account_views()})

    def _route_accounts_delete(self, payload):
        uid = payload.get("uid")
        if not uid:
            return self._error(400, "uid required")
        removed = POOL.remove(uid)
        log("account %s deleted" % uid[:8])
        return self._json(200, {"deleted": removed, "accounts": account_views()})

    def _route_accounts_import(self, payload):
        # Import a previously exported document (or any hand-written list
        # of accounts). Body shapes accepted, see wb_accounts._coerce_account_rows:
        #   {"format":"workbuddy-accounts","accounts":[...]}   <- our export
        #   [...]                                              <- bare list
        #   {"accessToken": ...}                               <- single account
        #   {"account":{...},"auth":{...}}                     <- desktop credential
        #
        # Options:
        #   dryRun    (bool) - validate and report, write nothing
        #   overwrite (bool) - replace accounts whose uid already exists
        #   realm     ("intl"|"cn") - force a realm instead of detecting it
        #
        # `data` carries the document. It is preferred over the bare body so
        # the body can also hold the options above.
        blob = payload.get("data") if "data" in payload else payload
        if not isinstance(blob, (dict, list)):
            return self._error(400, "the document must be a JSON object or array",
                               "invalid_request_error")
        rows, problem = wb_accounts._coerce_account_rows(blob)
        if problem:
            return self._error(400, "cannot read the document: %s" % problem,
                               "invalid_request_error")
        dry_run = bool(payload.get("dryRun"))
        overwrite = bool(payload.get("overwrite"))
        forced_realm = (payload.get("realm") or "").strip().lower() or None
        if forced_realm and forced_realm not in ("intl", "cn"):
            return self._error(400, "realm must be intl or cn", "invalid_request_error")
        if dry_run:
            # Validate every row without touching the pool so the caller can
            # see exactly what an import would do before committing to it.
            # Shares its rules with the real import, so the preview cannot
            # disagree with what would actually happen.
            return self._json(200, {
                "dryRun": True,
                "count": len(rows),
                "result": POOL.preview_import_rows(rows, realm=forced_realm, overwrite=overwrite),
                "accounts": account_views(),
            })
        report = POOL.import_rows(rows, realm=forced_realm, overwrite=overwrite)
        log("account import: %d added, %d updated, %d skipped, %d invalid"
            % (len(report["added"]), len(report["updated"]),
               len(report["skipped"]), len(report["invalid"])))
        return self._json(200, {
            "count": len(rows),
            "result": report,
            "accounts": account_views(),
        })

    def _handle_responses(self, payload):
        """Serve /v1/responses by translating to chat completions upstream."""
        # The gateway is stateless: it keeps no store of previous responses,
        # so it cannot replay a prior turn. Silently ignoring the field would
        # answer a follow-up as if it were a fresh conversation - the client
        # gets a normal-looking reply with the context missing. Say so instead.
        # 拒绝 namespace 工具，逼 Codex fallback 成 flat 工具清单。
        # 不这样做的话，MCP／外挂工具全部会被 app 判定为不可执行。
        if payload.get("previous_response_id"):
            return self._error(
                400,
                "previous_response_id is not supported: this gateway does not "
                "store response state. Send the full conversation in 'input' "
                "instead, or use a stateless client.",
                "invalid_request_error")
        session_key = extract_session_key(self.headers, payload)
        # 工具可能在顶层，也可能整份在 input 的 additional_tools 条目里。回程
        # 的两处取值（custom_names 判定条目类型、request_meta 回显工具清单）
        # 必须和 responses_to_chat() 出站合并后的清单一致，否则两边看到的
        # 是不同工具集合，回程类型判错 → 客户端匹配不到执行器。
        all_tools = payload.get("tools")
        all_tools = list(all_tools) if isinstance(all_tools, list) else []
        all_tools.extend(additional_tools_from_input(payload))
        custom_names = custom_tool_names(all_tools)
        chat_req = responses_to_chat(payload)
        ns_map = chat_req.pop("_namespace_map", None)
        # Echo these back on the response object; see chat_to_response.
        request_meta = {
            "tools": all_tools,
            "tool_choice": payload.get("tool_choice", "auto"),
            "parallel_tool_calls": payload.get("parallel_tool_calls", True),
        }
        model = payload.get("model") or "deepseek-v4.1-flash"
        want_stream = bool(payload.get("stream"))
        t_start = time.time()
        fp = prompt_fingerprint(chat_req.get("messages"))
        log(
            "responses: model=%s stream=%s msgs=%d effort=%r tools=%d/%d custom_tools=%s"
            % (model, want_stream, len(chat_req.get("messages") or []),
               chat_req.get("reasoning_effort"),
               len(chat_req.get("tools") or []), len(all_tools),
               sorted(custom_names) or "-")
        )
        try:
            req_realm = self._request_realm() or CURRENT_REALM
            blocked = self._cross_realm_error(chat_req.get("model"), req_realm)
            if blocked:
                return self._error(400, blocked, "invalid_request_error")
            banned = self._banned_model_error(chat_req.get("model"))
            if banned:
                return self._error(400, banned, "invalid_request_error")
            key_blocked = self._key_model_error(chat_req.get("model"))
            if key_blocked:
                return self._error(400, key_blocked, "invalid_request_error")
            upstream, account, effort = open_upstream(
                chat_req, session_key=session_key, target_realm=req_realm)
        except ContentRejected as exc:
            record_error(model, 403, exc.detail[:200],
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._error(403, "upstream 403: %s" % (exc.detail or "content rejected"),
                               "invalid_request_error")
        except RateLimited as exc:
            t = time.time() - t_start
            record_error(model, 429, exc.detail[:200], elapsed_ms=int(t * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._rate_limited(exc)
        except urllib.error.HTTPError as exc:
            detail = exc.read(600).decode("utf-8", "replace")
            record_error(model, exc.code, detail,
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._error(exc.code, f"upstream {exc.code}: {detail}")
        except Exception as exc:
            message = str(exc)
            record_error(model, 502, message,
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            if message.startswith("no usable account"):
                return self._error(503, message +
                                   " - add or enable one at the dashboard (/)")
            return self._error(502, f"upstream unreachable: {exc}")
        with upstream:
            if want_stream:
                return self._responses_stream_response(
                    upstream, model, custom_names, request_meta, fp, account, t_start, ns_map,
                    base_body=chat_req, session_key=session_key, realm=req_realm,
                    effort=effort)
            return self._responses_nonstream_response(
                upstream, model, custom_names, request_meta, fp, account, t_start, ns_map,
                base_body=chat_req, session_key=session_key, realm=req_realm,
                effort=effort)

    def _responses_stream_response(self, upstream, model, custom_names, request_meta, fp, account, t_start, namespace_map=None, base_body=None, session_key=None, realm=None, effort=None):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        holder = {"usage": None, "custom_names": custom_names,
                  "request_meta": request_meta,
                  "namespace_map": namespace_map,
                  "base_body": base_body,
                  "base_messages": (base_body or {}).get("messages"),
                  "session_key": session_key,
                  "realm": realm}
        first_ms = None
        try:
            # 一轮跑完如果模型要的是 web_search / web_fetch，就由反代
            # 执行、把结果喂回去再跑一轮。客户端从头到尾只看到一则连续的回应。
            rounds = 0
            total_usage = None
            while True:
                holder.pop("internal_calls", None)
                holder.pop("suppress_completion", None)
                holder["suppress_lifecycle"] = rounds > 0
                for frame in stream_responses_events(upstream, model, holder):
                    if first_ms is None:
                        first_ms = int((time.time() - t_start) * 1000)
                    self.wfile.write(clean_responses_frame(frame))
                    self.wfile.flush()
                # 每一轮的 token 都是真的花掉的，记帐要加总
                total_usage = sum_usage(total_usage, holder.get("usage"))
                internal = holder.get("internal_calls") or []
                if not internal:
                    break
                rounds += 1
                # 用完就收回工具，让模型自己收尾；这里不合成任何事件。
                give_up = rounds > wb_webtools.MAX_WEB_ROUNDS
                try:
                    upstream.close()
                except Exception:
                    pass
                upstream, account, _ = follow_up_with_tool_results(
                    internal, holder, model, session_key, t_start, drop_tools=give_up)
            if total_usage:
                holder["usage"] = total_usage
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            wall = int((time.time() - t_start) * 1000)
            record_usage(model, holder.get("usage"), stream=True, elapsed_ms=wall,
                         ttft_ms=first_ms,
                         gen_ms=(wall - first_ms) if first_ms is not None else None,
                         fp=fp, account=account.uid,
                         outcome="client_aborted", key=self._key_id(), effort=effort)
            return
        except Exception as exc:
            wall = int((time.time() - t_start) * 1000)
            record_error(model, 502, "stream aborted: %s" % exc,
                         elapsed_ms=wall, account=account.uid,
                         usage=holder.get("usage"), stream=True,
                         ttft_ms=first_ms,
                         gen_ms=(wall - first_ms) if first_ms is not None else None,
                         fp=fp, outcome="upstream_aborted", key=self._key_id())
            try:
                self.wfile.write(b"data: [DONE]" + bytes([10, 10]))
                self.wfile.flush()
            except Exception:
                pass
            return
        finally:
            # 代跑多轮时 upstream 会被换掉，外层的 with 只认得最开始那一条，
            # 最后一条要在这里收掉。
            try:
                upstream.close()
            except Exception:
                pass
        wall = int((time.time() - t_start) * 1000)
        record_usage(model, holder.get("usage"), stream=True, elapsed_ms=wall,
                     ttft_ms=first_ms,
                     gen_ms=(wall - first_ms) if first_ms is not None else None,
                     fp=fp, account=account.uid, key=self._key_id(), effort=effort)
        return

    def _responses_nonstream_response(self, upstream, model, custom_names, request_meta, fp, account, t_start, namespace_map=None, base_body=None, session_key=None, realm=None, effort=None):
        # 跟串流那条一样：客户端宣告 web_search / web_fetch 时由反代代跑。
        # 中间那几轮对客户端不可见，最后才组成一个 Responses 物件回传；不这样
        # 做的话 web_search 的 function_call 会直接漏给客户端，客户端只会回
        # 一句 unsupported call。
        sources = []
        rounds = 0
        # 开关关闭时不拦同名呼叫：那是客户端自己的工具。
        web_tools = web_tools_active(base_body)
        while True:
            try:
                chat_obj = aggregate_stream(upstream, model, None)
            except Exception as exc:
                record_error(model, 502, str(exc),
                             elapsed_ms=int((time.time() - t_start) * 1000),
                             account=account.uid, key=self._key_id())
                return self._error(502, f"upstream stream error: {exc}")
            calls = internal_calls_from_chat(chat_obj, web_tools=web_tools)
            if not calls:
                break
            rounds += 1
            give_up = rounds > wb_webtools.MAX_WEB_ROUNDS
            try:
                upstream.close()
            except Exception:
                pass
            holder = {"base_messages": (base_body or {}).get("messages"),
                      "base_body": base_body, "realm": realm,
                      "web_sources": sources}
            try:
                upstream, account, _ = follow_up_with_tool_results(
                    calls, holder, model, session_key, t_start, drop_tools=give_up)
            except Exception as exc:
                record_error(model, 502, "web tool follow-up failed: %s" % exc,
                             elapsed_ms=int((time.time() - t_start) * 1000),
                             account=account.uid, key=self._key_id())
                return self._error(502, "web tool follow-up failed: %s" % exc)
            sources = holder.get("web_sources") or sources
        wall = int((time.time() - t_start) * 1000)
        result = chat_to_response(chat_obj, model, custom_names, request_meta, namespace_map,
                                  sources=sources)
        record_usage(model, chat_obj.get("usage"), stream=False, elapsed_ms=wall, fp=fp,
                     account=account.uid, key=self._key_id(), effort=effort)
        return self._json(200, result)

    def _handle_messages_count_tokens(self, payload):
        """Best-effort Anthropic count_tokens endpoint.

        The WorkBuddy upstream has no Anthropic tokenizer. This estimate uses
        the same CJK-aware estimator as the gateway's own accounting and keeps
        the native response shape; it is intentionally not presented as an
        official model-token count.
        """
        try:
            chat = messages_to_chat(payload)
        except ValueError as exc:
            return self._anthropic_error(400, str(exc), "invalid_request_error")
        return self._json(200, {"input_tokens": _anthropic_estimate_chat_tokens(chat)})

    def _handle_messages(self, payload):
        """Serve an Anthropic Messages request through the chat pipeline."""
        try:
            chat_req = messages_to_chat(payload)
        except ValueError as exc:
            return self._anthropic_error(400, str(exc), "invalid_request_error")
        model = chat_req.get("model") or "unknown"
        want_stream = bool(chat_req.get("stream"))
        session_key = extract_session_key(self.headers, chat_req)
        t_start = time.time()
        fp = prompt_fingerprint(chat_req.get("messages"))
        try:
            client_ip = self.client_address[0] if self.client_address else ""
        except Exception:
            client_ip = ""
        log("messages: model=%s stream=%s msgs=%d effort=%r tools=%d"
            % (model, want_stream, len(chat_req.get("messages") or []),
               chat_req.get("reasoning_effort"), len(chat_req.get("tools") or [])))
        try:
            req_realm = self._request_realm() or CURRENT_REALM
            blocked = self._cross_realm_error(chat_req.get("model"), req_realm)
            if blocked:
                return self._anthropic_error(400, blocked, "invalid_request_error")
            banned = self._banned_model_error(chat_req.get("model"))
            if banned:
                return self._anthropic_error(400, banned, "invalid_request_error")
            key_blocked = self._key_model_error(chat_req.get("model"))
            if key_blocked:
                return self._anthropic_error(400, key_blocked, "invalid_request_error")
            upstream, account, effort = open_upstream(
                chat_req, session_key=session_key, target_realm=req_realm)
        except ContentRejected as exc:
            record_error(model, 403, exc.detail[:200],
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._anthropic_error(403, "upstream 403: %s" %
                                          (exc.detail or "content rejected"),
                                          "permission_error")
        except RateLimited as exc:
            record_error(model, 429, exc.detail[:200],
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._anthropic_rate_limited(exc)
        except urllib.error.HTTPError as exc:
            try:
                detail = exc.read(600).decode("utf-8", "replace")
            except Exception:
                detail = str(exc)
            record_error(model, exc.code, detail,
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._anthropic_error(exc.code,
                                         "upstream %s: %s" % (exc.code, detail))
        except Exception as exc:
            message = str(exc)
            low = message.lower()
            status = 503 if ("no account" in low or "no enabled account" in low) else 502
            record_error(model, status, message,
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            if status == 503:
                message += " - add or enable one at the dashboard (/)"
            return self._anthropic_error(status, message)
        with upstream:
            if want_stream:
                return self._messages_stream_response(
                    upstream, model, fp, account, t_start,
                    base_body=chat_req, session_key=session_key,
                    realm=req_realm, effort=effort)
            return self._messages_nonstream_response(
                upstream, model, fp, account, t_start,
                base_body=chat_req, session_key=session_key,
                realm=req_realm, effort=effort)

    def _messages_nonstream_response(self, upstream, model, fp, account, t_start,
                                     base_body=None, session_key=None, realm=None,
                                     effort=None):
        try:
            chat_obj = aggregate_stream(upstream, model, None)
            result = chat_to_messages(chat_obj)
            wall = int((time.time() - t_start) * 1000)
            record_usage(model, chat_obj.get("usage"), stream=False, elapsed_ms=wall,
                         fp=fp, account=account.uid, key=self._key_id(), effort=effort)
            return self._json(200, result)
        except Exception as exc:
            wall = int((time.time() - t_start) * 1000)
            record_error(model, 502, "messages upstream error: %s" % exc,
                         elapsed_ms=wall, account=account.uid, key=self._key_id())
            return self._anthropic_error(502, "upstream stream error: %s" % exc)
        finally:
            try:
                upstream.close()
            except Exception:
                pass

    def _messages_stream_response(self, upstream, model, fp, account, t_start,
                                  base_body=None, session_key=None, realm=None,
                                  session_meta=None, effort=None):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        if cors_origin_allowed(self.path):
            self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        holder = {"usage": None}
        first_ms = None
        try:
            for frame in stream_messages_events(upstream, model, holder):
                if first_ms is None:
                    first_ms = int((time.time() - t_start) * 1000)
                self.wfile.write(frame)
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            wall = int((time.time() - t_start) * 1000)
            record_usage(model, holder.get("usage"), stream=True, elapsed_ms=wall,
                         ttft_ms=first_ms,
                         gen_ms=(wall - first_ms) if first_ms is not None else None,
                         fp=fp, account=account.uid,
                         outcome="client_aborted", key=self._key_id(), effort=effort)
            return
        except Exception as exc:
            wall = int((time.time() - t_start) * 1000)
            record_error(model, 502, "messages stream aborted: %s" % exc,
                         elapsed_ms=wall, account=account.uid,
                         usage=holder.get("usage"), stream=True,
                         ttft_ms=first_ms, fp=fp, outcome="upstream_aborted",
                         key=self._key_id())
            try:
                self.wfile.write(anthropic_sse_frame("error", {
                    "error": {"type": "api_error", "message": str(exc)}}))
                self.wfile.flush()
            except Exception:
                pass
            return
        finally:
            try:
                upstream.close()
            except Exception:
                pass
        wall = int((time.time() - t_start) * 1000)
        record_usage(model, holder.get("usage"), stream=True, elapsed_ms=wall,
                     ttft_ms=first_ms,
                     gen_ms=(wall - first_ms) if first_ms is not None else None,
                     fp=fp, account=account.uid, key=self._key_id(), effort=effort)
        return

    def do_POST(self):
        path = self.path.split("?")[0]
        if path == "/settings/save":
            if not self._panel_ok():
                return self._error(401, "panel password required", "invalid_request_error")
            return self._handle_settings_save()
        if path == "/updates/check":
            # Panel-only management write, like the pricing ones. No body to
            # read: the action takes no parameters, so a button that posts
            # nothing must not be answered with "invalid JSON body".
            if not self._panel_ok():
                return self._error(401, "panel password required", "invalid_request_error")
            return self._route_update_check()
        if path in ("/pricing/refresh", "/pricing/mapping"):
            # Panel-only management writes (the pricing table is the panel's
            # own view of the estimate). Registered here because the generic
            # dispatcher below only routes chat and account paths - POST
            # /pricing/refresh used to 404, which made the panel's "立即取价"
            # button fail silently behind its toast.
            if not self._panel_ok():
                return self._error(401, "panel password required", "invalid_request_error")
            payload = self._payload_or_error()
            if payload is None:
                return
            if path == "/pricing/mapping":
                return self._route_pricing_mapping(payload)
            return self._route_pricing_refresh(payload)
        if path.startswith("/proxy/"):
            if not self._panel_ok():
                return self._error(
                    401, "panel password required", "invalid_request_error"
                )
            payload = self._payload_or_error()
            if payload is None:
                return
            return self._handle_proxy_slots(path, payload)
        if path in ("/agents/apply", "/agents/restore"):
            if not self._panel_ok():
                return self._error(
                    401, "panel password required", "invalid_request_error"
                )
            if not self._agents_client_allowed():
                return self._error(
                    403,
                    "agent config is only available from the machine running the gateway",
                    "invalid_request_error",
                )
            payload = self._payload_or_error()
            if payload is None:
                return
            if path == "/agents/apply":
                return self._handle_agents_apply(payload)
            return self._handle_agents_restore(payload)
        if path in ("/panel/login", "/panel/logout", "/panel/password"):
            return self._handle_panel(path)
        if self._is_panel_route(path) and not self._panel_ok():
            return self._error(401, "panel password required", "invalid_request_error")
        is_messages_route = path in (
            "/v1/messages", "/messages",
            "/v1/messages/count_tokens", "/messages/count_tokens",
        )
        is_account_route = (
            path.startswith("/accounts/")
            or path == "/realm"
            or path.startswith("/tasks")
            or path.startswith("/scheduler")
            or path.startswith("/logs")
        )
        if not is_account_route and not is_messages_route and path not in (
                "/v1/chat/completions", "/chat/completions",
                "/v1/completions", "/completions",
                "/v1/responses", "/responses"):
            return self._error(404, "not found", "invalid_request_error")
        if is_messages_route:
            if not self._key_ok():
                return self._anthropic_error(
                    401, "missing or invalid API key", "authentication_error")
        elif not self._authorized():
            return
        payload = self._payload_or_error(allow_list=(path == "/accounts/import"))
        if payload is None:
            return
        if is_account_route:
            return self._handle_accounts(path, payload)
        if is_messages_route:
            if path.endswith("/count_tokens"):
                return self._handle_messages_count_tokens(payload)
            if not _chat_slots.acquire(timeout=CHAT_SLOT_WAIT_SECONDS):
                message = ("gateway is at its concurrent chat limit "
                           "(%d in flight); retry shortly" % MAX_CONCURRENT_CHAT)
                record_error(payload.get("model") or "unknown", 503, message,
                             stream=payload.get("stream"), key=self._key_id())
                return self._anthropic_error(503, message, "overloaded_error")
            try:
                return self._handle_messages(payload)
            finally:
                _chat_slots.release()
        # Both OpenAI-shaped routes below can hold a thread for up to 600s.
        # Take a slot for the duration; release it in finally so every early
        # return (including client disconnects) gives the slot back.
        if not _chat_slots.acquire(timeout=CHAT_SLOT_WAIT_SECONDS):
            return self._error(503, "gateway is at its concurrent chat limit "
                                    "(%d in flight); retry shortly" % MAX_CONCURRENT_CHAT)
        try:
            return self._dispatch_chat_post(path, payload)
        finally:
            _chat_slots.release()

    def _dispatch_chat_post(self, path, payload):
        # 先挡背景请求：Codex 自己发的（记忆整理／环境建议／自动复核）
        # 不算「使用者实际使用」，一律本地拒绝，不碰上游。
        if BLOCK_BACKGROUND_REQUESTS:
            reason = background_request_reason(payload)
            if reason:
                try:
                    log("background request blocked: model=%s trigger=(%s)"
                        % (payload.get("model"), reason), level="INFO")
                except Exception:
                    pass
                return self._error(400, background_request_message(reason),
                                   "invalid_request_error")
        if path in ("/v1/responses", "/responses"):
            return self._handle_responses(payload)
        # Diagnostics: what the client actually asked for, and what we forward.
        # Only the knobs that change behaviour are logged - never message text.
        forwarded = build_upstream_body(payload)
        given = client_effort_of(payload) or payload.get("reasoning") \
            or payload.get("thinking") or payload.get("enable_thinking")
        log(
            "chat: model=%s client_effort=%r -> upstream_effort=%r stream=%s msgs=%d"
            % (
                payload.get("model"),
                given,
                upstream_effort_of(forwarded, payload.get("model")),
                bool(payload.get("stream")),
                len(forwarded.get("messages") or []),
            )
        )
        session_key = extract_session_key(self.headers, payload)
        fp = prompt_fingerprint(forwarded.get("messages"))
        want_stream = bool(payload.get("stream"))
        model = payload.get("model") or "hy4-preview"
        t_start = time.time()
        try:
            req_realm = self._request_realm() or CURRENT_REALM
            blocked = self._cross_realm_error(payload.get("model"), req_realm)
            if blocked:
                return self._error(400, blocked, "invalid_request_error")
            banned = self._banned_model_error(payload.get("model"))
            if banned:
                return self._error(400, banned, "invalid_request_error")
            key_blocked = self._key_model_error(payload.get("model"))
            if key_blocked:
                return self._error(400, key_blocked, "invalid_request_error")
            upstream, account, effort = open_upstream(
                payload, session_key=session_key, target_realm=req_realm,
                prebuilt_body=forwarded)
        except ContentRejected as exc:
            record_error(model, 403, exc.detail[:200],
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._error(403, "upstream 403: %s" % (exc.detail or "content rejected"),
                               "invalid_request_error")
        except RateLimited as exc:
            record_error(model, 429, exc.detail[:200],
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._rate_limited(exc)
        except urllib.error.HTTPError as exc:
            detail = exc.read(600).decode("utf-8", "replace")
            record_error(model, exc.code, detail,
                         elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            return self._error(exc.code, f"upstream {exc.code}: {detail}")
        except Exception as exc:
            message = str(exc)
            record_error(model, 502, message, elapsed_ms=int((time.time() - t_start) * 1000),
                         account=getattr(exc, "account_uid", None),
                         key=self._key_id())
            if message.startswith("no usable account"):
                # Only a genuinely empty/cooling pool is a 503. A throttled model
                # is reported as 429 by _rate_limited above instead.
                return self._error(503, message +
                                   " - add or enable one at the dashboard (/)")
            return self._error(502, f"upstream unreachable: {exc}")
        with upstream:
            if want_stream:
                return self._chat_stream_response(
                    upstream, model, fp, account, t_start, effort=effort)
            return self._chat_nonstream_response(
                upstream, model, fp, account, t_start, effort=effort)

    def _chat_stream_response(self, upstream, model, fp, account, t_start, effort=None):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            if cors_origin_allowed(self.path):
                self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            emitted = False
            last_usage = None
            first_ms = None
            streamed_text = []
            leak_filter = LeakedToolCallStreamFilter()
            synthesized_calls = False
            pending_finish = None
            # 写出缓冲：一个读取块里的事件合成一次 write。
            # wfile 是 wbufsize=0 的**无缓冲**文件（StreamRequestHandler 默认），
            # 每个 chunk 直接落成一次 sendall —— 一次 syscall 加一次 GIL 交还。
            # 真机 10 秒 5043 次 sendto，对应约一万次 futex，占全部 futex 的一成。
            # 攒到「下一个可能阻塞的读之前」再写，不改变任何延迟：块边界就是
            # socket 里当前已经有的全部字节（见 iter_sse_lines 的 before_read）。
            out_buf = []

            def flush_out():
                if out_buf:
                    self.wfile.write(b"".join(out_buf))
                    self.wfile.flush()
                    del out_buf[:]

            def write_chunk(payload):
                nonlocal emitted, first_ms
                if first_ms is None:
                    first_ms = int((time.time() - t_start) * 1000)
                emitted = True
                out_buf.append(f"data: {payload}\n\n".encode("utf-8"))

            def release(actions):
                nonlocal synthesized_calls
                for action in actions:
                    if action[0] == "text":
                        if action[1]:
                            write_chunk(chat_text_chunk(model, action[1]))
                    else:
                        synthesized_calls = True
                        for payload in chat_tool_call_chunks(model, action[1]):
                            write_chunk(payload)

            try:
                for line in iter_sse_lines(upstream, before_read=flush_out):
                    data = strip_data_prefix(line.decode("utf-8", "replace"))
                    if not data or data == "[DONE]" or data.startswith(":"):
                        continue
                    maybe = None
                    try:
                        maybe = json.loads(data)
                    except Exception:
                        maybe = None
                    if isinstance(maybe, dict):
                        u = maybe.get("usage")
                        if u:
                            if last_usage is None or (u.get("total_tokens") or 0) >= (last_usage.get("total_tokens") or 0):
                                last_usage = u
                        for ch in (maybe.get("choices") or []):
                            delta = ch.get("delta") or {}
                            if delta.get("reasoning_content"):
                                streamed_text.append(delta["reasoning_content"])
                            piece = delta.get("content")
                            if piece:
                                streamed_text.append(piece)
                                # Hold the text back: it may be a call the model
                                # printed instead of emitting, and a client that
                                # reads it as prose ends the turn with nothing to
                                # run ("it keeps stopping by itself").
                                delta.pop("content", None)
                                release(leak_filter.feed(piece))
                            if ch.get("finish_reason"):
                                # Hold the terminator too: whether this turn ends
                                # in tool_calls is only known once the text has
                                # been flushed at end of stream.
                                pending_finish = ch["finish_reason"]
                                ch["finish_reason"] = None
                    cleaned = clean_chunk(
                        json.dumps(maybe, ensure_ascii=False)
                        if isinstance(maybe, dict) else data)
                    if not cleaned:
                        continue
                    write_chunk(cleaned)
                release(leak_filter.flush())
                if synthesized_calls or pending_finish is not None:
                    write_chunk(chat_finish_chunk(
                        model, "tool_calls" if synthesized_calls else pending_finish))
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                # Client hung up; still account for what upstream produced.
                wall = int((time.time() - t_start) * 1000)
                record_usage(model, last_usage, stream=True,
                             elapsed_ms=wall, ttft_ms=first_ms,
                             gen_ms=(wall - first_ms) if first_ms is not None else None,
                             fp=fp, account=account.uid,
                             outcome="client_aborted", key=self._key_id(), effort=effort)
                return
            except Exception as exc:
                # Upstream quit mid-stream (timeout, incomplete read, ...).
                # The client would otherwise get a truncated stream with no
                # terminal marker, and the traceback reached the HTTP layer.
                wall = int((time.time() - t_start) * 1000)
                record_error(model, 502, "stream aborted: %s" % exc,
                             elapsed_ms=wall, account=account.uid,
                            usage=last_usage, stream=True, ttft_ms=first_ms,
                            gen_ms=(wall - first_ms) if first_ms is not None else None,
                             fp=fp, outcome="upstream_aborted", key=self._key_id())
                try:
                    flush_out()
                    self.wfile.write(b"data: [DONE]\n\n")
                    self.wfile.flush()
                except Exception:
                    pass
                return
            flush_out()
            if not emitted:
                err = json.dumps({"error": {"message": "empty upstream stream", "type": "server_error"}})
                self.wfile.write(f"data: {err}\n\n".encode("utf-8"))
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            wall = int((time.time() - t_start) * 1000)
            if last_usage is None or (last_usage.get("total_tokens") or 0) == 0:
                full_s = "".join(streamed_text)
                if full_s:
                    comp = estimate_tokens(full_s)
                    last_usage = {
                        "prompt_tokens": max(1, comp // 2),
                        "completion_tokens": comp,
                        "total_tokens": max(1, comp // 2) + comp,
                        "completion_tokens_details": {"reasoning_tokens": 0},
                        "prompt_tokens_details": {"cached_tokens": 0},
                    }
            record_usage(model, last_usage, stream=True,
                         elapsed_ms=wall, ttft_ms=first_ms,
                         gen_ms=(wall - first_ms) if first_ms is not None else None,
                         fp=fp, account=account.uid, key=self._key_id(), effort=effort)
            return

    def _chat_nonstream_response(self, upstream, model, fp, account, t_start, effort=None):
        try:
            result = aggregate_stream(upstream, model, None)
        except Exception as exc:
            record_error(model, 502, str(exc), elapsed_ms=int((time.time() - t_start) * 1000),
                         account=account.uid, key=self._key_id())
            return self._error(502, f"upstream stream error: {exc}")
        wall = int((time.time() - t_start) * 1000)
        first_at = result.get("first_chunk_at")
        # Measured from request arrival so streaming and non-streaming are comparable.
        first_ms = int((first_at - t_start) * 1000) if first_at else None
        record_usage(model, result.get("usage"), stream=False,
                     elapsed_ms=wall, ttft_ms=first_ms,
                     gen_ms=(wall - first_ms) if first_ms is not None else None,
                     fp=fp, account=account.uid, key=self._key_id(), effort=effort)
        return self._json(200, result)

def running_version():
    """The version this process reports, e.g. "1.6.19".

    tests/_test_release_engineering.py pins the two version literals and asserts
    they agree, so the update checker reads the running one from the handler
    instead of becoming a third copy that could drift away from both.
    """
    return Handler.server_version.split("/", 1)[-1]

def main():
    args = _parse_cli_args()
    _apply_cli_overrides(args)
    if _probe_running_instance(args):
        return
    api_key_generated = _bootstrap_runtime(args)
    if _report_first_run(args):
        return
    _log_startup_summary(args, api_key_generated)
    _serve_forever(args)

def _parse_cli_args():
    ap = argparse.ArgumentParser(description="WorkBuddy (workbuddy.ai) -> OpenAI-compatible proxy")
    ap.add_argument("--info", help="path to the WorkBuddy *.info credential file")
    ap.add_argument("--host", default=os.environ.get("HOST") or "127.0.0.1")
    ap.add_argument("--port", type=int, default=int(os.environ.get("PORT") or "8788"))
    ap.add_argument("--lan", action="store_true",
                    help="listen on every interface so other devices on the LAN can "
                         "reach it (implies --host 0.0.0.0 and forces an api key)")
    ap.add_argument("--api-key", default=os.environ.get("API_KEY") or os.environ.get("WB_PROXY_KEY") or None,
                    help="require this bearer token on /v1/* (optional)")
    ap.add_argument("--system-prompt", default=DEFAULT_SYSTEM_PROMPT,
                    help="system message injected when the request has none (required upstream)")
    ap.add_argument("--user-agent", default=None,
                    help="override the upstream User-Agent (default: mirror the official "
                         "WorkBuddy AI client)")
    ap.add_argument("--usage-dir", default=None,
                    help="where to store usage.jsonl (default: ./usage)")
    ap.add_argument("--accounts-dir", default=os.environ.get("ACCOUNTS_DIR") or None,
                    help="where the per-account credential files live (default: ./accounts)")
    ap.add_argument("--import-desktop", action="store_true",
                    help="import the desktop app credential as an account, then exit")
    ap.add_argument("--panel-password", default=None,
                    help="set the web panel password on startup (default: admin)")
    args = ap.parse_args()
    return args

def _apply_cli_overrides(args):
    global USAGE_DIR, USAGE_LOG, LIMIT_EVENTS_FILE, ESTIMATE_DAILY_FILE, \
        RECENT_INDEX_FILE
    # LAN mode binds every interface. The key is generated below, once
    # ACCOUNTS_DIR is resolved, so it can be persisted and reused.
    if args.lan and args.host == "127.0.0.1":
        args.host = "0.0.0.0"
    if args.user_agent:
        wb_accounts.USER_AGENT = args.user_agent.strip()
        log("user-agent : %s (override)" % wb_accounts.USER_AGENT)
    if args.usage_dir:
        USAGE_DIR = os.path.abspath(args.usage_dir)
        USAGE_LOG = os.path.join(USAGE_DIR, "usage.jsonl")
        # 事件日志（remaining 预算样本的来源）必须跟着 --usage-dir 走：这个
        # 常量在 import 时按默认目录算过，漏掉它真机上会一直写 ENOENT
        # （2026-10-10 实测 /usr/lib/workbuddy2api/usage/limit-events.jsonl）。
        LIMIT_EVENTS_FILE = os.path.join(USAGE_DIR, "limit-events.jsonl")
        # 按天归档的估算数据同理（少了它，跨天归档会一直写默认目录/ENOENT）。
        ESTIMATE_DAILY_FILE = os.path.join(USAGE_DIR, "estimate-daily.json")
        # recent 的行偏移索引同理（漏掉它，换目录后索引写到旧目录，
        # 新目录每次冷启动都要重扫一遍日志）。
        RECENT_INDEX_FILE = os.path.join(USAGE_DIR, "usage-recent-index.json")
    # 账号活动历史与用量日志同目录，并且要在任何后台线程起来之前定下来：调度器
    # 的第一轮巡检不能落在 --usage-dir 生效之前，否则记录会写进默认目录。
    wb_activity.set_data_dir(USAGE_DIR)

def _probe_running_instance(args):
    # Refuse to start a second copy. On Windows SO_REUSEADDR lets two sockets
    # bind the same port, which silently splits incoming connections between
    # them - confusing and hard to diagnose.
    try:
        probe = urllib.request.urlopen(
            f"http://{args.host if args.host != '0.0.0.0' else '127.0.0.1'}:{args.port}/health",
            timeout=2,
        )
        existing = json.loads(probe.read().decode("utf-8"))
    except Exception:
        existing = None  # nothing answering /health - let the bind below decide
    if isinstance(existing, dict):
        # Only OUR /health carries the account-pool fields ("accounts"). Other
        # services can occupy the same port and also answer /health with JSON
        # (a dev proxy, another gateway); treating that as "already running" made this
        # launcher exit silently while the port belonged to someone else - the
        # dashboard then showed a foreign UI and API calls failed with 401/404.
        foreign = existing.get("service") or "accounts" not in existing
        if foreign:
            who = existing.get("service") or "an unknown HTTP service"
            print()
            print(f"  [ERROR] port {args.port} is already taken by another program: {who}")
            print("          wb-proxy itself is NOT running - nothing was started.")
            print()
            print("  Fix: start wb-proxy on a different port, e.g.")
            print("          %s" % launcher_hint(args.port + 1))
            print(f"          python3 wb_proxy.py --port {args.port + 1}")
            print()
            print("  Check who owns the port:  %s" % port_owner_hint(args.port))
            print()
            raise SystemExit(1)
        print()
        print(f"  [已有一个反代在 {args.port} 端口运行，无需重复启动]")
        print(f"  账号: {existing.get('uid', '?')} @ {existing.get('domain', '?')}")
        print(f"  看板: http://127.0.0.1:{args.port}/")
        print()
        print("  如果要重启: 先把原来那个窗口关掉（或结束 python 进程），再运行本程序。")
        print()
        # Return True so main() stops here. A bare return gives None, which
        # main() reads as "no running copy" and it would carry on to bind the
        # port that is already taken.
        return True
    return False

def _bootstrap_runtime(args):
    global POOL, ACCOUNTS_DIR, API_KEY, SYSTEM_PROMPT
    global API_KEY_FILE_SET, SCHEDULER
    api_key_generated = False
    API_KEY = args.api_key
    SYSTEM_PROMPT = args.system_prompt
    if args.accounts_dir:
        ACCOUNTS_DIR = os.path.abspath(args.accounts_dir)
    # LAN mode must not ship a known key: the gateway spends the account's own
    # upstream quota, so a guessable default lets anyone on the network drain
    # it. Generate one on first use, persist it, and reuse it afterwards.
    if args.lan and not API_KEY:
        API_KEY, api_key_generated = wb_settings.ensure_launcher_key(ACCOUNTS_DIR)
    # A key saved from the panel wins over an auto-generated LAN key so a
    # change made in the browser survives a restart of the .bat file. An
    # explicit --api-key on the command line still takes precedence.
    global API_KEY_FILE_SET
    saved_key, key_from_panel = wb_settings.api_key_override(ACCOUNTS_DIR)
    if key_from_panel and not args.api_key:
        API_KEY = saved_key
        API_KEY_FILE_SET = True
    if args.panel_password:
        wb_settings.set_panel_password(ACCOUNTS_DIR, args.panel_password)
        log("panel      : password set from --panel-password")
    elif wb_settings.panel_password_is_default(ACCOUNTS_DIR):
        log("panel      : password is still the default 'admin' - change it in the panel")
    POOL = wb_accounts.AccountPool(ACCOUNTS_DIR, log=log)
    POOL.load()
    POOL.apply_proxy_slots()
    POOL.apply_reserve_credits()
    # WB_MAX_CONCURRENT_CHAT=auto: size the chat ceiling from the pool that was
    # just loaded. Runs before the listener exists, so no request can hold a
    # slot yet and the semaphore swap is safe.
    resize_chat_slots(POOL.count_ready())
    apply_daily_guards()
    load_persisted_realm()
    global SCHEDULER
    from wb_scheduler import Scheduler
    SCHEDULER = Scheduler(POOL)
    SCHEDULER.start()
    global PRICING
    # The policy table and its timeline live beside the usage log, so one
    # volume carries both and the request references resolve locally.
    wb_pricing.set_data_dir(USAGE_DIR)
    # The panel's pricing switches (master switch, variant inheritance) live in
    # settings.json; point the pricing side at the same file the panel writes.
    wb_pricing.set_settings_dir(ACCOUNTS_DIR)
    # The refresher always exists so /pricing can report its state; with the
    # master switch off its loop simply parks without fetching.
    PRICING = wb_pricing.PriceRefresher(
        wb_settings.pricing_refresh_minutes(ACCOUNTS_DIR))
    PRICING.start()
    # Credit balances back the expiring-credits dispatch preference, and only
    # the sign-in / daily-activity tasks used to refresh them. The refresher
    # keeps them current in the background so no request pays for a lookup.
    global CREDITS_REFRESHER
    CREDITS_REFRESHER = wb_accounts.CreditsRefresher(POOL)
    CREDITS_REFRESHER.start()
    global UPDATES
    # Release discovery only: the checker asks GitHub what the newest stable
    # release is and reports it. It never downloads, replaces or restarts
    # anything, and with the daily switch off (the default) it sends nothing.
    UPDATES = wb_updates.UpdateChecker(
        current_version=running_version(),
        settings_dir=ACCOUNTS_DIR,
        log=lambda msg: add_log_entry("[更新] %s" % msg, tag="update"),
    )
    UPDATES.start()
    return api_key_generated

def _report_first_run(args):
    if args.info:
        account = POOL.import_desktop_credential(args.info, source="file")
        log("imported account %s from %s" % (account.uid[:8], args.info))
    first_run = not POOL.accounts
    if first_run:
        # Never adopt the desktop client's login silently: just report what is
        # available and let the user import it from the dashboard.
        detected = desktop_credential_scan()
        usable = [d for d in detected if d.get("valid")]
        if usable:
            log("no accounts yet - detected %d desktop credential(s), NOT importing" % len(usable))
            for d in usable:
                log("  available: %s  %s  %s" % (
                    (d.get("uid") or "?")[:8], d.get("nickname") or "(no name)",
                    d.get("realmName") or d.get("realm")))
            log("open the dashboard and click [Scan desktop app] to import")
        else:
            log("no accounts yet - no desktop credentials found on this machine")
    if first_run and not POOL.accounts:
        # Do NOT exit here: the dashboard has to stay reachable so a new
        # account can be added through the browser login flow.
        log("still no accounts - starting anyway so you can log in via the dashboard")
    if args.import_desktop:
        for account in POOL.accounts:
            print("  %s  %s  %s" % (account.uid[:8], account.nickname, account.domain))
        return

def _log_startup_summary(args, api_key_generated):
    rep = current_account()
    log("accounts   : %d total, %d usable" % (len(POOL.accounts), POOL.count_ready()))
    for account in POOL.accounts:
        log("  - %s  %s  %s  %s" % (account.uid[:8], account.nickname or "(no name)",
                                    account.domain, wb_accounts._human_delta(
                                        (account.expires_at or 0) - time.time()) or "?"))
    log("store      : %s" % ACCOUNTS_DIR)
    log(f"credential : {rep.path if rep else chr(45)}")
    if os.path.exists(PRODUCT_CONFIG_CACHE):
        log(f"catalog    : {PRODUCT_CONFIG_CACHE}")
    else:
        log("catalog    : app cache not found - will use the model API instead")
    log(f"account    : {rep.uid if rep else chr(45)} @ {rep.domain if rep else chr(45)}")
    log(f"issuer     : {wb_accounts.jwt_issuer(rep.access_token) if rep else chr(45)}")
    log("realm      : %s (%s)" % (
        CURRENT_REALM,
        "www.workbuddy.ai" if CURRENT_REALM == "intl" else "copilot.tencent.com"))
    log("user-agent : %s" % wb_accounts.USER_AGENT)
    if args.host == "0.0.0.0":
        ips = local_ip_addresses() or ["<this-pc-ip>"]
        print()
        print("  " + "=" * 62)
        print("  LAN MODE - reachable from other devices")
        print()
        for ip in ips:
            print("    API       : http://%s:%s/v1" % (ip, args.port))
            print("    Dashboard : http://%s:%s/" % (ip, args.port))
        print()
        print("    API Key   : %s" % API_KEY)
        if api_key_generated:
            print("                (newly generated & saved to accounts/settings.json)")
        else:
            print("                (reused from accounts/settings.json)")
        print()
        print("    Open the dashboard (key already included):")
        print("      http://%s:%s/?key=%s" % (ips[0], args.port, API_KEY))
        print()
        print("    Clients: Base URL = the API address above, then paste the key.")
        print()
        if IS_WINDOWS:
            print("    If nothing can connect, allow python through the")
            print("    firewall: run allow-firewall.bat once as administrator.")
        elif sys.platform == "darwin":
            print("    If other devices cannot connect, allow incoming")
            print("    connections for Python (macOS asks automatically the")
            print("    first time it listens; on macOS 15+ also allow Local")
            print("    Network access for your terminal). Helper script:")
            print("    ./allow-firewall.command")
        else:
            print("    If other devices cannot connect, open the port in")
            print("    your firewall (ufw / firewalld) for the LAN subnet.")
        print("  " + "=" * 62)
        print()
        sys.stdout.flush()
    if not POOL.accounts:
        print()
        print("  " + "=" * 62)
        print("  NO ACCOUNTS YET")
        print()
        print("  Open the dashboard and click [Login new account]:")
        print("      http://127.0.0.1:%s/" % args.port)
        print()
        print("  The browser flow adds the account automatically.")
        print("  This window must stay open.")
        print("  " + "=" * 62)
        print()
        sys.stdout.flush()

class GatewayServer(ThreadingHTTPServer):
    """主服务的 HTTP 服务类：只为把 listen backlog 从 stdlib 默认的 5 提到 128。

    backlog=5 在突发并发下（面板打开一页就是 ~7 个并发请求，多标签更糟）会让
    内核来不及 accept 的 SYN 被丢弃、客户端按 1s 粒度重传，表现为一部分请求整整
    慢 1 秒（路由器实测：64 并发突发 43/64 卡 >=1s、最长 2.8s；backlog=128 后
    0/64、最长 0.18s）。listen() 在 TCPServer.__init__ 里就被调用，所以必须是
    类属性——实例化之后再改就晚了。
    """
    request_queue_size = 128

def _serve_forever(args):
    try:
        server = GatewayServer((args.host, args.port), Handler)
    except OSError as exc:
        # Port stolen between the probe above and this bind, or held by
        # something that does not answer /health: report it in plain words
        # instead of dumping a raw socketserver traceback.
        print()
        print(f"  [ERROR] failed to listen on {args.host}:{args.port} - {exc}")
        print("          the port is reserved or held by another program;")
        print("          wb-proxy did NOT start.")
        print()
        print("  Fix: stop the program holding the port, or pick another port:")
        print("          %s" % port_owner_hint(args.port))
        print("          %s" % launcher_hint(args.port + 1))
        print()
        raise SystemExit(1)
    # Only claim the address once the socket really exists, so a failed bind
    # never prints a "listening" line that contradicts the error below.
    # Report the state the request path actually enforces: the panel can turn
    # key checking on after startup, so reading API_KEY alone printed "off"
    # while every /v1 call was still being rejected with 401.
    if auth_required():
        _panel_keys = [k for k in configured_keys() if k.get("enabled")]
        _key_state = ("on (%d key(s) from the panel)" % len(_panel_keys)) if _panel_keys else "on (--api-key)"
    else:
        _key_state = "off"
    log(f"listening  : http://{args.host}:{args.port}/v1  (api key: {_key_state})")
    log(f"dashboard  : http://{args.host}:{args.port}/")
    # Keep the handler referenced for the process lifetime: SetConsoleCtrlHandler
    # stores a raw pointer, so a collected callback would crash on close.
    _ctrl_handler = install_console_close_handler()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        log("bye")
    finally:
        try:
            server.server_close()
        except Exception:
            pass

if __name__ == "__main__":
    try:
        # Keep console output readable regardless of the active code page.
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
    main()
