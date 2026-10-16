# -*- coding: utf-8 -*-
"""桌面端 at-rest 加密凭据的解密与静态密钥回收。

背景：桌面客户端自 2026-09-24 起把 `accessToken` / `refreshToken`（以及国内版
的 `nickname` / `phoneNumber`）从明文改成 `{"$wbEncrypted": suite, "envelope":
base64(...)}` 加密信封。看板照旧能读到 `.info` 文件，但拿出来的是信封文本，
导入账号池之后每个请求都 401——这就是「扫描桌面客户端账号」入口被隐藏的原因。
本模块把解密链路补回来，入口可以重新开放。

方案与客户端 `packages/at-rest-crypto` 一致：

    key       = sha256(atRestSecretKey_b64_string)      # 由客户端构建载荷派生
    keyId     = sha256(key).hex[0:16]
    AAD       = "WB-AAD\\0" | 0x01 | LP(framingMagic) | LP("sym-v1") | u32(suite)
                | LP(keyId) | framingCode | 0x00 | 0x00        (LP = u32be 长度 + utf8)
    envelope  = base64(JSON{suite,keyId,nonce,authTag,ciphertext})   AES-256-GCM
    framing   : 文件 -> WBEF1/1（keyblob），字段 -> WBEV1/2（.info 里的字段）

静态密钥编译在桌面客户端（WorkBuddy.exe / WorkBuddyAI.exe，见
`DESKTOP_EXE_CANDIDATES`）的原生模块里，磁盘上拿不到明文，所以只能从**正在
运行的**主进程内存里找回来（见 `recover_key`）：判据是 sha256(窗口)[:8] 等于
keyblob 里 static-v1 槽的 protectorKeyId，因此客户端升级后依然可用。

四处刻意的取舍：

* 纯 Python 实现 AES-256-GCM，不引入第三方依赖——本项目与绿色包都不带 node，
  也不装 pip 包，而这条链路只在导入账号时跑几次；
* 密钥只放在**进程内存**里（`_KEY_CACHE`），不落盘、不进日志：找回一次即可
  反复使用，网关重启后重新找回；keyblob 每次使用前都校验，客户端换钥后内存
  里的旧钥自动作废；
* 内存扫描分三档、命中即止（见 `hunt_key`），不再对每个字节偏移做一次哈希：
  客户端侧 `atRestSecretKey` 是 44 字符规范 base64，在内存里就是一段孤立的
  base64 token，正则直接把它捞出来再验一遍即可；只有捞不到时才退回逐窗口
  哈希，最后才逐字节兜底；
* 本模块只读，唯一的文件读取是客户端配置目录下的 keyblob，文件名写死，且
  `keyblob_path()` 会把结果钉回配置目录内。
"""
import base64
import hashlib
import hmac
import json
import os
import re
import sys
import threading
import time

#: 信封字段名（`{"$wbEncrypted": 1, "envelope": "..."}`）
ENVELOPE_KEY = "$wbEncrypted"

FRAMING_MAGIC = {"file": "WBEF1", "field": "WBEV1", "record": "WBER1", "stream": "WBES1"}
FRAMING_CODE = {"file": 1, "field": 2, "record": 3, "stream": 4}
SCHEME = "sym-v1"
SUITE = 1

#: keyblob 文件名（客户端配置目录下，写死，不接收调用方给的路径）
KEYBLOB_NAME = "keyblob"


class AtRestError(RuntimeError):
    """解密链路的可预期失败（缺文件、密钥不对、进程内存找不到密钥…）。"""


# --------------------------------------------------------------- 受限路径
def _safe_path(base, name):
    """把 `name` 钉死在 `base` 目录内，返回规范化后的绝对路径。

    三道闸：逐段拒绝 `..`、只取文件名（丢掉任何目录成分）、规范化后要求父目录
    正好等于 base。因此调用方给什么都出不了这个目录。
    """
    raw = str(name).replace("\\", "/")
    if ".." in [seg for seg in raw.split("/") if seg]:
        raise AtRestError("文件名不允许包含 .. ：%r" % (name,))
    root = os.path.realpath(os.path.abspath(os.path.expanduser(str(base))))
    leaf = os.path.basename(raw)
    if not leaf or leaf in (".", ".."):
        raise AtRestError("非法文件名：%r" % (name,))
    path = os.path.realpath(os.path.join(root, leaf))
    if os.path.dirname(path) != root:
        raise AtRestError("路径越界：%s 不在 %s 内" % (path, root))
    return path


