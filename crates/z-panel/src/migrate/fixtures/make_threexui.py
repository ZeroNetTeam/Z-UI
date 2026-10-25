"""Makes threexui_classic.sql and threexui_v3.sql, which the import tests read.

There is no 3X-UI code in them: the tables are written the way GORM makes
them from 3X-UI's models (`internal/database/model/model.go` and
`internal/xray/client_traffic.go`; column names and types from their tags),
and the JSON in them is what 3X-UI's own forms write (a client's `totalGB` is
in bytes, `expiryTime` in milliseconds and negative for "this long after the
first use", REALITY's public key sits under `realitySettings.settings`).
Run with python3 and openssl; it writes next to itself.
"""
import json
import subprocess
import tempfile
from pathlib import Path

here = Path(__file__).parent
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
    return f"INSERT INTO `{table}` ({', '.join('`'+c+'`' for c in row)}) VALUES ({', '.join(q(v) for v in row.values())});"


def certificate():
    with tempfile.TemporaryDirectory() as d:
        subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                        "-nodes", "-keyout", f"{d}/k.pem", "-out", f"{d}/c.pem", "-days", "3650",
                        "-subj", "/CN=trojan.example.com", "-addext", "subjectAltName=DNS:trojan.example.com"],
                       check=True, capture_output=True)
        cert = Path(f"{d}/c.pem").read_text().strip().splitlines()
        key = Path(f"{d}/k.pem").read_text().strip().splitlines()
    return cert, key


def uuid(n):
    return f"{n:08x}-1111-4222-8333-{n:012x}"


def reality(public):
    return {"network": "tcp", "security": "reality", "externalProxy": [],
            "realitySettings": {"show": False, "xver": 0, "dest": "yahoo.com:443", "serverNames": ["yahoo.com"],
                                "privateKey": "uJgMdjW0CJ3x9Eb0QxL2fT8yXoGzA4rVw1pHkNnS5cU",
                                "minClient": "", "maxClient": "", "maxTimediff": 0, "shortIds": ["ab12cd34"],
                                "settings": {"publicKey": public, "fingerprint": "chrome", "serverName": "", "spiderX": "/"}},
            "tcpSettings": {"acceptProxyProtocol": False, "header": {"type": "none"}}}


PUBLIC = "Zk3m8vQ1nR6tY0wA9bC2dE4fG7hJ5lN8pS1uV3xX6zA"
SNIFF = {"enabled": True, "destOverride": ["http", "tls", "quic", "fakedns"], "metadataOnly": False, "routeOnly": False}
cert, key = certificate()
tls = {"network": "tcp", "security": "tls", "externalProxy": [],
       "tlsSettings": {"serverName": "trojan.example.com", "minVersion": "1.2", "maxVersion": "1.3", "cipherSuites": "",
                       "rejectUnknownSni": False, "disableSystemRoot": False, "enableSessionResumption": False,
                       "certificates": [{"certificateFile": "", "keyFile": "", "certificate": cert, "key": key,
                                         "oneTimeLoading": False, "usage": "encipherment", "buildChain": False}],
                       "alpn": ["h2", "http/1.1"], "settings": {"allowInsecure": False, "fingerprint": "chrome"}},
       "tcpSettings": {"acceptProxyProtocol": False, "header": {"type": "none"}}}
ws = {"network": "ws", "security": "none", "externalProxy": [],
      "wsSettings": {"acceptProxyProtocol": False, "path": "/ws", "host": "", "headers": {}}}


def client(email, n, *, flow="", total=0, expiry=0, enable=True, sub, limit_ip=0, tg=0, comment="", reset=0, **extra):
    c = {"id": uuid(n), "flow": flow, "email": email, "limitIp": limit_ip, "totalGB": total, "expiryTime": expiry,
         "enable": enable, "tgId": tg, "subId": sub, "comment": comment, "reset": reset,
         "created_at": 1768037400000, "updated_at": 1768037400000}
    c.update(extra)
    return c


VISION = "xtls-rprx-vision"
ali = client("ali@home", 1, flow=VISION, total=10 * GIB, expiry=1803859200000, sub="ali0sub1id", limit_ip=2,
             tg=12345, comment="friend", reset=30)
