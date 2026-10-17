"""Runtime settings for the gateway: panel password and API key override.

Everything lives in `accounts/settings.json` so a change made from the web
panel survives a restart without editing the launcher .bat files. The panel
password is never stored in clear text - only a PBKDF2-SHA256 digest.

Only the Python standard library is required.
"""

import fnmatch
import hashlib
import hmac
import json
import os
import re
import secrets
import threading
import time

DEFAULT_PANEL_PASSWORD = "admin"
PBKDF2_ROUNDS = 120_000
SESSION_TTL = 7 * 24 * 3600
# How often the gateway pulls fresh prices, in minutes; keep in step with
# wb_pricing.DEFAULT_REFRESH_MINUTES.
DEFAULT_PRICING_REFRESH_MINUTES = 5.0
# The setting used to be counted in hours and stored under this key. It is read
# once on upgrade (x60) and rewritten under the new key, so 6 hours can never
# come back as 6 minutes.
LEGACY_PRICING_REFRESH_HOURS_KEY = "pricing_refresh_hours"
PRICING_REFRESH_MINUTES_KEY = "pricing_refresh_minutes"
# A month, the old cap converted: 720 hours = 43200 minutes.
MAX_PRICING_REFRESH_MINUTES = 24 * 30 * 60
# How stale an account's credit balance may get before the background
# refresher updates it, in hours. Only the sign-in / daily-activity tasks used
# to refresh balances, so a dispatch decision could rest on a balance days old.
# 12 hours is deliberately slack: the preference is measured in days, so half a
# day of drift moves an account by at most half a day inside a 7-day window,
# and the wider TTL keeps the refresher from spending upstream billing calls it
# does not need.
DEFAULT_CREDITS_REFRESH_HOURS = 12.0
MAX_CREDITS_REFRESH_HOURS = 24 * 30
CREDITS_REFRESH_HOURS_KEY = "credits_refresh_hours"
# Panel window height cap, in pixels: a dashboard window (section) taller than
# this scrolls internally with its title row pinned, instead of stretching the
# whole page. 0 turns the cap off. The default sits just above the log
# terminal's own 620px so the terminal stays untouched until the user asks for
# something smaller.
SECTION_MAX_HEIGHT_KEY = "section_max_height"
DEFAULT_SECTION_MAX_HEIGHT = 640
MAX_SECTION_MAX_HEIGHT = 4000
# Whether a model name may inherit its price from a suffix-stripped base
# (deepseek-r1-0528-lkeap → deepseek-r1-0528). Missing key reads as on.
PRICING_VARIANT_INHERIT_KEY = "pricing_variant_inherit"
# Master switch for the whole OpenRouter price-estimation feature. Missing key
# reads as on: an install that predates the setting behaves exactly as it did,
# and only an explicit false turns the feature off.
PRICING_ENABLED_KEY = "pricing_enabled"
# Whether the panel's account section is collapsed. Missing, or anything that is
# not the boolean true, reads as expanded: a fresh install and a value a hand
# edit or an older client left behind both keep the default view, and only an
# explicit true hides the accounts.
ACCOUNTS_COLLAPSED_KEY = "accounts_collapsed"
# Whether the panel's per-API-key table folds away its `(切换前)` row - the
# legacy tail of requests logged before the key field existed. Same
# normalisation as the disclosure above: only an explicit boolean true hides
# the row, so a hand edit or an older client cannot drop it by accident.
KEY_BEFORE_HIDDEN_KEY = "key_before_hidden"
# Which top-bar pages the panel keeps out of the nav, as a list of page keys
# ("tasks", "logs", ...). Deliberately not validated against a page registry:
# the pages live in dashboard.html, and the panel ignores a key it does not
# know, so adding or renaming a page upstream never needs a change here. A
# missing key, a hand-written string or any other malformed value reads as
# "nothing hidden" - the fail-safe direction for a display preference.
HIDDEN_PAGES_KEY = "hidden_pages"
# Page keys are the same lowercase identifiers switchMainTab() switches on.
PAGE_KEY_RE = re.compile(r"^[a-z][a-z0-9_-]{0,31}$")
# The one page that is never hidden, whatever the request or the file says: it
# is where this switch itself lives, so hiding it would leave the panel with no
# way back. Dropped during normalisation, so the stored file cannot hold it
# either and no client of this API can lock itself out.
PAGE_KEEP_VISIBLE = "settings"
# Bound on the stored list. The panel sends at most one entry per page, so this
# only limits what a hand edit can put in the file.
MAX_HIDDEN_PAGES = 32
# Whether the panel's account sections get a main tab of their own instead of
# sharing the gateway page. Missing, or anything that is not the boolean true,
# reads as off: an install that predates the setting keeps the single
# "网关与账号" tab it has always had, and only an explicit true splits it into
# 网关 + 账号. Same `is True` normalisation as the two switches above, so a
# hand-edited "false", a 1 or an object cannot split the nav by accident.
ACCOUNTS_SEPARATE_TAB_KEY = "accounts_separate_tab"

# Instance-wide default UI language. The dashboard can override this per
# browser with localStorage; this key is the fallback when no override exists.
UI_LANGUAGE_KEY = "ui_language"
UI_LANGUAGE_DEFAULT = "zh"
UI_LANGUAGE_VALUES = ("zh", "zh-Hant", "en")

# Whether the gateway checks GitHub for a newer release once a day. Missing key
# reads as off - the opposite of the switches above - because turning it on
# makes the gateway send a request on its own schedule.
UPDATE_CHECK_ENABLED_KEY = "update_check_enabled"
# The last-check bookkeeping for that daily check. One small object, never a
# general update-state store: see update_check_state().
UPDATE_CHECK_STATE_KEY = "update_check"

# 剩余用量优先调度：把「估计剩余快耗尽」的 (账号, 模型) 先交出去（剩余越少
# 权重越高）。缺失键读作关：估算本身是从撞线样本反推的，优先它是一次刻意的
# 取舍（那些组合会明显多接流量），所以老安装保持原样的纯轮询。
REMAINING_PRIORITY_ENABLED_KEY = "remaining_priority_enabled"

# 出口级瞬时失败退避（issue #271）：传输类失败与 502/503/504 不再进账号级熔断
# 计数，改记在出口上、退避降到秒级（见 wb_egress）。缺失键读作关：它改变的是
# 派发路径上的失败归账与退避尺度，老安装保持原样（传输错误仍喂账号熔断）。
EGRESS_BACKOFF_ENABLED_KEY = "egress_backoff_enabled"

_lock = threading.RLock()


def settings_path(accounts_dir):
    return os.path.join(accounts_dir, "settings.json")


def _digest(password, salt_hex, rounds=PBKDF2_ROUNDS):
    return hashlib.pbkdf2_hmac(
        "sha256", password.encode("utf-8"), bytes.fromhex(salt_hex), rounds
    ).hex()


# settings.json 的解析结果按 (path, mtime, size) 缓存：请求热路径上它被读约 10
# 次（prompt_config×2 / api_keys×2 / limits_data×3 / auto_switch_product /
# upstream_config×2），单次 156~273µs，合计 1.6~2.7ms/请求。所有写入都走 save()
# 的 os.replace 原子替换，mtime/size 必变，所以面板改动仍然即时生效；save() 里
# 再主动失效一次，连 mtime 精度粗（FAT 只有 2s）的同尺寸改写也不会读到旧值。
# 调用方拿到的是顶层浅拷贝：与原实现「每次重新解析、返回值随便改」的语义一致
# （现网所有 setter 都只改顶层键再 save()，嵌套值按约定只读），改返回值也不会
# 弄脏缓存。
_load_cache = {"key": None, "value": {}}


