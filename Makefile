MAKEFLAGS += -j$(shell sysctl -n hw.ncpu 2>/dev/null || nproc) --output-sync=line

TEST_FILES := $(wildcard tests/test_*.py)

.PHONY: lint format format-write typecheck test check \
        rust-test rust-fmt rust-fmt-check rust-clippy dev install \
        $(TEST_FILES)

lint:
	ruff check .

format:
	ruff format --check .

format-write:
	ruff format .

typecheck:
	pyright

test: rust-test $(TEST_FILES)

$(TEST_FILES): dev
	python -m pytest -q --tb=short $@ || { code=$$?; [ $$code -eq 5 ] && exit 0 || exit $$code; }

# --- Rust ---

rust-test: dev
	cargo test -q -p gremlins --lib && cargo test -q -p gremlins-pyext --lib

rust-fmt:
	cargo fmt --all

rust-fmt-check:
	cargo fmt --all -- --check

rust-clippy:
	cargo clippy -q --all-targets -- -D warnings

# --- Stubs ---

install-stubs: ## Install .py source stubs alongside the .so for pyright
	python crates/pyext/_install_stubs.py

# --- Build ---

dev: ## Build and install the native extension in dev mode
	maturin develop
	$(MAKE) install-stubs

install: ## Build and install the native extension in release mode
	maturin develop --release

check: lint format typecheck rust-fmt-check rust-clippy
	@grep -r 'from gremlins.executor.state' gremlins/ --include='*.py' | grep -v 'gremlins/executor/' && echo 'ERROR: state.py leak' && exit 1 || true
