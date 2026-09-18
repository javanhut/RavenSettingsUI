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

.PHONY: all deps build run probe test check clean install uninstall

# System libraries the gtk-rs crates link against: rvn package names, and the
# pkg-config modules (with the version floors Cargo.toml's features need) that
# show they are there. Installed through rvn only when missing.
RVN_DEPS := gtk4 libadwaita
PC_DEPS  := gtk4 >= 4.12, libadwaita-1 >= 1.5

all: build

deps:
	@pkg-config --exists '$(PC_DEPS)' || { \
		echo "Installing build dependencies with rvn: $(RVN_DEPS)"; \
		rvn install --repo-only -y $(RVN_DEPS); \
	}

build: deps
	cargo build --locked --workspace $(CARGO_FLAGS)

# The overlay is built too, so the Key Overlay page finds it beside raven-settings.
run: deps
	cargo build --workspace $(CARGO_FLAGS)
	cargo run $(CARGO_FLAGS) -p raven-settings

probe: deps
	cargo run $(CARGO_FLAGS) -p raven-settings -- --probe

test: deps
	cargo test --locked --workspace

check: deps
	cargo fmt --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	cargo test --locked --workspace

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
	install -Dm755 "$(TARGET_DIR)/raven-keycast" "$(DESTDIR)$(BINDIR)/raven-keycast"
	install -Dm644 "data/$(APP_ID).desktop" "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	install -Dm644 "data/$(APP_ID).metainfo.xml" "$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml"
	install -Dm644 "data/icons/hicolor/scalable/apps/$(APP_ID).svg" "$(DESTDIR)$(ICONDIR)/$(APP_ID).svg"
	install -Dm644 "data/90-backlight.rules" "$(DESTDIR)$(DATADIR)/$(BIN_NAME)/90-backlight.rules"
	$(update-caches)

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/raven-keycast"
	rm -f "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	rm -f "$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml"
	rm -f "$(DESTDIR)$(ICONDIR)/$(APP_ID).svg"
	rm -rf "$(DESTDIR)$(DATADIR)/$(BIN_NAME)"
	$(update-caches)