# --------------------------------------------------------------------- 信封
def _lp(text):
    raw = str(text).encode("utf-8")
    return len(raw).to_bytes(4, "big") + raw


def _u32(value):
    return int(value).to_bytes(4, "big")


def aad(key_id, suite, framing):
    """AAD 必须逐字节对齐客户端，否则 GCM 校验必然失败。"""
    if framing not in FRAMING_MAGIC:
        raise AtRestError("unknown framing %r" % (framing,))
    return b"".join((
        b"WB-AAD\0", b"\x01",
        _lp(FRAMING_MAGIC[framing]), _lp(SCHEME), _u32(suite), _lp(key_id),
        bytes([FRAMING_CODE[framing]]), b"\x00", b"\x00",
    ))


def derive_key_id(key):
    """protectorKeyId / envelope.keyId 的算法：sha256(key) 的前 16 个 hex。"""
    return hashlib.sha256(key).hexdigest()[:16]


def is_envelope(value):
    """是不是一个 `$wbEncrypted` 信封（而不是明文 token）。"""
    if isinstance(value, dict):
        return isinstance(value.get("envelope"), str) and ENVELOPE_KEY in value
    if isinstance(value, str):
        # 老版本曾把整个信封 JSON 序列化成字符串塞进字段里
        stripped = value.lstrip()
        return stripped.startswith("{") and ENVELOPE_KEY in stripped[:64]
    return False


def parse_envelope(value):
    """把 envelope（base64 字符串或 `$wbEncrypted` 字典）拆成 GCM 三段。"""
    if isinstance(value, dict):
        value = value.get("envelope") or ""
    try:
        env = json.loads(base64.b64decode(str(value)).decode("utf-8"))
    except Exception as exc:
        raise AtRestError("envelope 不是 base64(JSON)：%s" % exc)
    try:
        return {
            "suite": int(env.get("suite") or SUITE),
            "keyId": str(env.get("keyId") or ""),
            "nonce": base64.b64decode(env["nonce"]),
            "authTag": base64.b64decode(env["authTag"]),
            "ciphertext": base64.b64decode(env["ciphertext"]),
        }
    except Exception as exc:
        raise AtRestError("envelope 缺字段：%s" % exc)


def open_envelope(key, value, framing):
    """解开一个信封，返回明文字节。"""
    env = parse_envelope(value)
    expect = derive_key_id(key)
    if env["keyId"] and env["keyId"] != expect:
        raise AtRestError("envelope keyId %s 与当前密钥（%s）不匹配" % (env["keyId"], expect))
    return _gcm_open(key, env["nonce"], aad(env["keyId"] or expect, env["suite"], framing),
                     env["ciphertext"], env["authTag"])


def decrypt_field(key, value):
    """解密一个可能加密、也可能本来就是明文的字段。

    返回明文 `str`；空值返回 ""；非字符串且非信封的值返回 None，交给调用方决定
    怎么回退（例如国内版昵称可以退回 UID 前缀）。
    """
    if value is None or value == "":
        return ""
    if not is_envelope(value):
        return value if isinstance(value, str) else None
    return open_envelope(key, value, "field").decode("utf-8", "replace")


# ----------------------------------------------------------------- 纯 Python AES
def _build_sbox():
    sbox = [0] * 256
    p = q = 1
    while True:
        p = p ^ ((p << 1) & 0xFF) ^ (0x1B if p & 0x80 else 0)
        q ^= (q << 1) & 0xFF
        q ^= (q << 2) & 0xFF
        q ^= (q << 4) & 0xFF
        if q & 0x80:
            q ^= 0x09
        sbox[p] = (q ^ ((q << 1) | (q >> 7)) ^ ((q << 2) | (q >> 6))
                   ^ ((q << 3) | (q >> 5)) ^ ((q << 4) | (q >> 4)) ^ 0x63) & 0xFF
        if p == 1:
            break
    sbox[0] = 0x63
    return sbox


_SBOX = _build_sbox()
_MUL2 = [((i << 1) ^ 0x1B) & 0xFF if i & 0x80 else (i << 1) for i in range(256)]
_MUL3 = [_MUL2[i] ^ i for i in range(256)]


def _mix_columns(s):
    out = [0] * 16
    for c in range(4):
        a0, a1, a2, a3 = s[4 * c:4 * c + 4]
        out[4 * c + 0] = _MUL2[a0] ^ _MUL3[a1] ^ a2 ^ a3
        out[4 * c + 1] = a0 ^ _MUL2[a1] ^ _MUL3[a2] ^ a3
        out[4 * c + 2] = a0 ^ a1 ^ _MUL2[a2] ^ _MUL3[a3]
        out[4 * c + 3] = _MUL3[a0] ^ a1 ^ a2 ^ _MUL2[a3]
    return out


