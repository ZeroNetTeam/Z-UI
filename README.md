<div align="center">

# Z-UI

**A control panel for [zray-core](https://github.com/zeghostwriter/ZeroNet)**
· پنل مدیریت برای zray-core

</div>

Z-UI manages a zray-core server the way 3X-UI and PasarGuard manage Xray:
inbounds, users with traffic, time, device and speed limits, subscriptions,
admins and resellers, bots and backups, in one Rust binary with the web
dashboard inside. See [ROADMAP.md](ROADMAP.md) for the plan (in Persian).

> **Status:** early development, heading for v0.1. Not ready for production.

## Building

Z-UI builds against a checkout of zray-core next to it:

```sh
git clone https://github.com/zeghostwriter/ZeroNet resources/ZeroNet
git -C resources/ZeroNet switch z-ui/phase-0
make            # dashboard (npm) + release binary: target/release/z-ui
```

Rust 1.95+ and Node 22+ are needed. `cargo build` alone also works: without a
built dashboard the binary serves a page saying how to build one.

## Running

```sh
./target/release/z-ui --listen 0.0.0.0:2053 --base-path /my-secret-path
```

| Flag | Variable | Default | |
|---|---|---|---|
| `--listen` | `ZUI_LISTEN` | `0.0.0.0:2053` | Address of the panel |
| `--base-path` | `ZUI_BASE_PATH` | `/` | The panel answers only under this path, 404 elsewhere |
| `--data-dir` | `ZUI_DATA_DIR` | `data` | Database and core files |
| `--database-url` | `ZUI_DATABASE_URL` | `sqlite://<data-dir>/z-ui.db` | Or `postgres://…` |
| `--cert`, `--key` | `ZUI_CERT_FILE`, `ZUI_KEY_FILE` | | Serve HTTPS with this PEM certificate chain and key |

Logging follows `ZUI_LOG` (e.g. `ZUI_LOG=debug`).

### First start

A new panel has no admin. Whoever reached it first could otherwise claim it,
so the first admin can only be created from a one-time link printed in the
log:

```text
WARN no admin yet; create one at http://0.0.0.0:2053/my-secret-path/setup?token=…
```

### Locked out

From the server itself:

```sh
./z-ui admin reset-password admin                 # prints a new random password
./z-ui admin reset-password admin --disable-2fa   # for a lost authenticator
```

Sign-in is limited to 5 failures per address and 20 per username in 15
minutes; sessions last 24 hours and end on a password change.

## Layout

| Path | |
|---|---|
| `crates/z-panel` | The `z-ui` binary: HTTP API and the embedded dashboard |
| `crates/z-node` | The `Node` trait the panel drives; `LocalNode` runs the core in-process |
| `crates/z-db` | SQLite / PostgreSQL storage and migrations |
| `crates/z-common` | Types shared by the panel and nodes |
| `dashboard/` | React + Tailwind dashboard (Persian RTL and English) |

## Development

```sh
make dev     # panel on :2053, Vite on :5173 with hot reload
make test    # Rust and dashboard tests
make lint    # rustfmt, clippy, TypeScript
```

## License

MIT. Every part of the panel, dashboard included, is written for Z-UI; no
code is taken from 3X-UI (GPL-3.0) or PasarGuard (AGPL-3.0).
