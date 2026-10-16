# WebPulse - developer tasks (no cPanel needed). See docs/DEVELOPMENT.md.
#
#   make build            cargo build --release            -> target/release/webpulse
#   make test             cargo test (engine, web, packaging) + parity harness (when php exists)
#   make lint             cargo fmt --check, clippy, shellcheck, php -l of the cPanel shims
#   make fixtures         fake cPanel server with generated logs in tests/tmp/dev
#   make serve            dev dashboard for the account "alice" on http://127.0.0.1:$(PORT)
#   make serve-admin      dev dashboard as server administrator
#   make dist             source tarball dist/webpulse-<version>.tar.gz   (git archive)
#   make release          binary tarball dist/webpulse-<version>-<target>.tar.gz (+ .sha256)

CARGO   ?= cargo
PHP     ?= php
PORT    ?= 8099
TARGET  ?=
VERSION := $(shell cat VERSION)
BIN     := target/release/webpulse
DEVDIR  := tests/tmp/dev
DEVCFG  := $(DEVDIR)/config.json

SHELL_FILES = install.sh uninstall.sh installer/lib.sh hooks/*.sh scripts/*.sh whm/cgi/*.cgi
PHP_SHIMS   = cpanel/webpulse/index.live.php cpanel/webpulse/api.live.php cpanel/webpulse/lib/*.php

.PHONY: build test test-rust test-parity lint fmt fixtures serve serve-admin dist release geoip-fixture clean

build:
	$(CARGO) build --release

test: test-rust test-parity

# every test is a Rust integration test (tests/*.rs), the packaging ones included
# (tests/pkg_*.rs: installer, hooks, release scripts; the cPanel shim tests use php
# when it is installed and are skipped otherwise)
test-rust:
	$(CARGO) test --release

# Rust vs the 0.1.0 PHP release; needs php >= 8.1 and checks out tag v0.1.0 into tests/tmp/legacy
test-parity: build
	@if [ -x tests/parity/run.sh ] && command -v $(PHP) >/dev/null 2>&1; then tests/parity/run.sh; \
	else echo "skipping the parity harness (needs php and tests/parity/run.sh)"; fi

fmt:
	$(CARGO) fmt

lint:
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings
	@if command -v shellcheck >/dev/null 2>&1; then shellcheck -S warning $(SHELL_FILES); \
	else echo "shellcheck not found: skipped"; fi
	@if command -v $(PHP) >/dev/null 2>&1; then for f in $(PHP_SHIMS); do $(PHP) -l "$$f" >/dev/null || exit 1; done; fi
	@echo "lint ok"

fixtures: build
	WEBPULSE_BIN=$(BIN) scripts/devenv.sh dev

$(DEVCFG): $(BIN)
	WEBPULSE_BIN=$(BIN) scripts/devenv.sh dev

$(BIN):
	$(CARGO) build --release

# serve / serve-admin print http://127.0.0.1:$(PORT)/?token=... - open that URL (the token is required)
serve: $(DEVCFG)
	WEBPULSE_CONFIG=$(DEVCFG) $(BIN) serve --port $(PORT) --user alice

serve-admin: $(DEVCFG)
	WEBPULSE_CONFIG=$(DEVCFG) $(BIN) serve --port $(PORT) --admin

geoip-fixture:
	scripts/geoip_fixture.sh

dist:
	scripts/build_dist.sh

# TARGET=x86_64-unknown-linux-musl (default: host arch, musl); ZIG=1 uses cargo-zigbuild
release:
	scripts/build_release.sh $(if $(TARGET),--target $(TARGET)) $(if $(ZIG),--zigbuild)

clean:
	rm -rf tests/tmp dist
