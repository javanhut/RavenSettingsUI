//! Audio through PipeWire's `wpctl`, the same backend Huginn's quick settings
//! use, so a volume set here and one set there agree.

use anyhow::Result;

use crate::util::run;

#[derive(Debug, Clone, Default)]
pub struct Device {
    pub id: u32,
    pub name: String,
    pub is_default: bool,
    /// 0.0..=1.5 as wpctl reports it.
    pub volume: f64,
    pub muted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub sinks: Vec<Device>,
    pub sources: Vec<Device>,
}

pub fn available() -> bool {
    crate::util::have("wpctl")
}

pub fn snapshot() -> Result<Snapshot> {
    let text = run("wpctl", &["status"])?;
    let mut snap = parse_status(&text);
    for d in snap.sinks.iter_mut().chain(snap.sources.iter_mut()) {
        if let Ok(v) = run("wpctl", &["get-volume", &d.id.to_string()]) {
            let (vol, muted) = parse_volume(&v);
            d.volume = vol;
            d.muted = muted;
        }
    }
    Ok(snap)
}

pub fn set_volume(id: u32, volume: f64) -> Result<()> {
    let v = volume.clamp(0.0, 1.5);
    run(
        "wpctl",
        &["set-volume", &id.to_string(), &format!("{v:.2}")],
    )
    .map(|_| ())
}

pub fn set_mute(id: u32, muted: bool) -> Result<()> {
    run(
        "wpctl",
        &["set-mute", &id.to_string(), if muted { "1" } else { "0" }],
    )
    .map(|_| ())
}

pub fn set_default(id: u32) -> Result<()> {
    run("wpctl", &["set-default", &id.to_string()]).map(|_| ())
}

/// Parse `wpctl get-volume` output: `Volume: 0.50` or `Volume: 0.50 [MUTED]`.
pub fn parse_volume(text: &str) -> (f64, bool) {
    let muted = text.contains("[MUTED]");
    let vol = text
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    (vol, muted)
}

/// Parse the tree `wpctl status` prints. Only the Audio section's Sinks and
/// Sources are wanted; a `*` marks the default.
pub fn parse_status(text: &str) -> Snapshot {
    #[derive(PartialEq)]
    enum Section {
        None,
        Sinks,
        Sources,
    }
    let mut snap = Snapshot::default();
    let mut in_audio = false;
    let mut section = Section::None;
    for raw in text.lines() {
        let line = raw.trim_start_matches(|c: char| c.is_whitespace() || "│├└─".contains(c));
        let line = line.trim();
        if raw.starts_with("Audio") {
            in_audio = true;
            continue;
        }
        if raw.starts_with("Video") || raw.starts_with("Settings") {
            in_audio = false;
            section = Section::None;
            continue;
        }
        if !in_audio {
            continue;
        }
        if line.starts_with("Sinks:") {
            section = Section::Sinks;
            continue;
        }
        if line.starts_with("Sources:") {
            section = Section::Sources;
            continue;
        }
        if line.ends_with(':') {
            section = Section::None;
            continue;
        }
        if section == Section::None || line.is_empty() {
            continue;
        }
        let (is_default, rest) = match line.strip_prefix('*') {
            Some(r) => (true, r.trim()),
            None => (false, line),
        };
        let Some((id, name)) = rest.split_once('.') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<u32>() else {
            continue;
        };
        let name = match name.rfind('[') {
            Some(i) => name[..i].trim(),
            None => name.trim(),
        };
        if name.is_empty() {
            continue;
        }
        let dev = Device {
            id,
            name: name.to_string(),
            is_default,
            ..Default::default()
        };
        match section {
            Section::Sinks => snap.sinks.push(dev),
            Section::Sources => snap.sources.push(dev),
            Section::None => {}
        }
    }
    snap
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "PipeWire 'pipewire-0' [1.6.8]
 └─ Clients:
        32. WirePlumber                         [1.6.8]

Audio
 ├─ Devices:
 │      48. TU116 High Definition Audio Controller [alsa]
 │
 ├─ Sinks:
 │  *   57. Ryzen HD Audio Controller Analog Stereo [vol: 0.50]
 │      59. HDMI Output                          [vol: 1.00]
 │
 ├─ Sources:
 │  *   58. Ryzen HD Audio Controller Analog Stereo [vol: 1.00]
 │
 ├─ Filters:
 │
 └─ Streams:

Video
 ├─ Sinks:
 │      99. Not audio
";

    #[test]
    fn parses_sinks_and_sources() {
        let s = parse_status(STATUS);
        assert_eq!(s.sinks.len(), 2);
        assert_eq!(s.sinks[0].id, 57);
        assert!(s.sinks[0].is_default);
        assert_eq!(s.sinks[0].name, "Ryzen HD Audio Controller Analog Stereo");
        assert_eq!(s.sinks[1].id, 59);
        assert!(!s.sinks[1].is_default);
        assert_eq!(s.sources.len(), 1);
        assert_eq!(s.sources[0].id, 58);
    }

    #[test]
    fn parses_volume() {
        assert_eq!(parse_volume("Volume: 0.50\n"), (0.5, false));
        assert_eq!(parse_volume("Volume: 0.73 [MUTED]\n"), (0.73, true));
    }
}
