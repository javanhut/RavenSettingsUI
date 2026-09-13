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

/// RoostBar's own defaults for the two metrics interface scale drives. The
/// bar is drawn at these at 100%, so they are the base the factor multiplies
/// rather than whatever the file happens to say -- otherwise every save at
/// 110% would compound on the last one.
const BAR_HEIGHT: f64 = 28.0;
const BAR_FONT_SIZE: f64 = 13.5;

/// The bar's height at `scale`, in whole logical pixels. Clamped so a scale
/// that arrives from a hand-edited desktop.toml cannot ask for a bar of one
/// pixel or one that swallows the screen.
fn bar_height(scale: f64) -> u32 {
    (BAR_HEIGHT * scale).round().clamp(16.0, 96.0) as u32
}

/// The bar's font size at `scale`, to a tenth of a point -- the precision
/// config.example.toml is written in, and enough for a 10% step to move it.
fn bar_font_size(scale: f64) -> f64 {
    ((BAR_FONT_SIZE * scale).clamp(7.0, 48.0) * 10.0).round() / 10.0
}

/// Rewrite the handful of RoostBar keys that Settings owns, preserving every
/// other line (and comment) of the file.
pub fn sync_roostbar(cfg: &DesktopConfig) -> Result<()> {
    let path = roostbar_config();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let dark = !matches!(cfg.appearance.theme_mode, ThemeMode::Light);
    let (bg, fg, muted) = if dark {
        ("#D816161F", "#E8E8F0", "#ABABC2")
    } else {
        ("#D9F2F2F7", "#1C1C22", "#5E5E72")
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
        ("height", bar_height(cfg.appearance.scale).to_string()),
        (
            "font_size",
            // `{:.1}` and not `{}`: Rust prints a whole f64 as `13`, and
            // while toml does coerce that integer into RoostBar's
            // `font_size: f32`, the key is documented and read as a float.
            // A tenth always written keeps the file saying what it means.
            format!("{:.1}", bar_font_size(cfg.appearance.scale)),
        ),
    ];
    let new = rewrite_toml_keys(&text, &updates);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(&path, new.as_bytes())
}

/// Where a trailing comment begins on `line`, if it has one.
///
/// A `#` inside a double-quoted string is part of the value, not a comment:
/// every colour RoostBar takes is written `"#RRGGBB"`, and treating its `#`
/// as a comment re-appended the old value to the line on every sync.
fn comment_start(line: &str) -> Option<usize> {
    let mut quoted = false;
    let mut chars = line.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            // An escaped character inside a string is never a delimiter.
            '\\' if quoted => {
                chars.next();
            }
            '"' => quoted = !quoted,
            '#' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
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
                    let comment = comment_start(line)
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

/// Restart RoostBar, from the Personalization page's button.
///
/// Not the ordinary path any more: the bar stats its config on its slow poll
/// and rereads it when it moves, so `sync_roostbar` alone is enough. This
/// stays for a bar too old to do that, and for one whose fonts or geometry
/// left it in a state a reread cannot undo. The session launcher starts the
/// bar once, so the restart re-execs it from the same path itself.
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

/// The most a wallpaper file may weigh. Mirrors `MAX_FILE_BYTES` in
/// RavenCanvas's `raven-paint`, which refuses anything larger before it
/// decodes a byte; checking here means the refusal lands in this window
/// with a reason, rather than in the daemon's log after the file is copied.
pub const MAX_WALLPAPER_BYTES: u64 = 100 * 1024 * 1024;

/// The widest a converted movie is made. A wallpaper is scaled to the screen
/// by the daemon anyway, and every WebP frame is decoded in software for the
/// life of the session, so 4K source video is cut down rather than played at
/// full size. Sources narrower than this are left alone.
pub const MOTION_MAX_WIDTH: u32 = 1920;

/// Frames per second of a converted movie. Wallpapers do not need more, and
/// the decode cost is linear in it.
pub const MOTION_FPS: u32 = 24;

/// Still-picture extensions the desktop and the daemon both draw.
const IMAGE_EXTS: [&str; 3] = ["png", "jpg", "jpeg"];
/// Video containers ffmpeg is asked to convert. Anything it can demux would
/// do; these are the ones a "live wallpaper" download actually arrives as.
const VIDEO_EXTS: [&str; 4] = ["mp4", "webm", "mkv", "mov"];

/// What a chosen file is, decided by extension. The daemon sniffs the bytes
/// itself and refuses a mislabelled file with its own message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperKind {
    /// PNG or JPEG: `ravencanvas set image`.
    Image,
    /// An animated WebP, used as-is: `ravencanvas set motion`.
    Motion,
    /// A video, converted to an animated WebP first, then `set motion`.
    Video,
}

