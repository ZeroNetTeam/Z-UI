"""Makes pasarguard.sql and pasarguard.tokens.json, which the import tests read.

The database is made with PasarGuard's own models (so its tables, column
types and the way lists, enums, dates and JSON are written are PasarGuard's,
not a guess), and the subscription tokens are made by PasarGuard's own
`app/utils/jwt.py` and checked by its own `get_subscription_payload`.

Run with a PasarGuard checkout and SQLAlchemy 2, PyJWT and aiocache installed:

    python make_pasarguard.py /path/to/pasarguard-panel

Nothing here is part of the panel; it only says where the fixtures come from.
"""
import asyncio
import base64
import hashlib
import json
import os
import random
import sqlite3
import sys
import time
import types
from datetime import datetime
from pathlib import Path
from types import SimpleNamespace

root = sys.argv[1]
here = Path(__file__).parent


class Permissive(types.ModuleType):
    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)
        return SimpleNamespace(is_sqlite=True, is_mysql=False, is_postgresql=False,
                               url="sqlite+aiosqlite:///:memory:", echo_queries=False, pool_size=1,
                               max_overflow=1, pool_recycle=1, pool_timeout=1, connect_timeout=1)


cfg = Permissive("config")
sys.modules["config"] = cfg
sys.path.insert(0, root)
pkg = types.ModuleType("app")
pkg.__path__ = [root + "/app"]
sys.modules["app"] = pkg

from sqlalchemy import create_engine  # noqa: E402
from sqlalchemy.orm import Session  # noqa: E402

from app.db import models as m  # noqa: E402
from app.db.base import Base  # noqa: E402

random.seed(11)
SECRET = "5f3a9c1e7b2d4f60a8c3e1d9b7f50426c8a1e3d5f7092b4c6e8a0d2f4b6c8e1a"
GIB = 1024 ** 3


def uuid(n):
    return f"{n:08x}-aaaa-4bbb-8ccc-{n:012x}"


engine = create_engine("sqlite://")
Base.metadata.create_all(engine)
D = datetime

