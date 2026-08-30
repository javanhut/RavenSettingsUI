//! Package updates through `rvn`, the Raven package manager.
//!
//! Checking is a read-only dry run any user may perform (it asks the AUR
//! about foreign packages, so it can take a few seconds). Applying needs root
//! and streams `makepkg` output for AUR packages, so it is handed to a
//! terminal running `sudo rvn update` rather than hidden behind a spinner.

use anyhow::Result;

use crate::util::{have, run_all};

#[derive(Debug, Clone)]
pub struct Update {
    pub repo: String,
    pub name: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default)]
pub struct Check {
    pub updates: Vec<Update>,
    pub download: Option<String>,
}

pub fn available() -> bool {
    have("rvn")
}

pub fn check(refresh: bool) -> Result<Check> {
    let mut args = vec!["update", "--dry-run"];
    if !refresh {
        args.push("--no-refresh");
    }
    // rvn prints its report on stderr so stdout stays pipeable; read both.
    let (ok, text) = run_all("rvn", &args)?;
    if !ok {
        let last = text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("rvn failed");
        anyhow::bail!("{}", last.trim());
    }
    Ok(parse_check(&text))
}

/// Parse `rvn update --dry-run`. Lines look like
/// `├─ extra/harfbuzz 14.3.1-1 → 14.4.0-1` and `•  download 82.9 MB`.
pub fn parse_check(text: &str) -> Check {
    let mut out = Check::default();
    for raw in text.lines() {
        let line = raw
            .trim()
            .trim_start_matches(|c: char| "├└─│•▸".contains(c))
            .trim();
        if let Some(rest) = line.strip_prefix("download ") {
            out.download = Some(rest.trim().to_string());
            continue;
        }
        let arrow = if line.contains(" → ") {
            " → "
        } else {
            " -> "
        };
        if let Some((lhs, to)) = line.split_once(arrow) {
            let mut parts = lhs.split_whitespace();
            let Some(pkg) = parts.next() else { continue };
            let from = parts.next().unwrap_or("").to_string();
            let (repo, name) = pkg.split_once('/').unwrap_or(("", pkg));
            let to = to.split_whitespace().next().unwrap_or("").to_string();
            out.updates.push(Update {
                repo: repo.into(),
                name: name.into(),
                from,
                to,
            });
        }
    }
    out
}

/// The command a terminal should run to apply everything.
pub fn apply_command() -> Vec<String> {
    vec!["sudo".into(), "rvn".into(), "update".into()]
}

/// Whether Raven Store, the graphical front-end for rvn, is installed.
pub fn store_available() -> bool {
    have("raven-store")
}

/// Opens Raven Store on its Updates page.
pub fn open_store() -> Result<()> {
    std::process::Command::new("raven-store")
        .arg("--updates")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dry_run() {
        let t = "\n  ▸  updates to apply (2)\n     ├─ aur/brave-bin 1:1.93.138-1 → 1:1.94.117-1\n     └─ extra/rust 1.29.0-2 → 1:1.98.0-1 (replaces rustup)\n\n  •  download 82.9 MB   1 to rebuild from source\n\n  •  dry run — nothing was changed\n";
        let c = parse_check(t);
        assert_eq!(c.updates.len(), 2);
        assert_eq!(c.updates[0].repo, "aur");
        assert_eq!(c.updates[0].name, "brave-bin");
        assert_eq!(c.updates[1].to, "1:1.98.0-1");
        assert_eq!(
            c.download.as_deref(),
            Some("82.9 MB   1 to rebuild from source")
        );
    }
}
