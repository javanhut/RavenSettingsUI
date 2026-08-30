# Raven Settings. `make`, `make run`, `sudo make install`. lazy.toml mirrors
# these for imlazy; keep the two in step.

APP_ID    := com.ravensettings.Raven
BIN_NAME  := raven-settings
PREFIX    ?= /usr/local
BINDIR    := $(PREFIX)/bin
DATADIR   := $(PREFIX)/share
ICONDIR   := $(DATADIR)/icons/hicolor/scalable/apps
DESTDIR   ?=
PROFILE   ?= release
CARGO_FLAGS := $(if $(filter release,$(PROFILE)),--release,)
TARGET_DIR  := target/$(PROFILE)

.PHONY: all build run probe test check clean install uninstall

all: build

build:
	cargo build --locked $(CARGO_FLAGS)

run:
	cargo run $(CARGO_FLAGS)

probe:
	cargo run $(CARGO_FLAGS) -- --probe

test:
	cargo test --locked

check:
	cargo fmt --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo test --locked

clean:
	cargo clean

# Refreshing the caches is what makes the entry show up in launchers; skipped
# under DESTDIR, where the packager runs them itself.
define update-caches
	@if [ -z "$(DESTDIR)" ]; then \
		command -v update-desktop-database >/dev/null 2>&1 && \
			update-desktop-database -q "$(DATADIR)/applications" || true; \
		command -v gtk-update-icon-cache >/dev/null 2>&1 && \
			gtk-update-icon-cache -qtf "$(DATADIR)/icons/hicolor" || true; \
	fi
endef

install: build
	install -Dm755 "$(TARGET_DIR)/$(BIN_NAME)" "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	install -Dm644 "data/$(APP_ID).desktop" "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	install -Dm644 "data/$(APP_ID).metainfo.xml" "$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml"
	install -Dm644 "data/icons/hicolor/scalable/apps/$(APP_ID).svg" "$(DESTDIR)$(ICONDIR)/$(APP_ID).svg"
	install -Dm644 "data/90-backlight.rules" "$(DESTDIR)$(DATADIR)/$(BIN_NAME)/90-backlight.rules"
	$(update-caches)

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	rm -f "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	rm -f "$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml"
	rm -f "$(DESTDIR)$(ICONDIR)/$(APP_ID).svg"
	rm -rf "$(DESTDIR)$(DATADIR)/$(BIN_NAME)"
	$(update-caches)
