SHELL := /bin/bash
.DEFAULT_GOAL := help

PREFIX ?= $(HOME)/.local
BIN := $(PREFIX)/bin/shuo
CARGO ?= cargo
BREW_PREFIX ?= $(shell brew --prefix 2>/dev/null || echo /opt/homebrew)
ZSH_COMPLETION := $(BREW_PREFIX)/share/zsh/site-functions/_shuo

DEBUG_BIN := target/debug/shuo
RELEASE_BIN := target/release/shuo
RELEASE_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
RELEASE_TARGET := aarch64-apple-darwin
SWEEP_DAYS ?= 14
RELEASE_NAME := shuo-v$(RELEASE_VERSION)-$(RELEASE_TARGET)
DIST_DIR := dist

.PHONY: help
help:
	@echo "Local shuohua development targets:"
	@echo "  make debug           Build target/debug/shuo"
	@echo "  make release         Build target/release/shuo for local development"
	@echo "  make dist            Build a portable release tarball in a clean target dir"
	@echo "  make verify-dist     Verify checksum, layout, Mach-O contract, and smoke test"
	@echo "  make install         Release build, install to $(BIN), refresh zsh completion, install/restart launchd"
	@echo "  make install-debug   Debug build, install/restart launchd from target/debug/shuo"
	@echo "  make completions-zsh Refresh zsh completion at $(ZSH_COMPLETION)"
	@echo "  make restart         Restart installed launchd service via PATH shuo"
	@echo "  make stop            Stop installed launchd service via PATH shuo"
	@echo "  make status          Show daemon status via PATH shuo"
	@echo "  make doctor          Run doctor via PATH shuo"
	@echo "  make check           Update stable, then run fmt check, clippy, test"
	@echo "  make fmt             Format Rust code"
	@echo "  make sweep           Remove stale target/ artifacts (>$(SWEEP_DAYS) days unused)"
	@echo "  make clean           Remove the entire target/ directory"

.PHONY: debug
debug:
	$(CARGO) build

.PHONY: release
release:
	$(CARGO) build --release

.PHONY: dist
dist:
	DIST_DIR="$(DIST_DIR)" CARGO="$(CARGO)" scripts/build-dist.sh

.PHONY: verify-dist
verify-dist:
	scripts/verify-macos-dist.sh "$(DIST_DIR)/$(RELEASE_NAME).tar.gz"

.PHONY: install
install: dist
	mkdir -p "$(dir $(BIN))"
	install -m 755 "$(DIST_DIR)/$(RELEASE_NAME)/shuo" "$(BIN)"
	$(MAKE) completions-zsh SHUO_BIN=$(BIN)
	"$(BIN)" service install
	"$(BIN)" version

.PHONY: completions-zsh
completions-zsh:
	@mkdir -p "$(dir $(ZSH_COMPLETION))"
	$(or $(SHUO_BIN),shuo) completions zsh > "$(ZSH_COMPLETION)"

.PHONY: install-debug
install-debug: debug
	./$(DEBUG_BIN) service install
	./$(DEBUG_BIN) version

.PHONY: restart
restart:
	shuo service restart

.PHONY: stop
stop:
	shuo service stop

.PHONY: status
status:
	shuo service status

.PHONY: doctor
doctor:
	shuo doctor

.PHONY: fmt
fmt:
	$(CARGO) fmt

.PHONY: fmt-check
fmt-check:
	$(CARGO) +stable fmt --check

.PHONY: clippy
clippy:
	$(CARGO) +stable clippy --locked --all-targets -- -D warnings

.PHONY: test
test:
	$(CARGO) +stable test --locked

.PHONY: sweep
sweep:
	$(CARGO) sweep --time $(SWEEP_DAYS) .

.PHONY: clean
clean:
	$(CARGO) clean

.PHONY: check
check:
	rustup update stable
	rustup component add --toolchain stable clippy rustfmt
	$(CARGO) +stable fmt --check
	$(CARGO) +stable clippy --locked --all-targets -- -D warnings
	$(CARGO) +stable test --locked