def load(accounts_dir):
    """Return the persisted settings, or an empty dict on a fresh install.

    Cached on (path, mtime, size); the returned dict is a fresh top-level copy
    so callers keep the previous "mutate freely" semantics.
    """
    path = settings_path(accounts_dir)
    try:
        info = os.stat(path)
        key = (path, info.st_mtime, info.st_size)
    except OSError:
        key = (path, None, None)
    with _lock:
        if _load_cache["key"] == key:
            return dict(_load_cache["value"])
    data = {}
    try:
        with open(path, encoding="utf-8") as fh:
            parsed = json.load(fh)
        if isinstance(parsed, dict):
            data = parsed
    except FileNotFoundError:
        pass
    except Exception:
        pass
    with _lock:
        _load_cache.update({"key": key, "value": data})
    return dict(data)


def save(accounts_dir, data):
    """Atomic write so a crash cannot leave a half-written settings file."""
    with _lock:
        os.makedirs(accounts_dir, exist_ok=True)
        path = settings_path(accounts_dir)
        tmp = path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump(data, fh, ensure_ascii=False, indent=2)
        os.replace(tmp, path)
        # 写完主动失效：不依赖文件系统 mtime 的精度（FAT 只有 2s），保证同尺寸
        # 改写的下一个 load() 一定重新读盘。
        _load_cache["key"] = None
        return path


def deep_merge(base, patch):
    """Recursively merge patch into base; unknown sibling keys survive.

    The panel form only submits the keys it manages. Replacing a whole group
    would silently drop hand-written or future keys, so nested objects merge
    key by key. Returns a new dict; the inputs are not mutated.
    """
    if not isinstance(base, dict) or not isinstance(patch, dict):
        return patch
    out = dict(base)
    for key, value in patch.items():
        if isinstance(value, dict) and isinstance(out.get(key), dict):
            out[key] = deep_merge(out[key], value)
        else:
            out[key] = value
    return out


def panel_password_is_default(accounts_dir):
    data = load(accounts_dir)
    if not data.get("panel_password_hash"):
        return True
    return data.get("panel_password_default") is True


def verify_panel_password(accounts_dir, password):
    """True when `password` opens the web panel."""
    password = password or ""
    data = load(accounts_dir)
    stored = data.get("panel_password_hash")
    if not stored:
        return password == DEFAULT_PANEL_PASSWORD
    if data.get("panel_password_default") is True:
        return password == DEFAULT_PANEL_PASSWORD
    salt = data.get("panel_password_salt")
    if not salt:
        return False
    rounds = int(data.get("panel_password_rounds") or PBKDF2_ROUNDS)
    try:
        given = _digest(password, salt, rounds)
    except Exception:
        return False
    return hmac.compare_digest(given, stored)


def set_panel_password(accounts_dir, password):
    with _lock:
        data = load(accounts_dir)
        if password == DEFAULT_PANEL_PASSWORD:
            data.pop("panel_password_salt", None)
            data.pop("panel_password_rounds", None)
            data["panel_password_hash"] = ""
            data["panel_password_default"] = True
        else:
            salt = secrets.token_hex(16)
            data["panel_password_salt"] = salt
            data["panel_password_rounds"] = PBKDF2_ROUNDS
            data["panel_password_hash"] = _digest(password, salt)
            data["panel_password_default"] = False
        save(accounts_dir, data)


def api_key_override(accounts_dir):
    """Return (key, is_set). `is_set` means the panel manages the key."""
    data = load(accounts_dir)
    if not data.get("api_key_set"):
        return None, False
    return str(data.get("api_key") or ""), True


def set_api_key(accounts_dir, key):
    with _lock:
        data = load(accounts_dir)
        data["api_key"] = key or ""
        data["api_key_set"] = True
        save(accounts_dir, data)


def ensure_launcher_key(accounts_dir):
    """Return the persisted LAN key, creating one on first use.

    LAN mode must never ship a well-known default: the gateway spends the
    account's own upstream quota, so anyone on the same network could drain it.
    The value is generated once and stored so clients keep working across
    restarts. Returns (key, created) so the caller can tell the user whether
    this run minted a fresh credential.
    """
    with _lock:
        data = load(accounts_dir)
        existing = str(data.get("launcher_key") or "").strip()
        if existing:
            return existing, False
        key = "wb-" + secrets.token_urlsafe(24)
        data["launcher_key"] = key
        save(accounts_dir, data)
        return key, True


# --------------------------------------------------------------- API keys
# Each key can be bound to one upstream realm, so several clients can hit
# different exits at the same time instead of sharing the global switch.

REALMS = ("", "intl", "cn")


def _clean_model_patterns(value):
    """Normalize one key's model allow-list into a list of lowercase patterns.

    The panel posts a list; a hand-edited settings.json tends to hold a
    comma-separated string, so both shapes are accepted. Matching is done with
    fnmatch, which makes an exact name (`gpt-6-astra`) and a wildcard
    (`deepseek/*`) behave the same way. An empty result means "no restriction",
    which is what every key written before this field existed reads back as -
    an upgrade therefore keeps behaving exactly as before.
    """
    if isinstance(value, str):
        raw = [part for part in re.split(r"[,;\n]", value)]
    elif isinstance(value, (list, tuple, set)):
        raw = list(value)
    else:
        return []
    out = []
    for item in raw:
        pattern = str(item or "").strip().lower()
        if pattern and pattern not in out:
            out.append(pattern)
    return out


def key_allows_model(entry, model):
    """True when `entry` places no model restriction, or `model` matches it.

    A key with an empty list stays unrestricted, so nothing changes for
    installs that never touch this field. A restricted key that names no model
    is refused instead of waved through: nothing in the request says it is
    asking for something the key may use, and forwarding it only reaches the
    upstream carrying an empty model, spending an attempt on a call that cannot
    succeed.
    """
    patterns = _clean_model_patterns((entry or {}).get("models"))
    if not patterns:
        return True
    name = str(model or "").strip().lower()
    if not name:
        return False
    return any(fnmatch.fnmatchcase(name, pattern) for pattern in patterns)


def _clean_key_entry(entry):
    """Normalize one stored key entry; returns None when unusable.

    A deleted entry keeps its row even though its secret is gone: the id is
    what the usage log records, so dropping the row would make every past
    request of that key unattributable. Deletion is one-way - the secret is
    never stored again and the entry can only ever be read, not re-enabled.
    """
    if not isinstance(entry, dict):
        return None
    key = str(entry.get("key") or "").strip()
    deleted_at = str(entry.get("deleted_at") or "").strip()
    if not key and not deleted_at:
        return None
    realm = str(entry.get("realm") or "").strip().lower()
    if realm not in REALMS:
        realm = ""
    return {
        "id": str(entry.get("id") or secrets.token_hex(6)),
        "name": str(entry.get("name") or "").strip() or "未命名",
        "key": key,
        "realm": realm,
        "models": _clean_model_patterns(entry.get("models")),
        "enabled": False if deleted_at else entry.get("enabled", True) is not False,
        "created_at": entry.get("created_at") or time.strftime("%Y/%m/%d %H:%M"),
        "deleted_at": deleted_at,
    }


