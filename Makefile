# scaler Makefile
#
# Quick reference:
#   make              # show this help (default)
#   make all          # cargo build --release (alias for `make build`)
#   make build        # cargo build --release
#   make install      # build then copy binary to $(PREFIX)/bin (uses sudo if PREFIX requires root)
#   make uninstall    # remove binary from $(PREFIX)/bin
#   make test         # cargo test
#   make check        # fmt + clippy + test (the local CI quartet minus release build)
#   make clean        # cargo clean
#   make doctor       # build then run `scaler doctor` from the release binary
#
# Versioning (yarn-style):
#   make version                          # patch +1 (default, e.g. 0.2.0 -> 0.2.1)
#   make version BUMP=minor               # minor +1 (e.g. 0.2.0 -> 0.3.0)
#   make version BUMP=major               # major +1 (e.g. 0.2.0 -> 1.0.0)
#   make version VERSION=1.2.3            # set explicit version
#
# Overrides:
#   PREFIX=~/.local make install   # install to ~/.local/bin (no sudo needed)
#   CARGO=cargo-nightly make build # use a different cargo

CARGO ?= cargo
PREFIX ?= /usr/local
BIN_DIR := $(PREFIX)/bin
TARGET_DIR ?= target
BIN_NAME := scaler
RELEASE_BIN := $(TARGET_DIR)/release/$(BIN_NAME)

.PHONY: all build install uninstall test check fmt lint clean doctor help version

# Bare `make` shows the help block instead of triggering a release build —
# safer for first-time contributors who might run `make` to "see what
# happens" without intending to spend a minute on cargo build.
.DEFAULT_GOAL := help

all: build

help:
	@echo "scaler make targets:"
	@echo "  build      - cargo build --release"
	@echo "  install    - build + copy $(BIN_NAME) to $(BIN_DIR) (uses sudo if needed)"
	@echo "  uninstall  - remove $(BIN_DIR)/$(BIN_NAME)"
	@echo "  test       - cargo test"
	@echo "  check      - fmt + clippy + test"
	@echo "  fmt        - cargo fmt"
	@echo "  lint       - cargo clippy --tests -- -D warnings"
	@echo "  clean      - cargo clean"
	@echo "  doctor     - build then run ./$(RELEASE_BIN) doctor"
	@echo "  version    - bump version (default patch); BUMP=minor|major or VERSION=X.Y.Z"
	@echo ""
	@echo "Overrides: PREFIX=~/.local make install  (install to \$$HOME/.local/bin)"

build: $(RELEASE_BIN)

$(RELEASE_BIN):
	$(CARGO) build --release

install: build
	@set -e; \
	dest="$(BIN_DIR)"; \
	if mkdir -p "$$dest" 2>/dev/null && [ -w "$$dest" ]; then \
	    sudo=""; \
	else \
	    echo ">> $$dest needs elevated privileges, using sudo"; \
	    sudo="sudo"; \
	    $$sudo mkdir -p "$$dest"; \
	fi; \
	$$sudo install -m 0755 "$(RELEASE_BIN)" "$$dest/$(BIN_NAME)"; \
	echo ">> installed $$dest/$(BIN_NAME)"; \
	"$$dest/$(BIN_NAME)" version; \
	echo ""; \
	"$$dest/$(BIN_NAME)" doctor

uninstall:
	@set -e; \
	dest="$(BIN_DIR)"; \
	if [ ! -e "$$dest/$(BIN_NAME)" ]; then \
	    echo ">> $$dest/$(BIN_NAME) not found, nothing to do"; \
	    exit 0; \
	fi; \
	if [ -w "$$dest" ]; then \
	    rm -f "$$dest/$(BIN_NAME)"; \
	else \
	    sudo rm -f "$$dest/$(BIN_NAME)"; \
	fi; \
	echo ">> removed $$dest/$(BIN_NAME)"

test:
	$(CARGO) test

check: fmt lint test

fmt:
	$(CARGO) fmt -- --check

lint:
	$(CARGO) clippy --tests -- -D warnings

clean:
	$(CARGO) clean

doctor: build
	@./$(RELEASE_BIN) doctor

# Version management — yarn-style. See scripts/bump-version.sh for details.
# `make version`              -> patch +1 (default)
# `make version BUMP=minor`   -> minor +1, patch=0
# `make version BUMP=major`   -> major +1, minor=0, patch=0
# `make version VERSION=X.Y.Z`-> set explicit version (overrides BUMP)
version:
	@CARGO=$(CARGO) ./scripts/bump-version.sh "$(if $(VERSION),$(VERSION),$(if $(BUMP),$(BUMP),patch))"
