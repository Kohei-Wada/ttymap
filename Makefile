# ttymap install layout (XDG, nvim-style):
#
#   ~/.cargo/bin/ttymap                     — the binary (cargo install)
#   ~/.local/share/ttymap/plugin/           — bundled auto-discovered plugins
#   ~/.local/share/ttymap/lua/              — bundled require'able lib scripts
#
# Single-user, no root. `/etc/ttymap` and `/usr/local/share/ttymap`
# layouts are intentionally unsupported — ttymap is a per-user TUI,
# system-wide installs aren't worth the path-juggling.
#
# `cargo install` alone places only the binary; the binary fails
# fast when it can't find any runtime layer (none of `plugin/` or
# `lua/` exist on disk).

XDG_DATA_HOME ?= $(HOME)/.local/share
DATA_DIR      := $(XDG_DATA_HOME)/ttymap

.PHONY: help install install-bin install-runtime uninstall clean

# `make` with no target lists what's available. Mirrors the "first
# target is the default" make convention while making the default
# something safe (no side effects).
.DEFAULT_GOAL := help

help:
	@echo 'Targets:'
	@echo '  install          cargo install + place runtime under $$XDG_DATA_HOME/ttymap/'
	@echo '  install-bin      cargo install only (binary → ~/.cargo/bin/ttymap)'
	@echo '  install-runtime  place runtime only (plugin/ + lua/ under ~/.local/share/ttymap/)'
	@echo '  uninstall        remove binary and runtime'
	@echo '  clean            cargo clean'
	@echo '  help             show this message'
	@echo ''
	@echo 'Variables:'
	@echo '  XDG_DATA_HOME    install root (default: $$HOME/.local/share)'

install: install-bin install-runtime

install-bin:
	cargo install --path .

install-runtime:
	# Wipe + re-create both tiers so files removed from runtime/
	# (e.g. a bundled plugin merged into a multi-entry pack) don't
	# linger and get re-registered as duplicate palette entries on
	# the next run. Safe — both target dirs are exclusively for
	# bundled scripts; user overrides live under XDG_CONFIG_HOME/ttymap.
	rm -rf $(DATA_DIR)/plugin $(DATA_DIR)/lua
	mkdir -p $(DATA_DIR)/plugin $(DATA_DIR)/lua
	cp -r runtime/plugin/. $(DATA_DIR)/plugin/
	cp -r runtime/lua/. $(DATA_DIR)/lua/
	cp runtime/init.lua $(DATA_DIR)/init.lua

uninstall:
	cargo uninstall ttymap || true
	rm -rf $(DATA_DIR)

clean:
	cargo clean
