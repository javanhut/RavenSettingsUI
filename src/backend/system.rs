//! Facts about the machine and the person using it, plus power actions.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

/// raven-powerd's socket: the desktop's logind stand-in. Group `video`.
pub const POWER_SOCKET: &str = "/run/raven-power/ctl";
pub const POWER_POLICY: &str = "/etc/raven/power.toml";

#[derive(Debug, Clone, Default)]
pub struct OsRelease {
    pub name: String,
    pub pretty_name: String,
    pub version: String,
    pub version_id: String,
    pub build_id: String,
    pub home_url: String,
}

pub fn os_release() -> OsRelease {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let map = parse_os_release(&text);
    let get = |k: &str| map.get(k).cloned().unwrap_or_default();
    OsRelease {
        name: get("NAME"),
        pretty_name: get("PRETTY_NAME"),
        version: get("VERSION"),
        version_id: get("VERSION_ID"),
        build_id: get("BUILD_ID"),
        home_url: get("HOME_URL"),
    }
}

pub fn parse_os_release(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct Hardware {
    pub hostname: String,
    pub kernel: String,
    pub cpu: String,
    pub cpu_threads: usize,
    pub memory_bytes: u64,
    pub gpu: String,
    pub uptime: Duration,
}

pub fn hardware() -> Hardware {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let cpu = cpuinfo
        .lines()
        .find_map(|l| l.strip_prefix("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_else(|| "Unknown".into());
    let cpu_threads = cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let memory_bytes = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(0);
    let uptime = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(Duration::from_secs_f64)
        .unwrap_or_default();
    Hardware {
        hostname: std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        cpu,
        cpu_threads,
        memory_bytes,
        gpu: gpu_name(),
        uptime,
    }
}

/// Best effort: the first DRM card's driver, from sysfs. No lspci needed.
fn gpu_name() -> String {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return String::new();
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let driver = std::fs::read_link(e.path().join("device/driver"))
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()));
        if let Some(d) = driver {
            return d;
        }
    }
    String::new()
}

#[derive(Debug, Clone, Default)]
pub struct User {
    pub login: String,
    pub full_name: String,
    /// `~/.face` if it exists.
    pub avatar: Option<PathBuf>,
}

pub fn user() -> User {
    let login = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let full_name = passwd
        .lines()
        .filter(|l| l.starts_with(&format!("{login}:")))
        .find_map(|l| l.split(':').nth(4))
        .map(|g| g.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| login.clone());
    let face = home.join(".face");
    User {
        login,
        full_name,
        avatar: face.exists().then_some(face),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Suspend,
    PowerOff,
    Reboot,
}

impl PowerAction {
    fn verb(self) -> &'static str {
        match self {
            PowerAction::Suspend => "suspend",
            PowerAction::PowerOff => "poweroff",
            PowerAction::Reboot => "reboot",
        }
    }
}

/// Ask raven-powerd. One verb per line, one line back; a reply starting with
/// `error` is a refusal.
pub fn power(action: PowerAction) -> Result<()> {
    let mut stream = UnixStream::connect(POWER_SOCKET)
        .with_context(|| format!("raven-powerd is not reachable at {POWER_SOCKET}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    writeln!(stream, "{}", action.verb())?;
    stream.flush()?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    let reply = reply.trim();
    if reply.starts_with("error") {
        bail!("{reply}");
    }
    Ok(())
}

pub fn power_available() -> bool {
    std::path::Path::new(POWER_SOCKET).exists()
}

/// `/etc/raven/power.toml`, read-only here: it is root's file. Values are
/// `suspend`, `poweroff`, `reboot` or `ignore`.
#[derive(Debug, Clone, Default)]
pub struct PowerPolicy {
    pub power_button: String,
    pub sleep_button: String,
    pub lid_close: String,
}

pub fn power_policy() -> Result<PowerPolicy> {
    let text = std::fs::read_to_string(POWER_POLICY)?;
    let v: toml::Value = toml::from_str(&text)?;
    let pick = |t: &str, k: &str| {
        v.get(t)
            .and_then(|s| s.get(k))
            .and_then(|s| s.as_str())
            .unwrap_or("suspend")
            .to_string()
    };
    Ok(PowerPolicy {
        power_button: pick("buttons", "power"),
        sleep_button: pick("buttons", "sleep"),
        lid_close: pick("lid", "close"),
    })
}

/// Rewrite one key of the power policy. Needs root, so this is run through
/// `sudo -n`; the caller shows the manual command when that is refused.
pub fn set_power_policy(table: &str, key: &str, value: &str) -> Result<()> {
    if !["suspend", "poweroff", "reboot", "ignore"].contains(&value) {
        bail!("{value} is not a power action");
    }
    let script = format!(
        "sed -i '/^\\[{table}\\]/,/^\\[/ s/^{key} = .*/{key} = \"{value}\"/' {POWER_POLICY} && raven-rc restart powerd"
    );
    let out = std::process::Command::new("sudo")
        .args(["-n", "sh", "-c", &script])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("needs root. Run:\n  sudo sh -c '{script}'"));
    }
    Ok(())
}

pub fn locale() -> String {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "C".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_parses_quotes() {
        let m = parse_os_release("NAME=\"Raven Linux\"\nID=raven\n");
        assert_eq!(m["NAME"], "Raven Linux");
        assert_eq!(m["ID"], "raven");
    }
}
