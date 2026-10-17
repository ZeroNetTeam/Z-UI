# Z-UI build. `make` gives a release binary with the dashboard inside.

DASHBOARD := dashboard

.PHONY: all dashboard panel dev test lint clean

all: panel

dashboard:
	npm --prefix $(DASHBOARD) ci
	npm --prefix $(DASHBOARD) run build

panel: dashboard
	cargo build --release -p z-panel
	@echo "built target/release/z-ui"

# The panel on :2053 under /zpanel, and Vite on :5173 proxying its API.
dev:
	cargo run -p z-panel -- --listen 127.0.0.1:2053 --data-dir data &
	npm --prefix $(DASHBOARD) run dev

test:
	cargo test --workspace
	npm --prefix $(DASHBOARD) test

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	npm --prefix $(DASHBOARD) run typecheck

clean:
	cargo clean
	rm -rf $(DASHBOARD)/dist $(DASHBOARD)/node_modules
