//! raven-keycast: the keys and buttons being pressed, drawn on screen, for
//! screen recordings and demonstrations.
//!
//! A `wlr-layer-shell` surface on the overlay layer that asks for no keyboard
//! focus and has an empty input region, so it sits above every window and
//! every click goes through it to whatever is beneath. Keys come from the
//! evdev devices, which the `input` group Raven gives the desktop user can
//! read; no Wayland protocol hands one client the keys typed into another,
//! and none should.
//!
//! Settings are `[keycast]` in `~/.config/raven/desktop.toml`, re-read when
//! the file changes. While `enabled` is false the daemon holds no surface and
//! no device open, so leaving it running costs nothing until it is wanted.
//!
//! Nothing typed is written anywhere. While the lock screen is up keys are
//! dropped, and whatever was showing either side of a lock is cleared, so a
//! password typed at it cannot appear afterwards.

mod config;
mod keys;
mod model;
mod render;
mod scene;

use std::collections::{HashMap, HashSet};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, Mode as IoMode, PostAction, RegistrationToken};
use calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, QueueHandle,
};

use config::{Config, Position};
use render::{Canvas, Text};

/// How often the settings file, the lock screen and new devices are looked at.
const TICK: Duration = Duration::from_millis(500);
/// Devices are looked for every this many ticks.
const SCAN_EVERY: u32 = 4;
/// Keys are dropped while the lock screen is up; this is how old that answer
/// may be when a key arrives.
const LOCK_FRESH: Duration = Duration::from_millis(200);
/// A frame callback that has not come in this long is not coming (the
/// surface is not being drawn), so stop waiting on it.
const FRAME_GIVE_UP: Duration = Duration::from_secs(1);

struct Overlay {
    layer: LayerSurface,
    pool: SlotPool,
    _input: Region,
    width: u32,
    height: u32,
    scale: i32,
    configured: bool,
    /// A cleared buffer is what is on screen, so there is nothing to redraw.
    blank: bool,
}

struct Input {
    device: evdev::Device,
    token: RegistrationToken,
}

struct Keycast {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    qh: QueueHandle<Keycast>,
    handle: LoopHandle<'static, Keycast>,
    text: Text,
    config: Config,
    stamp: Option<(SystemTime, u64)>,
    overlay: Option<Overlay>,
    devices: HashMap<PathBuf, Input>,
    /// Event nodes that are neither keyboards nor mice, so they are not
    /// reopened on every scan.
    ignored: HashSet<PathBuf>,
    said_denied: bool,
    strokes: model::Strokes,
    layout: scene::Layout,
    locked: bool,
    lock_checked: Instant,
    ticks: u32,
    frame_pending: Option<Instant>,
    wake: Option<RegistrationToken>,
}

fn main() {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "raven-keycast: shows the keys and buttons being pressed on screen.\n\n\
             Configured by [keycast] in {} (Settings, Key Overlay).\n\
             While it is switched off there it runs idle, holding nothing open.",
            config::path().display()
        );
        return;
    }
    let _instance = match single_instance() {
        Ok(lock) => lock,
        Err(()) => {
            eprintln!("raven-keycast: already running");
            return;
        }
    };

    let conn = connect().unwrap_or_else(|| {
        eprintln!("raven-keycast: no Wayland compositor found within 15s");
        std::process::exit(1);
    });
    let (globals, queue) = registry_queue_init(&conn).expect("raven-keycast: registry");
    let qh: QueueHandle<Keycast> = queue.handle();
    let mut event_loop: EventLoop<Keycast> =
        EventLoop::try_new().expect("raven-keycast: event loop");
    WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .expect("raven-keycast: wayland source");

    let compositor = CompositorState::bind(&globals, &qh).expect("raven-keycast: wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh)
        .expect("raven-keycast: compositor has no zwlr_layer_shell_v1");
    let shm = Shm::bind(&globals, &qh).expect("raven-keycast: wl_shm");
    let text = Text::load().unwrap_or_else(|e| {
        eprintln!("raven-keycast: {e}");
        std::process::exit(1);
    });
    let config = config::load().unwrap_or_else(|e| {
        eprintln!("raven-keycast: {e}; using defaults");
        Config::default()
    });

    let mut state = Keycast {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        qh,
        handle: event_loop.handle(),
        text,
        config,
        stamp: config::stamp(),
        overlay: None,
        devices: HashMap::new(),
        ignored: HashSet::new(),
        said_denied: false,
        strokes: model::Strokes::default(),
        layout: scene::Layout::default(),
        locked: session_locked(),
        lock_checked: Instant::now(),
        ticks: 0,
        frame_pending: None,
        wake: None,
    };
    event_loop
        .handle()
        .insert_source(Timer::from_duration(TICK), |_, _, state: &mut Keycast| {
            state.tick();
            TimeoutAction::ToDuration(TICK)
        })
        .expect("raven-keycast: timer");
    state.apply_config();

    loop {
        event_loop
            .dispatch(None, &mut state)
            .expect("raven-keycast: event loop");
    }
}

