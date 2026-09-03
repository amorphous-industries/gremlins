PYTHON ?= python

MAKEFLAGS += -j$(shell sysctl -n hw.ncpu 2>/dev/null || nproc) --output-sync=line

TEST_FILES := $(wildcard tests/test_*.py)

.PHONY: lint format format-write autoformat typecheck test check \
        rust-test rust-fmt rust-fmt-check rust-clippy build dev install release \
        $(TEST_FILES)

lint:
	$(PYTHON) -m ruff check .

format:
	$(PYTHON) -m ruff format --check .

format-write:
	$(PYTHON) -m ruff format .

autoformat: format-write rust-fmt
	$(PYTHON) -m ruff check --fix .
	cargo clippy --fix --all-targets --allow-dirty

typecheck:
	$(PYTHON) -m pyright

test: rust-test $(TEST_FILES)

$(TEST_FILES): install
	$(PYTHON) -m pytest -q --tb=short $@ || { code=$$?; [ $$code -eq 5 ] && exit 0 || exit $$code; }

# --- Rust ---

rust-test: install
	cargo test -q -p gremlins --lib && cargo test -q -p gremlins-pyext --lib

rust-fmt:
	cargo fmt --all

rust-fmt-check:
	cargo fmt --all -- --check

rust-clippy:
	cargo clippy -q --all-targets -- -D warnings

# --- Build ---

build: ## Compile the native extension (no install)
	cargo build -p gremlins-pyext

dev: ## Build and install the native extension in dev mode (may skip if purely transitive deps changed)
	maturin develop

install: build ## Build + install the native extension (always fresh)
	maturin develop

release: ## Build and install the native extension in release mode
	maturin develop --release

check: lint format typecheck rust-fmt-check rust-clippy
	@grep -r 'from gremlins.executor.state' gremlins/ --include='*.py' | grep -v 'gremlins/executor/' && echo 'ERROR: state.py leak' && exit 1 || true
