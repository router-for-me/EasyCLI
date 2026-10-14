"""wb_fingerprint.py —— 统一设备指纹稳定派生模块 (derive_id)

无论是国内版 (copilot.tencent.com / codebuddy.cn) 还是国际版 (www.workbuddy.ai)，
均通过本模块基于账号 UID 和加盐哈希单向派生出固定的伪物理设备特征 (machineId, sessionId, reqId)，
确保每个账号永远来自一台固定的物理设备，且多账号之间天然隔离防风控关联。
"""
import hashlib
import time


def derive_id(uid: str, salt: str) -> str:
    """由 uid + salt 稳定派生一个 36 位十六进制设备/会话标识。

    幂等：同一账号每次调用产生相同值，彻底避免随机机器码导致的上游风控。
    """
    seed = f"{salt}:{uid or 'anonymous'}"
    return hashlib.md5(seed.encode("utf-8")).hexdigest()[:36]


def generate_request_id(uid: str) -> str:
    """生成带稳定前缀与微秒时间戳的防风控 X-Request-ID。"""
    prefix = derive_id(uid, "req")
    suffix = str(time.time_ns() % 1000000).zfill(6)
    return f"{prefix}-{suffix}"


def get_desktop_fingerprint(uid: str, nickname: str = "", os_name: str = "win32") -> dict:
    """生成上报事件所需的标准完整桌面端指纹。"""
    now = int(time.time() * 1000)
    return {
        "timezone": "Asia/Shanghai",
        "reportDelay": 2000,
        "userId": uid,
        "username": nickname,
        "userNickname": nickname,
        "product": "SaaS",
        "releaseDate": 1789036585355,
        "commit": "5f9692923c93033111c51ad7b003eb80204a9b75",
        "ideName": "WorkBuddy",
        "ideType": "WorkBuddy",
        "ideVersion": "5.5.6",
        "machineId": derive_id(uid, "machine"),
        "sessionId": derive_id(uid, "session"),
        "extName": "workbuddy-desktop",
        "extVersion": "5.5.6",
        "os": os_name,
        "arch": "x64",
        "osVersion": "10.0.26220",
        "cpuCores": 20,
        "memorySize": 24,
        "timestamp": now,
        "presentAt": now,
    }