def _unique_key_id(candidate, used):
    """Return `candidate`, or a variant that is not already in `used`.

    Ids used to be minted from the row's index in the submitted list, so a
    settings file written by an older build can hold two rows carrying the same
    id. `/settings/reveal` then answered with whichever row came first, which
    made the copy button on the other row hand out a different key. Later
    duplicates get a numeric suffix: the suffix is deterministic, so the id a
    `/settings` read just returned still resolves on the follow-up reveal.
    """
    candidate = str(candidate or "").strip()
    if candidate and candidate not in used:
        return candidate
    if candidate:
        suffix = 2
        while True:
            alt = "%s-%d" % (candidate, suffix)
            if alt not in used:
                return alt
            suffix += 1
    while True:
        alt = secrets.token_hex(6)
        if alt not in used:
            return alt


def api_keys(accounts_dir, include_deleted=False):
    """Every configured key, newest shape first.

    A settings file written by an older build only has the single
    `api_key`/`api_key_set` pair; that is surfaced as one unbound entry so
    upgrades keep working without a migration step. Ids are made unique here
    as well as on write, so a file that already holds a duplicate (and no
    longer has to be saved before it behaves) reads back as distinct rows.

    Deleted entries are hidden by default: they are gone as credentials, and
    match_api_key must never see them. Their rows stay on disk (and come back
    with include_deleted) because the usage log names a key by id, and an id
    with no name is not a table anyone can read.
    """
    data = load(accounts_dir)
    stored = data.get("api_keys")
    if isinstance(stored, list):
        out = []
        seen = set()
        seen_ids = set()
        for raw in stored:
            entry = _clean_key_entry(raw)
            if entry is None:
                continue
            # Deduplicate on the secret, and only when there is one: every
            # deleted entry has an empty key, and those are distinct rows.
            if entry["key"]:
                if entry["key"] in seen:
                    continue
                seen.add(entry["key"])
            entry["id"] = _unique_key_id(entry["id"], seen_ids)
            seen_ids.add(entry["id"])
            out.append(entry)
        if not include_deleted:
            out = [e for e in out if not e.get("deleted_at")]
        return out

    if data.get("api_key_set"):
        legacy = str(data.get("api_key") or "").strip()
        if legacy:
            return [{
                "id": "legacy",
                "name": "默认（跟随面板切换）",
                "key": legacy,
                "realm": "",
                "models": [],
                "enabled": True,
                "created_at": "",
                "deleted_at": "",
            }]
    return []


def _retired_key_entry(old):
    """The soft-deleted form of a stored key: name and dates kept, secret gone."""
    return {
        "id": old["id"],
        "name": old["name"],
        "key": "",
        "realm": old.get("realm") or "",
        "models": old.get("models") or [],
        "enabled": False,
        "created_at": old.get("created_at") or "",
        "deleted_at": old.get("deleted_at") or time.strftime("%Y/%m/%d %H:%M"),
    }


def set_api_keys(accounts_dir, keys, delete_ids=None):
    """Save the key list. Returns the saved (live) list.

    Removal is a soft delete: the usage log attributes spend by id, so losing
    an id would dump a key's whole history into "(未知 key)", and the secret is
    wiped at the same moment so a deleted key can never authenticate again.

    Two modes decide which stored keys get soft-deleted:

    - `delete_ids is None`: replace semantics. Any stored key absent from
      `keys` is retired. This is what an older panel relies on - it deletes a
      row by leaving it out of the submission - so it stays the default.
    - `delete_ids` is a list: upsert semantics. Only those ids are retired; a
      stored key the submission does not mention is left untouched. The panel
      uses this because its submission is whatever its in-memory rows happen
      to be, and a list that is stale or incomplete (a second browser tab, a
      save that raced the post-save reload, a reload that failed) must not
      retire a key the user never removed.
    """
    with _lock:
        previous = api_keys(accounts_dir, include_deleted=True)
        upsert = delete_ids is not None
        drop = {str(entry_id) for entry_id in (delete_ids or [])}
        # Upsert mode: a stored live secret, so a submitted row that lost its id
        # (a stale panel resubmitting a key it already saved) can adopt the
        # stored id and update that key in place instead of minting a new one
        # and splitting its usage history in two.
        live_secret_id = {}
        if upsert:
            for old in previous:
                if old["key"] and not old.get("deleted_at"):
                    live_secret_id.setdefault(old["key"], old["id"])
        cleaned = []
        seen = set()
        seen_ids = set()
        for raw in keys or []:
            raw_id = str((raw or {}).get("id") or "").strip() if isinstance(raw, dict) else ""
            entry = _clean_key_entry(raw)
            if entry is None:
                continue
            if entry["key"]:
                if entry["key"] in seen:
                    continue
                seen.add(entry["key"])
            if upsert and entry["key"] and raw_id not in live_secret_id.values():
                entry["id"] = live_secret_id.get(entry["key"], entry["id"])
            entry["id"] = _unique_key_id(entry["id"], seen_ids)
            seen_ids.add(entry["id"])
            cleaned.append(entry)
        for old in previous:
            if old["id"] in seen_ids:
                continue
            seen_ids.add(old["id"])
            # Upsert mode keeps a stored key the submission never mentioned.
            # An already-deleted row is kept too: it is read-only history.
            if upsert and old["id"] not in drop:
                # A submitted row may already carry this exact secret (the
                # stale submission re-sent it under a fresh id). Two live rows
                # with one secret is the duplicate this mode exists to avoid,
                # so the stale stored copy is retired instead.
                if old["key"] and old["key"] in seen:
                    cleaned.append(_retired_key_entry(old))
                    continue
                if old["key"]:
                    seen.add(old["key"])
                cleaned.append(old)
                continue
            cleaned.append(_retired_key_entry(old))
        data = load(accounts_dir)
        data["api_keys"] = cleaned
        # The single-key fields are now derived; drop them so there is one
        # source of truth and the list survives a restart.
        data.pop("api_key", None)
        data.pop("api_key_set", None)
        save(accounts_dir, data)
        return [e for e in cleaned if not e.get("deleted_at")]


def match_api_key(accounts_dir, supplied, extra_keys=()):
    """Find which configured key a request presented, if any.

    Returns a copy of the entry (with a `source` field) so the caller can read
    the bound realm, or None when nothing matches.
    """
    supplied = (supplied or "").strip()
    if not supplied:
        return None
    for entry in api_keys(accounts_dir):
        # A deleted entry is already stored with enabled=False; the explicit
        # check keeps a hand-edited settings file from reviving one.
        if entry["enabled"] and not entry.get("deleted_at") \
                and hmac.compare_digest(supplied, entry["key"]):
            out = dict(entry)
            out["source"] = "panel"
            return out
    for candidate in extra_keys:
        candidate = (candidate or "").strip()
        if candidate and hmac.compare_digest(supplied, candidate):
            return {
                "id": "launcher",
                "name": "启动参数",
                "key": candidate,
                "realm": "",
                "models": [],
                "enabled": True,
                "source": "launcher",
            }
    return None


def auth_disabled(accounts_dir):
    """True when the operator switched API-key checking off entirely."""
    return load(accounts_dir).get("auth_disabled") is True


def set_auth_disabled(accounts_dir, disabled):
    with _lock:
        data = load(accounts_dir)
        data["auth_disabled"] = bool(disabled)
        save(accounts_dir, data)


# ------------------------------------------------------------------ limits
# The four guards share one shape: a global default that covers both realms,
# plus an optional per-realm override. An override left empty inherits the
# global value, so an install that never touches it behaves exactly as before,
# and one that does only ever has to reason about a single number per guard.
LIMIT_KEYS = ("reserve_credits", "daily_token_limit",
              "daily_credit_limit", "model_daily_token_limit",
              "expiring_window_days")
