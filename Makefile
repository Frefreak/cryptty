.DEFAULT_GOAL := help

CARGO ?= cargo
UV ?= uv
PREFIX ?= $(HOME)/.local

.PHONY: help build test macos-app install-macos install-linux

help:
	@printf '%s\n' \
	  'make build          Build a release binary' \
	  'make test           Run tests' \
	  'make macos-app      Build target/macos/Cryptty.app' \
	  'make install-macos  Install ~/Applications/Cryptty.app' \
	  'make install-linux  Install binary, desktop entry, and icon' \
	  '                    PREFIX defaults to ~/.local'

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

macos-app:
	$(UV) run --script scripts/build-macos-app

install-macos:
	$(UV) run --script scripts/build-macos-app --install

install-linux:
	@test "$$(uname -s)" = Linux || \
	  { echo 'install-linux requires Linux' >&2; exit 1; }
	@command -v desktop-file-edit >/dev/null || \
	  { echo 'install-linux requires desktop-file-utils' >&2; exit 1; }
	$(CARGO) install --path . --locked --root "$(PREFIX)"
	install -Dm644 assets/cryptty-linux-512.png \
	  "$(PREFIX)/share/icons/hicolor/512x512/apps/cryptty.png"
	install -Dm644 assets/cryptty.desktop \
	  "$(PREFIX)/share/applications/org.cryptty.desktop"
	@prefix=$$(realpath -m -- "$(PREFIX)") && \
	  desktop-file-edit \
	    --set-key=Exec --set-value="\"$$prefix/bin/cryptty\"" \
	    --set-key=TryExec --set-value="$$prefix/bin/cryptty" \
	    --set-icon="$$prefix/share/icons/hicolor/512x512/apps/cryptty.png" \
	    "$$prefix/share/applications/org.cryptty.desktop"
	update-desktop-database "$(PREFIX)/share/applications"
