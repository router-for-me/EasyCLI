# -*- coding: utf-8 -*-
"""wb_updates.py — 更新发现（只发现，不安装）。

这个模块回答一个问题：官方仓库有没有比当前运行版本更新的稳定版？

它**只做发现**：拉一次 GitHub 的 releases 元数据（API 拿不到就退回网页端点）、
比较版本号、把结论交给面板。不下载、不解包、不替换文件、不重启进程——那些属于
后续阶段。

三条硬约束：

1. **默认关闭**。每日检查是 operator 显式打开的偏好；缺键、false 都是关闭，
   关闭时后台一个请求都不发。
2. **失败不致命**。网络超时、HTTP 错误、JSON 坏了都只记一行短原因，网关照常
   服务；后台检查永远不挡请求路径。
3. **只留安全运维字段**。落盘的只有「上次尝试时间 / 上次成功时间 / 见过的版本
   号」；URL、请求头、响应体一律不落。

纯标准库，Python 3.9 兼容。
"""

import json
import os
import re
import threading
import time
import urllib.error
import urllib.request

import wb_settings

# 官方仓库的 releases 接口——主路径。走 API 不抓 HTML：页面结构变了不该让检查失效。
RELEASES_URL = "https://api.github.com/repos/ardeyouxipianyi/workbuddy2api-hub/releases"
# 回退路径：网页端点的 302 Location 末尾就是最新稳定版 tag。它按网页计流量，
# 不受 API 那 60 次/小时/出口 IP 的匿名配额限制，也不需要凭据。
LATEST_RELEASE_URL = "https://github.com/ardeyouxipianyi/workbuddy2api-hub/releases/latest"
TAG_RE = re.compile(r"/releases/tag/([^/?#]+)")
# 可选凭据：认证请求的配额是 5000 次/小时。只从环境变量读，绝不落盘
# （模块头第 3 条）。
TOKEN_ENV = "GITHUB_TOKEN"
USER_AGENT = "wb2api-hub-update-check"
REQUEST_TIMEOUT = 5
# 一份 releases 文档几十 KB；给足余量但设上限，坏响应不能把内存拉满。
MAX_BODY = 2 * 1024 * 1024

# 每日检查的节流窗口。约 24 小时：重启不会让检查变得频繁，也不会因为一次
# 失败就整天不再试。
SCHEDULED_INTERVAL_SECONDS = 24 * 3600
# 启动后先等一会儿再花掉这个请求，让监听端口和账号池先就绪。
FIRST_DELAY_SECONDS = 30
# 后台线程醒来的间隔。真正的节流靠 due() 判断，这里只决定「多久看一眼」。
POLL_SECONDS = 300

# 只认 vX.Y.Z 这种能排序的稳定版本号。带后缀的（-rc1）、日期 tag、移动分支名
# 一律跳过：猜一个顺序比不报更糟。
VERSION_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)$")


def parse_version(text):
    """vX.Y.Z / X.Y.Z -> (major, minor, patch)；其它一律 None。"""
    match = VERSION_RE.match(str(text or "").strip())
    if not match:
        return None
    return tuple(int(part) for part in match.groups())


def is_newer(candidate, current):
    """candidate 是否严格新于 current；任一边不可解析就是 False。"""
    left, right = parse_version(candidate), parse_version(current)
    return bool(left and right and left > right)


def safe_url(value):
    """发布页链接，或 ""。

    这个值来自远端文档，会被面板原样显示：只放行 GitHub 的 https 发布页这一种
    形状，别的一律丢掉。
    """
    text = str(value or "").strip()
    if not text.startswith("https://github.com/"):
        return ""
    return text[:300]


def pick_latest(releases, current_version):
    """从一份 releases 文档里挑出最新的稳定版，没有就返回 None。

    草稿与预发布跳过；tag 不是纯 vX.Y.Z 的跳过；取版本号最大的那个而不是列表
    里第一个——重新发布的旧 tag 不能靠排序取胜。
    """
    best = None
    for entry in releases if isinstance(releases, list) else []:
        if not isinstance(entry, dict):
            continue
        if entry.get("draft") or entry.get("prerelease"):
            continue
        version = parse_version(entry.get("tag_name"))
        if version is None:
            continue
        if best is None or version > best[0]:
            best = (version, entry)
    if best is None:
        return None
    version, entry = best
    text = "%d.%d.%d" % version
    return {
        "version": text,
        "tag": str(entry.get("tag_name") or "").strip()[:64],
        "url": safe_url(entry.get("html_url")),
        "published_at": str(entry.get("published_at") or "").strip()[:32],
        "newer": is_newer(text, current_version),
    }


