//! `[keycast]` in `~/.config/raven/desktop.toml`, and the two appearance
//! keys the overlay follows: the accent and whether animations are smooth.
//!
//! Read with its own small schema rather than a dependency on raven-settings,
//! on the house convention: every key optional, unknown keys ignored, and a
//! file that does not parse leaves the last good settings in place.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use crate::model::{Mode, Rules};
use crate::render::Rgba;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    BottomCentre,
    BottomLeft,
    BottomRight,
    TopCentre,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub position: Position,
    /// Multiplies every measurement: 1.0 is medium.
    pub size: f32,
    pub rules: Rules,
    pub accent: Rgba,
    pub motion: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self::from(File::default())
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct Appearance {
    accent: String,
    smooth_animations: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            accent: "#7AA2F7".into(),
            smooth_animations: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct Keycast {
    enabled: bool,
    position: String,
    size: String,
    mode: String,
    show_mouse: bool,
    hide_after_ms: u32,
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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct File {
    appearance: Appearance,
    keycast: Keycast,
}

impl From<File> for Config {
    fn from(f: File) -> Self {
        let k = f.keycast;
        Config {
            enabled: k.enabled,
            position: match k.position.as_str() {
                "bottom-left" => Position::BottomLeft,
                "bottom-right" => Position::BottomRight,
                "top-centre" | "top-center" | "top" => Position::TopCentre,
                _ => Position::BottomCentre,
            },
            size: match k.size.as_str() {
                "small" => 0.8,
                "large" => 1.3,
                _ => 1.0,
            },
            rules: Rules {
                mode: if k.mode == "shortcuts" {
                    Mode::Shortcuts
                } else {
                    Mode::All
                },
                show_mouse: k.show_mouse,
                hide_after: Duration::from_millis(k.hide_after_ms.clamp(300, 30_000) as u64),
            },
            accent: Rgba::parse_hex(&f.appearance.accent)
                .unwrap_or_else(|| Rgba::parse_hex("#7AA2F7").expect("default accent")),
            motion: f.appearance.smooth_animations,
        }
    }
}

pub fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("raven")
        .join("desktop.toml")
}

/// What changes when the file is rewritten, to notice it without a watcher.
pub fn stamp() -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path()).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The settings on disk. A missing file is the defaults; a broken one is an
/// error, so the caller can keep what it had.
pub fn load() -> Result<Config, String> {
    let path = path();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(_) => Ok(Config::default()),
    }
}

fn parse(text: &str) -> Result<Config, toml::de::Error> {
    toml::from_str::<File>(text).map(Config::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_off_with_defaults() {
        let c = parse("").unwrap();
        assert!(!c.enabled);
        assert_eq!(c.position, Position::BottomCentre);
        assert_eq!(c.rules.mode, Mode::All);
        assert_eq!(c.rules.hide_after, Duration::from_secs(2));
        assert!(c.motion);
    }

    #[test]
    fn reads_its_section_and_ignores_the_rest() {
        let c = parse(
            "[general]\nterminal = \"x\"\n\
             [appearance]\naccent = \"#FF0000\"\nsmooth_animations = false\n\
             [keycast]\nenabled = true\nposition = \"top-centre\"\nsize = \"large\"\n\
             mode = \"shortcuts\"\nshow_mouse = false\nhide_after_ms = 5000\n",
        )
        .unwrap();
        assert!(c.enabled);
        assert_eq!(c.position, Position::TopCentre);
        assert_eq!(c.size, 1.3);
        assert_eq!(c.rules.mode, Mode::Shortcuts);
        assert!(!c.rules.show_mouse);
        assert_eq!(c.rules.hide_after, Duration::from_secs(5));
        assert_eq!(c.accent, Rgba::parse_hex("#FF0000").unwrap());
        assert!(!c.motion);
    }

    #[test]
    fn a_broken_file_is_an_error() {
        assert!(parse("[keycast\nenabled = ").is_err());
    }
}