LIMIT_REALMS = ("intl", "cn")
LIMIT_SCOPES = ("global",) + LIMIT_REALMS
LIMITS_KEY = "limits"

# Guards whose global default is not "off". Every key still reads 0 as off;
# only the expiring-credits window ships enabled, because 0 would make the
# preference a silent no-op until someone turned it on by hand.
LIMIT_DEFAULTS = {"expiring_window_days": 7}


def _default_global(key):
    """The global value an install reads before it ever saves one."""
    try:
        return max(0, int(LIMIT_DEFAULTS.get(key, 0)))
    except (TypeError, ValueError):
        return 0


def _empty_limit_entry(key=None):
    """One guard: a global default plus a slot per realm (None = inherit)."""
    return {"global": _default_global(key), "intl": None, "cn": None}


def _coerce_global(value):
    """A global threshold. Junk and negatives collapse to 0 (off), which is
    also what every install predating the setting reads as."""
    try:
        number = int(value or 0)
    except (TypeError, ValueError):
        number = 0
    return max(0, number)


def _coerce_override(value):
    """A per-realm override. None (or a blank form field) means "inherit the
    global default", kept distinct from an explicit 0 ("off for this realm")."""
    if value is None or value == "":
        return None
    try:
        number = int(value)
    except (TypeError, ValueError):
        return None
    return max(0, number)


def _fold_legacy_limits(data):
    """Move the four flat top-level guards into `limits`.

    Returns (changed, limits). The old keys are dropped as they are folded in,
    so a later rollback cannot resurrect a stale limit from beside the grouped
    copy.
    """
    limits = {}
    changed = False
    for key in LIMIT_KEYS:
        entry = _empty_limit_entry(key)
        if key in data:
            entry["global"] = _coerce_global(data.pop(key))
            changed = True
        limits[key] = entry
    return changed, limits


def _normalize_limits(raw):
    """A copy of the stored map with every key and scope filled in, so a
    partial or hand-edited settings.json still reads as a complete shape."""
    limits = {}
    for key in LIMIT_KEYS:
        entry = raw.get(key)
        if not isinstance(entry, dict):
            entry = {}
        global_raw = entry.get("global")
        # A missing global falls back to that guard's own default (0 for every
        # guard but the expiring window); an explicit 0 is a real "off" and is
        # kept as one, so a saved "off" never silently comes back on.
        global_value = (_default_global(key) if global_raw is None
                        else _coerce_global(global_raw))
        limits[key] = {
            "global": global_value,
            "intl": _coerce_override(entry.get("intl")),
            "cn": _coerce_override(entry.get("cn")),
        }
    return limits


def limits_data(accounts_dir):
    """The grouped limits map, migrating a flat pre-grouping file on first read.

    Reading is what upgrades: the first look at an old settings.json folds the
    four flat keys into `limits` and rewrites the file, so the rest of the
    gateway only ever sees one shape.
    """
    with _lock:
        data = load(accounts_dir)
        raw = data.get(LIMITS_KEY)
        if isinstance(raw, dict):
            return _normalize_limits(raw)
        changed, limits = _fold_legacy_limits(data)
        if changed:
            data[LIMITS_KEY] = limits
            try:
                save(accounts_dir, data)
            except Exception:
                # A read-only accounts dir must not take the panel down; the
                # folded values are still returned and the next read retries.
                pass
        return limits


def limits_snapshot(accounts_dir):
    """The panel's view of every guard, keyed by limit then by scope."""
    return limits_data(accounts_dir)


def limit_value(accounts_dir, key, realm=None):
    """One guard's effective value for a realm.

    realm None (or "") returns the global default; an intl/cn override wins
    when it is set. An unknown key reads as 0 (off), so a typo cannot wedge
    the request path.
    """
    entry = limits_data(accounts_dir).get(key) or _empty_limit_entry()
    if realm in LIMIT_REALMS:
        override = entry.get(realm)
        if override is not None:
            return override
    return entry.get("global") or 0


def limit_values_many(accounts_dir, keys):
    """Several guards resolved for every realm, off one settings read.

    limit_values() per key re-reads the file (cached by mtime/size, so a stat)
    and re-normalizes the whole grouped map every call; the daily guards all
    want the same map on the request path, so they share one read here. One
    implementation only: limit_values() is this with a single key.
    """
    grouped = limits_data(accounts_dir)
    out = {}
    for key in keys:
        entry = grouped.get(key) or _empty_limit_entry()
        global_value = entry.get("global") or 0
        values = {"global": global_value}
        for realm in LIMIT_REALMS:
            override = entry.get(realm)
            values[realm] = global_value if override is None else override
        out[key] = values
    return out


def limit_values(accounts_dir, key):
    """One guard resolved for every realm plus the global it inherits.

    {"global": g, "intl": ..., "cn": ...}. The intl/cn entries already carry
    the global value when no override is set, so the pool can hand each
    account its own realm without a second settings lookup.
    """
    return limit_values_many(accounts_dir, (key,))[key]


def set_limit(accounts_dir, key, scope, value):
    """Persist one guard at one scope. Returns the stored entry.

    scope is "global", "intl" or "cn"; a None/blank value clears an intl/cn
    override back to "inherit", while the global slot always stores a number.
    """
    if key not in LIMIT_KEYS:
        raise ValueError("unknown limit: %s" % key)
    if scope not in LIMIT_SCOPES:
        raise ValueError("unknown scope: %s" % scope)
    with _lock:
        data = load(accounts_dir)
        raw = data.get(LIMITS_KEY)
        if isinstance(raw, dict):
            limits = _normalize_limits(raw)
        else:
            _changed, limits = _fold_legacy_limits(data)
        entry = limits[key]
        if scope == "global":
            entry["global"] = _coerce_global(value)
        else:
            entry[scope] = _coerce_override(value)
        data[LIMITS_KEY] = limits
        save(accounts_dir, data)
    return entry


def reserve_credits(accounts_dir, realm=None):
    """Global low-credit guard: an account at or below this balance stays idle.

    Zero disables the guard, which keeps installs that predate the setting
    behaving exactly as before.
    """
    return limit_value(accounts_dir, "reserve_credits", realm)


def set_reserve_credits(accounts_dir, value):
    """Persist the guard threshold. Returns the stored value."""
    return set_limit(accounts_dir, "reserve_credits", "global", value)["global"]


def daily_token_limit(accounts_dir, realm=None):
    """Global daily guard: an account that already burned this many tokens
    today stays idle until local midnight.

    Zero disables the guard, which keeps installs that predate the setting
    behaving exactly as before.
    """
    return limit_value(accounts_dir, "daily_token_limit", realm)


def set_daily_token_limit(accounts_dir, value):
    """Persist the daily token threshold. Returns the stored value."""
    return set_limit(accounts_dir, "daily_token_limit", "global", value)["global"]


def daily_credit_limit(accounts_dir, realm=None):
    """Daily credit guard: an account that already spent this many credits
    today serves free models only until local midnight, so a client that
    would keep burning credits on paid models rotates to another account
    instead of spending the whole balance.

    Zero disables the guard, which keeps installs that predate the setting
    behaving exactly as before.
    """
    return limit_value(accounts_dir, "daily_credit_limit", realm)


def set_daily_credit_limit(accounts_dir, value):
    """Persist the daily credit threshold. Returns the stored value."""
    return set_limit(accounts_dir, "daily_credit_limit", "global", value)["global"]


def model_daily_token_limit(accounts_dir, realm=None):
    """Per-model daily guard: an account that already burned this many
    tokens today on ONE model stops being handed out for that model until
    local midnight, while every other model keeps working.

    Zero disables the guard, which keeps installs that predate the setting
    behaving exactly as before.
    """
    return limit_value(accounts_dir, "model_daily_token_limit", realm)


