# ttymap install layout (XDG, nvim-style):
#
#   ~/.cargo/bin/ttymap                     — the binary (cargo install)
#   ~/.local/share/ttymap/lua/              — bundled libs + plugins
#                                             (`plugin/<name>.lua` and
#                                             `ttymap/<name>.lua` both
#                                             live here, resolved via
#                                             standard `package.path`)
#
# Single-user, no root: `make install` never needs sudo and never
# writes outside `$HOME`. Do not run it as root to get a system-wide
# install — use the packager path below instead.
#
# Packagers (AUR, …) build a system layout with:
#
#   make install-system DESTDIR="$pkgdir" PREFIX=/usr
#
# which stages `$DESTDIR$PREFIX/{bin/ttymap,share/ttymap/}` as an
# unprivileged user; the package manager does the privileged part.
# The binary resolves `/usr/local/share/ttymap` and `/usr/share/ttymap`
# as its lowest-priority runtime tiers, so a per-user `make install`
# always shadows a packaged copy.
#
# `cargo install` alone places only the binary; the binary fails
# fast when it can't find any runtime layer (no `lua/` on disk).

XDG_DATA_HOME ?= $(HOME)/.local/share
DATA_DIR      := $(XDG_DATA_HOME)/ttymap

# Packager-only knobs. Unset by default, so every target above
# behaves exactly as it did before these existed.
PREFIX        ?= /usr/local
DESTDIR       ?=
SYS_BIN_DIR   := $(DESTDIR)$(PREFIX)/bin
SYS_DATA_DIR  := $(DESTDIR)$(PREFIX)/share/ttymap

.PHONY: help install install-bin install-runtime \
        install-system install-system-bin install-system-runtime \
        uninstall clean

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
	@echo 'Packaging (not for interactive use):'
	@echo '  install-system   stage $$DESTDIR$$PREFIX/{bin,share/ttymap} for a distro package'
	@echo ''
	@echo 'Variables:'
	@echo '  XDG_DATA_HOME    install root (default: $$HOME/.local/share)'
	@echo '  PREFIX           packaging prefix (default: /usr/local)'
	@echo '  DESTDIR          packaging staging root (default: empty)'

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
	cp -r runtime/lua/. $(DATA_DIR)/lua/
	cp runtime/init.lua $(DATA_DIR)/init.lua

# ── Packaging ───────────────────────────────────────────────────────
#
# Never invoked by a normal install. Writes only under $(DESTDIR),
# so makepkg / dpkg-buildpackage stage it unprivileged.

install-system: install-system-bin install-system-runtime

install-system-bin:
	cargo build --release --locked
	install -Dm755 target/release/ttymap $(SYS_BIN_DIR)/ttymap

install-system-runtime:
	# Same tree as the per-user install, rooted at the packaging
	# prefix. No rm -rf here: the staging dir belongs to the
	# package manager, which owns file removal on upgrade.
	install -d $(SYS_DATA_DIR)/lua
	cp -r runtime/lua/. $(SYS_DATA_DIR)/lua/
	install -Dm644 runtime/init.lua $(SYS_DATA_DIR)/init.lua

uninstall:
	# Try both the current name and previous names so an upgrade
	# path that crossed the rename boundary still cleans up.
	cargo uninstall ttymap-app || true
	rm -rf $(DATA_DIR)

clean:
	cargo clean
