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
| Network | `cawd` over its socket `/run/caw/caw.sock` (newline JSON, the `caw-ipc` wire form) | Scan, join (passphrase and enterprise credentials asked in a dialog, never on argv), disconnect, wired ports up/down. Joining needs the account in the `caw` group; the page offers to run the `usermod` in your terminal. Starting or stopping cawd is `sudo raven-rc` in your terminal too. Forgetting a saved network is not in cawd's protocol yet. |
| Bluetooth | BlueZ on the system bus via zbus | Power, visibility, discovery while the page is open, pair/trust/connect, forget. A `KeyboardDisplay` agent turns passkey confirmations, PIN and passkey requests into dialogs. |
| Sound | `wpctl` | Same backend as Huginn's quick settings, so the two never disagree. Default sink/source, volume, mute. |
| Display | `raven_output_layout_v1` (raven_shell_v1 **version 3**) + `/sys/class/backlight` | Per-screen scale and position, applied together. Needs a compositor that offers v3; older ones get a clear message. Brightness needs the udev rule below; the page offers to install it in your terminal. |
| Storage | `lsblk -J`, `df` | |
| Updates | `rvn update --dry-run` (report is on stderr) | Installing opens your terminal on `rvn update`, which goes through `rvnd` on `/run/rvn/ctl` when that socket is reachable (no password), and on `sudo rvn update` otherwise. Either way prompts and `makepkg` output stay visible. |
| General | `/etc/raven/power.toml` (read), `/run/raven-power/ctl` for sleep/restart/power off | Changing the button and lid policy rewrites root's file and restarts `powerd`; that runs in your terminal. |
| Personalization | `$XDG_STATE_HOME/raven/pins` (dock), `~/.config/roostbar/config.toml` (bar), `~/.config/mimeapps.list` through GIO (default apps) | The compositor reads `pins` at start, so dock edits show at next login. |
| Appearance | `~/.config/raven/desktop.toml`, read by Huginn; wallpaper via `ravencanvas set --persist`; pushed to RoostBar and GTK | See below. Without RavenCanvas, installing the wallpaper system-wide is offered as a command for your terminal. |
| Privacy | `$XDG_STATE_HOME/raven/frecency` and app search histories | |
| Key Overlay | `raven-keycast` (in this repo), reading `[keycast]` in desktop.toml; `~/.config/raven/services/raven-keycast.toml` for `raven-init --user` | Switching it on starts the overlay now and at every login. Reading keys needs the account in the `input` group; the page offers the `usermod` in your terminal. See below. |

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
wallpaper = ""               # a still, under ~/.local/share/raven/wallpaper/ (first frame of a live one)

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
  `ravencanvas set image <file> --persist` for a PNG or JPEG and
  `ravencanvas set motion <file> --persist` for an animated WebP, which draws
  it on the desktop and the login screen and persists it in
  `~/.config/raven/canvas.toml`. A video (MP4, WebM, MKV, MOV) is offered in
  the picker only when `ffmpeg` is installed; it is converted once, here, to
  a looping animated WebP — at most 1920 wide and 24 fps, retried smaller
  until it fits RavenCanvas's 100 MiB file limit — because the daemon plays
  WebP and nothing else, on purpose (see RavenCanvas's README, "Live
  wallpapers, and why they are not MP4s"). `desktop.toml` gets a PNG of the
  first frame, since Huginn draws that field itself when the daemon is not
  running and it draws stills only.
- **RoostBar** and **GTK** are written to on every save, as above.

- `appearance.transparency` makes this window glass (translucent), and
  `appearance.blur` tells Huginn to blur the desktop behind it — the
  compositor treats `raven-settings` as a glass window and runs its launcher
  blur pass under its rectangle (`Huginn::glass_window`).

Shadows, animation speed and interface scale are recorded for applications
and the bar; the compositor has no switches for those yet.

## Key Overlay — raven-keycast

The keys and buttons being pressed, drawn on screen, for screen recordings,
demonstrations and teaching. Its own binary in `keycast/`, with no GTK: a
`wlr-layer-shell` surface drawn into shm the way RoostBar is.

- **Where it draws.** The `overlay` layer, which Huginn keeps above every
  window including a fullscreen one, with no keyboard interactivity and an
  empty input region, so every click goes through it to what is beneath.
- **Where the keys come from.** `/dev/input/event*`, through evdev, which
  the `input` group Raven gives the desktop user may read. No Wayland
  protocol hands one client the keys typed into another, and none should.
  Devices are rescanned every two seconds, so a keyboard plugged in later
  shows up.
- **What it looks like.** Each stroke is a glass panel of keycaps:
  `Ctrl + Shift + T` together, `A ×3` for a repeat. A group rises and fades
  in; a cap's face presses down onto its lip and tints towards the accent
  while the key is held, sends a ring outward as it goes down, and springs
  back when released; the count pops; a group sinks and fades out once its
  keys have been up for the hide-after time, and the row slides over to
  close the gap. With `smooth_animations = false` groups only fade and caps
  change colour without moving. Frames are drawn only while something
  moves.
- **Privacy.** Nothing typed is written anywhere. `mode = "shortcuts"`
  shows only keys pressed with Ctrl, Alt or Super and keys that type no
  text, so typing stays off screen. Huginn draws nothing but the lock screen
  while locked; on top of that, keys are dropped while `raven-lock` is
  running and the overlay is cleared either side of a lock, so a password
  typed there cannot appear after it.
- **Starting and stopping.** It re-reads desktop.toml when the file
  changes; while `enabled` is false it holds no surface and no device open.
  The page's switch saves the setting, writes
  `~/.config/raven/services/raven-keycast.toml` (with `enabled` to match)
  so the session supervisor starts it at login, and starts or stops the
  process now: through `raven-rc --user` when that supervisor already knows
  the service, directly otherwise (logging to
  `~/.local/state/raven/log/raven-keycast.log`). One instance per session,
  held by a lock in `$XDG_RUNTIME_DIR`. The supervisor starts before the
  compositor and so passes no `WAYLAND_DISPLAY`; without one the overlay
  connects to the `wayland-*` socket in `$XDG_RUNTIME_DIR`.

```toml
[keycast]
enabled = false
position = "bottom-centre"   # bottom-centre | bottom-left | bottom-right | top-centre
size = "medium"              # small | medium | large
mode = "all"                 # all | shortcuts
show_mouse = true
hide_after_ms = 2000
```

Caps are labelled by physical key as on a US keyboard: the overlay reads
devices, not the compositor's keymap. It shows on the screen the compositor
puts new layer surfaces on.

`RAVEN_KEYCAST_FRAMES=<dir> cargo test --release -p raven-keycast -- --ignored frames`
plays a sequence of keys at 60 Hz and saves frames as raw RGB, for looking at
the overlay without a compositor (`ffmpeg -f rawvideo -pix_fmt rgb24 -s 2000x196 -i F.rgb F.png`).

## Privilege: sockets first, then your terminal, never sudo from the GUI

The rule is RavenLinux's (ARCHITECTURE.md, "Sleep" and the paragraphs after
it): privilege is a verb granted by a group on a socket owned by a daemon
that is already the policy gatekeeper. `raven-settings` never runs `sudo`,
`pkexec` or `run0` itself, and has no password prompt of its own. Where a
daemon offers the action, the page talks to its socket; where none does, the
page shows the exact command and, once you agree, opens your terminal on it
(`backend/terminal.rs`, the pattern Raven Store uses to apply updates), so
sudo asks for the password where you can see it. Per site:

| Action | Path | Why |
|---|---|---|
| Sleep, restart, power off | `raven-powerd` on `/run/raven-power/ctl` (group `video`) | The daemon exists for this. |
| Clock and time zone | `raven-timed` on `/run/raven-time/ctl` (group `video`) | Same. |
| Join, scan, ports up/down | `cawd` on `/run/caw/caw.sock` (group `caw`) | Same. |
| Install updates | `rvn update` in your terminal; rvn itself uses `rvnd` on `/run/rvn/ctl` (group `wheel`) when it can connect, else `sudo rvn update` in that terminal | rvnd is the door for packages. The terminal stays because the plan is confirmed and `makepkg` runs there. |
| Start or stop cawd | `sudo raven-rc start|stop cawd` in your terminal | Init's socket is root-only and no daemon fronts service control; nothing unprivileged writes to PID 1. |
| Power button and lid policy | `sudo sh -c 'sed … /etc/raven/power.toml && raven-rc restart powerd'` in your terminal | `raven-powerd`'s socket takes `suspend`, `poweroff`, `reboot` and nothing about policy; the file is root's. |
| Add the account to `caw` | `sudo usermod -aG caw $USER` in your terminal | Group membership has no daemon. |
| Backlight udev rule | `sudo sh -c '… > /etc/udev/rules.d/90-backlight.rules && chgrp/chmod …'` in your terminal | `raven-controlsd` (`/run/raven-controls/ctl`) drives the keyboard backlight and fans, not `/sys/class/backlight`. |
| System-wide wallpaper | `sudo sh -c 'rm -f … && install …'` in your terminal, only when RavenCanvas is absent | `ravencanvas set --persist` is the no-root path; `/usr/share/wallpaper/set` is root's and nothing owns it. |

Every dialog that leads to a terminal also offers to copy the command, so
running it by hand is always an option.

## Setup on a machine

- **Wi-Fi changes**: `sudo usermod -aG caw $USER`, then log out and in
  (the Network page offers this).
- **Brightness**: `sudo cp data/90-backlight.rules /etc/udev/rules.d/ && sudo udevadm trigger -s backlight`
  (or after `make install`, from `/usr/local/share/raven-settings/`; the
  Display page offers the equivalent).
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
keycast/               raven-keycast, the on-screen keystroke overlay (no GTK)
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
