//! Pushing desktop.toml choices out to the components that can act on them
//! today: RoostBar's config, GTK's colour scheme, and the wallpaper store.
//!
//! Huginn (the compositor) has no configuration surface yet, so its share of
//! these — accent, blur, shadows, animations — is recorded in desktop.toml
//! and waits for the compositor to read it. See README "Compositor hook".

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::{atomic_write, DesktopConfig, ThemeMode};
use crate::util::{have, run};

fn roostbar_config() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("roostbar")
        .join("config.toml")
}

/// Rewrite the handful of RoostBar keys that Settings owns, preserving every
/// other line (and comment) of the file.
pub fn sync_roostbar(cfg: &DesktopConfig) -> Result<()> {
    let path = roostbar_config();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let dark = !matches!(cfg.appearance.theme_mode, ThemeMode::Light);
    let (bg, fg, muted) = if dark {
        ("#D916161F", "#C0CAF5", "#565F89")
    } else {
        ("#D9F2F3F8", "#1A1B26", "#8A8FA8")
    };
    let bg = if cfg.appearance.transparency {
        bg.to_string()
    } else {
        format!("#FF{}", &bg[3..])
    };
    let clock = if cfg.general.clock_24h {
        "%H:%M"
    } else {
        "%I:%M %p"
    };
    let updates = [
        ("accent", format!("\"{}\"", cfg.appearance.accent)),
        ("background", format!("\"{bg}\"")),
        ("foreground", format!("\"{fg}\"")),
        ("muted", format!("\"{muted}\"")),
        (
            "position",
            format!("\"{}\"", cfg.personalization.bar_position),
        ),
        ("clock_format", format!("\"{clock}\"")),
        ("show_date", cfg.general.show_date.to_string()),
    ];
    let new = rewrite_toml_keys(&text, &updates);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(&path, new.as_bytes())
}

/// Replace `key = …` lines at the top level; append keys that are absent.
pub fn rewrite_toml_keys(text: &str, updates: &[(&str, String)]) -> String {
    let mut seen = vec![false; updates.len()];
    let mut out = String::new();
    let mut in_table = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_table = true;
        }
        let mut replaced = false;
        if !in_table {
            for (i, (key, value)) in updates.iter().enumerate() {
                let is_key = trimmed
                    .strip_prefix(key)
                    .map(|r| r.trim_start().starts_with('='))
                    .unwrap_or(false);
                if is_key {
                    let comment = line
                        .find('#')
                        .filter(|&i| i > line.find('=').unwrap_or(0))
                        .map(|i| format!("  {}", line[i..].trim_end()))
                        .unwrap_or_default();
                    out.push_str(&format!("{key} = {value}{comment}\n"));
                    seen[i] = true;
                    replaced = true;
                    break;
                }
            }
        }
        if !replaced {
            out.push_str(line);
            out.push('\n');
        }
    }
    let missing: Vec<String> = updates
        .iter()
        .zip(seen)
        .filter(|(_, s)| !s)
        .map(|((k, v), _)| format!("{k} = {v}"))
        .collect();
    if !missing.is_empty() {
        // Top-level keys must precede any table, so insert before the first.
        let block = format!("{}\n", missing.join("\n"));
        match out.find("\n[") {
            Some(i) => out.insert_str(i + 1, &block),
            None => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&block);
            }
        }
    }
    out
}

/// Ask RoostBar to reread its config. It has no reload signal, so this is a
/// restart: the session launcher would start it once, and it re-execs
/// itself from the same path.
pub fn restart_roostbar() -> Result<()> {
    let out = std::process::Command::new("pgrep")
        .args(["-x", "roostbar"])
        .output()?;
    if !out.status.success() {
        return Ok(());
    }
    let exe = std::fs::read_link(format!(
        "/proc/{}/exe",
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .lines()
            .next()
            .unwrap_or("0")
    ))
    .unwrap_or_else(|_| PathBuf::from("roostbar"));
    let _ = std::process::Command::new("pkill")
        .args(["-x", "roostbar"])
        .status();
    std::process::Command::new("setsid")
        .arg(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("could not restart roostbar")?;
    Ok(())
}

/// GTK apps (this one included, RavenFileManager, portals) follow
/// `org.gnome.desktop.interface color-scheme`; and GTK 3 apps read
/// settings.ini. Set both so nothing is left out.
pub fn sync_gtk(cfg: &DesktopConfig) -> Result<()> {
    let scheme = match cfg.appearance.theme_mode {
        ThemeMode::Dark => "prefer-dark",
        ThemeMode::Light => "prefer-light",
        ThemeMode::Auto => "default",
    };
    if have("gsettings") {
        let _ = run(
            "gsettings",
            &["set", "org.gnome.desktop.interface", "color-scheme", scheme],
        );
    }
    let dark = matches!(cfg.appearance.theme_mode, ThemeMode::Dark);
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    for dir in ["gtk-3.0", "gtk-4.0"] {
        let path = base.join(dir).join("settings.ini");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "[Settings]\n".into());
        let mut lines: Vec<String> = text
            .lines()
            .filter(|l| {
                !l.trim_start()
                    .starts_with("gtk-application-prefer-dark-theme")
            })
            .map(str::to_string)
            .collect();
        if !lines.iter().any(|l| l.trim() == "[Settings]") {
            lines.insert(0, "[Settings]".into());
        }
        let at = lines
            .iter()
            .position(|l| l.trim() == "[Settings]")
            .unwrap_or(0)
            + 1;
        lines.insert(
            at,
            format!("gtk-application-prefer-dark-theme={}", dark as u8),
        );
        std::fs::create_dir_all(path.parent().unwrap())?;
        atomic_write(&path, format!("{}\n", lines.join("\n")).as_bytes())?;
    }
    Ok(())
}

