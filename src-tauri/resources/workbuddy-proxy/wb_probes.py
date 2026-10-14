# -*- coding: utf-8 -*-
"""wb_probes.py — 真实输出上限探测结果（panel scripts/probe_max_tokens.py 的 hub 化）。

网关送 max_tokens=1,000,000 时上游会静默钳制到模型真实上限；探测脚本送一次
「要求超长输出」的请求，若 finish_reason == "length" 则本次 completion_tokens
就是上游的钳制值。结果写 accounts/output_probes.json，/v1/models 对应模型标注
output_clamp / max_output_tokens_clamped（仅标注，不覆盖模型规格值）。

纯标准库，Python 3.9 兼容。
"""

import json
import os
import threading
import time
import urllib.request

PROBE_FILENAME = "output_probes.json"
CACHE_TTL = 60

_lock = threading.Lock()
_cache = {"at": 0.0, "dir": None, "data": {}}

PROBE_PROMPT = ("Write the integers from 1 to 200000, one per line, "
                "with no commentary and no formatting.")


def probe_path(directory):
    return os.path.join(str(directory or "."), PROBE_FILENAME)


def load_probes(directory, force=False):
    """Cached read of the probe file (missing/corrupt -> empty dict)."""
    now = time.time()
    with _lock:
        if (not force and _cache["dir"] == directory
                and now - _cache["at"] < CACHE_TTL):
            return dict(_cache["data"])
    data = {}
    try:
        with open(probe_path(directory), encoding="utf-8") as fh:
            payload = json.load(fh)
        if isinstance(payload, dict):
            data = payload
    except Exception:
        data = {}
    with _lock:
        _cache.update({"at": now, "dir": directory, "data": data})
    return dict(data)


def save_probe(directory, model, result):
    path = probe_path(directory)
    with _lock:
        try:
            with open(path, encoding="utf-8") as fh:
                data = json.load(fh)
            if not isinstance(data, dict):
                data = {}
        except Exception:
            data = {}
        data[str(model)] = result
        try:
            os.makedirs(directory, exist_ok=True)
            tmp = path + ".tmp"
            with open(tmp, "w", encoding="utf-8") as fh:
                json.dump(data, fh, ensure_ascii=False, indent=2)
            os.replace(tmp, path)
        except OSError:
            return None
        _cache.update({"at": time.time(), "dir": directory, "data": data})
    return result


def clamp_for(directory, model):
    """The probed clamp for a model, or None when unknown."""
    entry = load_probes(directory).get(str(model))
    if isinstance(entry, dict):
        value = entry.get("clamped")
        try:
            return int(value) if value else None
        except (TypeError, ValueError):
            return None
    return None


def probe_model(base_url, api_key, model, max_tokens=1000000,
                prompt=None, timeout=900, dry_run=False, urlopen=None):
    """Send one long-output request and infer the upstream's real clamp."""
    url = str(base_url).rstrip("/") + "/v1/chat/completions"
    body = {
        "model": model,
        "messages": [{"role": "user", "content": prompt or PROBE_PROMPT}],
        "max_tokens": int(max_tokens),
        "stream": False,
    }
    if dry_run:
        return {"dry_run": True, "url": url, "request": body}
    headers = {"Content-Type": "application/json", "Accept": "application/json"}
    if api_key:
        headers["Authorization"] = "Bearer " + str(api_key)
    req = urllib.request.Request(
        url, data=json.dumps(body).encode("utf-8"), method="POST", headers=headers)
    opener = urlopen or urllib.request.urlopen
    with opener(req, timeout=timeout) as resp:
        payload = json.loads(resp.read().decode("utf-8"))
    usage = payload.get("usage") or {}
    choices = payload.get("choices") or [{}]
    choice = choices[0] if choices else {}
    finish = choice.get("finish_reason")
    completion = usage.get("completion_tokens") or 0
    clamped = int(completion) if finish == "length" and completion else None
    return {
        "model": model,
        "requested_max_tokens": int(max_tokens),
        "observed_completion_tokens": int(completion),
        "finish_reason": finish,
        "clamped": clamped,
        "at": time.strftime("%Y-%m-%dT%H:%M:%S"),
    }
