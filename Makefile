# Z-UI build. `make` gives a release binary with the dashboard inside.

DASHBOARD := dashboard

.PHONY: all dashboard panel dev test test-scripts lint docker clean

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
	$(MAKE) test-scripts

# The installer and the menu, into a temporary root, with a program built here.
test-scripts:
	cargo build -p z-panel
	bash deploy/tests/run.sh

# An image built from source (it fetches zray-core itself).
docker:
	docker build -t z-ui .

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	npm --prefix $(DASHBOARD) run typecheck

clean:
	cargo clean
	rm -rf $(DASHBOARD)/dist $(DASHBOARD)/node_modules
