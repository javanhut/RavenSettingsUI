//! Screens through `raven_output_layout_v1`, the compositor's own extension
//! (there is no wlr-output-management on Raven), and backlight through sysfs.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, Proxy as _, QueueHandle,
};

#[allow(non_upper_case_globals, unused, clippy::all, missing_docs)]
pub mod protocol {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/raven-shell-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/raven-shell-v1.xml");
}

use protocol::raven_output_layout_v1::{self, RavenOutputLayoutV1};
use protocol::raven_shell_manager_v1::RavenShellManagerV1;

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    pub physical_width: i32,
    pub physical_height: i32,
    pub mm_width: i32,
    pub mm_height: i32,
    pub focused: bool,
}

impl Output {
    pub fn diagonal_inches(&self) -> Option<f64> {
        if self.mm_width <= 0 || self.mm_height <= 0 {
            return None;
        }
        let w = self.mm_width as f64;
        let h = self.mm_height as f64;
        Some((w * w + h * h).sqrt() / 25.4)
    }
}

/// One staged change; `scale` of 0 means automatic.
#[derive(Debug, Clone, Default)]
pub struct Change {
    pub name: String,
    pub position: Option<(i32, i32)>,
    pub scale: Option<f64>,
}

#[derive(Default)]
struct State {
    pending: Vec<Output>,
    outputs: Vec<Output>,
    done: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RavenShellManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &RavenShellManagerV1,
        _: protocol::raven_shell_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RavenOutputLayoutV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &RavenOutputLayoutV1,
        event: raven_output_layout_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            raven_output_layout_v1::Event::Output {
                name,
                x,
                y,
                width,
                height,
                scale,
                physical_width,
                physical_height,
                mm_width,
                mm_height,
                focused,
            } => state.pending.push(Output {
                name,
                x,
                y,
                width,
                height,
                scale,
                physical_width,
                physical_height,
                mm_width,
                mm_height,
                focused: focused == 1,
            }),
            raven_output_layout_v1::Event::Done => {
                state.outputs = std::mem::take(&mut state.pending);
                state.done = true;
            }
        }
    }
}

struct Session {
    conn: Connection,
    queue: wayland_client::EventQueue<State>,
    state: State,
    layout: RavenOutputLayoutV1,
    _manager: RavenShellManagerV1,
}

impl Session {
    fn open() -> Result<Self> {
        let conn = Connection::connect_to_env().context("no Wayland display")?;
        let (globals, queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let manager: RavenShellManagerV1 = globals.bind(&qh, 1..=3, ()).map_err(|e| {
            anyhow!("the compositor does not offer raven_shell_manager_v1 ({e}); is this Huginn?")
        })?;
        if manager.version() < 3 {
            bail!(
                "the running compositor speaks raven_shell_v1 version {} and screen arrangement needs version 3. Update RavenGUI (imlazy install) and log in again",
                manager.version()
            );
        }
        let layout = manager.get_output_layout(&qh, ());
        Ok(Self {
            conn,
            queue,
            state: State::default(),
            layout,
            _manager: manager,
        })
    }

    fn wait_done(&mut self) -> Result<Vec<Output>> {
        self.state.done = false;
        for _ in 0..20 {
            self.queue.roundtrip(&mut self.state)?;
            if self.state.done {
                return Ok(self.state.outputs.clone());
            }
        }
        bail!("the compositor never finished listing screens")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.layout.destroy();
        let _ = self.conn.flush();
    }
}

pub fn outputs() -> Result<Vec<Output>> {
    let mut s = Session::open()?;
    s.wait_done()
}

/// Stage every change, apply them together, and return the arrangement the
/// compositor settled on (it may have nudged overlapping screens apart).
pub fn apply(changes: &[Change]) -> Result<Vec<Output>> {
    let mut s = Session::open()?;
    s.wait_done()?;
    for c in changes {
        if let Some((x, y)) = c.position {
            s.layout.set_position(c.name.clone(), x, y);
        }
        if let Some(scale) = c.scale {
            s.layout.set_scale(c.name.clone(), scale);
        }
    }
    s.layout.apply();
    s.conn.flush()?;
    s.wait_done()
}

// ---- backlight ---------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Backlight {
    pub name: String,
    pub path: PathBuf,
    pub brightness: u32,
    pub max: u32,
    pub writable: bool,
}

impl Backlight {
    pub fn percent(&self) -> f64 {
        if self.max == 0 {
            0.0
        } else {
            self.brightness as f64 * 100.0 / self.max as f64
        }
    }
}

pub fn backlights() -> Vec<Backlight> {
    let Ok(dir) = std::fs::read_dir("/sys/class/backlight") else {
        return vec![];
    };
    let mut out = Vec::new();
    for e in dir.flatten() {
        let path = e.path();
        let read = |f: &str| -> Option<u32> {
            std::fs::read_to_string(path.join(f))
                .ok()?
                .trim()
                .parse()
                .ok()
        };
        let (Some(brightness), Some(max)) = (read("brightness"), read("max_brightness")) else {
            continue;
        };
        let writable = std::fs::OpenOptions::new()
            .write(true)
            .open(path.join("brightness"))
            .is_ok();
        out.push(Backlight {
            name: e.file_name().to_string_lossy().to_string(),
            path,
            brightness,
            max,
            writable,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn set_brightness(b: &Backlight, percent: f64) -> Result<()> {
    let value = ((percent.clamp(1.0, 100.0) / 100.0) * b.max as f64).round() as u32;
    std::fs::write(b.path.join("brightness"), value.to_string()).with_context(|| {
        format!(
            "{} is not writable. Grant the video group access with a udev rule (see README)",
            b.path.join("brightness").display()
        )
    })
}

/// The udev rule that lets the session set brightness without root.
pub const UDEV_RULE: &str = r#"ACTION=="add", SUBSYSTEM=="backlight", RUN+="/bin/chgrp video /sys/class/backlight/%k/brightness", RUN+="/bin/chmod g+w /sys/class/backlight/%k/brightness""#;
