"""Makes marzban.sql, marzban_xray.json and marzban.tokens.json, which the import tests read.

There is no Marzban code in them: the tables are written the way SQLAlchemy
makes them from Marzban's models (`app/db/models.py`; an `Enum` column holds
the member's *name*, `expire` is an integer of seconds, other times are
DATETIME text), the Xray configuration is the one Marzban keeps in a file,
and the tokens are made the way Marzban's `create_subscription_token` made
them (a JWT in the early versions, then `base64(username,time)` followed by
ten characters of a SHA-256 of it and the key). Needs PyJWT for the JWT.
"""
import base64
import hashlib
import json
from pathlib import Path

import jwt as pyjwt

here = Path(__file__).parent
SECRET = "c2f6a1e49b7d3058e1a4c7f9b2d6803e5a1c4e7f90b3d6a28c5e1f4b7a0d3c69"
GIB = 1024 ** 3


def q(value):
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "1" if value else "0"
    if isinstance(value, (int, float)):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"


def insert(table, **row):
    return f'INSERT INTO "{table}" ({", ".join(row)}) VALUES ({", ".join(q(v) for v in row.values())});'


def uuid(n):
    return f"{n:08x}-2222-4333-8444-{n:012x}"


xray = {
    "log": {"loglevel": "warning"},
    "inbounds": [
        {"tag": "VLESS TCP REALITY", "listen": "0.0.0.0", "port": 443, "protocol": "vless",
         "settings": {"clients": [], "decryption": "none"},
         "streamSettings": {"network": "tcp", "tcpSettings": {}, "security": "reality",
                            "realitySettings": {"show": False, "dest": "yahoo.com:443", "xver": 0,
                                                "serverNames": ["yahoo.com"],
                                                "privateKey": "wP3sQ8nL1vY6tR0kE5zD9cB2xM7aF4hJ1gU3oI6yT8w",
                                                "shortIds": ["ab12cd34"]}},
         "sniffing": {"enabled": True, "destOverride": ["http", "tls"]}},
        {"tag": "VMess WS", "listen": "0.0.0.0", "port": 8080, "protocol": "vmess", "settings": {"clients": []},
         "streamSettings": {"network": "ws", "wsSettings": {"path": "/vmess"}, "security": "none"}},
        {"tag": "Trojan TCP", "listen": "0.0.0.0", "port": 2083, "protocol": "trojan", "settings": {"clients": []},
         "streamSettings": {"network": "tcp", "security": "none"}},
        {"tag": "Shadowsocks TCP", "listen": "0.0.0.0", "port": 1080, "protocol": "shadowsocks",
         "settings": {"clients": [], "network": "tcp,udp"}},
        {"tag": "API", "listen": "127.0.0.1", "port": 62789, "protocol": "dokodemo-door",
         "settings": {"address": "127.0.0.1"}},
    ],
    "outbounds": [{"protocol": "freedom", "tag": "DIRECT"}],
    "routing": {"rules": []},
}

users = [  # id, name, status, used, limit, expire(seconds), strategy, created, extra
    (1, "zed", "active", 5 * GIB, 20 * GIB, 1830000000, "month", "2026-01-12 10:00:00.000000", dict(note="main", online_at="2026-10-22 18:00:00.000000")),
    (2, "yan", "on_hold", 0, 0, None, "no_reset", "2026-09-01 08:00:00.000000", dict(on_hold_expire_duration=604800, on_hold_timeout="2026-12-31 00:00:00.000000")),
    (3, "xia", "limited", int(2.5 * GIB), 2 * GIB, None, "no_reset", "2026-03-03 03:03:03.000000", dict()),
    (4, "wei", "disabled", 0, 0, None, "no_reset", "2026-05-05 05:05:05.000000", dict(sub_revoked_at="2026-07-01 00:00:00.000000")),
]
# user → [(proxy id, type name, settings)]
proxies = {
    1: [(1, "VMess", {"id": uuid(1)}), (2, "VLESS", {"id": uuid(1), "flow": "xtls-rprx-vision"}),
        (3, "Trojan", {"password": "trojan-zed"}), (4, "Shadowsocks", {"password": "ss-zed", "method": "chacha20-ietf-poly1305"})],
    2: [(5, "VMess", {"id": uuid(2)})],
    3: [(6, "VLESS", {"id": uuid(3), "flow": ""}), (7, "Trojan", {"password": "trojan-xia"})],
    4: [(8, "VMess", {"id": uuid(4)}), (9, "VLESS", {"id": uuid(4), "flow": ""})],
}

