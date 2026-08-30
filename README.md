# Raven Settings

One window for everything you would otherwise set with a scatter of CLIs on
Raven Linux: Wi-Fi and wired networks, Bluetooth pairing, sound, screens and
brightness, theme and wallpaper, the dock and bar, default applications,
storage, privacy, updates and system information.

GTK 4 + libadwaita, in Rust, following the layout of the design mockup:
sidebar with your account and the section list, search in the header, and
cards on each page.

```
make            # build (release)
make run        # run it
make probe      # print what every backend sees, for diagnosing a machine
sudo make install
```

or with imlazy: `imlazy build`, `imlazy run`, `imlazy probe`, `imlazy install`.

## What talks to what

Nothing here is faked. Each page drives the component that actually owns the
setting on Raven, and says so when that component is not there.

| Page | Backend | Notes |
|---|---|---|
| Network | `cawd` over its socket `/run/caw/caw.sock` (newline JSON, the `caw-ipc` wire form) | Scan, join (passphrase and enterprise credentials asked in a dialog, never on argv), disconnect, wired ports up/down. Joining needs the account in the `caw` group; the page offers the `usermod` line. Forgetting a saved network is not in cawd's protocol yet. |
| Bluetooth | BlueZ on the system bus via zbus | Power, visibility, discovery while the page is open, pair/trust/connect, forget. A `KeyboardDisplay` agent turns passkey confirmations, PIN and passkey requests into dialogs. |
| Sound | `wpctl` | Same backend as Huginn's quick settings, so the two never disagree. Default sink/source, volume, mute. |
| Display | `raven_output_layout_v1` (raven_shell_v1 **version 3**) + `/sys/class/backlight` | Per-screen scale and position, applied together. Needs a compositor that offers v3; older ones get a clear message. Brightness needs the udev rule below. |
| Storage | `lsblk -J`, `df` | |
| Updates | `rvn update --dry-run` (report is on stderr) | Installing opens your terminal on `sudo rvn update` so prompts and `makepkg` output stay visible. |
| General | `/etc/raven/power.toml` (read; writes via `sudo -n` or shows the command), `/run/raven-power/ctl` for sleep/restart/power off | |
| Personalization | `$XDG_STATE_HOME/raven/pins` (dock), `~/.config/roostbar/config.toml` (bar), `~/.config/mimeapps.list` through GIO (default apps) | The compositor reads `pins` at start, so dock edits show at next login. |
| Appearance | `~/.config/raven/desktop.toml`, read by Huginn; wallpaper via `ravencanvas set --persist`; pushed to RoostBar and GTK | See below. |
| Privacy | `$XDG_STATE_HOME/raven/frecency` and app search histories | |

## desktop.toml — the desktop-wide settings file

Everything the user chooses about how the desktop looks and behaves is
written to `~/.config/raven/desktop.toml` (not `config.toml`, which
RavenFileManager owns). TOML, every key optional, a parse error never fatal:

```toml
[appearance]
theme_mode = "dark"          # light | dark | auto
accent = "#7AA2F7"
scale = 1.0
transparency = true
shadows = true
blur = true
smooth_animations = true
animation_speed = "normal"   # slow | normal | fast
wallpaper = ""               # per-user copy under ~/.local/share/raven/wallpaper/

[general]
terminal = "raven-terminal"
lock_after_minutes = 10
clock_24h = true
show_date = true

[personalization]
dock_position = "centre"
dock_layout = "grid"
bar_position = "top"

[privacy]
remember_app_usage = true
bluetooth_discoverable = false
```

On every save the app also:

- rewrites the keys it owns in `~/.config/roostbar/config.toml` (`accent`,
  `background`, `foreground`, `muted`, `position`, `clock_format`,
  `show_date`), preserving everything else in the file, and
- sets `org.gnome.desktop.interface color-scheme` plus
  `gtk-{3,4}.0/settings.ini` so GTK apps follow the theme mode.

### What reads it

- **Huginn** (RavenGUI) loads it at start and reloads on change
  (`huginn-comp/src/desktop_config.rs` + `configwatch.rs`): accent,
  `smooth_animations` → reduced motion, `lock_after_minutes`, `terminal`, and
  `wallpaper` as its own background when `ravencanvasd` is not running.
  Quick settings has an "All settings" row that opens this app, and
  `Super+Ctrl+P` does the same.
- **RavenCanvas** gets the wallpaper directly: the Appearance page runs
  `ravencanvas set image <file> --persist`, which draws it on the desktop and
  the login screen and persists it in `~/.config/raven/canvas.toml`.
- **RoostBar** and **GTK** are written to on every save, as above.

- `appearance.transparency` makes this window glass (translucent), and
  `appearance.blur` tells Huginn to blur the desktop behind it — the
  compositor treats `raven-settings` as a glass window and runs its launcher
  blur pass under its rectangle (`Huginn::glass_window`).

Shadows, animation speed and interface scale are recorded for applications
and the bar; the compositor has no switches for those yet.

## Setup on a machine

- **Wi-Fi changes**: `sudo usermod -aG caw $USER`, then log out and in.
- **Brightness**: `sudo cp data/90-backlight.rules /etc/udev/rules.d/ && sudo udevadm trigger -s backlight`
  (or after `make install`, from `/usr/local/share/raven-settings/`).
- **Bluetooth**: `sudo rvn install -y bluez`, copy
  `/usr/share/raven/services/bluetoothd.toml` to `/etc/raven/init.d/`,
  `sudo raven-rc reload && sudo raven-rc start bluetoothd`.
- **Screen arrangement**: a Huginn that advertises `raven_shell_manager_v1`
  version 3 (the current RavenGUI tree does; `imlazy install` there).

`raven-settings --probe` prints what each backend can see and is the first
thing to run when a page says something is unavailable.

## Layout

```
src/main.rs            entry, --probe
src/config.rs          desktop.toml schema, load/save
src/util.rs            process helpers
src/backend/           one module per subsystem, no GTK (apps.rs uses GIO)
src/ui/mod.rs          App, background work, dialogs
src/ui/theme.rs        palette as libadwaita named colours + card CSS
src/ui/widgets.rs      page/card/row builders
src/ui/window.rs       sidebar, search, page stack
src/ui/pages/          one module per page
protocols/             raven-shell-v1.xml, vendored from RavenGUI
data/                  desktop entry, icon, metainfo, udev rule
```

Backends are plain blocking Rust; the UI runs them through
`gio::spawn_blocking` and gets the result back on the main loop. Prompts that
a backend raises from another thread (a Wi-Fi passphrase, a Bluetooth
passkey) hop to the main loop with `glib::idle_add_once` and answer over a
channel.

The window is tiled by Huginn and may get a quarter of the screen: under
860px the sidebar collapses behind a header button and two-column pages stack
to one column; the minimum size is 480×360 and every page scrolls. The
headerbar's minimize button goes to Huginn's dock like the gesture does.

`RAVEN_SETTINGS_SNAPSHOT=<dir> raven-settings` renders every page to a PNG in
that directory and exits — handy for checking the UI from a shell;
`RAVEN_SETTINGS_SNAPSHOT_SIZE=620x720` picks the window size.