def set_model_daily_token_limit(accounts_dir, value):
    """Persist the per-model daily token threshold. Returns the stored value."""
    return set_limit(accounts_dir, "model_daily_token_limit", "global", value)["global"]


def expiring_window_days(accounts_dir, realm=None):
    """Window, in days, inside which an account's soonest-expiring credit
    package makes the pool hand that account out first, so credits about to
    lapse are spent before they are lost.

    Zero disables the preference and dispatch falls back to a plain
    round-robin; the shipped default is 7 days.
    """
    return limit_value(accounts_dir, "expiring_window_days", realm)


def set_expiring_window_days(accounts_dir, value):
    """Persist the window. Returns the stored value."""
    return set_limit(accounts_dir, "expiring_window_days", "global", value)["global"]


def credits_refresh_hours(accounts_dir):
    """How stale a credit balance may get before the background refresher
    updates it, in hours.

    The dispatch preference reads the balance, and only the sign-in and
    daily-activity tasks used to refresh it, so an account could be judged on a
    balance days old - or on whatever was on disk when the process started.
    Zero disables the refresher. Anything not a number falls back to the
    default, so a hand-edited settings.json cannot wedge the loop.
    """
    with _lock:
        data = load(accounts_dir)
        raw = data.get(CREDITS_REFRESH_HOURS_KEY)
    try:
        value = float(raw)
    except (TypeError, ValueError):
        return DEFAULT_CREDITS_REFRESH_HOURS
    if value < 0:
        return DEFAULT_CREDITS_REFRESH_HOURS
    return min(value, MAX_CREDITS_REFRESH_HOURS)


def set_credits_refresh_hours(accounts_dir, value):
    """Persist the refresh TTL. Returns the stored value."""
    try:
        hours = float(value)
    except (TypeError, ValueError):
        hours = 0.0
    hours = max(0.0, min(MAX_CREDITS_REFRESH_HOURS, hours))
    with _lock:
        data = load(accounts_dir)
        data[CREDITS_REFRESH_HOURS_KEY] = hours
        save(accounts_dir, data)
    return hours


def section_max_height(accounts_dir):
    """Panel window height cap in pixels; 0 means no limit.

    Anything that is not a whole number falls back to the default, so a
    hand-edited settings.json cannot leave the dashboard with windows a pixel
    tall; a value past MAX_SECTION_MAX_HEIGHT is clamped, not rejected. bool is
    checked first on purpose: it is an int subclass, and True would otherwise
    read as a 1px cap.
    """
    with _lock:
        raw = load(accounts_dir).get(SECTION_MAX_HEIGHT_KEY)
    if isinstance(raw, bool):
        return DEFAULT_SECTION_MAX_HEIGHT
    try:
        value = int(raw)
    except (TypeError, ValueError, OverflowError):
        # OverflowError is its own case: json.loads turns 1e400 into inf, and
        # int(inf) raises it - one broken line must not 500 the settings view.
        return DEFAULT_SECTION_MAX_HEIGHT
    if value < 0:
        return DEFAULT_SECTION_MAX_HEIGHT
    return min(value, MAX_SECTION_MAX_HEIGHT)


def set_section_max_height(accounts_dir, value):
    """Persist the cap. Returns the stored value."""
    if isinstance(value, bool):
        number = DEFAULT_SECTION_MAX_HEIGHT
    else:
        try:
            number = int(value)
        except (TypeError, ValueError, OverflowError):
            number = DEFAULT_SECTION_MAX_HEIGHT
    number = max(0, min(MAX_SECTION_MAX_HEIGHT, number))
    with _lock:
        data = load(accounts_dir)
        data[SECTION_MAX_HEIGHT_KEY] = number
        save(accounts_dir, data)
    return number


def _clamp_refresh_minutes(value):
    """A month is well past "often enough"; the cap keeps a typo from parking
    the next refresh beyond any horizon the panel can show."""
    return max(0.0, min(MAX_PRICING_REFRESH_MINUTES, value))


def _migrate_pricing_refresh(data, accounts_dir):
    """(found, minutes) - convert a pre-minutes settings.json in place.

    `pricing_refresh_hours` used to hold hours. Reading it as minutes would
    turn 6 hours into 6 minutes, so the value is multiplied by 60 here, written
    under the new key and the old key dropped - one time, on the first read
    after the upgrade. A value that is not a number just loses the stale key
    and falls back to the default; a legacy 0 still means "off".
    """
    legacy = data.pop(LEGACY_PRICING_REFRESH_HOURS_KEY, None)
    if legacy is None:
        return False, None
    try:
        minutes = _clamp_refresh_minutes(float(legacy) * 60.0)
    except (TypeError, ValueError):
        minutes = None
    else:
        data[PRICING_REFRESH_MINUTES_KEY] = minutes
    try:
        save(accounts_dir, data)
    except Exception:
        # A read-only accounts dir must not take the panel down; the converted
        # value is still returned, and the next read simply migrates again.
        pass
    return True, minutes


def pricing_refresh_minutes(accounts_dir):
    """How often the gateway refreshes the OpenRouter price history, in minutes.

    Zero disables the refresh, which keeps installs that predate the setting
    on the bundled snapshot. A settings.json written before the unit changed
    carries `pricing_refresh_hours`, migrated here on first read (see
    `_migrate_pricing_refresh`). Anything not a number falls back to the
    default, so a hand-edited settings.json cannot wedge the refresh loop.
    """
    with _lock:
        data = load(accounts_dir)
        raw = data.get(PRICING_REFRESH_MINUTES_KEY)
        if raw is None:
            found, minutes = _migrate_pricing_refresh(data, accounts_dir)
            if not found:
                return DEFAULT_PRICING_REFRESH_MINUTES
            if minutes is None:
                return DEFAULT_PRICING_REFRESH_MINUTES
            return minutes
    try:
        value = float(raw)
    except (TypeError, ValueError):
        return DEFAULT_PRICING_REFRESH_MINUTES
    return value if value > 0 else 0.0


def set_pricing_refresh_minutes(accounts_dir, value):
    """Persist the refresh interval in minutes. Returns the stored value."""
    try:
        value = float(value)
    except (TypeError, ValueError):
        return pricing_refresh_minutes(accounts_dir)
    value = _clamp_refresh_minutes(value)
    with _lock:
        data = load(accounts_dir)
        data[PRICING_REFRESH_MINUTES_KEY] = value
        # The minutes key is the one in force; a leftover legacy key would only
        # confuse a later rollback into reading a stale interval.
        data.pop(LEGACY_PRICING_REFRESH_HOURS_KEY, None)
        save(accounts_dir, data)
    return value


def pricing_variant_inherit(accounts_dir):
    """Whether a model name may inherit its price from a suffix-stripped base.

    On unless the operator turns it off: a hub model carrying a channel suffix
    (`deepseek-r1-0528-lkeap`) is the same entity as its base model upstream,
    and without this the gateway would show "未定价" for a model it can price
    exactly. Off restores the previous behaviour - only the override table and
    an exact name match can price a model - so an install that wants the
    strictest possible rule keeps it. A settings.json that predates the key
    reads back as on, which is the default this ships with.
    """
    value = load(accounts_dir).get(PRICING_VARIANT_INHERIT_KEY)
    return True if value is None else value is True