neg = client("neg@home", 2, flow=VISION, expiry=-2592000000, sub="negsub")
off = client("off@home", 3, flow=VISION, enable=False, sub="offsub")
quota = client("quota@home", 4, flow=VISION, total=GIB, sub="quotasub")
old = client("old@home", 5, flow=VISION, expiry=1700000000000, sub="oldsub")
ali_vmess = client("ali@home", 11, total=10 * GIB, expiry=1803859200000, sub="ali0sub1id")
bob = client("bob@home", 6, sub="bobsub")
shared = client("shared@home", 7, sub="bobsub")
troj = {"password": "trojan-secret-1", "flow": "", "email": "troj@home", "limitIp": 0, "totalGB": 0, "expiryTime": 0,
        "enable": True, "tgId": 0, "subId": "trojsub", "comment": "", "reset": 0}
hy = {"auth": "hy-auth-1", "email": "hy@home", "limitIp": 0, "totalGB": 0, "expiryTime": 0, "enable": True,
      "tgId": 0, "subId": "hysub", "comment": "", "reset": 0}
ss = {"method": "", "password": "ss-client-pass", "email": "ss@home", "limitIp": 0, "totalGB": 0, "expiryTime": 0,
      "enable": True, "tgId": 0, "subId": "sssub", "comment": "", "reset": 0}

inbounds = [
    dict(id=1, remark="reality", port=443, protocol="vless", tag="inbound-443",
         settings={"clients": [ali, neg, off, quota, old], "decryption": "none", "fallbacks": []},
         stream=reality(PUBLIC)),
    dict(id=2, remark="vmess-ws", port=8443, protocol="vmess", tag="inbound-8443",
         settings={"clients": [ali_vmess, bob, shared]}, stream=ws),
    dict(id=3, remark="trojan-tls", port=2096, protocol="trojan", tag="inbound-2096",
         settings={"clients": [troj], "fallbacks": []}, stream=tls),
    dict(id=4, remark="hysteria", port=4443, protocol="hysteria", tag="inbound-4443",
         settings={"version": 2, "clients": [hy]}, stream={"network": "hysteria", "security": "tls"}),
    dict(id=5, remark="ss", port=1080, protocol="shadowsocks", tag="inbound-1080",
         settings={"method": "chacha20-ietf-poly1305", "password": "server-pass", "network": "tcp,udp",
                   "clients": [ss]}, stream={"network": "tcp", "security": "none"}),
]

