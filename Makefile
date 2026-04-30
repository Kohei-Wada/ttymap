# ttymap install layout (XDG):
#
#   ~/.cargo/bin/ttymap                     — the binary (cargo install)
#   ~/.local/share/ttymap/lua/              — bundled Lua plugins + libs
#
# Single-user, no root. `/etc/ttymap` and `/usr/local/share/ttymap`
# layouts are intentionally unsupported — ttymap is a per-user TUI,
# system-wide installs aren't worth the path-juggling.
#
# `cargo install` alone places only the binary; the binary fails
# fast (with a "did you make install?" message) when it can't find
# `~/.local/share/ttymap/lua/`. See issue #183.

XDG_DATA_HOME ?= $(HOME)/.local/share
DATA_DIR      := $(XDG_DATA_HOME)/ttymap

.PHONY: help install install-bin install-runtime uninstall clean

# `make` with no target lists what's available. Mirrors the "first
# target is the default" make convention while making the default
# something safe (no side effects).
.DEFAULT_GOAL := help

help:
	@echo 'Targets:'
	@echo '  install          cargo install + place runtime under $$XDG_DATA_HOME/ttymap/lua/'
	@echo '  install-bin      cargo install only (binary → ~/.cargo/bin/ttymap)'
	@echo '  install-runtime  place runtime only (~/.local/share/ttymap/lua/)'
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
	mkdir -p $(DATA_DIR)/lua
	cp -r runtime/lua/. $(DATA_DIR)/lua/

uninstall:
	cargo uninstall ttymap || true
	rm -rf $(DATA_DIR)

clean:
	cargo clean
