# ttymap install layout (XDG, nvim-style):
#
#   ~/.cargo/bin/ttymap                     — the binary (cargo install)
#   ~/.local/share/ttymap/lua/              — bundled libs + plugins
#                                             (`plugin/<name>.lua` and
#                                             `ttymap/<name>.lua` both
#                                             live here, resolved via
#                                             standard `package.path`)
#
# Single-user, no root. `/etc/ttymap` and `/usr/local/share/ttymap`
# layouts are intentionally unsupported — ttymap is a per-user TUI,
# system-wide installs aren't worth the path-juggling.
#
# `cargo install` alone places only the binary; the binary fails
# fast when it can't find any runtime layer (no `lua/` on disk).

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
	@echo '  install-runtime  place runtime only (lua/ + init.lua under ~/.local/share/ttymap/)'
	@echo '  uninstall        remove binary and runtime'
	@echo '  clean            cargo clean'
	@echo '  help             show this message'
	@echo ''
	@echo 'Variables:'
	@echo '  XDG_DATA_HOME    install root (default: $$HOME/.local/share)'

install: install-bin install-runtime

install-bin:
	# --force so re-installing replaces the binary in place. Cargo
	# also uses --force when the previous install came from a
	# differently-named source crate (the binary used to ship from
	# the root `ttymap` crate, then `ttymap-tui`; it now ships from
	# `ttymap-app` after the #351 Step 0 rename).
	cargo install --path ttymap-app --force

install-runtime:
	# Wipe + re-create the lua/ tree so files removed from runtime/
	# (e.g. a bundled plugin merged into a multi-entry pack) don't
	# linger and get re-registered as duplicate palette entries on
	# the next run. Safe — the target dir is exclusively for
	# bundled scripts; user overrides live under XDG_CONFIG_HOME/ttymap.
	# Also wipe a stale `plugin/` from previous installs (pre-#NNN
	# layout had `plugin/` as a sibling of `lua/`).
	rm -rf $(DATA_DIR)/plugin $(DATA_DIR)/lua
	mkdir -p $(DATA_DIR)/lua
	cp -r ttymap-app/runtime/lua/. $(DATA_DIR)/lua/
	cp ttymap-app/runtime/init.lua $(DATA_DIR)/init.lua

uninstall:
	# Try both the current name and previous names so an upgrade
	# path that crossed the rename boundary still cleans up.
	cargo uninstall ttymap-app || true
	cargo uninstall ttymap-tui || true
	rm -rf $(DATA_DIR)

clean:
	cargo clean
