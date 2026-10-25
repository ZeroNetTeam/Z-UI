#!/usr/bin/env python3
"""Panel benchmark: Z-UI against the panels it replaces.

    python3 bench/run.py [--panels zui,3xui,pasarguard] [--users 1000,10000]
                         [--seconds 15] [--out bench/results]

Each panel runs alone, inside a 1 CPU / 1 GB cgroup (see lib.py), with the same
job: one plain VLESS inbound and N users that each have a subscription link.
The numbers written are what was measured, whatever they are.
"""

import argparse
import datetime
import json
import pathlib
import platform
import random
import subprocess
import time

import lib
from lib import ROOT, oha, request, summarise

SETTLE = 4.0


def machine():
    cpu = ""
    for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            cpu = line.split(":", 1)[1].strip()
            break
    return {
        "cpu": cpu,
        "kernel": platform.release(),
        "cores": __import__("os").cpu_count(),
        "panel_limits": f"1 CPU (quota 100%, pinned to core {lib.PANEL_CPU}), {lib.MEMORY_MAX} memory, no swap",
        "load_generator": f"oha, pinned to cores {lib.LOAD_CPUS}",
        "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
    }


def idle(panel, seconds=6.0):
    """Memory and CPU of a panel with nothing asking it anything."""
    time.sleep(SETTLE)
    before = panel.running.cpu_seconds()
    time.sleep(seconds)
    cpu = (panel.running.cpu_seconds() - before) / seconds * 100
    return {"rss_mb": panel.running.rss_kb() / 1024, "cpu_percent": cpu, "processes": panel.running.processes()}


def load(panel, label, make_report, seconds, connections):
    """Warm the panel up gently, then run `make_report(seconds, connections)`
    with its memory and CPU watched. The warm-up is the same for every panel:
    a few connections for a few seconds, so a program that starts lazily is
    not measured on the one moment 64 requests reach it cold."""
    make_report(4, 4)
    run = panel.running
    cpu0 = run.cpu_seconds()
    run.start_sampling()
    report = make_report(seconds, connections)
    peak = run.stop_sampling()
    cpu = run.cpu_seconds() - cpu0
    result = summarise(report)
    result["peak_rss_mb"] = peak / 1024
    result["cpu_ms_per_request"] = cpu / max(result["requests"], 1) * 1000
    print(f"    {label}: {result['rps']:.0f} req/s, p50 {result['p50_ms']:.1f} ms, p99 {result['p99_ms']:.1f} ms, "
          f"ok {result['ok_share'] * 100:.1f}%, peak {result['peak_rss_mb']:.0f} MB, {result['cpu_ms_per_request']:.3f} cpu-ms/req", flush=True)
    return result


def bench(panel, users, seconds):
    print(f"== {panel.name}, {users} users", flush=True)
    out = {"users": users}
    ready = panel.start()
    out["start_empty_s"] = ready
    out["idle_empty"] = idle(panel, 3.0)
    print(f"    empty: ready in {ready:.2f}s, {out['idle_empty']['rss_mb']:.0f} MB", flush=True)
    out["seed_s"] = panel.seed(users)
    print(f"    seeded in {out['seed_s']:.1f}s", flush=True)
    tokens = panel.collect_tokens()
    assert len(tokens) >= users, (len(tokens), users)

    # Start again with everything stored: what a restart or a reboot costs.
    panel.stop()
    time.sleep(1)
    panel.port_keep = True
    out["start_loaded_s"] = panel.start()
    out["idle_loaded"] = idle(panel)
    print(f"    with {users} users: ready in {out['start_loaded_s']:.2f}s, {out['idle_loaded']['rss_mb']:.0f} MB, "
          f"{out['idle_loaded']['cpu_percent']:.1f}% cpu idle, {out['idle_loaded']['processes']} processes", flush=True)

    urls = pathlib.Path(panel.dir) / "sub-urls.txt"
    random.seed(1)
    sample = random.sample(tokens, min(len(tokens), 10000))
    urls.write_text("\n".join(panel.sub_url(t) for t in sample) + "\n")
    out["subscription"] = load(
        panel, "subscription",
        lambda s, c: oha(None, c, s, headers=panel.sub_headers(), urls_file=urls), seconds, 64)

    url, headers = panel.list_request()
    out["user_list"] = load(panel, "user list (50 per page)", lambda s, c: oha(url, c, s, headers=headers), seconds, 16)
    panel.stop()
    return out


def versions():
    out = {}
    try:
        out["z-ui"] = subprocess.run([str(ROOT / "target/bench-release/release/z-ui"), "--version"],
                                     capture_output=True, text=True).stdout.strip()
    except OSError:
        pass
    return out


def main():
    from panels import PANELS

    parser = argparse.ArgumentParser()
    parser.add_argument("--panels", default="zui,3xui,pasarguard")
    parser.add_argument("--users", default="1000,10000")
    parser.add_argument("--seconds", type=int, default=15)
    parser.add_argument("--out", default=str(ROOT / "bench" / "results"))
    args = parser.parse_args()

    result = {"machine": machine(), "panels": {}}
    for key in args.panels.split(","):
        runs = []
        for users in [int(n) for n in args.users.split(",")]:
            panel = PANELS[key]()
            try:
                runs.append(bench(panel, users, args.seconds))
            except Exception as error:  # a panel that cannot do the job is a result too
                print(f"    FAILED: {error}", flush=True)
                runs.append({"users": users, "error": str(error)})
            finally:
                panel.stop()
        result["panels"][key] = {"name": PANELS[key].name, "version": PANELS[key].version(), "runs": runs}
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    (out / f"{stamp}.json").write_text(json.dumps(result, indent=2))
    (out / "latest.json").write_text(json.dumps(result, indent=2))
    print(f"written to {out}/{stamp}.json")


if __name__ == "__main__":
    main()