impl Keycast {
    /// Bring what exists in line with `self.config`: a surface and devices
    /// while enabled, nothing while not.
    fn apply_config(&mut self) {
        if !self.config.enabled {
            if self.overlay.take().is_some() {
                eprintln!("raven-keycast: off");
            }
            self.close_devices();
            self.strokes.clear();
            self.strokes.forget_held();
            self.layout.reset();
            self.frame_pending = None;
            return;
        }
        if self.devices.is_empty() {
            self.scan_devices();
        }
        match &mut self.overlay {
            Some(ov) => {
                place(&ov.layer, &self.config);
                ov.layer.commit();
                ov.blank = false;
                self.redraw_soon();
            }
            None => self.create_overlay(),
        }
    }

    fn create_overlay(&mut self) {
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some("raven-keycast"),
            None,
        );
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(0);
        place(&layer, &self.config);
        // An empty input region: every click lands on whatever is beneath.
        let input = match Region::new(&self.compositor) {
            Ok(region) => region,
            Err(e) => {
                eprintln!("raven-keycast: input region: {e}");
                return;
            }
        };
        layer.wl_surface().set_input_region(Some(input.wl_region()));
        layer.commit();
        let (w, h) = scene::surface_size(self.config.size);
        let pool = match SlotPool::new((w * h * 4) as usize, &self.shm) {
            Ok(pool) => pool,
            Err(e) => {
                eprintln!("raven-keycast: shm pool: {e}");
                return;
            }
        };
        self.overlay = Some(Overlay {
            layer,
            pool,
            _input: input,
            width: w,
            height: h,
            scale: 1,
            configured: false,
            blank: false,
        });
        eprintln!("raven-keycast: on");
    }

    fn tick(&mut self) {
        let stamp = config::stamp();
        if stamp != self.stamp {
            self.stamp = stamp;
            match config::load() {
                Ok(config) if config != self.config => {
                    self.config = config;
                    self.apply_config();
                }
                Ok(_) => {}
                Err(e) => eprintln!("raven-keycast: {e}; keeping the settings it had"),
            }
        }
        if !self.config.enabled {
            return;
        }
        if self.overlay.is_none() {
            self.create_overlay();
        }
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks.is_multiple_of(SCAN_EVERY) {
            self.scan_devices();
        }
        self.check_lock(Instant::now());
        if self
            .frame_pending
            .is_some_and(|asked| asked.elapsed() > FRAME_GIVE_UP)
        {
            self.frame_pending = None;
            self.render();
        }
    }

    fn check_lock(&mut self, now: Instant) {
        self.lock_checked = now;
        let locked = session_locked();
        if locked != self.locked {
            self.locked = locked;
            // Whatever was showing either side of a lock goes, so nothing
            // typed at the lock screen can surface after it.
            self.strokes.clear();
            self.redraw_soon();
        }
    }

    fn scan_devices(&mut self) {
        let Ok(dir) = std::fs::read_dir("/dev/input") else {
            return;
        };
        let mut present = HashSet::new();
        for entry in dir.flatten() {
            let path = entry.path();
            let is_event = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"));
            if !is_event {
                continue;
            }
            present.insert(path.clone());
            if !self.devices.contains_key(&path) && !self.ignored.contains(&path) {
                self.open_device(path);
            }
        }
        // A node that went away may come back as a different device.
        self.ignored.retain(|p| present.contains(p));
    }

    fn open_device(&mut self, path: PathBuf) {
        let device = match evdev::Device::open(&path) {
            Ok(device) => device,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied && !self.said_denied {
                    self.said_denied = true;
                    eprintln!(
                        "raven-keycast: cannot read {}: {e}. Reading keys needs the input group: \
                         sudo usermod -aG input $USER, then log out and back in",
                        path.display()
                    );
                }
                return;
            }
        };
        let wanted = device.supported_keys().is_some_and(|k| {
            k.contains(evdev::KeyCode::KEY_A) || k.contains(evdev::KeyCode::BTN_LEFT)
        });
        if !wanted {
            self.ignored.insert(path);
            return;
        }
        let fd = match device
            .set_nonblocking(true)
            .and_then(|()| device.as_fd().try_clone_to_owned())
        {
            Ok(fd) => fd,
            Err(e) => {
                eprintln!("raven-keycast: {}: {e}", path.display());
                return;
            }
        };
        let key = path.clone();
        let source = Generic::new(fd, Interest::READ, IoMode::Level);
        match self
            .handle
            .insert_source(source, move |_, _, state: &mut Keycast| {
                Ok(state.read_device(&key))
            }) {
            Ok(token) => {
                eprintln!(
                    "raven-keycast: reading {} ({})",
                    device.name().unwrap_or("unnamed device"),
                    path.display()
                );
                self.devices.insert(path, Input { device, token });
            }
            Err(e) => eprintln!("raven-keycast: {}: {}", path.display(), e.error),
        }
    }

    fn close_devices(&mut self) {
        for (_, input) in self.devices.drain() {
            self.handle.remove(input.token);
        }
        self.ignored.clear();
    }

    fn read_device(&mut self, path: &Path) -> PostAction {
        let mut keys = Vec::new();
        let result = match self.devices.get_mut(path) {
            Some(input) => input.device.fetch_events().map(|events| {
                for event in events {
                    if let evdev::EventSummary::Key(_, code, value) = event.destructure() {
                        keys.push((code.0, value));
                    }
                }
            }),
            None => return PostAction::Remove,
        };
        if let Err(e) = result {
            if e.kind() != std::io::ErrorKind::WouldBlock {
                // Unplugged, most likely: whatever it had down is up now.
                self.devices.remove(path);
                self.strokes.forget_held();
                return PostAction::Remove;
            }
        }
        if keys.is_empty() {
            return PostAction::Continue;
        }
        let now = Instant::now();
        if now.duration_since(self.lock_checked) >= LOCK_FRESH {
            self.check_lock(now);
        }
        let rules = self.config.rules.clone();
        for (code, value) in keys {
            match value {
                1 if !self.locked => self.strokes.press(code, now, &rules),
                0 => self.strokes.release(code, now),
                // 2 is the keyboard's own autorepeat: one press, one stroke.
                _ => {}
            }
        }
        self.redraw_soon();
        PostAction::Continue
    }

    /// Draw now, unless a frame is already on its way, which will draw.
    fn redraw_soon(&mut self) {
        if self.frame_pending.is_none() {
            self.render();
        }
    }

    fn render(&mut self) {
        let now = Instant::now();
        let rules = self.config.rules.clone();
        self.strokes.tick(now, &rules);
        let Some(ov) = self.overlay.as_mut() else {
            return;
        };
        if !ov.configured {
            return;
        }
        let empty = self.strokes.groups.is_empty();
        if empty && ov.blank {
            self.layout.reset();
            return;
        }

        let (pw, ph) = (ov.width * ov.scale as u32, ov.height * ov.scale as u32);
        let (buffer, bytes) = match ov.pool.create_buffer(
            pw as i32,
            ph as i32,
            pw as i32 * 4,
            wl_shm::Format::Argb8888,
        ) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("raven-keycast: buffer: {e}");
                return;
            }
        };
        let mut canvas = Canvas {
            buf: bytes,
            width: pw,
            height: ph,
        };
        let mut animating = false;
        if empty {
            canvas.clear();
            self.layout.reset();
        } else {
            let style = scene::Style {
                scale: self.config.size * ov.scale as f32,
                accent: self.config.accent,
                motion: self.config.motion,
                position: self.config.position,
            };
            let sliding = scene::draw(
                &mut canvas,
                &self.text,
                &self.strokes,
                &style,
                &mut self.layout,
                now,
            );
            animating = sliding || self.strokes.animating(now);
        }
        ov.blank = empty;

        let surface = ov.layer.wl_surface();
        surface.set_buffer_scale(ov.scale);
        surface.damage_buffer(0, 0, pw as i32, ph as i32);
        if let Err(e) = buffer.attach_to(surface) {
            eprintln!("raven-keycast: attach: {e}");
            return;
        }
        if animating {
            surface.frame(&self.qh, FrameCallbackData(surface.clone()));
        }
        ov.layer.commit();
        self.frame_pending = animating.then_some(now);

        if !animating {
            if let Some(at) = self.strokes.next_deadline(&rules) {
                self.wake_at(at);
            }
        }
    }

    /// Draw again at `at`, when a group is due to leave and nothing else
    /// would ask for a frame.
    fn wake_at(&mut self, at: Instant) {
        if let Some(token) = self.wake.take() {
            self.handle.remove(token);
        }
        self.wake = self
            .handle
            .insert_source(Timer::from_deadline(at), |_, _, state: &mut Keycast| {
                state.wake = None;
                state.redraw_soon();
                TimeoutAction::Drop
            })
            .ok();
    }
}