/// Where the user's wallpaper lives. The compositor reads
/// `/usr/share/wallpaper/set/wallpaper.*` (root-owned) at start, so the
/// per-user copy is what the desktop should learn to read; installing it
/// system-wide is offered as a privileged command.
pub const SYSTEM_WALLPAPER_DIR: &str = "/usr/share/wallpaper/set";

pub fn user_wallpaper_dir() -> PathBuf {
    crate::config::data_dir().join("wallpaper")
}

pub fn install_user_wallpaper(src: &Path) -> Result<PathBuf> {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !["png", "jpg", "jpeg"].contains(&ext.as_str()) {
        bail!("the desktop draws PNG and JPEG wallpapers only");
    }
    let dir = user_wallpaper_dir();
    std::fs::create_dir_all(&dir)?;
    for old in std::fs::read_dir(&dir)?.flatten() {
        let _ = std::fs::remove_file(old.path());
    }
    let dest = dir.join(format!("wallpaper.{ext}"));
    std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
    Ok(dest)
}

/// Set the wallpaper the way Raven does it: through RavenCanvas, which draws
/// the desktop and the login screen and persists the choice per user in
/// `~/.config/raven/canvas.toml`. No root needed. Returns false when
/// `ravencanvas` is not installed, in which case the compositor's own
/// fallback (desktop.toml's `wallpaper`) is all that applies.
pub fn set_wallpaper_via_canvas(path: &Path) -> Result<bool> {
    if !have("ravencanvas") {
        return Ok(false);
    }
    run(
        "ravencanvas",
        &["set", "image", &path.to_string_lossy(), "--persist"],
    )
    .map(|_| true)
    .context("ravencanvas refused the image")
}

/// The command that puts a wallpaper where the compositor reads it.
pub fn system_wallpaper_command(src: &Path) -> String {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
    format!(
        "sudo sh -c 'rm -f {dir}/wallpaper.* && install -m 644 \"{src}\" {dir}/wallpaper.{ext}'",
        dir = SYSTEM_WALLPAPER_DIR,
        src = src.display(),
    )
}

pub fn current_system_wallpaper() -> Option<PathBuf> {
    let dir = std::fs::read_dir(SYSTEM_WALLPAPER_DIR).ok()?;
    dir.flatten()
        .map(|e| e.path())
        .find(|p| p.file_stem().map(|s| s == "wallpaper").unwrap_or(false))
}

/// The compositor's dock state file. It reads this at startup only, so an
/// edit here takes effect at the next login — the page says so.
pub fn pins_path() -> PathBuf {
    crate::config::state_dir().join("pins")
}

pub fn read_pins() -> (String, String, Vec<String>) {
    let text = std::fs::read_to_string(pins_path()).unwrap_or_default();
    let mut position = "Centre".to_string();
    let mut orientation = "Grid".to_string();
    let mut pins = Vec::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('\t') else {
            continue;
        };
        match k {
            "position" => position = v.trim().to_string(),
            "orientation" => orientation = v.trim().to_string(),
            "pin" => pins.push(v.trim().to_string()),
            _ => {}
        }
    }
    (position, orientation, pins)
}

pub fn write_pins(position: &str, orientation: &str, pins: &[String]) -> Result<()> {
    let mut text = format!("position\t{position}\norientation\t{orientation}\n");
    for p in pins {
        text.push_str(&format!("pin\t{p}\n"));
    }
    let path = pins_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    atomic_write(&path, text.as_bytes())
}

/// Launcher frecency history, for the Privacy page.
pub fn frecency_path() -> PathBuf {
    crate::config::state_dir().join("frecency")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_keys_and_keeps_comments() {
        let src =
            "# header\nposition = \"top\"   # where\nheight = 26\n\n[table]\naccent = \"x\"\n";
        let out = rewrite_toml_keys(
            src,
            &[
                ("position", "\"bottom\"".into()),
                ("accent", "\"#123456\"".into()),
            ],
        );
        assert!(out.contains("position = \"bottom\"  # where"));
        assert!(out.contains("height = 26"));
        // accent under [table] is untouched; a top-level one is added before it.
        assert!(out.contains("accent = \"#123456\"\n[table]\naccent = \"x\""));
    }

    #[test]
    fn appends_to_empty() {
        let out = rewrite_toml_keys("", &[("accent", "\"#1\"".into())]);
        assert_eq!(out, "accent = \"#1\"\n");
    }
}