def describe_failure(exc):
    """一行能安全外发的失败原因。

    urllib 的异常消息里可能带着完整 URL 和响应体，两者都不该进 settings.json
    或日志，所以这里只取异常类型和 HTTP 状态码。
    """
    code = getattr(exc, "code", None)
    if isinstance(code, int):
        return "HTTP %d" % code
    if isinstance(exc, json.JSONDecodeError):
        return "invalid JSON"
    if isinstance(exc, ValueError):
        return "unexpected release document"
    if isinstance(exc, (urllib.error.URLError, OSError)):
        return "network error: %s" % type(exc).__name__
    return type(exc).__name__


def fetch_releases(url=RELEASES_URL, timeout=REQUEST_TIMEOUT):
    """releases 文档的原始 JSON 文本；任何失败都抛异常给调用方。

    API 那一跳走的是**匿名**配额：60 次/小时/出口 IP。国内用户普遍从共享代理
    节点出网，这份配额是整个出口共用的，被邻居先耗光时这一跳**必然**失败。所以
    API 那一跳只要没成——403 也好、连接被掐也好——就退回网页端点，把同一条结论
    拿回来；那边也失败才抛异常，抛的是 API 的原因，它才是主路径。
    """
    try:
        return _fetch_api(url, timeout)
    except Exception as api_error:
        try:
            return json.dumps([fetch_latest_release(timeout)])
        except Exception:
            raise api_error


def _fetch_api(url, timeout):
    headers = {
        "User-Agent": USER_AGENT,
        "Accept": "application/vnd.github+json",
    }
    token = str(os.environ.get(TOKEN_ENV) or "").strip()
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read(MAX_BODY).decode("utf-8", "replace")


def fetch_latest_release(timeout=REQUEST_TIMEOUT):
    """网页端点上的最新稳定版，返回一条 releases 条目。

    releases/latest 会 302 到 /releases/tag/<tag>；urlopen 跟完重定向后
    geturl() 就是那条最终地址，tag 直接从 URL 里取——不解析 HTML，页面改版也
    不会让检查失效。发布日期这一跳给不出来，留空。
    """
    request = urllib.request.Request(LATEST_RELEASE_URL,
                                     headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        final = response.geturl()
    match = TAG_RE.search(final or "")
    if not match:
        raise ValueError("no release tag in the redirect target")
    return {
        "tag_name": match.group(1),
        "draft": False,
        "prerelease": False,
        "html_url": final,
        "published_at": "",
    }


def _stamp(value):
    if not value:
        return None
    return time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(value))