sql = ["BEGIN TRANSACTION;",
       "CREATE TABLE jwt (id INTEGER NOT NULL, secret_key VARCHAR(64) NOT NULL, PRIMARY KEY (id));",
       "CREATE TABLE inbounds (id INTEGER NOT NULL, tag VARCHAR(256) NOT NULL, PRIMARY KEY (id), UNIQUE (tag));",
       "CREATE TABLE admins (id INTEGER NOT NULL, username VARCHAR(34), hashed_password VARCHAR(128), created_at DATETIME, is_sudo BOOLEAN, PRIMARY KEY (id));",
       "CREATE TABLE users (id INTEGER NOT NULL, username VARCHAR(34), status VARCHAR(8) NOT NULL, used_traffic BIGINT, data_limit BIGINT, "
       "expire INTEGER, created_at DATETIME, admin_id INTEGER, data_limit_reset_strategy VARCHAR(5) NOT NULL, sub_revoked_at DATETIME, "
       "note VARCHAR(500), sub_updated_at DATETIME, sub_last_user_agent VARCHAR(512), online_at DATETIME, edit_at DATETIME, "
       "on_hold_timeout DATETIME, on_hold_expire_duration BIGINT, auto_delete_in_days INTEGER, last_status_change DATETIME, "
       "PRIMARY KEY (id), UNIQUE (username), FOREIGN KEY(admin_id) REFERENCES admins (id));",
       "CREATE TABLE proxies (id INTEGER NOT NULL, user_id INTEGER, type VARCHAR(11) NOT NULL, settings JSON NOT NULL, PRIMARY KEY (id), "
       "FOREIGN KEY(user_id) REFERENCES users (id));",
       "CREATE TABLE exclude_inbounds_association (proxy_id INTEGER, inbound_tag VARCHAR(256), FOREIGN KEY(proxy_id) REFERENCES proxies (id), "
       "FOREIGN KEY(inbound_tag) REFERENCES inbounds (tag));",
       "CREATE TABLE hosts (id INTEGER NOT NULL, remark VARCHAR(256) NOT NULL, address VARCHAR(256) NOT NULL, port INTEGER, "
       "inbound_tag VARCHAR(256) NOT NULL, sni VARCHAR(256), host VARCHAR(256), security VARCHAR(14) NOT NULL, alpn VARCHAR(14) NOT NULL, "
       "fingerprint VARCHAR(9) NOT NULL, allowinsecure BOOLEAN, is_disabled BOOLEAN, mux_enable BOOLEAN NOT NULL DEFAULT 0, "
       "fragment_setting VARCHAR(100), noise_setting VARCHAR(2000), random_user_agent BOOLEAN NOT NULL DEFAULT 0, "
       "use_sni_as_host BOOLEAN NOT NULL DEFAULT 0, path VARCHAR(256), PRIMARY KEY (id), FOREIGN KEY(inbound_tag) REFERENCES inbounds (tag));",
       "CREATE TABLE user_templates (id INTEGER NOT NULL, name VARCHAR(64), data_limit BIGINT, expire_duration BIGINT, "
       "username_prefix VARCHAR(20), username_suffix VARCHAR(20), PRIMARY KEY (id), UNIQUE (name));",
       "CREATE TABLE template_inbounds_association (user_template_id INTEGER, inbound_tag VARCHAR(256), "
       "FOREIGN KEY(user_template_id) REFERENCES user_templates (id), FOREIGN KEY(inbound_tag) REFERENCES inbounds (tag));",
       "CREATE TABLE node_user_usages (id INTEGER NOT NULL, created_at DATETIME, user_id INTEGER, node_id INTEGER, used_traffic BIGINT, "
       "PRIMARY KEY (id), UNIQUE (created_at, user_id, node_id));",
       "CREATE TABLE user_usage_logs (id INTEGER NOT NULL, user_id INTEGER, used_traffic_at_reset BIGINT NOT NULL, reset_at DATETIME, "
       "PRIMARY KEY (id), FOREIGN KEY(user_id) REFERENCES users (id));",
       "CREATE TABLE next_plans (id INTEGER NOT NULL, user_id INTEGER, data_limit BIGINT NOT NULL, expire INTEGER, "
       "add_remaining_traffic BOOLEAN NOT NULL DEFAULT 0, fire_on_either BOOLEAN NOT NULL DEFAULT 1, PRIMARY KEY (id), FOREIGN KEY(user_id) REFERENCES users (id));",
       insert("jwt", id=1, secret_key=SECRET)]