def set_pricing_variant_inherit(accounts_dir, enabled):
    """Persist the variant-inheritance switch. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data[PRICING_VARIANT_INHERIT_KEY] = enabled
        save(accounts_dir, data)
    return enabled


def pricing_enabled(accounts_dir):
    """Master switch for the OpenRouter price estimation, on unless turned off.

    Off disables the feature end to end: no price fetch, no policy table, no
    per-row cost and no cost columns. A settings.json that predates the key
    reads back as on, which is the behaviour every install ships with - the
    switch only exists to let an operator turn the whole thing off.
    """
    value = load(accounts_dir).get(PRICING_ENABLED_KEY)
    return True if value is None else value is True


def set_pricing_enabled(accounts_dir, enabled):
    """Persist the master switch. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data[PRICING_ENABLED_KEY] = enabled
        save(accounts_dir, data)
    return enabled


def ui_language(accounts_dir):
    """Instance-wide default UI language (zh / zh-Hant / en)."""
    value = load(accounts_dir).get(UI_LANGUAGE_KEY)
    if isinstance(value, str) and value in UI_LANGUAGE_VALUES:
        return value
    return UI_LANGUAGE_DEFAULT


def set_ui_language(accounts_dir, value):
    """Persist the instance-wide default UI language."""
    if not isinstance(value, str) or value not in UI_LANGUAGE_VALUES:
        raise ValueError("ui_language must be zh, zh-Hant or en")
    with _lock:
        data = load(accounts_dir)
        data[UI_LANGUAGE_KEY] = value
        save(accounts_dir, data)
    return value


UPSTREAM_DEFAULTS = {
    "header_timeout_seconds": 120,
    "idle_timeout_seconds": 300,
    "device_token": "",
    "device_token_file": "",
}


def validate_upstream_patch(raw):
    """Strict validation for a panel-saved upstream patch."""
    out = {}
    for key, value in (raw or {}).items():
        if key in ("header_timeout_seconds", "idle_timeout_seconds"):
            if isinstance(value, bool) or not isinstance(value, int):
                raise ValueError("%s must be a whole number of seconds" % key)
            if value < 1:
                raise ValueError("%s cannot be less than 1" % key)
            out[key] = value
        elif key in ("device_token", "device_token_file"):
            if not isinstance(value, str):
                raise ValueError("%s must be a string" % key)
            out[key] = value.strip()
        else:
            raise ValueError("unknown upstream setting %r" % key)
    return out


def upstream_config(accounts_dir):
    """Chat socket timeouts and the optional X-Device-Token source."""
    stored = load(accounts_dir).get("upstream")
    stored = stored if isinstance(stored, dict) else {}
    out = {}
    for key, default in UPSTREAM_DEFAULTS.items():
        value = stored.get(key, default)
        if key in ("header_timeout_seconds", "idle_timeout_seconds"):
            out[key] = (value if isinstance(value, int) and not isinstance(value, bool)
                        and value >= 1 else default)
        else:
            out[key] = value.strip() if isinstance(value, str) else default
    return out


def set_upstream_config(accounts_dir, cfg):
    """Persist the upstream settings. Returns the stored config."""
    current = upstream_config(accounts_dir)
    if isinstance(cfg, dict):
        current.update({k: v for k, v in cfg.items() if k in UPSTREAM_DEFAULTS})
    clean = validate_upstream_patch(current)
    with _lock:
        data = load(accounts_dir)
        data["upstream"] = deep_merge(data.get("upstream"), clean)
        save(accounts_dir, data)
    return clean


PROMPT_DEFAULTS = {
    "mode": "passthrough",
    "file": "",
}


def validate_prompt_patch(raw):
    """Strict validation for a panel-saved prompt patch."""
    out = {}
    for key, value in (raw or {}).items():
        if key == "mode":
            if not isinstance(value, str):
                raise ValueError("prompt mode must be a string")
            mode = value.strip().lower()
            if mode not in ("passthrough", "custom", "append"):
                raise ValueError("prompt mode must be passthrough, custom or append")
            out["mode"] = mode
        elif key == "file":
            if not isinstance(value, str):
                raise ValueError("prompt file must be a string")
            out["file"] = value.strip()
        else:
            raise ValueError("unknown prompt setting %r" % key)
    return out


def prompt_config(accounts_dir):
    """Gateway system-prompt mode (default passthrough = legacy behaviour)."""
    stored = load(accounts_dir).get("prompt")
    stored = stored if isinstance(stored, dict) else {}
    mode = stored.get("mode", PROMPT_DEFAULTS["mode"])
    if not isinstance(mode, str) or mode.strip().lower() not in (
            "passthrough", "custom", "append"):
        mode = PROMPT_DEFAULTS["mode"]
    else:
        mode = mode.strip().lower()
    file_path = stored.get("file", PROMPT_DEFAULTS["file"])
    if not isinstance(file_path, str):
        file_path = PROMPT_DEFAULTS["file"]
    return {"mode": mode, "file": file_path.strip()}


def set_prompt_config(accounts_dir, cfg):
    """Persist the prompt settings. Returns the stored config."""
    current = prompt_config(accounts_dir)
    if isinstance(cfg, dict):
        current.update({k: v for k, v in cfg.items() if k in PROMPT_DEFAULTS})
    clean = validate_prompt_patch(current)
    with _lock:
        data = load(accounts_dir)
        data["prompt"] = clean
        save(accounts_dir, data)
    return clean


def auto_switch_product(accounts_dir):
    """Whether an upstream 429 may rotate an account's outbound identity.

    Off unless the operator turns it on. Rotating identity spends the request's
    retry budget and leaves the account on a channel nobody picked, so the
    gateway does not decide that on its own - and an install that predates the
    setting keeps behaving exactly as it did.
    """
    return load(accounts_dir).get("auto_switch_product") is True


def set_auto_switch_product(accounts_dir, enabled):
    """Persist the auto-switch toggle. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data["auto_switch_product"] = enabled
        save(accounts_dir, data)
    return enabled


def remaining_priority_enabled(accounts_dir):
    """Whether pairs close to their estimated remaining quota are served first.

    Off unless the operator turns it on. The remaining-usage estimate is
    inferred from cap events and the usage log, and preferring the pairs that
    are close to their estimated limit is a deliberate trade: they take a
    visibly larger share of the traffic, which is the point of the preference
    (spend the quota that would otherwise lapse with the window). An install
    that predates the switch keeps the plain round-robin, so nothing changes
    until it is explicitly turned on.
    """
    return load(accounts_dir).get(REMAINING_PRIORITY_ENABLED_KEY) is True


def set_remaining_priority_enabled(accounts_dir, enabled):
    """Persist the remaining-usage priority switch. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data[REMAINING_PRIORITY_ENABLED_KEY] = enabled
        save(accounts_dir, data)
    return enabled


def egress_backoff_enabled(accounts_dir):
    """Whether transient failures are accounted per egress instead of per account.

    Off unless the operator turns it on. Turning it on changes how a transport
    failure or a 502/503/504 is punished: it stops feeding the account breaker
    (30 minutes, doubling to 6h) and instead grows a seconds-level backoff on the
    outbound path the request actually used (see wb_egress). That is the fix for
    issue #271 - a few seconds of link flakiness used to park healthy accounts
    for half an hour - but it is a change to dispatch behaviour, so an install
    that predates the switch keeps the old accounting.
    """
    return load(accounts_dir).get(EGRESS_BACKOFF_ENABLED_KEY) is True