traffic = [  # email, inbound, enable, up, down, expiry, total, reset, last_online
    ("ali@home", 1, True, GIB, 2 * GIB, 1803859200000, 10 * GIB, 30, 1792692300000),
    ("neg@home", 1, True, 0, 0, -2592000000, 0, 0, 0),
    ("off@home", 1, False, 5, 5, 0, 0, 0, 0),
    ("quota@home", 1, True, GIB, GIB // 5, 0, GIB, 0, 0),
    ("old@home", 1, True, 100, 200, 1700000000000, 0, 0, 0),
    ("bob@home", 2, True, 7, 8, 0, 0, 0, 0),
    ("shared@home", 2, True, 0, 0, 0, 0, 0, 0),
    ("troj@home", 3, True, 1000, 2000, 0, 0, 0, 0),
    ("ss@home", 5, True, 0, 0, 0, 0, 0, 0),
]
settings = {"subPath": "/sub/", "subJsonPath": "/json/", "subClashPath": "/clash/", "subTitle": "My VPN",
            "subUpdates": "6", "subSupportUrl": "https://t.me/support", "subJsonEnable": "true", "subClashEnable": "false",
            "webPort": "2053", "subPort": "2097"}


def classic():
    out = ["BEGIN TRANSACTION;",
           "CREATE TABLE `users` (`id` integer,`username` text,`password` text,PRIMARY KEY (`id`));",
           'CREATE TABLE `inbounds` (`id` integer,`user_id` integer,`up` integer,`down` integer,`total` integer,'
           '`all_time` integer DEFAULT 0,`remark` text,`enable` numeric,`expiry_time` integer,`traffic_reset` text DEFAULT "never",'
           '`last_traffic_reset_time` integer DEFAULT 0,`listen` text,`port` integer,`protocol` text,`settings` text,'
           '`stream_settings` text,`tag` text UNIQUE,`sniffing` text,PRIMARY KEY (`id`));',
           "CREATE TABLE `client_traffics` (`id` integer,`inbound_id` integer,`enable` numeric,`email` text UNIQUE,`up` integer,"
           "`down` integer,`all_time` integer,`expiry_time` integer,`total` integer,`reset` integer DEFAULT 0,"
           "`last_online` integer DEFAULT 0,PRIMARY KEY (`id`),CONSTRAINT `fk_inbounds_client_stats` FOREIGN KEY (`inbound_id`) REFERENCES `inbounds`(`id`));",
           "CREATE TABLE `settings` (`id` integer,`key` text,`value` text,PRIMARY KEY (`id`));",
           insert("users", id=1, username="admin", password="$2a$10$notarealhash")]
    for i in inbounds:
        out.append(insert("inbounds", id=i["id"], user_id=1, up=0, down=0, total=0, all_time=0, remark=i["remark"],
                          enable=True, expiry_time=0, traffic_reset="never", last_traffic_reset_time=0, listen="",
                          port=i["port"], protocol=i["protocol"], settings=json.dumps(i["settings"], indent=2),
                          stream_settings=json.dumps(i["stream"], indent=2), tag=i["tag"], sniffing=json.dumps(SNIFF, indent=2)))
    for n, (email, inbound, enable, up, down, expiry, total, reset, online) in enumerate(traffic, 1):
        out.append(insert("client_traffics", id=n, inbound_id=inbound, enable=enable, email=email, up=up, down=down,
                          all_time=up + down, expiry_time=expiry, total=total, reset=reset, last_online=online))
    for n, (k, v) in enumerate(settings.items(), 1):
        out.append(insert("settings", id=n, key=k, value=v))
    out.append("COMMIT;")
    return "\n".join(out) + "\n"


def v3():
    """The newer shape: a `clients` table and `client_inbounds`; inbounds hold no clients."""
    out = ["BEGIN TRANSACTION;",
           'CREATE TABLE `inbounds` (`id` integer,`user_id` integer,`up` integer,`down` integer,`total` integer,'
           '`remark` text,`sub_sort_index` integer DEFAULT 1,`exclude_from_sub` numeric DEFAULT false,`enable` numeric,'
           '`expiry_time` integer,`traffic_reset` text DEFAULT "never",`traffic_reset_day` integer DEFAULT 1,'
           '`last_traffic_reset_time` integer DEFAULT 0,`listen` text,`port` integer,`protocol` text,`settings` text,'
           '`stream_settings` text,`tag` text UNIQUE,`sniffing` text,`node_id` integer,`share_addr_strategy` text DEFAULT "node",'
           '`share_addr` text,`disable_flow` numeric DEFAULT false,`origin_node_guid` text,PRIMARY KEY (`id`));',
           "CREATE TABLE `clients` (`id` integer,`email` text NOT NULL UNIQUE,`sub_id` text,`uuid` text,`password` text,`auth` text,"
           "`flow` text,`security` text,`reverse` text,`wg_private_key` text,`wg_public_key` text,`wg_allowed_ips` text,"
           "`wg_pre_shared_key` text,`wg_keep_alive` integer DEFAULT 0,`wg_forwarded_ports` text,`secret` text,`ad_tag` text DEFAULT '',"
           "`limit_ip` integer,`limit_hwid` integer DEFAULT 0,`total_gb` integer,`expiry_time` integer,`enable` numeric DEFAULT true,"
           "`tg_id` integer,`group_name` text DEFAULT '',`comment` text,`reset` integer DEFAULT 0,`reset_day` integer DEFAULT 0,"
           "`reset_weekday` integer DEFAULT 0,`reset_max` integer DEFAULT 0,`traffic_reset` text DEFAULT 'never',"
           "`traffic_reset_day` integer DEFAULT 1,`created_at` integer,`updated_at` integer,`sync_orphaned_at` integer DEFAULT 0,PRIMARY KEY (`id`));",
           "CREATE TABLE `client_inbounds` (`client_id` integer,`inbound_id` integer,`flow_override` text,`created_at` integer,"
           "PRIMARY KEY (`client_id`,`inbound_id`));",
           "CREATE TABLE `client_traffics` (`id` integer,`inbound_id` integer,`enable` numeric,`email` text UNIQUE,`up` integer,"
           "`down` integer,`expiry_time` integer,`total` integer,`reset` integer DEFAULT 0,`reset_day` integer DEFAULT 0,"
           "`reset_weekday` integer DEFAULT 0,`reset_max` integer DEFAULT 0,`reset_count` integer DEFAULT 0,"
           "`last_online` integer DEFAULT 0,PRIMARY KEY (`id`));",
           "CREATE TABLE `settings` (`id` integer,`key` text,`value` text,PRIMARY KEY (`id`));"]
    local = [dict(i, settings={k: v for k, v in i["settings"].items() if k != "clients"}) for i in inbounds[:3]]
    for i in local:
        out.append(insert("inbounds", id=i["id"], user_id=1, up=0, down=0, total=0, remark=i["remark"], sub_sort_index=1,
                          exclude_from_sub=False, enable=True, expiry_time=0, traffic_reset="never", traffic_reset_day=1,
                          last_traffic_reset_time=0, listen="", port=i["port"], protocol=i["protocol"],
                          settings=json.dumps(i["settings"]), stream_settings=json.dumps(i["stream"]), tag=i["tag"],
                          sniffing=json.dumps(SNIFF), node_id=None, share_addr_strategy="node", share_addr="",
                          disable_flow=False, origin_node_guid=""))
    out.append(insert("inbounds", id=9, user_id=1, up=0, down=0, total=0, remark="on-node", sub_sort_index=1,
                      exclude_from_sub=False, enable=True, expiry_time=0, traffic_reset="never", traffic_reset_day=1,
                      last_traffic_reset_time=0, listen="", port=443, protocol="vless",
                      settings=json.dumps({"decryption": "none"}), stream_settings=json.dumps(ws), tag="node2-443",
                      sniffing=json.dumps(SNIFF), node_id=2, share_addr_strategy="node", share_addr="",
                      disable_flow=False, origin_node_guid="node-guid-2"))
    rows = [  # id, email, kind, secret, flow, extra
        (1, "ali@home", "uuid", uuid(1), VISION, dict(sub_id="ali0sub1id", limit_ip=2, total_gb=10 * GIB, expiry_time=1803859200000,
                                                      tg_id=12345, comment="friend", traffic_reset="monthly", group_name="vip")),
        (2, "weekly@home", "uuid", uuid(2), VISION, dict(sub_id="weeklysub", total_gb=5 * GIB, reset_weekday=3)),
        (3, "troj@home", "password", "trojan-secret-1", "", dict(sub_id="trojsub")),
        (4, "hwid@home", "uuid", uuid(4), "", dict(sub_id="hwidsub", limit_hwid=3, reset=10)),
    ]
    for id_, email, kind, secret, flow, extra in rows:
        base = dict(id=id_, email=email, sub_id="", uuid=secret if kind == "uuid" else "", password=secret if kind == "password" else "",
                    auth="", flow=flow, security="", reverse="", limit_ip=0, limit_hwid=0, total_gb=0, expiry_time=0, enable=True,
                    tg_id=0, group_name="", comment="", reset=0, reset_day=0, reset_weekday=0, reset_max=0,
                    traffic_reset="never", traffic_reset_day=1, created_at=1768037400000, updated_at=1768037400000)
        base.update(extra)
        out.append(insert("clients", **base))
    for client_id, inbound_id in [(1, 1), (1, 2), (2, 1), (3, 3), (4, 2), (4, 9)]:
        out.append(insert("client_inbounds", client_id=client_id, inbound_id=inbound_id, flow_override="", created_at=1768037400000))
    traffic3 = [("ali@home", 1, True, GIB, 2 * GIB, 1803859200000, 10 * GIB, 1792692300000),
                ("weekly@home", 1, True, 10, 20, 0, 5 * GIB, 0),
                ("troj@home", 3, True, 1000, 2000, 0, 0, 0),
                ("hwid@home", 2, True, 0, 0, 0, 0, 0)]
    for n, (email, inbound, enable, up, down, expiry, total, online) in enumerate(traffic3, 1):
        out.append(insert("client_traffics", id=n, inbound_id=inbound, enable=enable, email=email, up=up, down=down,
                          expiry_time=expiry, total=total, reset=0, reset_day=0, reset_weekday=0, reset_max=0, reset_count=0,
                          last_online=online))
    for n, (k, v) in enumerate({"subPath": "/s/", "subTitle": "", "subUpdates": "12"}.items(), 1):
        out.append(insert("settings", id=n, key=k, value=v))
    out.append("COMMIT;")
    return "\n".join(out) + "\n"


(here / "threexui_classic.sql").write_text(classic())
(here / "threexui_v3.sql").write_text(v3())
print("written")