impl WallpaperKind {
    pub fn of(path: &Path) -> Option<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())?;
        if IMAGE_EXTS.contains(&ext.as_str()) {
            Some(Self::Image)
        } else if ext == "webp" {
            Some(Self::Motion)
        } else if VIDEO_EXTS.contains(&ext.as_str()) {
            Some(Self::Video)
        } else {
            None
        }
    }

    /// The `ravencanvas set <THING>` word for the installed file.
    pub fn canvas_mode(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Motion | Self::Video => "motion",
        }
    }
}

/// The user's installed wallpaper: what the daemon plays, and a still of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledWallpaper {
    pub kind: WallpaperKind,
    /// The file handed to `ravencanvas set`.
    pub path: PathBuf,
    /// A PNG or JPEG that looks like `path`. For an image it is `path`
    /// itself; for a movie it is the first frame, when ffmpeg could write
    /// one. This is what the thumbnail shows and what desktop.toml records,
    /// because Huginn draws that field itself when the daemon is not running
    /// and it draws stills only.
    pub still: Option<PathBuf>,
}

pub fn user_wallpaper_dir() -> PathBuf {
    crate::config::data_dir().join("wallpaper")
}

/// Whether a video can be converted here at all.
pub fn can_convert_video() -> bool {
    have("ffmpeg")
}