with Session(engine) as db:
    db.add(m.JWT(secret_key=SECRET))

    xray = {
        "log": {"loglevel": "warning"},
        "inbounds": [
            {"tag": "VLESS TCP REALITY", "listen": "0.0.0.0", "port": 443, "protocol": "vless",
             "settings": {"clients": [], "decryption": "none"},
             "streamSettings": {"network": "tcp", "tcpSettings": {}, "security": "reality",
                                "realitySettings": {"show": False, "dest": "yahoo.com:443", "xver": 0,
                                                    "serverNames": ["yahoo.com", "www.yahoo.com"],
                                                    "privateKey": base64.urlsafe_b64encode(random.randbytes(32)).decode().rstrip("="),
                                                    "shortIds": ["ab12cd34"]}},
             "sniffing": {"enabled": True, "destOverride": ["http", "tls", "quic"]}},
            {"tag": "Trojan WS", "listen": "0.0.0.0", "port": 8443, "protocol": "trojan",
             "settings": {"clients": []},
             "streamSettings": {"network": "ws", "wsSettings": {"path": "/trojan"}, "security": "none"}},
            {"tag": "VMess WS", "listen": "0.0.0.0", "port": 8080, "protocol": "vmess",
             "settings": {"clients": []},
             "streamSettings": {"network": "ws", "wsSettings": {"path": "/vmess"}, "security": "none"}},
            {"tag": "Shadowsocks TCP", "listen": "0.0.0.0", "port": 1080, "protocol": "shadowsocks",
             "settings": {"clients": [], "network": "tcp,udp"}},
            {"tag": "API_INBOUND", "listen": "127.0.0.1", "port": 62789, "protocol": "dokodemo-door",
             "settings": {"address": "127.0.0.1"}},
        ],
        "outbounds": [{"protocol": "freedom", "tag": "DIRECT"}, {"protocol": "blackhole", "tag": "BLOCK"}],
        "routing": {"rules": [{"inboundTag": ["API_INBOUND"], "outboundTag": "API", "type": "field"}]},
    }
    db.add(m.CoreConfig(name="Default", config=xray, exclude_inbound_tags={"API_INBOUND"}, fallbacks_inbound_tags=set()))
    inbounds = {tag: m.ProxyInbound(tag=tag) for tag in ["VLESS TCP REALITY", "Trojan WS", "VMess WS", "Shadowsocks TCP"]}
    db.add_all(inbounds.values())
    db.flush()

    # The three roles PasarGuard's own migration seeds, and two admins.
    owner_role = m.AdminRole(name="owner", is_owner=True)
    owner_role.id = 1
    admin_role = m.AdminRole(name="administrator")
    admin_role.id = 2
    operator_role = m.AdminRole(name="operator")
    operator_role.id = 3
    db.add_all([owner_role, admin_role, operator_role])
    db.flush()
    owner_admin = m.Admin(username="root", hashed_password="$2b$12$notarealhashnotarealhashnotarealhashnotarealhash", role_id=1)
    shop = m.Admin(username="shop", hashed_password="$2b$12$notarealhashnotarealhashnotarealhashnotarealhash", role_id=3)
    db.add_all([owner_admin, shop])
    db.flush()

    premium = m.Group(name="Premium", inbounds=[inbounds["VLESS TCP REALITY"], inbounds["Trojan WS"]])
    basic = m.Group(name="Basic", inbounds=[inbounds["VMess WS"], inbounds["Shadowsocks TCP"]])
    retired = m.Group(name="Retired", inbounds=[inbounds["VMess WS"]], is_disabled=True)
    db.add_all([premium, basic, retired])

    def proxies(n, flow="", method="chacha20-ietf-poly1305", vmess=None, ss=None):
        return {"vmess": {"id": vmess or uuid(n)}, "vless": {"id": uuid(n), "flow": flow},
                "trojan": {"password": f"trojan-pass-{n}"},
                "shadowsocks": {"password": ss or f"ss-pass-{n}", "method": method}}

    users = {}

    def user(name, n, created, **kw):
        expire = kw.pop("expire", None)
        if name in ("alice", "carol"):
            kw["admin_id"] = shop.id
        groups = kw.pop("groups", [])
        u = m.User(username=name, proxy_settings=kw.pop("proxy_settings", proxies(n)), **kw)
        u.created_at = created
        u.expire = expire
        u.groups = groups
        users[name] = u
        db.add(u)
        return u

    user("alice", 1, D(2026, 1, 10, 9, 30), status=m.UserStatus.active, used_traffic=3 * GIB, data_limit=10 * GIB,
         data_limit_reset_strategy=m.DataLimitResetStrategy.month, expire=D(2027, 3, 1, 0, 0),
         note="VIP customer", online_at=D(2026, 10, 22, 18, 5), groups=[premium, basic],
         proxy_settings=proxies(1, flow="xtls-rprx-vision"))
    user("bob_sh", 2, D(2026, 8, 2, 12, 0), status=m.UserStatus.on_hold, on_hold_expire_duration=30 * 86400,
         on_hold_timeout=D(2026, 12, 1, 0, 0), groups=[basic])
    user("carol", 3, D(2026, 3, 3, 8, 0), status=m.UserStatus.limited, used_traffic=int(1.2 * GIB), data_limit=GIB,
         groups=[premium], hwid_limit=2)
    user("dave", 4, D(2026, 1, 5, 7, 0), status=m.UserStatus.expired, used_traffic=2 * GIB, data_limit=5 * GIB,
         expire=D(2026, 9, 1, 0, 0), sub_revoked_at=D(2026, 6, 1, 0, 0), groups=[basic])
    user("eve.test", 5, D(2026, 5, 5, 5, 5), status=m.UserStatus.disabled, groups=[],
         proxy_settings=proxies(5, vmess=uuid(55), ss="other-ss-pass"))
    user("frank", 6, D(2026, 2, 14, 14, 0), status=m.UserStatus.active, used_traffic=GIB, data_limit=20 * GIB,
         data_limit_reset_strategy=m.DataLimitResetStrategy.week, groups=[premium])
    db.flush()

    db.add(m.NextPlan(user_id=users["frank"].id, user_template_id=None, data_limit=50 * GIB, expire=30 * 86400,
                      add_remaining_traffic=True))
    db.add(m.UserUsageResetLogs(user_id=users["frank"].id, used_traffic_at_reset=5 * GIB))
    template = m.UserTemplate(name="monthly-10g", username_prefix=None, username_suffix=None, extra_settings=None,
                              groups=[premium], data_limit=10 * GIB, expire_duration=30 * 86400)
    db.add(template)

    host = m.ProxyHost(remark="{USERNAME} - {DATA_LEFT}", port=2053, path="/cdn", priority=1, allowinsecure=False,
                       address={"cdn.example.com"}, sni={"cdn.example.com"}, host={"cdn.example.com"},
                       security=m.ProxyHostSecurity.tls, alpn=[m.ProxyHostALPN.h2, m.ProxyHostALPN.h1],
                       fingerprint=m.ProxyHostFingerprint.chrome, mux_settings={"enabled": True}, status=[])
    host.inbound_tag = "Trojan WS"
    db.add(host)
    host2 = m.ProxyHost(remark="reality direct", port=None, path=None, priority=2, allowinsecure=None,
                        address={"direct.example.com"}, is_disabled=True, alpn=[], status=[],
                        fingerprint=m.ProxyHostFingerprint.none)
    host2.inbound_tag = "VLESS TCP REALITY"
    db.add(host2)

    for node in (None, 1):
        for hour, used in ((5, GIB), (6, GIB // 2)):
            db.add(m.NodeUserUsage(created_at=D(2026, 10, 22, hour), user_id=users["alice"].id, node_id=node,
                                   used_traffic=used if node is None else used // 4))
    db.add(m.NodeUserUsage(created_at=D(2026, 10, 21, 23), user_id=users["carol"].id, node_id=None, used_traffic=GIB))
    db.commit()

raw = engine.raw_connection()
sql = "\n".join(raw.driver_connection.iterdump()) + "\n"
(here / "pasarguard.sql").write_text(sql)

# ---- tokens, made and judged by PasarGuard's own code
import jwt as pyjwt  # noqa: E402

# PasarGuard's token code, loaded as it is; only its way of reading the key
# from the database is replaced (below, per call) by the key given here.
import importlib.util  # noqa: E402

crud = types.ModuleType("app.db.crud")
crud.__path__ = []
general = types.ModuleType("app.db.crud.general")
general.get_jwt_secret_key = lambda db: None
sys.modules["app.db.crud"] = crud
sys.modules["app.db.crud.general"] = general
spec = importlib.util.spec_from_file_location("pasarguard_jwt", root + "/app/utils/jwt.py")
pg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pg)


def secret_of(value):
    async def get():
        return value
    return get


def stamp(*a):
    return int(D(*a).replace(tzinfo=__import__("datetime").timezone.utc).timestamp())


def b64(data):
    return base64.b64encode(data.encode(), altchars=b"-_").decode().rstrip("=")


def v3(user_id, ts, secret=SECRET):
    pg.get_secret_key = secret_of(secret)
    real = time.time
    time.time = lambda: ts
    try:
        return asyncio.run(pg.create_subscription_token(user_id))
    finally:
        time.time = real


def legacy(username, ts, hexsig=False):
    data = b64(f"{username},{ts}")
    digest = hashlib.sha256((data + SECRET).encode()).digest()
    sig = hashlib.sha256((data + SECRET).encode()).hexdigest()[:10] if hexsig else base64.b64encode(digest, altchars=b"-_").decode()[:10]
    return data + sig


def jwt_for(username, ts, access="subscription", secret=SECRET, exp=None):
    payload = {"sub": username, "access": access, "iat": ts}
    if exp:
        payload["exp"] = exp
    return pyjwt.encode(payload, secret, algorithm="HS256")


tokens = {
    "v3_alice": v3(1, stamp(2026, 2, 1)),
    "v3_bob": v3(2, stamp(2026, 8, 3)),
    "v3_dave_before_revoke": v3(4, stamp(2026, 3, 1)),
    "v3_dave_after_revoke": v3(4, stamp(2026, 7, 1)),
    "v3_wrong_key": v3(1, stamp(2026, 2, 1), secret="not-the-panels-key"),
    "jwt_alice": jwt_for("alice", stamp(2026, 2, 1)),
    "jwt_expired": jwt_for("alice", stamp(2026, 2, 1), exp=stamp(2026, 3, 1)),
    "jwt_not_a_subscription": jwt_for("alice", stamp(2026, 2, 1), access="admin"),
    "legacy_carol": legacy("carol", stamp(2026, 4, 1)),
    "legacy_carol_hex": legacy("carol", stamp(2026, 4, 1), hexsig=True),
    "legacy_carol_tampered": legacy("carol", stamp(2026, 4, 1))[:-1] + "x",
}


def claim(token):
    pg.get_secret_key = secret_of(SECRET)
    result = asyncio.run(pg.get_subscription_payload(token))
    if result is None:
        return None
    out = {"created_at": int(result["created_at"].timestamp())}
    out["user_id" if "user_id" in result else "username"] = result.get("user_id", result.get("username"))
    return out


out = {"secret": SECRET, "tokens": [{"name": n, "token": t, "claim": claim(t)} for n, t in tokens.items()]}
(here / "pasarguard.tokens.json").write_text(json.dumps(out, indent=2) + "\n")
print(len(sql), "bytes of SQL;", sum(1 for t in out["tokens"] if t["claim"]), "tokens accepted by PasarGuard,",
      sum(1 for t in out["tokens"] if not t["claim"]), "refused")