def set_egress_backoff_enabled(accounts_dir, enabled):
    """Persist the egress-level backoff switch. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data[EGRESS_BACKOFF_ENABLED_KEY] = enabled
        save(accounts_dir, data)
    return enabled


def daily_chat_web(accounts_dir):
    """Whether the intl daily check-in also opens a web-channel conversation.

    On unless the operator turns it off: the desktop-identity chat completion
    this automation used to send does not register the daily activity, while a
    web conversation does (issues #75, #59). An install that never touched the
    setting keeps the web step, because that is the behaviour that earns the
    credits; the toggle exists so a deployment can opt back into the old
    single-request check-in.
    """
    value = load(accounts_dir).get("daily_chat_web")
    return True if value is None else value is True


def set_daily_chat_web(accounts_dir, enabled):
    """Persist the web-channel toggle. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data["daily_chat_web"] = enabled
        save(accounts_dir, data)
    return enabled


def daily_chat_model(accounts_dir):
    """Which model the intl daily activity runs on; "" keeps the built-in one.

    The activity is two throwaway conversations ("Hi") whose only job is to
    register the day, so it has no reason to spend credits. The built-in
    default is already a free catalogue model, and this lets a deployment move
    the activity onto a different free model - useful when the default one is
    the same model the real traffic hammers, so the daily run does not eat its
    window quota. Empty means "unset": the caller falls back to the built-in id.
    """
    value = load(accounts_dir).get("daily_chat_model")
    return str(value or "").strip()


def set_daily_chat_model(accounts_dir, model):
    """Persist the daily-activity model id. Returns the stored string."""
    model = str(model or "").strip()
    with _lock:
        data = load(accounts_dir)
        data["daily_chat_model"] = model
        save(accounts_dir, data)
    return model


def local_web_tools(accounts_dir):
    """Whether the gateway runs web_search / web_fetch calls itself.

    Off unless the operator turns it on. Forwarding the client's declaration
    untouched is what this gateway has done since v1.5.3, and it is what an
    install that never touched the switch keeps doing: the upstream has no
    server-side search tool, so a client declaring one runs it in its own
    process. Turning the switch on swaps the declaration for the gateway's own
    function and executes the calls locally (wb_webtools), which also means the
    gateway itself fetches the URLs a model asks for - hence opt-in only.
    """
    return load(accounts_dir).get("local_web_tools") is True


def set_local_web_tools(accounts_dir, enabled):
    """Persist the local web-tools switch. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data["local_web_tools"] = enabled
        save(accounts_dir, data)
    return enabled


def accounts_collapsed(accounts_dir):
    """Whether the panel's account section is collapsed.

    Expanded unless the stored value is the boolean true. `is True` is the whole
    normalisation: a hand-edited "false", a 1, an object or a missing key all
    read as expanded, so none of them can hide the accounts by accident. The
    panel only ever writes a real boolean through set_accounts_collapsed.
    """
    return load(accounts_dir).get(ACCOUNTS_COLLAPSED_KEY) is True


def set_accounts_collapsed(accounts_dir, collapsed):
    """Persist the account-section disclosure state. Returns the stored boolean."""
    collapsed = bool(collapsed)
    with _lock:
        data = load(accounts_dir)
        data[ACCOUNTS_COLLAPSED_KEY] = collapsed
        save(accounts_dir, data)
    return collapsed


def key_before_hidden(accounts_dir):
    """Whether the per-API-key table folds away its `(切换前)` row.

    Shown unless the stored value is the boolean true, with the same `is True`
    normalisation as accounts_collapsed: the row is history worth seeing, so a
    hand-edited "false", a 1, an object or a missing key must all leave it
    visible rather than hide it by accident.
    """
    return load(accounts_dir).get(KEY_BEFORE_HIDDEN_KEY) is True


def set_key_before_hidden(accounts_dir, hidden):
    """Persist the `(切换前)` row disclosure state. Returns the stored boolean."""
    hidden = bool(hidden)
    with _lock:
        data = load(accounts_dir)
        data[KEY_BEFORE_HIDDEN_KEY] = hidden
        save(accounts_dir, data)
    return hidden


def _clean_page_keys(value):
    """Normalise a stored or patched page list into a clean list of keys.

    Anything that is not a list of well-formed keys is dropped rather than
    rejected, for the same reason the booleans above normalise instead of
    raising: the file is hand-editable, and the fail-safe direction for a
    display preference is "nothing hidden". Duplicates and over-long lists are
    collapsed so a hand edit cannot turn the panel's nav into a repeat of one
    entry.
    """
    if not isinstance(value, (list, tuple)):
        return []
    out = []
    for item in value:
        if not isinstance(item, str):
            continue
        key = item.strip()
        if not PAGE_KEY_RE.match(key) or key == PAGE_KEEP_VISIBLE or key in out:
            continue
        out.append(key)
        if len(out) >= MAX_HIDDEN_PAGES:
            break
    return out


def hidden_pages(accounts_dir):
    """The top-bar pages the panel hides. Empty on a fresh install.

    Normalised on every read, so a hand edit that puts a string, an unknown key
    or a duplicate in the file cannot hide a page twice or hide something the
    panel would then be unable to put back.
    """
    return _clean_page_keys(load(accounts_dir).get(HIDDEN_PAGES_KEY))


def set_hidden_pages(accounts_dir, pages):
    """Persist the hidden-page list. Returns the stored (normalised) list."""
    cleaned = _clean_page_keys(pages)
    with _lock:
        data = load(accounts_dir)
        data[HIDDEN_PAGES_KEY] = cleaned
        save(accounts_dir, data)
    return cleaned


def accounts_separate_tab(accounts_dir):
    """Whether the panel gives the account sections a main tab of their own.

    Off unless the stored value is the boolean true, with the same `is True`
    normalisation as the disclosures above: the merged "网关与账号" tab is what
    every install has always shown, so a hand-edited "false", a 1, an object or
    a missing key must all keep it rather than split the nav by accident.
    """
    return load(accounts_dir).get(ACCOUNTS_SEPARATE_TAB_KEY) is True


def set_accounts_separate_tab(accounts_dir, separate):
    """Persist the account-tab switch. Returns the stored boolean."""
    separate = bool(separate)
    with _lock:
        data = load(accounts_dir)
        data[ACCOUNTS_SEPARATE_TAB_KEY] = separate
        save(accounts_dir, data)
    return separate


def update_check_enabled(accounts_dir):
    """Whether the gateway checks for a newer release once a day.

    Off unless the operator turns it on, and off for an install that predates
    the key: this is the one setting here whose missing value means "no", since
    enabling it makes the gateway talk to GitHub on its own schedule. The manual
    check in the panel ignores this switch entirely.
    """
    return load(accounts_dir).get(UPDATE_CHECK_ENABLED_KEY) is True


def set_update_check_enabled(accounts_dir, enabled):
    """Persist the daily-check switch. Returns the stored boolean."""
    enabled = bool(enabled)
    with _lock:
        data = load(accounts_dir)
        data[UPDATE_CHECK_ENABLED_KEY] = enabled
        save(accounts_dir, data)
    return enabled


def update_check_state(accounts_dir):
    """The last-check bookkeeping: when it ran, and the version it saw.

    Deliberately three fields. The 24h cadence needs `last_attempt` to survive a
    restart, and `latest_version` is what lets the panel answer right after one;
    everything else about a check lives in memory. Nothing from the HTTP
    exchange - URL, headers, body - is ever stored here.
    """
    stored = load(accounts_dir).get(UPDATE_CHECK_STATE_KEY)
    stored = stored if isinstance(stored, dict) else {}
    out = {"last_attempt": 0.0, "last_success": 0.0, "latest_version": ""}
    for key in ("last_attempt", "last_success"):
        value = stored.get(key)
        if isinstance(value, (int, float)) and not isinstance(value, bool) and value > 0:
            out[key] = float(value)
    version = stored.get("latest_version")
    if isinstance(version, str):
        out["latest_version"] = version.strip()[:32]
    return out


def record_update_check(accounts_dir, at=None, success=False, latest_version=""):
    """Write one check's outcome. Returns the stored state.

    One narrow write path on purpose: a checker that could write arbitrary keys
    into settings.json would turn it into an update-state database, which is
    what this is meant not to become.
    """
    stamp = float(at if at is not None else time.time())
    with _lock:
        data = load(accounts_dir)
        state = data.get(UPDATE_CHECK_STATE_KEY)
        state = dict(state) if isinstance(state, dict) else {}
        state["last_attempt"] = stamp
        if success:
            state["last_success"] = stamp
        version = str(latest_version or "").strip()[:32]
        if version:
            state["latest_version"] = version
        data[UPDATE_CHECK_STATE_KEY] = state
        save(accounts_dir, data)
    return update_check_state(accounts_dir)


_SLOT_ID_RE = re.compile(r"^slot-(\d+)$")


def _clean_slot_entry(entry, fallback_id=None):
    """Normalize one stored slot; returns None when unusable.

    The exit fields are written by a probe and stay empty until one runs. An
    empty name means "label this slot by its exit", which is what the panel
    shows; it is never silently replaced by the id here, or the operator could
    not tell an auto-named slot from a renamed one.
    """
    if not isinstance(entry, dict):
        return None
    url = str(entry.get("url") or "").strip()
    if not url:
        return None
    slot_id = str(entry.get("id") or "").strip()
    if not slot_id:
        slot_id = fallback_id or ""
    try:
        probed_at = int(entry.get("probed_at") or 0)
    except (TypeError, ValueError):
        probed_at = 0
    return {
        "id": slot_id,
        "name": str(entry.get("name") or "").strip(),
        "url": url,
        "enabled": entry.get("enabled", True) is not False,
        "ip": str(entry.get("ip") or "").strip(),
        "country": str(entry.get("country") or "").strip(),
        "country_code": str(entry.get("country_code") or "").strip().upper(),
        "ip_type": str(entry.get("ip_type") or "").strip().lower(),
        "isp": str(entry.get("isp") or "").strip(),
        "asn": str(entry.get("asn") or "").strip(),
        "probed_at": probed_at,
    }


def _next_slot_id(existing):
    """Next unused `slot-<n>` id for a legacy list stored without ids."""
    highest = 0
    for entry in existing:
        match = _SLOT_ID_RE.match(str(entry.get("id") or ""))
        if match:
            highest = max(highest, int(match.group(1)))
    return "slot-%d" % (highest + 1)


def _slot_seq(data):
    """Highest slot number ever issued in this store."""
    try:
        return int(data.get("proxy_slot_seq") or 0)
    except Exception:
        return 0


def proxy_slots(accounts_dir):
    """Every configured proxy slot, in stored order."""
    data = load(accounts_dir)
    stored = data.get("proxy_slots")
    if not isinstance(stored, list):
        return []
    out, seen = [], set()
    for raw in stored:
        entry = _clean_slot_entry(raw)
        if entry and entry["url"] not in seen:
            seen.add(entry["url"])
            if not entry["id"]:
                entry["id"] = _next_slot_id(out)
            out.append(entry)
    return out


def set_proxy_slots(accounts_dir, slots):
    """Replace the whole slot list. Returns the stored list.

    Ids are drawn from a counter that only ever grows. Accounts persist the
    id they are bound to, so recycling a freed id would silently re-point an
    existing account at a newly added slot's exit IP.
    """
    with _lock:
        data = load(accounts_dir)
        stored = data.get("proxy_slots")
        stored = stored if isinstance(stored, list) else []

        seq = _slot_seq(data)
        # Seed from both the incoming and the outgoing list, so an id that is
        # being removed in this very save can never be handed to a new entry.
        for entry in list(stored) + list(slots or []):
            if not isinstance(entry, dict):
                continue
            match = _SLOT_ID_RE.match(str(entry.get("id") or ""))
            if match:
                seq = max(seq, int(match.group(1)))
        # A legacy list stored without ids is displayed as slot-1..slot-N, so
        # keep the counter above those too.
        seq = max(seq, len(stored))

        cleaned, seen = [], set()
        for raw in slots or []:
            entry = _clean_slot_entry(raw)
            if not entry or entry["url"] in seen:
                continue
            seen.add(entry["url"])
            if not entry["id"] or any(e["id"] == entry["id"] for e in cleaned):
                seq += 1
                entry["id"] = "slot-%d" % seq
            cleaned.append(entry)
        data["proxy_slots"] = cleaned
        data["proxy_slot_seq"] = seq
        save(accounts_dir, data)
        return cleaned


def update_proxy_slot(accounts_dir, slot_id, fields, defaults=None):
    """Merge `fields` into one stored slot; returns the merged copy, or None.

    Both the read and the write happen under the store lock. A plain
    read-modify-write from the caller would race with a panel save and write
    back a list that no longer matches what is on disk, silently reverting
    whatever the other writer changed.

    `defaults` are applied only to fields that are still empty at write time,
    so a value the operator typed while the caller was working is never
    overwritten by a derived one.
    """
    slot_id = str(slot_id or "").strip()
    if not slot_id:
        return None
    with _lock:
        out, found = [], None
        for entry in proxy_slots(accounts_dir):
            if entry["id"] == slot_id and found is None:
                entry = dict(entry)
                entry.update(fields)
                for key, value in (defaults or {}).items():
                    if not entry.get(key):
                        entry[key] = value
                found = entry
            out.append(entry)
        if found is None:
            return None
        data = load(accounts_dir)
        data["proxy_slots"] = out
        save(accounts_dir, data)
        return found


def drop_missing_bindings(pool, slots):
    """Clear bindings that point at a slot which no longer exists.

    Without this the stale id stays on the account, and a later slot that
    happens to receive that id would capture the account.
    """
    valid = {entry["id"] for entry in slots}
    changed = 0
    for account in list(getattr(pool, "accounts", []) or []):
        if account.proxy_slot and account.proxy_slot not in valid:
            account.proxy_slot = ""
            try:
                account.save(pool.dir)
            except Exception:
                pass
            changed += 1
    if changed:
        pool.apply_proxy_slots(slots)
    return changed


def find_proxy_slot(accounts_dir, slot_id):
    slot_id = str(slot_id or "").strip()
    if not slot_id:
        return None
    for entry in proxy_slots(accounts_dir):
        if entry["id"] == slot_id:
            return entry
    return None


class PanelSessions(object):
    """In-memory bearer tokens handed out after a successful panel login.

    Deliberately not persisted: restarting the gateway logs browsers out, which
    is the safer default for a LAN tool that people expose behind a port map.
    """

    def __init__(self, ttl=SESSION_TTL):
        self.ttl = ttl
        self._tokens = {}
        self._lock = threading.RLock()

    def create(self):
        token = secrets.token_urlsafe(24)
        with self._lock:
            self._tokens[token] = time.time() + self.ttl
        return token

    def valid(self, token):
        if not token:
            return False
        with self._lock:
            expiry = self._tokens.get(token)
            if not expiry:
                return False
            if expiry < time.time():
                self._tokens.pop(token, None)
                return False
            return True

    def revoke(self, token):
        if not token:
            return
        with self._lock:
            self._tokens.pop(token, None)

    def revoke_all(self):
        with self._lock:
            self._tokens.clear()