/// The ffmpeg invocation that turns a video into a looping animated WebP.
///
/// `quality` is libwebp's 0-100 scale; `max_width` and `fps` cap the output.
/// Audio is dropped, the loop count is set to forever, and stdin is closed so
/// a prompt about overwriting can never hang a background thread.
pub fn convert_command(
    src: &Path,
    dest: &Path,
    max_width: u32,
    fps: u32,
    quality: u32,
) -> Vec<String> {
    // scale to at most max_width wide, keeping the aspect and even dimensions
    // (libwebp wants even sizes; -2 rounds the height that way).
    let vf = format!("scale='min({max_width},iw)':-2,fps={fps}");
    [
        "ffmpeg",
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        &src.to_string_lossy(),
        "-an",
        "-vf",
        &vf,
        "-c:v",
        "libwebp",
        "-q:v",
        &quality.to_string(),
        "-loop",
        "0",
        &dest.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Write the first frame of `src` (a video or animated WebP) to `dest` as a
/// PNG. Best effort: a still is a convenience for the thumbnail and Huginn's
/// fallback, not something a live wallpaper needs to play.
fn write_still(src: &Path, dest: &Path) -> Option<PathBuf> {
    if !have("ffmpeg") {
        return None;
    }
    let vf = format!("scale='min({MOTION_MAX_WIDTH},iw)':-2");
    let args = [
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        &src.to_string_lossy(),
        "-an",
        "-vf",
        &vf,
        "-frames:v",
        "1",
        &dest.to_string_lossy(),
    ];
    match run("ffmpeg", &args) {
        Ok(_) if dest.is_file() => Some(dest.to_path_buf()),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!("no still for {}: {e:#}", src.display());
            None
        }
    }
}

/// Convert `src` to an animated WebP at `dest`, retrying smaller when the
/// result is over the daemon's file limit.
fn convert_video(src: &Path, dest: &Path) -> Result<()> {
    if !can_convert_video() {
        bail!("converting a video to a live wallpaper needs ffmpeg, which is not installed");
    }
    // Each step trades quality for size; most clips fit on the first.
    let attempts = [
        (MOTION_MAX_WIDTH, MOTION_FPS, 70),
        (MOTION_MAX_WIDTH, MOTION_FPS, 50),
        (1280, 20, 50),
    ];
    let mut last = 0;
    for (w, fps, q) in attempts {
        let cmd = convert_command(src, dest, w, fps, q);
        let args: Vec<&str> = cmd[1..].iter().map(String::as_str).collect();
        run(&cmd[0], &args).context("ffmpeg could not convert the video")?;
        last = std::fs::metadata(dest)?.len();
        if last <= MAX_WALLPAPER_BYTES {
            return Ok(());
        }
        tracing::info!(
            "{} is {} at {w}px/{fps}fps/q{q}; trying smaller",
            dest.display(),
            crate::util::human_bytes(last)
        );
    }
    let _ = std::fs::remove_file(dest);
    bail!(
        "even at 1280px and 20 fps the converted movie is {}, past the {} limit; use a shorter clip",
        crate::util::human_bytes(last),
        crate::util::human_bytes(MAX_WALLPAPER_BYTES)
    )
}

pub fn install_user_wallpaper(src: &Path) -> Result<InstalledWallpaper> {
    let Some(kind) = WallpaperKind::of(src) else {
        bail!("the desktop draws PNG and JPEG pictures, animated WebP, and videos it can convert to WebP");
    };
    let size = std::fs::metadata(src)
        .with_context(|| format!("cannot read {}", src.display()))?
        .len();
    // A video is allowed to be larger: it is converted, not copied, and the
    // limit applies to what comes out.
    if kind != WallpaperKind::Video && size > MAX_WALLPAPER_BYTES {
        bail!(
            "{} is {}, past the {} limit for a wallpaper",
            src.display(),
            crate::util::human_bytes(size),
            crate::util::human_bytes(MAX_WALLPAPER_BYTES)
        );
    }
    let dir = user_wallpaper_dir();
    std::fs::create_dir_all(&dir)?;
    // Build the new set in a staging directory so a failed conversion (which
    // can take minutes on a long clip) leaves the current wallpaper intact.
    let stage = dir.join(format!(".new.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let built = install_into(src, kind, &stage);
    let built = match built {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    };
    for old in std::fs::read_dir(&dir)?.flatten() {
        if old.path() != stage {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let mut out = InstalledWallpaper {
        kind,
        path: dir.join(built.path.file_name().unwrap()),
        still: built
            .still
            .as_ref()
            .map(|s| dir.join(s.file_name().unwrap())),
    };
    std::fs::rename(&built.path, &out.path)?;
    if let Some(s) = &built.still {
        if let Some(dest) = &out.still {
            if std::fs::rename(s, dest).is_err() {
                out.still = None;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&stage);
    Ok(out)
}

fn install_into(src: &Path, kind: WallpaperKind, dir: &Path) -> Result<InstalledWallpaper> {
    match kind {
        WallpaperKind::Image => {
            let ext = src
                .extension()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            let dest = dir.join(format!("wallpaper.{ext}"));
            std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
            Ok(InstalledWallpaper {
                kind,
                still: Some(dest.clone()),
                path: dest,
            })
        }
        WallpaperKind::Motion => {
            let dest = dir.join("wallpaper.webp");
            std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
            let still = write_still(&dest, &dir.join("still.png"));
            Ok(InstalledWallpaper {
                kind,
                path: dest,
                still,
            })
        }
        WallpaperKind::Video => {
            let dest = dir.join("wallpaper.webp");
            convert_video(src, &dest)?;
            // From the source, not the WebP: every ffmpeg demuxes an MP4,
            // and only recent ones open an animated WebP.
            let still = write_still(src, &dir.join("still.png"));
            Ok(InstalledWallpaper {
                kind,
                path: dest,
                still,
            })
        }
    }
}

/// Set the wallpaper the way Raven does it: through RavenCanvas, which draws
/// the desktop and the login screen and persists the choice per user in
/// `~/.config/raven/canvas.toml`. No root needed. Returns false when
/// `ravencanvas` is not installed, in which case the compositor's own
/// fallback (desktop.toml's `wallpaper`) is all that applies.
pub fn set_wallpaper_via_canvas(w: &InstalledWallpaper) -> Result<bool> {
    if !have("ravencanvas") {
        return Ok(false);
    }
    run(
        "ravencanvas",
        &[
            "set",
            w.kind.canvas_mode(),
            &w.path.to_string_lossy(),
            "--persist",
        ],
    )
    .map(|_| true)
    .context("ravencanvas refused the file")
}

/// The command that puts a wallpaper where the compositor reads it. The
/// directory is root's and no daemon owns it (RavenCanvas is the no-root
/// path, tried first), so the page offers to run this in the user's
/// terminal (see `backend::terminal`); it is never run from this process.
pub fn system_wallpaper_command(src: &Path) -> Vec<String> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
    let script = format!(
        "rm -f {dir}/wallpaper.* && install -m 644 {src} {dir}/wallpaper.{ext}",
        dir = SYSTEM_WALLPAPER_DIR,
        src = crate::backend::terminal::shell_quote(&src.to_string_lossy()),
    );
    vec!["sudo".into(), "sh".into(), "-c".into(), script]
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
    fn wallpaper_command_quotes_the_source() {
        let cmd = system_wallpaper_command(Path::new("/home/me/it's here.png"));
        assert_eq!(&cmd[..3], ["sudo", "sh", "-c"]);
        assert!(cmd[3].contains("'/home/me/it'\\''s here.png'"));
        assert!(cmd[3].ends_with("wallpaper.png"));
        let ok = std::process::Command::new("sh")
            .args(["-n", "-c", &cmd[3]])
            .status()
            .map(|s| s.success())
            .unwrap_or(true);
        assert!(ok, "sh -n rejected {}", cmd[3]);
    }

    #[test]
    fn wallpaper_kind_by_extension() {
        use WallpaperKind::*;
        assert_eq!(WallpaperKind::of(Path::new("a.PNG")), Some(Image));
        assert_eq!(WallpaperKind::of(Path::new("a.jpeg")), Some(Image));
        assert_eq!(WallpaperKind::of(Path::new("rain.webp")), Some(Motion));
        assert_eq!(WallpaperKind::of(Path::new("clip.mp4")), Some(Video));
        assert_eq!(WallpaperKind::of(Path::new("clip.MKV")), Some(Video));
        assert_eq!(WallpaperKind::of(Path::new("a.gif")), None);
        assert_eq!(WallpaperKind::of(Path::new("noext")), None);
        assert_eq!(Image.canvas_mode(), "image");
        assert_eq!(Motion.canvas_mode(), "motion");
        assert_eq!(Video.canvas_mode(), "motion");
    }

    #[test]
    fn convert_command_shape() {
        let cmd = convert_command(
            Path::new("/in/a b.mp4"),
            Path::new("/out/w.webp"),
            1920,
            24,
            70,
        );
        assert_eq!(cmd[0], "ffmpeg");
        // Each argument is its own argv entry, so a space in the path is safe.
        assert!(cmd.contains(&"/in/a b.mp4".to_string()));
        assert!(cmd.contains(&"-nostdin".to_string()));
        assert!(cmd.contains(&"-an".to_string()));
        assert!(cmd.contains(&"scale='min(1920,iw)':-2,fps=24".to_string()));
        let q = cmd.iter().position(|a| a == "-q:v").unwrap();
        assert_eq!(cmd[q + 1], "70");
        let l = cmd.iter().position(|a| a == "-loop").unwrap();
        assert_eq!(cmd[l + 1], "0");
        assert_eq!(cmd.last().unwrap(), "/out/w.webp");
    }

    #[test]
    fn a_hash_inside_a_quoted_value_is_not_a_comment() {
        // The bug this guards: every colour is `"#RRGGBB"`, and the old
        // comment scan took the value's own `#` for a comment, appending the
        // previous value to the line on every sync.
        let src = "background = \"#D916161F\"\naccent = \"#7AA2F7\"  # chosen\n";
        let out = rewrite_toml_keys(
            src,
            &[
                ("background", "\"#FF16161F\"".into()),
                ("accent", "\"#3B9EFF\"".into()),
            ],
        );
        assert_eq!(
            out,
            "background = \"#FF16161F\"\naccent = \"#3B9EFF\"  # chosen\n"
        );
        // And it stays put when synced again.
        let again = rewrite_toml_keys(
            &out,
            &[
                ("background", "\"#FF16161F\"".into()),
                ("accent", "\"#3B9EFF\"".into()),
            ],
        );
        assert_eq!(again, out);
        assert_eq!(comment_start("a = \"x\\\"#y\" # z"), Some(12));
        assert_eq!(comment_start("a = \"#x\""), None);
    }

    #[test]
    fn scale_drives_the_bar_metrics() {
        // 100% is RoostBar's own default, untouched.
        assert_eq!(bar_height(1.0), 28);
        assert_eq!(bar_font_size(1.0), 13.5);
        // The ends of the Appearance slider, 80% and 120%.
        assert_eq!(bar_height(0.8), 22);
        assert_eq!(bar_font_size(0.8), 10.8);
        assert_eq!(bar_height(1.2), 34);
        assert_eq!(bar_font_size(1.2), 16.2);
        // A nonsense scale from a hand-edited desktop.toml is clamped, not
        // passed through to a bar of two pixels.
        assert_eq!(bar_height(0.0), 16);
        assert_eq!(bar_height(40.0), 96);
        assert_eq!(bar_font_size(0.0), 7.0);
        assert_eq!(bar_font_size(40.0), 48.0);
    }

    #[test]
    fn syncing_twice_at_one_scale_does_not_compound() {
        // The metrics come off RoostBar's defaults, never off the file, so
        // the second save at 110% must write what the first one did.
        let once = rewrite_toml_keys(
            "height = 28
font_size = 13.5
",
            &[
                ("height", bar_height(1.1).to_string()),
                ("font_size", format!("{:.1}", bar_font_size(1.1))),
            ],
        );
        assert_eq!(once, "height = 31
font_size = 14.9
");
        let twice = rewrite_toml_keys(
            &once,
            &[
                ("height", bar_height(1.1).to_string()),
                ("font_size", format!("{:.1}", bar_font_size(1.1))),
            ],
        );
        assert_eq!(twice, once);
    }

    #[test]
    fn appends_to_empty() {
        let out = rewrite_toml_keys("", &[("accent", "\"#1\"".into())]);
        assert_eq!(out, "accent = \"#1\"\n");
    }
}
