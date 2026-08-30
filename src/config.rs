//! The desktop-wide settings file, `~/.config/raven/desktop.toml`.
//!
//! This is the one place Settings writes what the user chose about how the
//! desktop looks and behaves. It follows the house convention (TOML, every key
//! optional, a parse error never fatal) so the compositor, RoostBar and any
//! other Raven component can read it without depending on this crate.
//!
//! Not `~/.config/raven/config.toml`: RavenFileManager already owns that name
//! for its own settings, and two writers on one file would fight.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_ACCENT: &str = "#7AA2F7";

/// The seven accents offered in the Appearance page, in display order. The
/// first is the compositor's compiled-in accent.
pub const ACCENTS: [(&str, &str); 7] = [
    ("Raven", "#7AA2F7"),
    ("Sky", "#3B9EFF"),
    ("Teal", "#22C5DD"),
    ("Green", "#5FCF5F"),
    ("Amber", "#F5A623"),
    ("Rose", "#F7768E"),
    ("Violet", "#B279F7"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    #[default]
    Dark,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AnimationSpeed {
    Slow,
    #[default]
    Normal,
    Fast,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme_mode: ThemeMode,
    /// `#RRGGBB`.
    pub accent: String,
    /// Interface scale as a factor: 1.0 is 100%.
    pub scale: f64,
    pub transparency: bool,
    pub shadows: bool,
    pub blur: bool,
    pub smooth_animations: bool,
    pub animation_speed: AnimationSpeed,
    /// Absolute path of the wallpaper the user picked. Empty means the
    /// image's default (`/usr/share/wallpaper/set/wallpaper.*`).
    pub wallpaper: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            accent: DEFAULT_ACCENT.into(),
            scale: 1.0,
            transparency: true,
            shadows: true,
            blur: true,
            smooth_animations: true,
            animation_speed: AnimationSpeed::Normal,
            wallpaper: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Command the desktop should use for "open a terminal".
    pub terminal: String,
    /// Minutes of idle before the screen locks; 0 = never.
    pub lock_after_minutes: u32,
    /// 24-hour clock in the bar.
    pub clock_24h: bool,
    pub show_date: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            terminal: "raven-terminal".into(),
            lock_after_minutes: 10,
            clock_24h: true,
            show_date: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Personalization {
    /// Where the pinned-app dock sits: centre, top, bottom, left, right.
    pub dock_position: String,
    /// grid, row or column.
    pub dock_layout: String,
    /// top or bottom, for RoostBar.
    pub bar_position: String,
}

impl Default for Personalization {
    fn default() -> Self {
        Self {
            dock_position: "centre".into(),
            dock_layout: "grid".into(),
            bar_position: "top".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    /// Keep the launcher's frecency history (`$XDG_STATE_HOME/raven/frecency`).
    pub remember_app_usage: bool,
    /// Keep the bar's Bluetooth adapter discoverable to other devices.
    pub bluetooth_discoverable: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            remember_app_usage: true,
            bluetooth_discoverable: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    pub appearance: Appearance,
    pub general: General,
    pub personalization: Personalization,
    pub privacy: Privacy,
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("raven")
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("raven")
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("raven")
}

pub fn path() -> PathBuf {
    config_dir().join("desktop.toml")
}

impl DesktopConfig {
    pub fn load() -> Self {
        Self::load_from(&path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    tracing::warn!("{}: {e}; using defaults", path.display());
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = format!(
            "# Raven desktop settings. Written by raven-settings; read by the desktop.\n\n{}",
            toml::to_string_pretty(self)?
        );
        atomic_write(path, text.as_bytes())
    }
}

/// Write via a sibling temp file and rename, so a crash mid-write leaves the
/// old file intact rather than a truncated one.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut cfg = DesktopConfig::default();
        cfg.appearance.accent = "#123456".into();
        cfg.appearance.theme_mode = ThemeMode::Light;
        let text = toml::to_string(&cfg).unwrap();
        let back: DesktopConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.appearance.accent, "#123456");
        assert_eq!(back.appearance.theme_mode, ThemeMode::Light);
    }

    #[test]
    fn missing_keys_take_defaults() {
        let cfg: DesktopConfig = toml::from_str("[appearance]\naccent = \"#000000\"\n").unwrap();
        assert_eq!(cfg.appearance.accent, "#000000");
        assert_eq!(cfg.appearance.scale, 1.0);
        assert_eq!(cfg.general.terminal, "raven-terminal");
    }
}
