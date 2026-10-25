"""One adapter per panel: how to start it, fill it with the same users, and
which URLs to hit. A panel is filled through its own API (or, where it has
none for the job, its own database) so that it works the way it would in use.

Every panel gets the same job: one plain VLESS inbound, N users that all
belong to it, and a subscription link for each that returns that one config.
"""

import pathlib
import shutil
import sqlite3
import time

from lib import ROOT, Constrained, free_port, request, wait_http

WORK = ROOT / "target" / "bench-work"


class Panel:
    name = ""
    key = ""

    def __init__(self):
        self.port = free_port()
        self.dir = WORK / self.key
        shutil.rmtree(self.dir, ignore_errors=True)
        self.dir.mkdir(parents=True)
        self.running = None
        self.tokens = []

    # to be filled in -------------------------------------------------------
    def command(self):
        raise NotImplementedError

    def env(self):
        return {}

    def cwd(self):
        return self.dir

    def ready_url(self):
        return f"http://127.0.0.1:{self.port}/"

    def seed(self, users: int):
        raise NotImplementedError

    def collect_tokens(self):
        raise NotImplementedError

    def sub_url(self, token):
        raise NotImplementedError

    def list_request(self):
        """(url, headers) of a call that lists the first page of users."""
        raise NotImplementedError

    def sub_headers(self):
        return {"User-Agent": "v2rayNG/1.9.30"}

    @classmethod
    def version(cls):
        return ""

    # shared ----------------------------------------------------------------
    def start(self):
        self.running = Constrained(self.key, self.command(), self.env(), self.cwd(), self.dir / "panel.log")
        return wait_http(self.ready_url())

    def stop(self):
        if self.running:
            self.running.stop()
            self.running = None


class ZUI(Panel):
    name = "Z-UI"
    key = "zui"
    admin = ("bench", "Bench-pass-123456")

    def command(self):
        binary = pathlib.Path(__import__("os").environ.get("ZUI_BIN", ROOT / "target" / "bench-release" / "release" / "z-ui"))
        return [str(binary), "--listen", f"127.0.0.1:{self.port}", "--base-path", "/", "--data-dir", str(self.dir / "data")]

    def env(self):
        return {"ZUI_ADMIN_USER": self.admin[0], "ZUI_ADMIN_PASSWORD": self.admin[1], "ZUI_LOG": "warn"}

    def _api(self, method, path, body=None):
        status, _, reply = request(method, f"http://127.0.0.1:{self.port}/api{path}", body, {"Cookie": self.cookie})
        if status >= 300:
            raise RuntimeError(f"Z-UI {method} {path} -> {status}: {reply}")
        return reply

    def seed(self, users: int):
        status, head, reply = request(
            "POST", f"http://127.0.0.1:{self.port}/api/auth/login",
            {"username": self.admin[0], "password": self.admin[1]},
        )
        assert status == 200, reply
        self.cookie = head["Set-Cookie"].split(";")[0]
        self._api("POST", "/inbounds", {
            "remark": "bench", "listen": "0.0.0.0", "port": free_port(), "protocol": "vless",
            "settings": {"decryption": "none"},
            "stream": {"network": "raw", "security": "none"},
        })
        group = self._api("GET", "/groups")[0]["id"]
        start = time.monotonic()
        done = 0
        # The panel numbers the users itself; ask for them in slices.
        while done < users:
            count = min(1000, users - done)
            self._api("POST", "/users/bulk-create", {
                "prefix": "user", "start": done + 1, "count": count, "group_ids": [group],
                "data_limit": 50 * 2**30, "expire_at": int(time.time()) + 86400 * 30,
            })
            done += count
        return time.monotonic() - start

    def collect_tokens(self):
        # Read from the panel's own database, read-only.
        db = sqlite3.connect(f"file:{self.dir / 'data' / 'z-ui.db'}?mode=ro", uri=True)
        self.tokens = [row[0] for row in db.execute("select sub_token from users order by id")]
        db.close()
        return self.tokens

    def sub_url(self, token):
        return f"http://127.0.0.1:{self.port}/sub/{token}"

    def list_request(self):
        return f"http://127.0.0.1:{self.port}/api/users?limit=50", {"Cookie": self.cookie}

    @classmethod
    def version(cls):
        import re
        text = (ROOT / "Cargo.toml").read_text()
        return re.search(r'^version = "([^"]+)"', text, re.M).group(1)