for n, tag in enumerate(["VLESS TCP REALITY", "VMess WS", "Trojan TCP", "Shadowsocks TCP"], 1):
    sql.append(insert("inbounds", id=n, tag=tag))
for id_, name, status, used, limit, expire, strategy, created, extra in users:
    sql.append(insert("users", id=id_, username=name, status=status, used_traffic=used, data_limit=limit, expire=expire,
                      created_at=created, data_limit_reset_strategy=strategy, **extra))
for user, rows in proxies.items():
    for pid, kind, settings in rows:
        sql.append(insert("proxies", id=pid, user_id=user, type=kind, settings=json.dumps(settings)))
sql.append(insert("exclude_inbounds_association", proxy_id=9, inbound_tag="VLESS TCP REALITY"))
sql.append(insert("hosts", id=1, remark="🚀 {USERNAME} [{DATA_LEFT}]", address="cdn.example.com", port=2053, inbound_tag="VMess WS",
                  sni="cdn.example.com", host="cdn.example.com", security="tls", alpn="h2_http_1_1", fingerprint="chrome",
                  allowinsecure=False, is_disabled=False, mux_enable=True, path="/vmess"))
sql.append(insert("hosts", id=2, remark="plain", address="direct.example.com", port=None, inbound_tag="Trojan TCP", sni="", host="",
                  security="inbound_default", alpn="none", fingerprint="none", allowinsecure=None, is_disabled=False))
sql.append(insert("user_templates", id=1, name="basic-5g", data_limit=5 * GIB, expire_duration=30 * 86400))
sql.append(insert("template_inbounds_association", user_template_id=1, inbound_tag="VMess WS"))
sql.append(insert("template_inbounds_association", user_template_id=1, inbound_tag="Trojan TCP"))
sql.append(insert("node_user_usages", id=1, created_at="2026-10-22 05:00:00.000000", user_id=1, node_id=None, used_traffic=GIB))
sql.append(insert("node_user_usages", id=2, created_at="2026-10-22 05:00:00.000000", user_id=1, node_id=1, used_traffic=GIB // 2))
sql.append(insert("user_usage_logs", id=1, user_id=1, used_traffic_at_reset=7 * GIB, reset_at="2026-09-12 10:00:00.000000"))
sql.append(insert("next_plans", id=1, user_id=3, data_limit=10 * GIB, expire=2592000, add_remaining_traffic=False, fire_on_either=True))
sql.append("COMMIT;")
(here / "marzban.sql").write_text("\n".join(sql) + "\n")
(here / "marzban_xray.json").write_text(json.dumps(xray, indent=2) + "\n")


def b64(text):
    return base64.b64encode(text.encode(), altchars=b"-_").decode().rstrip("=")


def legacy(username, ts):
    data = b64(f"{username},{ts}")
    return data + base64.b64encode(hashlib.sha256((data + SECRET).encode()).digest(), altchars=b"-_").decode()[:10]


def stamp(y, m, d):
    import calendar
    return calendar.timegm((y, m, d, 0, 0, 0))


tokens = [
    {"name": "legacy_zed", "token": legacy("zed", stamp(2026, 2, 1)), "who": "zed", "valid": True},
    {"name": "jwt_zed", "token": pyjwt.encode({"sub": "zed", "access": "subscription", "iat": stamp(2026, 2, 1)}, SECRET, algorithm="HS256"),
     "who": "zed", "valid": True},
    {"name": "legacy_wei_before_revoke", "token": legacy("wei", stamp(2026, 6, 1)), "who": "wei", "valid": False},
    {"name": "legacy_wei_after_revoke", "token": legacy("wei", stamp(2026, 8, 1)), "who": "wei", "valid": True},
    {"name": "legacy_unknown", "token": legacy("nobody", stamp(2026, 2, 1)), "who": None, "valid": False},
    {"name": "legacy_zed_before_creation", "token": legacy("zed", stamp(2025, 12, 1)), "who": "zed", "valid": False},
]
(here / "marzban.tokens.json").write_text(json.dumps({"secret": SECRET, "tokens": tokens}, indent=2) + "\n")
print("written")