class _AES256(object):
    """AES-256 分组加密（GCM 只需要正向加密）。状态按列主序排成 16 字节。"""

    __slots__ = ("_w",)

    def __init__(self, key):
        if len(key) != 32:
            raise AtRestError("AES-256 需要 32 字节密钥，得到 %d" % len(key))
        words = [list(key[4 * i:4 * i + 4]) for i in range(8)]
        rcon = 1
        for i in range(8, 60):
            t = list(words[i - 1])
            if i % 8 == 0:
                t = t[1:] + t[:1]
                t = [_SBOX[b] for b in t]
                t[0] ^= rcon
                rcon = _MUL2[rcon]
            elif i % 8 == 4:
                t = [_SBOX[b] for b in t]
            words.append([words[i - 8][j] ^ t[j] for j in range(4)])
        self._w = words

    def encrypt_block(self, block):
        w = self._w
        s = [block[i] ^ w[i // 4][i % 4] for i in range(16)]
        for rnd in range(1, 14):
            s = [_SBOX[b] for b in s]
            s = [s[(i + 4 * (i % 4)) % 16] for i in range(16)]      # ShiftRows
            s = _mix_columns(s)
            base = rnd * 4
            s = [s[i] ^ w[base + i // 4][i % 4] for i in range(16)]
        s = [_SBOX[b] for b in s]
        s = [s[(i + 4 * (i % 4)) % 16] for i in range(16)]
        s = [s[i] ^ w[56 + i // 4][i % 4] for i in range(16)]
        return bytes(s)


_REDUCTION = 0xE1000000000000000000000000000000


def _gf_mul(x, y):
    """GF(2^128) 乘法（GCM 的块哈希），约简多项式 x^128+x^7+x^2+x+1。"""
    z = 0
    v = y
    for i in range(128):
        if (x >> (127 - i)) & 1:
            z ^= v
        if v & 1:
            v = (v >> 1) ^ _REDUCTION
        else:
            v >>= 1
    return z


def _blocks(data):
    if len(data) % 16:
        data = data + b"\x00" * (16 - len(data) % 16)
    return [data[i:i + 16] for i in range(0, len(data), 16)]


def _ghash(h, aad_bytes, ciphertext):
    x = 0
    tail = (len(aad_bytes) * 8).to_bytes(8, "big") + (len(ciphertext) * 8).to_bytes(8, "big")
    for block in _blocks(aad_bytes) + _blocks(ciphertext) + [tail]:
        x = _gf_mul(x ^ int.from_bytes(block, "big"), h)
    return x


def _j0(aes, nonce):
    if len(nonce) == 12:
        return nonce + b"\x00\x00\x00\x01"
    h = int.from_bytes(aes.encrypt_block(b"\x00" * 16), "big")
    return _ghash(h, b"", nonce).to_bytes(16, "big")


def _gctr(aes, j0, data):
    out = bytearray()
    counter = int.from_bytes(j0, "big")
    for off in range(0, len(data), 16):
        counter = (counter & ~0xFFFFFFFF) | ((counter + 1) & 0xFFFFFFFF)
        stream = aes.encrypt_block(counter.to_bytes(16, "big"))
        out += bytes(a ^ b for a, b in zip(data[off:off + 16], stream))
    return bytes(out)


def _gcm_open(key, nonce, aad_bytes, ciphertext, tag):
    aes = _AES256(key)
    h = int.from_bytes(aes.encrypt_block(b"\x00" * 16), "big")
    j0 = _j0(aes, nonce)
    expect = (int.from_bytes(aes.encrypt_block(j0), "big") ^ _ghash(h, aad_bytes, ciphertext))
    if not hmac.compare_digest(expect.to_bytes(16, "big"), tag):
        raise AtRestError("GCM 校验失败——密钥不对或信封被改动过")
    return _gctr(aes, j0, ciphertext)


def seal(key, plaintext, framing, key_id=None, nonce=None):
    """把明文封成 `$wbEncrypted` 信封。

    生产链路只需要解密，这个函数是给它做自检与单测用的（也顺带把格式写清楚）。
    """
    if isinstance(plaintext, str):
        plaintext = plaintext.encode("utf-8")
    key_id = key_id or derive_key_id(key)
    nonce = nonce or os.urandom(12)
    aad_bytes = aad(key_id, SUITE, framing)
    aes = _AES256(key)
    h = int.from_bytes(aes.encrypt_block(b"\x00" * 16), "big")
    j0 = _j0(aes, nonce)
    ct = _gctr(aes, j0, plaintext)
    tag = (int.from_bytes(aes.encrypt_block(j0), "big") ^ _ghash(h, aad_bytes, ct)).to_bytes(16, "big")
    envelope = {
        "suite": SUITE, "keyId": key_id,
        "nonce": base64.b64encode(nonce).decode("ascii"),
        "authTag": base64.b64encode(tag).decode("ascii"),
        "ciphertext": base64.b64encode(ct).decode("ascii"),
    }
    blob = json.dumps(envelope, separators=(",", ":")).encode("utf-8")
    return {ENVELOPE_KEY: SUITE, "envelope": base64.b64encode(blob).decode("ascii")}


# ------------------------------------------------------------------ keyblob
#: 桌面客户端的配置目录候选（keyblob 所在）。与 wb_proxy.read_product_config_models
#: 的口径一致：国际版装在 ~/.workbuddy-ai，国内版装在 ~/.workbuddy；不同版本
#: 的安装器两者都可能写，所以按存在性探测而不是写死一个。
CONFIG_DIR_CANDIDATES = (".workbuddy-ai", ".workbuddy")


def config_dir():
    """客户端配置目录（keyblob 所在）。可用环境变量指向便携版的位置。

    未设环境变量时按 CONFIG_DIR_CANDIDATES 顺序取第一个存在 keyblob 的目录
    （兼容 `~/.workbuddy-ai` 的新版安装布局）；都不存在时回落到第一个候选，
    让 load_keyblob() 报出常规的「keyblob 不存在」而不是这里的探测细节。
    """
    env = (os.environ.get("WORKBUDDY_CONFIG_DIR")
           or os.environ.get("CODEBUDDY_CONFIG_DIR"))
    if env:
        return os.path.realpath(os.path.abspath(os.path.expanduser(env)))
    home = os.path.expanduser("~")
    for name in CONFIG_DIR_CANDIDATES:
        candidate = os.path.join(home, name)
        if os.path.isfile(os.path.join(candidate, KEYBLOB_NAME)):
            return os.path.realpath(candidate)
    return os.path.realpath(os.path.join(home, CONFIG_DIR_CANDIDATES[0]))


def keyblob_path():
    """固定为 <配置目录>/keyblob —— 文件名写死，调用方无法指向别处。"""
    return _safe_path(config_dir(), KEYBLOB_NAME)


def load_keyblob():
    """读取客户端的 keyblob（固定路径，见 keyblob_path）。"""
    path = keyblob_path()
    if not os.path.isfile(path):
        raise AtRestError("keyblob 不存在：%s（桌面客户端没装或没登录过？）" % path)
    try:
        with open(path, encoding="utf-8") as fh:      # 路径已由 keyblob_path 钉在配置目录内
            return json.load(fh)
    except AtRestError:
        raise
    except Exception as exc:
        raise AtRestError("keyblob 读取失败：%s" % exc)


def read_key_id():
    """static-v1 槽的 protectorKeyId —— 内存里搜密钥时的判据。"""
    for slot in (load_keyblob().get("slots") or []):
        if slot.get("type") == "static-v1" and slot.get("protectorKeyId"):
            return str(slot["protectorKeyId"]).lower()
    raise AtRestError("keyblob 里没有带 protectorKeyId 的 static-v1 槽（%s）" % keyblob_path())


def unwrap_keyblob(key):
    """自检 1+2：密钥对得上 protectorKeyId，且能解出 32 字节主密钥。"""
    blob = load_keyblob()
    for slot in (blob.get("slots") or []):
        if slot.get("type") != "static-v1" or not slot.get("wrapped"):
            continue
        if str(slot.get("protectorKeyId") or "").lower() != derive_key_id(key):
            raise AtRestError("密钥与 keyblob 的 protectorKeyId 不匹配")
        master = open_envelope(key, slot["wrapped"], "file")
        if len(master) != 32 or derive_key_id(master) != str(blob.get("keyId") or ""):
            raise AtRestError("keyblob 解出的主密钥校验失败")
        return master
    raise AtRestError("keyblob 里没有可用的 static-v1 槽")


# ----------------------------------------------------------- 进程内存里找密钥
#
# 三档扫描，命中即止，内存段按字节数均分给多个进程并行扫：
#
#   1) 密钥载荷：客户端把 atRestSecretKey 当成 44 字符的规范 base64 传递
#      （`isCanonical32ByteBase64`），JSON.parse 之后它在内存里就是一段孤立的
#      base64 token。正则匹配走 C 层（约 200 MB/s/进程），每个候选只做两次
#      sha256，比逐窗口哈希快两个数量级；
#   2) stride=8 逐窗口哈希：原生 Buffer 至少 16 字节对齐，先按 8 字节栅格找，
#      命中率极高又快 8 倍；
#   3) stride=1 逐窗口哈希：兜底，保证不漏。
#
# 第 1 档的候选同样要过 keyblob 的 GCM 解封校验（unwrap_keyblob）才采信，
# 校验不过就当没找到、继续往后扫，所以快档不会引入误判。
CHUNK = 4 << 20
OVERLAP = 31
#: 分片粒度：把大段切开再分给各进程，否则一个 V8 大堆会把负载全压在单个进程上
SLICE = 64 << 20
MODE_LITERAL = "literal"
MODE_SECRET = "secret"
MODE_WINDOW = "window"
#: 扫描子进程的入口参数（见 _worker_cli）
WORKER_FLAG = "--scan-worker"

#: 密钥载荷的字段名（客户端 `parseKeyPayload` 认的那个键），直接按字面量找
_LITERAL_PLAIN = b"atRestSecretKey"
_LITERAL_UTF16 = b"a\x00t\x00R\x00e\x00s\x00t\x00S\x00e\x00c\x00r\x00e\x00t\x00K\x00e\x00y\x00"
#: 44 字符规范 base64（32 字节 + 一个 '='）
_B64_44 = re.compile(rb"[A-Za-z0-9+/]{43}=")

#: 44 字符规范 base64，且前后不能再接 base64 字符——否则一张 data-url 图片里
#: 就能切出上百万个候选。JS 字符串是 UTF-16，所以两种排布都找。
_SECRET_PATTERNS = (
    (re.compile(rb"(?<![A-Za-z0-9+/])[A-Za-z0-9+/]{43}=(?![A-Za-z0-9+/=])"), False),
    (re.compile(rb"(?<![A-Za-z0-9+/]\x00)(?:[A-Za-z0-9+/]\x00){43}=\x00(?![A-Za-z0-9+/=]\x00)"), True),
)

_WIN32 = {}


def _win32():
    """惰性初始化 Win32 句柄与结构体（在 Linux / macOS 上 import 本模块不应报错）。"""
    if _WIN32:
        return _WIN32
    import ctypes as C
    import ctypes.wintypes as W

    k32 = C.WinDLL("kernel32", use_last_error=True)
    k32.OpenProcess.restype = W.HANDLE
    k32.CreateToolhelp32Snapshot.restype = W.HANDLE
    k32.VirtualQueryEx.restype = C.c_size_t

    class PROCESSENTRY32(C.Structure):
        _fields_ = [
            ("dwSize", W.DWORD), ("cntUsage", W.DWORD), ("th32ProcessID", W.DWORD),
            ("th32DefaultHeapID", C.c_void_p), ("th32ModuleID", W.DWORD),
            ("cntThreads", W.DWORD), ("th32ParentProcessID", W.DWORD),
            ("pcPriClassBase", C.c_long), ("dwFlags", W.DWORD),
            ("szExeFile", C.c_char * 260),
        ]

    class MEMORY_BASIC_INFORMATION(C.Structure):
        _fields_ = [
            ("BaseAddress", C.c_void_p), ("AllocationBase", C.c_void_p),
            ("AllocationProtect", W.DWORD), ("__pad1", W.DWORD),
            ("RegionSize", C.c_size_t), ("State", W.DWORD), ("Protect", W.DWORD),
            ("Type", W.DWORD), ("__pad2", W.DWORD),
        ]

    _WIN32.update({
        "C": C, "W": W, "k32": k32,
        "PROCESSENTRY32": PROCESSENTRY32, "MBI": MEMORY_BASIC_INFORMATION,
        "TH32CS_SNAPPROCESS": 0x00000002,
        "PROCESS_QUERY_INFORMATION": 0x0400,
        "PROCESS_VM_READ": 0x0010,
        "MEM_COMMIT": 0x1000,
        "MEM_PRIVATE": 0x20000,
        "PAGE_GUARD": 0x100,
        "READABLE": 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80,
    })
    return _WIN32


def _list_procs():
    win = _win32()
    C, k32 = win["C"], win["k32"]
    snap = k32.CreateToolhelp32Snapshot(win["TH32CS_SNAPPROCESS"], 0)
    if snap == -1:
        raise AtRestError("CreateToolhelp32Snapshot 失败")
    out = []
    entry = win["PROCESSENTRY32"]()
    entry.dwSize = C.sizeof(entry)
    ok = k32.Process32First(snap, C.byref(entry))
    while ok:
        out.append((entry.th32ProcessID, entry.th32ParentProcessID,
                    entry.szExeFile.decode("latin-1")))
        ok = k32.Process32Next(snap, C.byref(entry))
    k32.CloseHandle(snap)
    return out


#: 桌面客户端主进程的候选 exe 名。不同版本的安装器叫法不同：有 WorkBuddy.exe，
#: 也有 WorkBuddyAI.exe（如 F:\workBuddy\WorkBuddyAI\ 这类默认布局）。
DESKTOP_EXE_CANDIDATES = ("workbuddy.exe", "workbuddyai.exe")


def _main_pids(exe_names=DESKTOP_EXE_CANDIDATES):
    """Electron 主进程 = 父进程不是同一个 exe 的那个（其余是渲染进程）。"""
    if isinstance(exe_names, str):
        exe_names = (exe_names,)
    wanted = {name.lower() for name in exe_names}
    wb = [(pid, ppid) for pid, ppid, name in _list_procs()
          if name.lower() in wanted]
    if not wb:
        raise AtRestError("没有发现正在运行的 %s —— 请先启动并登录桌面客户端"
                          % " / ".join(exe_names))
    wbp = {pid for pid, _ in wb}
    return [pid for pid, ppid in wb if ppid not in wbp]


def _private_regions(handle):
    win = _win32()
    C, k32 = win["C"], win["k32"]
    regions = []
    addr = 0
    mbi = win["MBI"]()
    while addr < 0x7FFFFFFFFFFF:
        if not k32.VirtualQueryEx(handle, C.c_void_p(addr), C.byref(mbi), C.sizeof(mbi)):
            break
        base = mbi.BaseAddress or 0
        size = mbi.RegionSize or 0
        if (mbi.State == win["MEM_COMMIT"] and not (mbi.Protect & win["PAGE_GUARD"])
                and (mbi.Protect & win["READABLE"]) and mbi.Type == win["MEM_PRIVATE"]):
            regions.append((base, size))
        addr = base + size
    return regions


def _scan_regions(job):
    """在若干内存段里找密钥。job = (pid, regions, key_id, mode, stride)。

    job 也来自扫描子进程（JSON，所以 regions 是 list），两种形状都能吃。
    """
    pid, regions, key_id, mode, stride = job
    win = _win32()
    C, k32 = win["C"], win["k32"]
    handle = k32.OpenProcess(
        win["PROCESS_QUERY_INFORMATION"] | win["PROCESS_VM_READ"], False, pid)
    if not handle:
        return []
    target = bytes.fromhex(key_id)
    hits = []
    buf = C.create_string_buffer(CHUNK)
    read = C.c_size_t(0)
    sha = hashlib.sha256
    try:
        for base, size in regions:
            off = 0
            carry = b""
            while off < size:
                want = min(CHUNK, size - off)
                ok = k32.ReadProcessMemory(handle, C.c_void_p(base + off), buf, want,
                                           C.byref(read))
                data = carry + (buf.raw[:read.value] if ok and read.value else b"")
                start = base + off - len(carry)
                if mode == MODE_LITERAL:
                    for literal, utf16 in ((_LITERAL_PLAIN, False), (_LITERAL_UTF16, True)):
                        idx = data.find(literal)
                        while idx >= 0:
                            match = _B64_44.search(data[idx:idx + 128])
                            if match:
                                cand = match.group()
                                if utf16:
                                    cand = cand.replace(b"\x00", b"")
                                key = sha(cand).digest()
                                if sha(key).hexdigest()[:16] == key_id:
                                    hits.append((start + idx, key.hex()))
                                    return hits
                            idx = data.find(literal, idx + 1)
                elif mode == MODE_SECRET:
                    for pattern, utf16 in _SECRET_PATTERNS:
                        for match in pattern.finditer(data):
                            cand = match.group()
                            if utf16:
                                cand = cand.replace(b"\x00", b"")
                            key = sha(cand).digest()
                            if sha(key).hexdigest()[:16] == key_id:
                                hits.append((start + match.start(), key.hex()))
                                return hits
                else:
                    # 对齐栅格按绝对地址算，跨 chunk 也不会错位
                    first = (-start) % stride
                    view = memoryview(data)
                    for i in range(first, len(data) - 31, stride):
                        if sha(view[i:i + 32]).digest()[:8] == target:
                            hits.append((start + i, bytes(view[i:i + 32]).hex()))
                            return hits
                carry = data[-OVERLAP:] if len(data) > OVERLAP else data
                off += want
    finally:
        k32.CloseHandle(handle)
    return hits


def _run_scan(jobs, say):
    """并行跑一批扫描任务，任意一个命中就收工（其余子进程直接杀掉）。

    并行用子进程而不是 multiprocessing：绿色包内置的 Python 是精简发行版，
    根本没有 multiprocessing 模块，而 multiprocessing 的 spawn 在受限环境里
    也常常不可用；`sys.executable 本文件 --scan-worker` 这条路在任何 Python 上
    都能跑。
    """
    if not jobs:
        return []
    if len(jobs) == 1:
        return _scan_regions(jobs[0])
    try:
        return _run_scan_workers(jobs, say)
    except Exception as exc:
        say("子进程并行不可用（%s），改用单进程…" % exc)
    hits = []
    for job in jobs:
        hits.extend(_scan_regions(job))
        if hits:
            break
    return hits


def _run_scan_workers(jobs, say):
    import subprocess
    from concurrent.futures import ThreadPoolExecutor, as_completed

    script = os.path.abspath(__file__)
    live = {}
    lock = threading.Lock()

    def one(index, job):
        proc = subprocess.Popen([sys.executable, script, WORKER_FLAG],
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, text=True,
                                cwd=os.path.dirname(script))
        with lock:
            live[index] = proc
        out, _ = proc.communicate(json.dumps(job))
        return json.loads(out) if out and out.strip() else []

    def kill_all():
        with lock:
            procs = list(live.values())
        for proc in procs:
            try:
                proc.kill()
            except Exception:
                pass

    hits, failures = [], 0
    pool = ThreadPoolExecutor(max_workers=len(jobs))
    try:
        futures = [pool.submit(one, i, job) for i, job in enumerate(jobs)]
        for future in as_completed(futures):
            try:
                result = future.result()
            except Exception:
                failures += 1
                continue
            if result:
                hits.extend(result)
                kill_all()          # 找到了，剩下的段不用再扫
                break
    finally:
        kill_all()
        pool.shutdown(wait=True)
    if failures and failures == len(jobs):
        raise AtRestError("所有扫描子进程都启动失败")
    return hits


def _worker_cli():
    """子进程入口：从 stdin 读一个 job，扫完把命中打到 stdout。

    由 `_run_scan_workers` 以 `python wb_atrest.py --scan-worker` 拉起。
    """
    job = json.loads(sys.stdin.read() or "[]")
    if not job:
        return 2
    hits = _scan_regions(tuple(job))
    sys.stdout.write(json.dumps(hits))
    sys.stdout.flush()
    return 0


def _slices(regions, limit=SLICE):
    """把内存段切成不超过 limit 的片，好让多个进程真的并行。

    不切的话，Electron 那种"一个 1 GB 私有大段 + 一堆小段"的布局会把整块
    大段压在单个 worker 上，其余 worker 扫完小段就闲着。
    """
    out = []
    for base, size in regions:
        off = 0
        while off < size:
            piece = min(limit, size - off)
            out.append((base + off, piece))
            if off + piece >= size:
                break
            off += piece - OVERLAP      # 邻片重叠 31 字节，跨片的窗口不会漏掉
    return out


def hunt_key(key_id=None, workers=None, log=None):
    """从桌面客户端主进程内存里找回静态密钥，返回 64 位小写 hex。

    只读：OpenProcess(PROCESS_VM_READ|PROCESS_QUERY_INFORMATION) + VirtualQueryEx
    + ReadProcessMemory，不往目标进程写任何东西。
    """
    if os.name != "nt":
        raise AtRestError("从进程内存回收密钥只支持 Windows（当前系统：%s）" % sys.platform)

    def say(msg):
        if log:
            log(msg)

    key_id = (key_id or os.environ.get("WB_ATREST_KEYID") or read_key_id()).strip().lower()
    if len(key_id) != 16 or any(c not in "0123456789abcdef" for c in key_id):
        raise AtRestError("keyId 必须是 16 个 hex 字符，得到 %r" % key_id)

    win = _win32()
    k32 = win["k32"]
    workers = max(1, int(workers or min(24, os.cpu_count() or 4)))
    last_error = None
    for pid in _main_pids():
        handle = k32.OpenProcess(
            win["PROCESS_QUERY_INFORMATION"] | win["PROCESS_VM_READ"], False, pid)
        if not handle:
            last_error = ("无法读取 pid %d 的内存（错误码 %d）—— 试试以管理员身份运行"
                          % (pid, k32.GetLastError()))
            say(last_error)
            continue
        regions = _private_regions(handle)
        k32.CloseHandle(handle)
        if not regions:
            continue
        total = sum(size for _, size in regions)
        say("pid %d：%d 个可读私有内存段，共 %.0f MB" % (pid, len(regions), total / 1e6))

        # 大段先切片，再按字节数均分给各 worker
        pieces = _slices(regions)
        slots = max(1, min(workers, len(pieces)))
        groups, load = [], [0] * slots
        for piece in pieces:
            slot = load.index(min(load))
            if slot == len(groups):
                groups.append([])
            groups[slot].append(piece)
            load[slot] += piece[1]

        for mode, stride, label in ((MODE_LITERAL, 1, "密钥载荷字段"),
                                    (MODE_SECRET, 1, "载荷 base64"),
                                    (MODE_WINDOW, 8, "8 字节栅格"),
                                    (MODE_WINDOW, 1, "逐字节兜底")):
            jobs = [(pid, group, key_id, mode, stride) for group in groups if group]
            started = time.time()
            hits = _run_scan(jobs, say)
            say("按%s扫描完成：%.1fs，%d 个候选" % (label, time.time() - started, len(hits)))
            for addr, key_hex in hits:
                say("候选密钥 @0x%X" % addr)
                try:
                    unwrap_keyblob(bytes.fromhex(key_hex))     # 用 GCM 解封复核
                except Exception:
                    continue
                return key_hex
    raise AtRestError(last_error or "没能在桌面客户端进程内存里找到密钥（keyId=%s）" % key_id)


# ------------------------------------------------------------------ 内存密钥
#: 只放在内存里：找到一次即可反复使用，不落盘、不进日志、不随凭证导出。
_KEY_CACHE = {"key": None, "recoveredAt": 0.0}
_KEY_LOCK = threading.Lock()


def cached_key():
    """取进程内缓存的密钥（顺带用 keyblob 复核一次，客户端换钥即失效）。"""
    with _KEY_LOCK:
        key = _KEY_CACHE["key"]
    if not key:
        return None
    try:
        unwrap_keyblob(key)
    except Exception:
        with _KEY_LOCK:
            _KEY_CACHE["key"] = None
        return None
    return key


def forget_key():
    """丢掉内存里的密钥（下次导入会重新回收）。"""
    with _KEY_LOCK:
        _KEY_CACHE["key"] = None
    return True


def recover_key(force=False, log=None):
    """拿到可用密钥：内存缓存 ->（必要时）进程内存现场回收。失败抛 AtRestError。"""
    if not force:
        cached = cached_key()
        if cached:
            return cached
    key_hex = hunt_key(log=log)
    key = bytes.fromhex(key_hex)
    unwrap_keyblob(key)          # 自检 1+2 不过就别留下
    with _KEY_LOCK:
        _KEY_CACHE["key"] = key
        _KEY_CACHE["recoveredAt"] = time.time()
    if log:
        log("已从桌面客户端进程内存回收 at-rest 密钥（keyId=%s），仅保存在内存中"
            % derive_key_id(key))
    return key


def status():
    """给看板用的只读体检结果（不触发任何扫描）。"""
    info = {
        "platform": "windows" if os.name == "nt" else sys.platform,
        "huntSupported": os.name == "nt",
        "keyblob": "",
        "keyblobFound": False,
        "keyId": "",
        "keyCached": False,
        "error": "",
    }
    try:
        info["keyblob"] = keyblob_path()
        info["keyblobFound"] = os.path.isfile(info["keyblob"])
    except AtRestError as exc:
        info["error"] = str(exc)
    try:
        info["keyId"] = read_key_id()
    except Exception as exc:
        info["error"] = info["error"] or str(exc)
    try:
        info["keyCached"] = cached_key() is not None
    except Exception:
        info["keyCached"] = False
    return info


if __name__ == "__main__":
    # 只给 _run_scan_workers 用：把本文件当脚本拉起来当扫描子进程。
    if WORKER_FLAG in sys.argv:
        sys.exit(_worker_cli())
    sys.exit("this module is a library; the only CLI entry is %s" % WORKER_FLAG)