class ThreeXUI(Panel):
    """3X-UI v3 (Go), built from resources/3x-ui with its own frontend. Its
    subscription server is a second listener, on `subPort`."""

    name = "3X-UI"
    key = "3xui"
    admin = ("bench", "Bench-pass-123456")

    def __init__(self):
        super().__init__()
        self.sub_port = free_port()
        self.binary = str(ROOT / "target" / "bench-panels" / "3x-ui-bin")
        for sub in ("db", "bin", "log"):
            (self.dir / sub).mkdir()

    def env(self):
        return {
            "XUI_PORT": str(self.port), "XUI_DB_FOLDER": str(self.dir / "db"),
            "XUI_BIN_FOLDER": str(self.dir / "bin"), "XUI_LOG_FOLDER": str(self.dir / "log"),
            "XUI_LOG_LEVEL": "warning",
        }

    def command(self):
        return [self.binary, "run"]

    def _setup(self):
        import os
        import subprocess
        env = {**os.environ, **self.env()}
        for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
            env.pop(key, None)
        run = lambda *args: subprocess.run([self.binary, "setting", *args], env=env, capture_output=True, text=True)
        run("-username", self.admin[0], "-password", self.admin[1], "-port", str(self.port))
        token = run("-getApiToken").stdout.strip().splitlines()[-1].split()[-1]
        db = sqlite3.connect(self.dir / "db" / "x-ui.db")
        # The panel fills in the settings it lacks the first time it runs, so a
        # row may not be there yet: say the sub port either way.
        if db.execute("UPDATE settings SET value = ? WHERE key = 'subPort'", (str(self.sub_port),)).rowcount == 0:
            db.execute("INSERT INTO settings (key, value) VALUES ('subPort', ?)", (str(self.sub_port),))
        db.commit()
        row = db.execute("SELECT value FROM settings WHERE key = 'webBasePath'").fetchone()
        db.close()
        self.token, self.base = token, (row[0] if row else "/").rstrip("/")

    def start(self):
        if not hasattr(self, "token"):
            self._setup()
        ready = super().start()
        # The panel picks a random subscription path of its own on first start.
        db = sqlite3.connect(f"file:{self.dir / 'db' / 'x-ui.db'}?mode=ro", uri=True)
        self.sub_path = db.execute("SELECT value FROM settings WHERE key = 'subPath'").fetchone()[0].rstrip("/")
        db.close()
        return ready

    def ready_url(self):
        return f"http://127.0.0.1:{self.sub_port}/"

    def _api(self, method, path, body=None):
        status, _, reply = request(method, f"http://127.0.0.1:{self.port}{self.base}/panel/api{path}", body,
                                   {"Authorization": f"Bearer {self.token}"})
        if status >= 300 or (isinstance(reply, dict) and reply.get("success") is False):
            raise RuntimeError(f"3X-UI {method} {path} -> {status}: {str(reply)[:300]}")
        return reply

    def seed(self, users: int):
        import json
        import secrets
        import uuid
        made = self._api("POST", "/inbounds/add", {
            "remark": "bench", "enable": True, "listen": "", "port": free_port(), "protocol": "vless",
            "settings": json.dumps({"clients": [], "decryption": "none", "fallbacks": []}),
            "streamSettings": json.dumps({"network": "tcp", "security": "none", "tcpSettings": {"acceptProxyProtocol": False, "header": {"type": "none"}}}),
            "sniffing": json.dumps({"enabled": False, "destOverride": ["http", "tls"]}),
            "expiryTime": 0, "total": 0,
        })
        inbound = made["obj"]["id"]
        self.sub_ids = []
        start = time.monotonic()
        done = 0
        while done < users:
            batch = []
            for n in range(done + 1, min(done + 500, users) + 1):
                sub = secrets.token_hex(8)
                self.sub_ids.append(sub)
                batch.append({"client": {
                    "id": str(uuid.uuid4()), "email": f"user{n}", "subId": sub, "enable": True, "flow": "",
                    "limitIp": 0, "totalGB": 50 * 2**30, "expiryTime": (int(time.time()) + 86400 * 30) * 1000,
                    "tgId": 0, "comment": "", "reset": 0,
                }, "inboundIds": [inbound]})
            self._api("POST", "/clients/bulkCreate", batch)
            done += len(batch)
        return time.monotonic() - start

    def collect_tokens(self):
        return self.sub_ids

    def sub_url(self, token):
        return f"http://127.0.0.1:{self.sub_port}{self.sub_path}/{token}"

    def list_request(self):
        return (f"http://127.0.0.1:{self.port}{self.base}/panel/api/clients/list/paged?page=1&pageSize=50",
                {"Authorization": f"Bearer {self.token}"})

    @classmethod
    def version(cls):
        return (ROOT / "resources" / "3x-ui" / "internal" / "config" / "version").read_text().strip()


