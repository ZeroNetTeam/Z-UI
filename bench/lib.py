"""Shared pieces of the panel benchmark: run a panel inside a 1 CPU / 1 GB
cgroup, measure what it uses, and drive it with oha.

Everything here is standard library. The panel runs in a systemd user scope
(`systemd-run --user --scope`) with `CPUQuota=100%`, `MemoryMax=1G`, no swap,
and pinned to one core; the load generator is pinned to other cores. Memory is
read from the scope's cgroup, so it covers every process a panel starts.
"""

import http.client
import json
import os
import pathlib
import socket
import subprocess
import threading
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OHA = ROOT / "target" / "bench-tools" / "bin" / "oha"
PANEL_CPU = os.environ.get("BENCH_PANEL_CPU", "2")
LOAD_CPUS = os.environ.get("BENCH_LOAD_CPUS", "6-11")
MEMORY_MAX = os.environ.get("BENCH_MEMORY_MAX", "1G")


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_http(url: str, timeout: float = 120.0) -> float:
    """Seconds until `url` answers at all (any status), or raise."""
    start = time.monotonic()
    while time.monotonic() - start < timeout:
        try:
            urllib.request.urlopen(url, timeout=2).read()
            return time.monotonic() - start
        except urllib.error.HTTPError:
            return time.monotonic() - start
        except (urllib.error.URLError, ConnectionError, socket.timeout, http.client.HTTPException, OSError):
            time.sleep(0.02)
    raise TimeoutError(f"{url} did not answer in {timeout}s")


def request(method: str, url: str, body=None, headers=None, timeout=120):
    """(status, headers, parsed-or-text body)."""
    data = None
    headers = dict(headers or {})
    if body is not None:
        if isinstance(body, (dict, list)):
            data = json.dumps(body).encode()
            headers.setdefault("Content-Type", "application/json")
        elif isinstance(body, str):
            data = body.encode()
        else:
            data = body
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as reply:
            raw, status, head = reply.read(), reply.status, reply.headers
    except urllib.error.HTTPError as error:
        raw, status, head = error.read(), error.code, error.headers
    text = raw.decode("utf-8", "replace")
    try:
        return status, head, json.loads(text)
    except ValueError:
        return status, head, text


_runs = iter(range(1, 10**6))


class Constrained:
    """A command running inside a 1 CPU / 1 GB cgroup scope."""

    def __init__(self, name, argv, env=None, cwd=None, log=None):
        self.unit = f"zbench-{name}-{os.getpid()}-{next(_runs)}"
        full = os.environ.copy()
        full.update(env or {})
        # The machine has a local proxy for the shell; the panels talk to loopback.
        for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
            full.pop(key, None)
        command = [
            "systemd-run", "--user", "--scope", "--quiet", f"--unit={self.unit}",
            "-p", f"MemoryMax={MEMORY_MAX}", "-p", "MemorySwapMax=0", "-p", "CPUQuota=100%",
            "--", "taskset", "-c", PANEL_CPU, *argv,
        ]
        self.log = open(log, "wb") if log else subprocess.DEVNULL
        self.proc = subprocess.Popen(command, env=full, cwd=cwd, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        self.started = time.monotonic()
        self._cgroup = None
        self._peak_rss = 0
        self._sampling = False

    def cgroup(self) -> pathlib.Path:
        if self._cgroup is None:
            for _ in range(100):
                out = subprocess.run(
                    ["systemctl", "--user", "show", "-p", "ControlGroup", "--value", f"{self.unit}.scope"],
                    capture_output=True, text=True,
                ).stdout.strip()
                if out:
                    self._cgroup = pathlib.Path("/sys/fs/cgroup") / out.lstrip("/")
                    break
                time.sleep(0.05)
            else:
                raise RuntimeError("no cgroup for " + self.unit)
        return self._cgroup

    def pids(self):
        try:
            return [int(p) for p in (self.cgroup() / "cgroup.procs").read_text().split()]
        except OSError:
            return []

    def rss_kb(self) -> int:
        total = 0
        for pid in self.pids():
            try:
                for line in pathlib.Path(f"/proc/{pid}/status").read_text().splitlines():
                    if line.startswith("VmRSS:"):
                        total += int(line.split()[1])
            except OSError:
                pass
        return total

    def cpu_seconds(self) -> float:
        for line in (self.cgroup() / "cpu.stat").read_text().splitlines():
            if line.startswith("usage_usec"):
                return int(line.split()[1]) / 1e6
        return 0.0

    def processes(self) -> int:
        return len(self.pids())

    def start_sampling(self, every=0.2):
        self._sampling = True
        self._peak_rss = 0

        def loop():
            while self._sampling:
                self._peak_rss = max(self._peak_rss, self.rss_kb())
                time.sleep(every)

        self._thread = threading.Thread(target=loop, daemon=True)
        self._thread.start()

    def stop_sampling(self) -> int:
        self._sampling = False
        self._thread.join()
        return self._peak_rss

    def stop(self):
        subprocess.run(["systemctl", "--user", "stop", f"{self.unit}.scope"], capture_output=True)
        try:
            self.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def oha(url, connections, seconds, headers=None, urls_file=None, method="GET", body=None, keepalive=True):
    """Run oha and return its summary as a dict (see `oha --output-format json`)."""
    command = ["taskset", "-c", LOAD_CPUS, str(OHA), "--no-tui", "--output-format", "json",
               "-c", str(connections), "-z", f"{seconds}s", "-m", method]
    for name, value in (headers or {}).items():
        command += ["-H", f"{name}: {value}"]
    if body is not None:
        command += ["-d", body if isinstance(body, str) else json.dumps(body)]
    if not keepalive:
        command += ["--disable-keepalive"]
    if urls_file:
        command += ["--urls-from-file", str(urls_file)]
    else:
        command += [url]
    env = {k: v for k, v in os.environ.items() if "proxy" not in k.lower()}
    result = subprocess.run(command, capture_output=True, text=True, env=env)
    if result.returncode != 0:
        raise RuntimeError(result.stderr[-2000:])
    return json.loads(result.stdout)


def summarise(report):
    """The numbers that matter out of an oha report."""
    summary = report["summary"]
    percentiles = report.get("latencyPercentiles", {})
    codes = report.get("statusCodeDistribution", {})
    total = sum(codes.values()) or 1
    ok = sum(count for code, count in codes.items() if str(code).startswith("2"))
    return {
        "rps": summary["requestsPerSec"],
        "success": summary["successRate"],
        "ok_share": ok / total,
        "p50_ms": (percentiles.get("p50") or 0) * 1000,
        "p95_ms": (percentiles.get("p95") or 0) * 1000,
        "p99_ms": (percentiles.get("p99") or 0) * 1000,
        "status": codes,
        "requests": total,
    }