/// Anchor, margins and size for the configured position.
fn place(layer: &LayerSurface, config: &Config) {
    let (w, h) = scene::surface_size(config.size);
    let edge = (48.0 * config.size) as i32;
    let side = edge / 2;
    let (anchor, (top, right, bottom, left)) = match config.position {
        Position::BottomCentre => (Anchor::BOTTOM, (0, 0, edge, 0)),
        Position::BottomLeft => (Anchor::BOTTOM | Anchor::LEFT, (0, 0, edge, side)),
        Position::BottomRight => (Anchor::BOTTOM | Anchor::RIGHT, (0, side, edge, 0)),
        Position::TopCentre => (Anchor::TOP, (edge, 0, 0, 0)),
    };
    layer.set_anchor(anchor);
    layer.set_margin(top, right, bottom, left);
    layer.set_size(w, h);
}

/// Whether the lock screen is up. Huginn starts `raven-lock` to lock the
/// session and it exits once it has let go, so its process is the lock.
fn session_locked() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    dir.flatten().any(|entry| {
        let is_pid = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
        is_pid
            && std::fs::read(entry.path().join("comm"))
                .is_ok_and(|comm| comm.strip_suffix(b"\n").unwrap_or(&comm) == b"raven-lock")
    })
}

/// One overlay per session. `Err` when another holds the lock; `Ok(None)`
/// when there is nowhere to keep one, which is not worth refusing to run.
fn single_instance() -> Result<Option<std::fs::File>, ()> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("raven-keycast.lock"))
    else {
        return Ok(None);
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Err(()),
        Err(_) => Ok(None),
    }
}

/// Started with the session, the compositor may still be coming up.
fn connect() -> Option<Connection> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(conn) = Connection::connect_to_env() {
            return Some(conn);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

impl CompositorHandler for Keycast {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if let Some(ov) = &mut self.overlay {
            let factor = factor.max(1);
            if ov.scale != factor {
                ov.scale = factor;
                ov.blank = false;
            }
        }
        self.redraw_soon();
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = None;
        self.render();
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Keycast {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Keycast {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        // The screen it was on went away; the next tick makes another.
        self.overlay = None;
        self.frame_pending = None;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(ov) = &mut self.overlay {
            let (w, h) = configure.new_size;
            let (want_w, want_h) = scene::surface_size(self.config.size);
            ov.width = if w == 0 { want_w } else { w };
            ov.height = if h == 0 { want_h } else { h };
            ov.configured = true;
            ov.blank = false;
        }
        self.frame_pending = None;
        self.render();
    }
}

impl ShmHandler for Keycast {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Keycast {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_registry!(Keycast);
delegate_dispatch2!(Keycast);