class PasarGuard(Panel):
    """PasarGuard 5.x (Python, FastAPI + SQLAlchemy over SQLite), run from the
    copy of resources/pasarguard-panel with the dependency versions its own
    uv.lock pins, as its `start.sh` does (migrations, then `main.py`)."""

    name = "PasarGuard"
    key = "pasarguard"
    # Its password policy wants two capitals, two digits and a symbol.
    admin = ("bench", "BEnch#pass-12345")

    def __init__(self):
        super().__init__()
        self.source = ROOT / "target" / "bench-panels" / "pasarguard"
        self.python = str(ROOT / "target" / "bench-panels" / "pg-venv" / "bin" / "python")

    def env(self):
        return {
            "SQLALCHEMY_DATABASE_URL": f"sqlite+aiosqlite:///{self.dir / 'db.sqlite3'}",
            "UVICORN_HOST": "127.0.0.1", "UVICORN_PORT": str(self.port),
            "SUDO_USERNAME": self.admin[0], "SUDO_PASSWORD": self.admin[1],
            "XRAY_SUBSCRIPTION_URL_PREFIX": f"http://127.0.0.1:{self.port}",
            "DEBUG": "false",
        }

    def cwd(self):
        return self.source

    def command(self):
        return [self.python, "main.py"]

    def _setup(self):
        import os
        import subprocess
        env = {**os.environ, **self.env()}
        for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
            env.pop(key, None)
        done = subprocess.run([self.python, "-m", "alembic", "upgrade", "head"], cwd=self.source, env=env,
                              capture_output=True, text=True)
        if done.returncode != 0:
            raise RuntimeError("alembic: " + done.stderr[-600:])
        # The first admin is made the way its docs say: a one-time key from
        # the CLI, used on the setup route once the panel is running.
        key = subprocess.run([self.python, "pasarguard-cli.py", "generate-temp-key"], cwd=self.source, env=env,
                             capture_output=True, text=True)
        import re
        found = re.search(r"Temp key:\s*(\S+)", re.sub(r"\x1b\[[0-9;]*m", "", key.stdout))
        if not found:
            raise RuntimeError("temp key: " + (key.stdout + key.stderr)[-600:])
        self.temp_key = found.group(1)
        self.migrated = True

    def start(self):
        if not getattr(self, "migrated", False):
            self._setup()
        return super().start()

    def _api(self, method, path, body=None, form=None):
        headers = {"Authorization": f"Bearer {self.token}"} if getattr(self, "token", None) else {}
        if form is not None:
            from urllib.parse import urlencode
            body, headers["Content-Type"] = urlencode(form), "application/x-www-form-urlencoded"
        url = f"http://127.0.0.1:{self.port}/api{path}"
        status, head, reply = request(method, url, body, headers)
        if status in (307, 308):  # a route that wants its trailing slash
            url = url + "/" if not url.endswith("/") else url.rstrip("/")
            status, head, reply = request(method, url, body, headers)
        if status >= 300:
            raise RuntimeError(f"PasarGuard {method} {path} -> {status}: {str(reply)[:300]}")
        return reply

    def seed(self, users: int):
        import concurrent.futures
        self._api("POST", "/setup/owner", {"key": self.temp_key, "username": self.admin[0], "password": self.admin[1]})
        self.token = self._api("POST", "/admin/token", form={
            "username": self.admin[0], "password": self.admin[1], "grant_type": "password"})["access_token"]
        tag = "bench"
        self._api("POST", "/core", {
            "name": "bench",
            "type": "xray",
            "config": {
                "log": {"loglevel": "warning"},
                "inbounds": [{"tag": tag, "listen": "0.0.0.0", "port": free_port(), "protocol": "vless",
                              "settings": {"decryption": "none", "clients": []},
                              "streamSettings": {"network": "tcp", "security": "none"}}],
                "outbounds": [{"protocol": "freedom", "tag": "direct"}],
            },
            "exclude_inbound_tags": [], "fallbacks_inbound_tags": [],
        })
        self._api("POST", "/host", {"remark": "bench", "address": ["127.0.0.1"], "port": 443,
                                    "inbound_tag": tag, "priority": 1})
        group = self._api("POST", "/group", {"name": "bench", "inbound_tags": [tag]})["id"]
        self.urls = [None] * users
        expire = int(time.time()) + 86400 * 30

        def make(n):
            reply = self._api("POST", "/user", {
                "username": f"user{n}", "proxy_settings": {}, "data_limit": 50 * 2**30,
                "data_limit_reset_strategy": "no_reset", "status": "active", "group_ids": [group],
                "expire": expire,
            })
            self.urls[n - 1] = reply["subscription_url"]

        start = time.monotonic()
        with concurrent.futures.ThreadPoolExecutor(8) as pool:
            list(pool.map(make, range(1, users + 1)))
        return time.monotonic() - start

    def collect_tokens(self):
        return self.urls

    def sub_url(self, token):
        return token  # the panel gives the whole address

    def list_request(self):
        return f"http://127.0.0.1:{self.port}/api/users?limit=50", {"Authorization": f"Bearer {self.token}"}

    @classmethod
    def version(cls):
        import re
        return re.search(r'^version = "([^"]+)"', (ROOT / "resources" / "pasarguard-panel" / "pyproject.toml").read_text(), re.M).group(1)


PANELS = {"zui": ZUI, "3xui": ThreeXUI, "pasarguard": PasarGuard}
