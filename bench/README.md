# Benchmark: Z-UI against the panels it replaces

How fast is the panel itself, next to 3X-UI and PasarGuard, on a small server?
Everything needed to repeat it is in this directory; the numbers in the main
README are what `run.py` wrote, unedited.

## What is measured

Each panel runs **alone**, with the same job: one plain VLESS inbound and *N*
users that all belong to it, each with a subscription link that returns that
one config.

| Measurement | How |
|---|---|
| Subscription fetches per second, and latency | After a 4 s warm-up on 4 connections (the same for every panel), 64 connections for 15 s, each request a different user's link (up to 10,000 links, random order), with a v2rayNG user agent. Keep-alive on. |
| Dashboard user list | After the same warm-up, 16 connections for 15 s asking for the first 50 users, signed in. |
| Memory at rest | Resident memory of everything the panel started, 10 s after it is up with the users stored. |
| Peak memory under load | The highest of those, sampled every 0.2 s while the load runs. |
| CPU per subscription fetch | The panel's CPU time over the run divided by the requests it answered. |
| Start time | Process started to first HTTP answer, with an empty database and with the users stored. |

*N* is 1,000 and 10,000.

## The server

The panel runs in a systemd scope with **1 CPU** (`CPUQuota=100%`, pinned to
one core), **1 GB of memory** and no swap: a small VPS. The load generator
([oha](https://github.com/hatoo/oha)) is pinned to other cores. Memory is read
from the scope's cgroup, so it covers every process the panel starts (a Python
panel's workers, a core if the panel runs one). The machine's own numbers are
in the results file.

## What is fair, and what is not

- Each panel is filled **through its own API** (3X-UI's `bulkCreate`,
  PasarGuard's `/api/user`, Z-UI's `bulk-create`), and started the way its own
  documentation does: PasarGuard with the dependency versions its `uv.lock`
  pins, its migrations first, its first owner made with its CLI's one-time key.
- 3X-UI and PasarGuard are built from the copies under `resources/`, with their
  own dashboards, so what is served and kept in memory is what a release has.
- **Z-UI's figures include its proxy core**, which runs inside the panel's
  process and has the 10,000 users loaded. The others' cores (Xray) are
  separate programs that this benchmark does not start, so their figures leave
  the core out. That counts against Z-UI.
- Different panels do different things per request. 3X-UI builds a
  subscription by reading the clients of every inbound; PasarGuard goes
  through Python, SQLAlchemy and its cache; Z-UI reads the user, its groups,
  inbounds, hosts and certificates and writes the config. All of that is the
  point of the comparison, but it means the work is not the same line for line.
- The subscription is fetched on a **kept-alive** connection, so what is
  measured is the panel and not the kernel's connection set-up, and over plain
  HTTP, so TLS is not part of it for any of them.
- One machine, one run per figure. Expect a few percent of noise; a difference
  of two times or more is not noise.

## Running it

```sh
cargo install oha --locked --root target/bench-tools
cargo build --release -p z-panel --target-dir target/bench-release
# 3X-UI and PasarGuard are built from resources/ (see "Building the others")
python3 bench/run.py --panels zui,3xui,pasarguard --users 1000,10000
python3 bench/plot.py            # docs/bench/*.png from bench/results/latest.json
```

`run.py` takes `--panels`, `--users`, `--seconds` and `--out`. It needs
`systemd-run --user` with the `cpu` and `memory` controllers delegated (the
default on a current systemd desktop or server), `taskset`, and Python 3.

### Building the others

```sh
# 3X-UI: its frontend, then the Go binary
cp -a resources/3x-ui target/bench-panels/3x-ui-src && cd target/bench-panels/3x-ui-src
(cd frontend && npm ci && npm run build) && CGO_ENABLED=1 go build -ldflags '-w -s' -o ../3x-ui-bin main.go

# PasarGuard: its dependency versions from uv.lock, its dashboard (vite build)
cp -a resources/pasarguard-panel target/bench-panels/pasarguard
python3 -m venv target/bench-panels/pg-venv     # Python 3.14
#   pip install --no-deps  every name==version in pasarguard/uv.lock
(cd target/bench-panels/pasarguard/dashboard && npm install --legacy-peer-deps \
   && VITE_BASE_API=/ npx vite build --outDir build --assetsDir statics && cp build/index.html build/404.html)
```