class UpdateChecker(threading.Thread):
    """后台的每日更新检查 + 面板要的状态。

    daemon 线程：进程退出不需要等它，`stop()` 只是让循环尽快收工。
    """

    def __init__(self, current_version, settings_dir, fetcher=None, log=None,
                 first_delay=FIRST_DELAY_SECONDS, poll_seconds=POLL_SECONDS):
        super().__init__(daemon=True, name="update-check")
        self.current_version = str(current_version or "").strip()
        self.settings_dir = settings_dir
        # 注入点：测试给一个假的 fetcher，就不必碰网络。
        self._fetch = fetcher or fetch_releases
        self._log = log
        self._first_delay = max(0.0, float(first_delay))
        self._poll_seconds = max(1.0, float(poll_seconds))
        self._stop_event = threading.Event()
        self._wake = threading.Event()
        self._lock = threading.Lock()
        self._checking = False
        self._latest = None
        self._last_error = ""
        self._last_attempt = 0.0
        self._last_success = 0.0
        self._load_state()

    # ---- 状态 -----------------------------------------------------------
    def _load_state(self):
        """把上次的记账读回来，重启后面板立刻有东西可看。"""
        try:
            state = wb_settings.update_check_state(self.settings_dir)
        except Exception:
            return
        self._last_attempt = state.get("last_attempt") or 0.0
        self._last_success = state.get("last_success") or 0.0
        version = state.get("latest_version") or ""
        if version:
            self._latest = {
                "version": version, "tag": "", "url": "", "published_at": "",
                "newer": is_newer(version, self.current_version),
            }

    def _remember(self, success, version=""):
        try:
            wb_settings.record_update_check(
                self.settings_dir, at=self._last_attempt,
                success=success, latest_version=version)
        except Exception:
            # 记账失败不该影响一次已经完成的检查：内存里的结论照样能用。
            pass

    def enabled(self):
        """每日检查的偏好；缺键或 false 都是关闭。"""
        try:
            return wb_settings.update_check_enabled(self.settings_dir)
        except Exception:
            return False

    def due(self, now=None):
        """约 24 小时的节流窗口是否已经放行。"""
        now = time.time() if now is None else float(now)
        if not self._last_attempt:
            return True
        return (now - self._last_attempt) >= SCHEDULED_INTERVAL_SECONDS

    def status(self):
        picked = self._latest or {}
        latest = picked.get("version") or ""
        return {
            "current_version": self.current_version,
            "latest_version": latest or None,
            "update_available": is_newer(latest, self.current_version),
            "enabled": self.enabled(),
            "checking": self._checking,
            "last_attempt": _stamp(self._last_attempt),
            "last_success": _stamp(self._last_success),
            "last_error": self._last_error,
            "release_url": picked.get("url") or "",
            "published_at": picked.get("published_at") or "",
        }

    def log(self, msg):
        if not self._log:
            return
        try:
            self._log(msg)
        except Exception:
            pass

    # ---- 检查 -----------------------------------------------------------
    def check(self, manual=False):
        """跑一次发现，返回状态字典。

        `manual=True` 绕过开关和 24 小时窗口：操作员点了「立即检查」就必须真的
        去问一次。`manual=False` 是后台路径，关闭或没到点就直接返回，一个请求
        都不发。

        同步执行，耗时由 fetcher 的超时封顶；它不持有任何请求路径要等的锁。
        """
        status = self.status()
        if not manual:
            if not self.enabled():
                status["skipped"] = "disabled"
                return status
            if not self.due():
                status["skipped"] = "not due yet"
                return status
        with self._lock:
            if self._checking:
                status["skipped"] = "already running"
                return status
            self._checking = True
        try:
            self._run_check()
        finally:
            with self._lock:
                self._checking = False
        return self.status()

    def _run_check(self):
        self._last_attempt = time.time()
        self._last_error = ""
        try:
            document = json.loads(self._fetch(RELEASES_URL, REQUEST_TIMEOUT))
            if not isinstance(document, list):
                raise ValueError("releases document is not a list")
            picked = pick_latest(document, self.current_version)
        except Exception as exc:
            # 契约上失败不致命：保留上一次的结论，只把一行短原因报出去。
            self._last_error = describe_failure(exc)
            self._remember(success=False)
            self.log("更新检查失败：%s" % self._last_error)
            return
        self._latest = picked
        self._last_success = self._last_attempt
        version = (picked or {}).get("version") or ""
        self._remember(success=True, version=version)
        if picked and picked.get("newer"):
            self.log("发现新版本 %s（当前 %s）" % (picked["version"], self.current_version))
        elif picked:
            self.log("已是最新版本 %s" % self.current_version)

    # ---- 后台线程 -------------------------------------------------------
    def wake(self):
        """叫醒停着的循环——偏好刚从关拨到开时用。"""
        self._wake.set()

    def stop(self):
        """让循环尽快收工；daemon 线程本来也不会挡住进程退出。"""
        self._stop_event.set()
        self._wake.set()

    def _park(self, seconds):
        self._wake.wait(seconds)
        self._wake.clear()

    def run(self):
        # 先让监听端口和账号池就绪，再花掉第一个请求。
        if self._stop_event.wait(self._first_delay):
            return
        while not self._stop_event.is_set():
            try:
                self.check(manual=False)
            except Exception as exc:
                self.log("更新检查异常：%s" % describe_failure(exc))
            self._park(self._poll_seconds)
