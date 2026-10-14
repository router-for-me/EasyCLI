"""Where a proxy exits, and what kind of address that exit is.

The gateway only ever learns an exit's IP address (api.ipify.org, fetched
through the proxy itself). The country and the address kind therefore come from
a public lookup: ip-api.com answers without a key, and `lang=zh-CN` returns the
country name the panel shows.

`hosting` is the signal behind the 住宅 / 机房 split - an address announced by a
hosting provider is treated as a machine room, everything else as residential.
That is a heuristic, not a registry fact, so the announcing network is stored
alongside it and shown to the operator.

A lookup that fails is reported as unknown rather than raised: a proxy that
carries traffic is still usable when the lookup is blocked or rate limited.
"""

import json
import urllib.parse
import urllib.request

# `message` is requested so a refusal ("private range", "reserved range",
# rate limit) is visible in the payload instead of surfacing as a bare failure.
GEO_ENDPOINT = ("http://ip-api.com/json/%s?lang=zh-CN"
                "&fields=status,message,country,countryCode,isp,org,as,"
                "hosting,proxy,mobile")
LOOKUP_TIMEOUT = 8

RESIDENTIAL = "residential"
DATACENTER = "datacenter"

TYPE_LABELS = {RESIDENTIAL: "住宅", DATACENTER: "机房"}

# Fields a slot stores about its exit; every one of them may be empty, which
# means "not known" and never "assume something".
FIELDS = ("country", "country_code", "ip_type", "isp", "asn")


def type_label(ip_type):
    """Chinese label for a stored kind, or "" when the kind is unknown."""
    return TYPE_LABELS.get(str(ip_type or "").strip().lower(), "")


def empty():
    """The unknown-exit answer, with the same shape as a successful lookup."""
    return dict.fromkeys(FIELDS, "")


def classify(payload):
    """Turn one ip-api.com reply into the fields a slot stores.

    Anything unusable - a failed status, a missing body, a `hosting` flag that
    is neither true nor false - comes back empty rather than guessed.
    """
    if not isinstance(payload, dict) or payload.get("status") != "success":
        return empty()
    hosting = payload.get("hosting")
    if hosting is True:
        ip_type = DATACENTER
    elif hosting is False:
        ip_type = RESIDENTIAL
    else:
        ip_type = ""
    return {
        "country": str(payload.get("country") or "").strip(),
        "country_code": str(payload.get("countryCode") or "").strip().upper(),
        "ip_type": ip_type,
        "isp": str(payload.get("isp") or payload.get("org") or "").strip(),
        "asn": str(payload.get("as") or "").strip(),
    }


def _open(request, timeout):
    """Indirection so tests can answer without a network."""
    return urllib.request.urlopen(request, timeout=timeout)


def lookup(ip, timeout=LOOKUP_TIMEOUT):
    """Country and address kind for one exit IP; empty fields when unknown.

    The request goes out directly rather than through the proxy being
    described: the lookup has to work even when that proxy only reaches the
    upstream, and it must not be answered by the exit we are asking about.
    """
    address = str(ip or "").strip()
    if not address:
        return empty()
    url = GEO_ENDPOINT % urllib.parse.quote(address, safe="")
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "wb-proxy"})
        with _open(request, timeout) as response:
            payload = json.loads(response.read().decode("utf-8", "replace"))
    except Exception:
        return empty()
    return classify(payload)


def slot_name(country, ip_type):
    """Default slot name: the exit's country and kind, e.g. "美国 住宅".

    No country means no name. "住宅" on its own would not tell two slots apart,
    so the caller is better off falling back to the slot id.
    """
    where = str(country or "").strip()
    if not where:
        return ""
    kind = type_label(ip_type)
    return (where + " " + kind) if kind else where
