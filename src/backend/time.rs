//! The clock and its zone, through `raven-timed`.
//!
//! The daemon owns the two privileged things involved -- stepping the clock
//! and moving `/etc/localtime` -- and listens on `/run/raven-time/ctl`, one
//! line in, text out, exactly as raven-powerd does on its socket. Queries and
//! writes alike need the caller to be root or in the `video` group, which the
//! session already is for the screen.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// raven-timed's socket. Group `video`, like raven-powerd's.
pub const TIME_SOCKET: &str = "/run/raven-time/ctl";

pub fn available() -> bool {
    std::path::Path::new(TIME_SOCKET).exists()
}

/// One request, whole reply. The daemon closes the stream after answering,
/// so reading to the end is reading the message.
fn ask(line: &str, timeout: Duration) -> Result<String> {
    let mut stream = UnixStream::connect(TIME_SOCKET)
        .with_context(|| format!("raven-timed is not reachable at {TIME_SOCKET}"))?;
    stream.set_read_timeout(Some(timeout))?;
    writeln!(stream, "{line}")?;
    stream.flush()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    if reply.starts_with("error") {
        bail!("{}", reply.trim());
    }
    Ok(reply)
}

#[derive(Debug, Clone, Default)]
pub struct LastSync {
    /// RFC 3339, UTC, as the daemon reports it.
    pub at: String,
    /// e.g. `+0.012s`.
    pub offset: String,
    pub server: String,
}

#[derive(Debug, Clone, Default)]
pub struct TimeStatus {
    pub zone: String,
    pub sync_on: bool,
    pub last: Option<LastSync>,
}

/// `status`: three lines -- `zone <z>`, `sync <on|off>`, `last <...>`.
pub fn status() -> Result<TimeStatus> {
    let status = parse_status(&ask("status", Duration::from_secs(2))?);
    if status.zone.is_empty() {
        bail!("raven-timed sent an answer with no zone in it");
    }
    Ok(status)
}

fn parse_status(reply: &str) -> TimeStatus {
    let mut status = TimeStatus::default();
    for line in reply.lines() {
        if let Some(zone) = line.strip_prefix("zone ") {
            status.zone = zone.trim().to_string();
        } else if let Some(state) = line.strip_prefix("sync ") {
            status.sync_on = state.trim() == "on";
        } else if let Some(last) = line.strip_prefix("last ") {
            let mut words = last.split_whitespace();
            if let (Some(at), Some(offset), Some(server)) =
                (words.next(), words.next(), words.next())
            {
                status.last = Some(LastSync {
                    at: at.to_string(),
                    offset: offset.to_string(),
                    server: server.to_string(),
                });
            }
        }
    }
    status
}

/// Every zone this machine's zoneinfo can name, sorted, `Area/City`.
pub fn zones() -> Result<Vec<String>> {
    let reply = ask("zones", Duration::from_secs(2))?;
    Ok(reply.lines().map(str::to_string).collect())
}

pub fn set_zone(zone: &str) -> Result<()> {
    ask(&format!("zone {zone}"), Duration::from_secs(2)).map(|_| ())
}

pub fn set_sync(on: bool) -> Result<()> {
    let verb = if on { "sync on" } else { "sync off" };
    ask(verb, Duration::from_secs(2)).map(|_| ())
}

/// `sync`: ask the servers now. The daemon may spend its per-server timeout
/// on each configured server before it has anything to say, hence the long
/// leash. Returns the daemon's own words for the toast.
pub fn sync_now() -> Result<String> {
    ask("sync", Duration::from_secs(20)).map(|reply| reply.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_reply_parses() {
        let status = parse_status(
            "zone America/New_York\nsync on\nlast 2026-09-01T12:00:00Z +1.432s 0.pool.ntp.org\n",
        );
        assert_eq!(status.zone, "America/New_York");
        assert!(status.sync_on);
        let last = status.last.unwrap();
        assert_eq!(last.at, "2026-09-01T12:00:00Z");
        assert_eq!(last.offset, "+1.432s");
        assert_eq!(last.server, "0.pool.ntp.org");
    }

    #[test]
    fn a_machine_that_never_synced_has_no_last() {
        let status = parse_status("zone UTC\nsync off\nlast never\n");
        assert_eq!(status.zone, "UTC");
        assert!(!status.sync_on);
        assert!(status.last.is_none());
    }
}
