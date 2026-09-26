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

/// The glass the compositor draws its panels in — the launcher, the dock,
/// quick settings, notifications, title bars — as (label, value written to
/// `appearance.glass_theme`, a word of description). Huginn's
/// `theme::Theme` reads the values; the first is its default.
pub const GLASS_THEMES: [(&str, &str, &str); 5] = [
    ("Black Glass", "black", "Smoked, near-black"),
    ("Fog Glass", "fog", "Soft blue-grey frost"),
    ("Arctic Glass", "arctic", "Pale, icy blue"),
    ("Midnight Glass", "midnight", "Deep navy"),
    ("Rose Glass", "rose", "Dusky rose"),
];

/// The launcher layouts Huginn draws, as (label, value written to
/// `appearance.launcher_layout`). The first is its default.
pub const LAUNCHER_LAYOUTS: [(&str, &str); 2] = [("Grid", "list"), ("Arc", "arc")];

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
    /// Absolute path of a still picture of the wallpaper the user picked.
    /// Empty means the image's default (`/usr/share/wallpaper/set/wallpaper.*`).
    /// For a live wallpaper this is the first frame as a PNG; the movie
    /// itself is RavenCanvas's business (`~/.config/raven/canvas.toml`), and
    /// Huginn, which reads this field when the daemon is not running, draws
    /// stills only.
    pub wallpaper: String,
    /// One of the values in [`GLASS_THEMES`].
    pub glass_theme: String,
    /// One of the values in [`LAUNCHER_LAYOUTS`].
    pub launcher_layout: String,
    /// Keys this build does not know, kept so that saving does not delete
    /// them. The compositor grows keys of its own in this section; a
    /// Settings that rewrote the file from only the fields it knows would
    /// quietly undo every one of them the next time anything was changed.
    #[serde(flatten)]
    pub other: toml::Table,
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
            glass_theme: GLASS_THEMES[0].1.into(),
            launcher_layout: LAUNCHER_LAYOUTS[0].1.into(),
            other: toml::Table::new(),
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
    /// While locked, turn the screens off: 0 = immediately, negative = never,
    /// otherwise after this many seconds without input.
    pub lock_screen_off_seconds: i64,
    /// 24-hour clock in the bar.
    pub clock_24h: bool,
    pub show_date: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            terminal: "raven-terminal".into(),
            lock_after_minutes: 10,
            lock_screen_off_seconds: 0,
            clock_24h: true,
            show_date: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Personalization {
    // The pinned application bar is deliberately absent. It lives in
    // `$XDG_STATE_HOME/raven/pins`, which the compositor writes too, and a
    // copy of the edge here would be a second answer to a question that
    // already has one — stale the moment the edge was changed from quick
    // settings. See `backend::integrations::read_pins`.
    /// top or bottom, for RoostBar.
    pub bar_position: String,
}

impl Default for Personalization {
    fn default() -> Self {
        Self {
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

/// The on-screen keystroke overlay, read by `raven-keycast`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keycast {
    pub enabled: bool,
    /// bottom-centre, bottom-left, bottom-right or top-centre.
    pub position: String,
    /// small, medium or large.
    pub size: String,
    /// `all`, or `shortcuts`: only keys pressed with Ctrl, Alt or Super, and
    /// keys that type no text.
    pub mode: String,
    pub show_mouse: bool,
    /// How long a stroke stays once its keys are released.
    pub hide_after_ms: u32,
}

impl Default for Keycast {
    fn default() -> Self {
        Self {
            enabled: false,
            position: "bottom-centre".into(),
            size: "medium".into(),
            mode: "all".into(),
            show_mouse: true,
            hide_after_ms: 2000,
        }
    }
}

/// Notification cards, drawn by the compositor, which reads this section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// Only critical notifications appear; the rest wait in quick settings.
    pub do_not_disturb: bool,
    /// Seconds a card stays when its application leaves that to the desktop.
    pub timeout_seconds: u32,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            do_not_disturb: false,
            timeout_seconds: 6,
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
    pub keycast: Keycast,
    pub notifications: Notifications,
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
    fn the_notifications_section_is_written_and_read_back() {
        let mut cfg = DesktopConfig::default();
        cfg.notifications.do_not_disturb = true;
        cfg.notifications.timeout_seconds = 10;
        let text = toml::to_string(&cfg).unwrap();
        assert!(
            text.contains("[notifications]"),
            "saved with the rest: {text}"
        );
        let back: DesktopConfig = toml::from_str(&text).unwrap();
        assert!(back.notifications.do_not_disturb);
        assert_eq!(back.notifications.timeout_seconds, 10);

        let absent: DesktopConfig = toml::from_str("[appearance]\n").unwrap();
        assert!(!absent.notifications.do_not_disturb);
        assert_eq!(absent.notifications.timeout_seconds, 6);
    }

    #[test]
    fn the_glass_theme_and_launcher_layout_are_written_and_read_back() {
        let mut cfg = DesktopConfig::default();
        cfg.appearance.glass_theme = "fog".into();
        cfg.appearance.launcher_layout = "arc".into();
        let text = toml::to_string(&cfg).unwrap();
        let back: DesktopConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.appearance.glass_theme, "fog");
        assert_eq!(back.appearance.launcher_layout, "arc");
        let absent: DesktopConfig = toml::from_str("[appearance]\n").unwrap();
        assert_eq!(absent.appearance.glass_theme, "black");
        assert_eq!(absent.appearance.launcher_layout, "list");
    }

    #[test]
    fn keys_this_build_does_not_know_survive_a_save() {
        let cfg: DesktopConfig = toml::from_str(
            "[appearance]\naccent = \"#000000\"\nsomething_new = 3\n\n[general]\nterminal = \"kitty\"\n",
        )
        .unwrap();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("something_new = 3"), "dropped: {text}");
        let back: DesktopConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.appearance.accent, "#000000");
        assert_eq!(back.general.terminal, "kitty");
    }

    #[test]
    fn missing_keys_take_defaults() {
        let cfg: DesktopConfig = toml::from_str("[appearance]\naccent = \"#000000\"\n").unwrap();
        assert_eq!(cfg.appearance.accent, "#000000");
        assert_eq!(cfg.appearance.scale, 1.0);
        assert_eq!(cfg.general.terminal, "raven-terminal");
    }
}
