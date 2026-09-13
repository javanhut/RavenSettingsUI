//! The keystroke overlay, `raven-keycast`: whether it can run here, and
//! starting and stopping it.
//!
//! What it shows and where is `[keycast]` in desktop.toml, which the daemon
//! re-reads on its own; this module only decides whether a process exists.
//! Switching it on writes a definition to `~/.config/raven/services`, so the
//! session's `raven-init --user` starts it at every login from then on, and
//! starts it now: through that supervisor when it already knows the
//! definition, directly otherwise.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config;
use crate::util;

pub const BINARY: &str = "raven-keycast";
/// Owns `/dev/input/event*`; Raven gives it to the desktop user.
pub const INPUT_GROUP: &str = "input";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAccess {
    Readable,
    Denied,
    NoDevices,
}

/// The daemon: beside this executable first, so a development build finds
/// its own, then on PATH.
pub fn binary() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(BINARY)));
    if let Some(path) = beside.filter(|p| p.is_file()) {
        return Some(path);
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(BINARY))
            .find(|p| p.is_file())
    })
}

pub fn running() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    dir.flatten().any(|entry| {
        let is_pid = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
        is_pid
            && std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|comm| comm.trim_end() == BINARY)
    })
}

/// Whether this account can read the keyboards the daemon reads.
pub fn input_access() -> InputAccess {
    let Ok(dir) = std::fs::read_dir("/dev/input") else {
        return InputAccess::NoDevices;
    };
    let mut any = false;
    for entry in dir.flatten() {
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("event"))
        {
            continue;
        }
        any = true;
        if std::fs::File::open(entry.path()).is_ok() {
            return InputAccess::Readable;
        }
    }
    if any {
        InputAccess::Denied
    } else {
        InputAccess::NoDevices
    }
}

pub fn service_path() -> PathBuf {
    config::config_dir()
        .join("services")
        .join(format!("{BINARY}.toml"))
}

/// A `raven-init --user` definition, in the schema of
/// `/usr/share/raven/user-services`.
pub fn service_definition(exe: &Path, enabled: bool) -> String {
    let exe = toml::Value::String(exe.display().to_string());
    format!(
        "# raven-keycast -- the on-screen keystroke overlay. Written by raven-settings\n\
         # (Key Overlay); raven-init --user reads it when the session starts.\n\
         [[services]]\n\
         name = \"{BINARY}\"\n\
         description = \"On-screen keystroke overlay\"\n\
         exec = {exe}\n\
         args = []\n\
         after = []\n\
         restart = true\n\
         enabled = {enabled}\n\
         critical = false\n"
    )
}

fn write_service(exe: &Path, enabled: bool) -> Result<()> {
    let path = service_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    config::atomic_write(&path, service_definition(exe, enabled).as_bytes())
}

/// A session supervisor is running and `raven-rc` can talk to it.
fn supervised() -> bool {
    std::env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|dir| Path::new(&dir).join("raven-init/ctl").exists())
        && util::have("raven-rc")
}

pub fn start() -> Result<()> {
    let exe = binary().context("raven-keycast is not installed")?;
    write_service(&exe, true)?;
    if running() {
        return Ok(());
    }
    // The supervisor knows only the definitions it read when the session
    // started; one written just now is news to it and the start fails, so
    // that case falls through to starting the daemon here.
    if supervised() && util::run("raven-rc", &["--user", "start", BINARY]).is_ok() {
        return Ok(());
    }
    let log_dir = config::state_dir().join("log");
    std::fs::create_dir_all(&log_dir).ok();
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(format!("{BINARY}.log")));
    let mut cmd = if util::have("setsid") {
        let mut c = Command::new("setsid");
        c.arg(&exe);
        c
    } else {
        Command::new(&exe)
    };
    cmd.stdin(Stdio::null());
    match log.and_then(|f| f.try_clone().map(|g| (f, g))) {
        Ok((out, err)) => cmd.stdout(out).stderr(err),
        Err(_) => cmd.stdout(Stdio::null()).stderr(Stdio::null()),
    };
    cmd.spawn()
        .with_context(|| format!("could not start {}", exe.display()))?;
    Ok(())
}

/// Stop the daemon and keep it from starting at the next login. It hides
/// itself as soon as desktop.toml says it is off, so this is tidying up.
pub fn stop() -> Result<()> {
    if let Some(exe) = binary() {
        write_service(&exe, false)?;
    }
    if !running() {
        return Ok(());
    }
    if supervised() {
        let _ = util::run("raven-rc", &["--user", "stop", BINARY]);
    }
    for _ in 0..10 {
        if !running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    util::run("pkill", &["-x", BINARY])
        .map(|_| ())
        .context("could not stop raven-keycast")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_definition_is_valid_toml_with_the_path_quoted() {
        let text = service_definition(Path::new("/opt/a \"b\"/raven-keycast"), true);
        let v: toml::Value = toml::from_str(&text).unwrap();
        let svc = &v["services"][0];
        assert_eq!(svc["name"].as_str(), Some(BINARY));
        assert_eq!(svc["exec"].as_str(), Some("/opt/a \"b\"/raven-keycast"));
        assert_eq!(svc["enabled"].as_bool(), Some(true));
        assert_eq!(svc["restart"].as_bool(), Some(true));
    }
}
