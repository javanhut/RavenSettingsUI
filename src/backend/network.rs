//! Wi-Fi and wired networking through `cawd`, Raven's wireless daemon.
//!
//! The daemon speaks newline-delimited JSON on `/run/caw/caw.sock` (see the
//! `caw-ipc` crate in github.com/javanhut/CAW). The wire form is mirrored here
//! rather than depended on, so the passphrase can be supplied from a dialog
//! instead of the CLI's terminal prompt.
//!
//! Queries are open to every user; anything that changes state needs the
//! caller to be root or in the `caw` group, and the daemon says so.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const SOCKET_PATH: &str = "/run/caw/caw.sock";
pub const GROUP: &str = "caw";
pub const SERVICE: &str = "cawd";

#[derive(Debug, Clone, Serialize)]
enum Request {
    ListPorts,
    PortUp { name: String, up: bool },
    Scan { port: Option<String> },
    Connect { ssid: String, port: Option<String> },
    Disconnect { ssid: String },
    Forget { ssid: String },
    Status,
    Secret { token: u64, value: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct PortSummary {
    pub name: String,
    pub mac: String,
    pub up: bool,
    pub carrier: bool,
    pub wireless: bool,
    pub addrs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // mirrors cawd's wire form
pub struct NetworkSummary {
    pub ssid: String,
    pub bssid: String,
    pub signal_dbm: i32,
    pub freq_mhz: u32,
    pub security: String,
    pub known: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionStatus {
    pub port: String,
    pub ssid: Option<String>,
    pub state: String,
    pub addrs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
enum Response {
    Ok,
    Ports(Vec<PortSummary>),
    Networks(Vec<NetworkSummary>),
    Status(ConnectionStatus),
    Error { message: String },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum SecretKind {
    Passphrase,
    Username,
    Password,
}

#[derive(Debug, Clone, Deserialize)]
enum Event {
    Scanning,
    Associating {
        bssid: String,
    },
    Authenticating,
    Configuring,
    Connected,
    Failed {
        reason: String,
    },
    NeedSecret {
        token: u64,
        prompt: String,
        kind: SecretKind,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ServerMessage {
    Event(Event),
    Response(Response),
}

/// Progress reported while a connect is in flight.
#[derive(Debug, Clone)]
pub enum Progress {
    Scanning,
    Associating(String),
    Authenticating,
    Configuring,
    Connected,
    Failed(String),
}

impl Progress {
    pub fn label(&self) -> String {
        match self {
            Progress::Scanning => "Scanning…".into(),
            Progress::Associating(b) => format!("Associating with {b}…"),
            Progress::Authenticating => "Authenticating…".into(),
            Progress::Configuring => "Getting an address…".into(),
            Progress::Connected => "Connected".into(),
            Progress::Failed(r) => format!("Failed: {r}"),
        }
    }
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> Result<Self> {
        let stream = UnixStream::connect(SOCKET_PATH)
            .with_context(|| format!("cawd is not running (no socket at {SOCKET_PATH})"))?;
        stream.set_read_timeout(Some(Duration::from_secs(90)))?;
        let writer = stream.try_clone()?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// Whether a cawd is listening. This connects rather than checking that
    /// the path exists: cawd has no signal handler and std's listener does
    /// not unlink on drop, so a stopped cawd can leave the socket file
    /// behind, and a file that nothing accepts on is not a daemon. cawd's
    /// own "another cawd is listening" startup check and rvn's reach() make
    /// the same probe. The socket is 0666, so connecting needs no group.
    pub fn available() -> bool {
        UnixStream::connect(SOCKET_PATH).is_ok()
    }

    fn send(&mut self, req: &Request) -> Result<()> {
        let mut line = serde_json::to_vec(req)?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        self.writer.flush()?;
        Ok(())
    }

    fn next(&mut self) -> Result<ServerMessage> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                bail!("cawd closed the connection");
            }
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            return serde_json::from_str(t).with_context(|| format!("bad reply from cawd: {t}"));
        }
    }

    /// Send a request and wait for its response, ignoring events. Used for
    /// queries, which produce none.
    fn call(&mut self, req: Request) -> Result<Response> {
        self.send(&req)?;
        loop {
            match self.next()? {
                ServerMessage::Response(Response::Error { message }) => {
                    return Err(anyhow!(message))
                }
                ServerMessage::Response(r) => return Ok(r),
                ServerMessage::Event(_) => continue,
            }
        }
    }

    pub fn ports(&mut self) -> Result<Vec<PortSummary>> {
        match self.call(Request::ListPorts)? {
            Response::Ports(p) => Ok(p),
            other => bail!("unexpected reply: {other:?}"),
        }
    }

    pub fn port_up(&mut self, name: &str, up: bool) -> Result<()> {
        self.call(Request::PortUp {
            name: name.into(),
            up,
        })
        .map(|_| ())
    }

    pub fn scan(&mut self, port: Option<&str>) -> Result<Vec<NetworkSummary>> {
        match self.call(Request::Scan {
            port: port.map(str::to_owned),
        })? {
            Response::Networks(mut n) => {
                n.sort_by_key(|n| std::cmp::Reverse(n.signal_dbm));
                Ok(n)
            }
            other => bail!("unexpected reply: {other:?}"),
        }
    }

    pub fn status(&mut self) -> Result<ConnectionStatus> {
        match self.call(Request::Status)? {
            Response::Status(s) => Ok(s),
            other => bail!("unexpected reply: {other:?}"),
        }
    }

    pub fn disconnect(&mut self, ssid: &str) -> Result<()> {
        self.call(Request::Disconnect { ssid: ssid.into() })
            .map(|_| ())
    }

    /// Delete a saved network, leaving it first when it is the one joined.
    pub fn forget(&mut self, ssid: &str) -> Result<()> {
        self.call(Request::Forget { ssid: ssid.into() })
            .map(|_| ())
    }

    /// Join a network. `secret` is asked for each credential the daemon
    /// wants (a passphrase; a username and password for enterprise); return
    /// `None` to cancel. `progress` is told each step.
    pub fn connect_network(
        &mut self,
        ssid: &str,
        port: Option<&str>,
        mut secret: impl FnMut(SecretKind, &str) -> Option<String>,
        mut progress: impl FnMut(Progress),
    ) -> Result<()> {
        self.send(&Request::Connect {
            ssid: ssid.into(),
            port: port.map(str::to_owned),
        })?;
        loop {
            match self.next()? {
                ServerMessage::Event(ev) => match ev {
                    Event::Scanning => progress(Progress::Scanning),
                    Event::Associating { bssid } => progress(Progress::Associating(bssid)),
                    Event::Authenticating => progress(Progress::Authenticating),
                    Event::Configuring => progress(Progress::Configuring),
                    Event::Connected => progress(Progress::Connected),
                    Event::Failed { reason } => progress(Progress::Failed(reason)),
                    Event::NeedSecret {
                        token,
                        prompt,
                        kind,
                    } => match secret(kind, &prompt) {
                        Some(value) => self.send(&Request::Secret { token, value })?,
                        None => bail!("cancelled"),
                    },
                },
                ServerMessage::Response(Response::Ok) => return Ok(()),
                ServerMessage::Response(Response::Error { message }) => {
                    return Err(anyhow!(message))
                }
                ServerMessage::Response(other) => bail!("unexpected reply: {other:?}"),
            }
        }
    }
}

/// Whether the current user may issue state-changing requests: root, or a
/// member of the `caw` group.
pub fn can_change() -> bool {
    if is_root() {
        return true;
    }
    let out = std::process::Command::new("id").arg("-Gn").output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .any(|g| g == GROUP),
        Err(_) => false,
    }
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .map(|s| s.lines().any(|l| l.starts_with("Uid:\t0\t")))
        .unwrap_or(false)
}

/// The command that starts or stops the cawd service. Nothing unprivileged
/// can ask raven-init to start a service (init's socket is root-only, and
/// there is no daemon between the session and it for this), so the page
/// runs this in the user's terminal, where sudo asks for the password: see
/// `backend::terminal`. It is never run from this process.
pub fn daemon_command(running: bool) -> Vec<String> {
    let action = if running { "start" } else { "stop" };
    ["sudo", "raven-rc", action, SERVICE]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Wait for cawd to start accepting connections after a start, or to stop
/// accepting them after a stop. The daemon needs a moment before it
/// answers, and when the change is made in a terminal the person typing a
/// password needs longer. Returns whether cawd reached the wanted state in
/// time.
pub fn wait_until(available: bool, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if Client::available() == available {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Client::available() == available
}

/// Every access point broadcasting one network name, shown as one row.
///
/// A scan reports each radio separately, so a dual-band router or a mesh
/// shows up once per band and per node. Joining is by name and cawd picks the
/// best of them itself, so the list only needs one entry per name.
#[derive(Debug, Clone)]
pub struct NetworkGroup {
    /// Empty for the group of hidden networks, which have no name to share.
    pub ssid: String,
    /// Strongest first.
    pub access_points: Vec<NetworkSummary>,
    pub known: bool,
}

impl NetworkGroup {
    pub fn hidden(&self) -> bool {
        self.ssid.is_empty()
    }

    pub fn signal_dbm(&self) -> i32 {
        self.access_points[0].signal_dbm
    }

    /// The security modes on offer, strongest access point's first.
    pub fn security(&self) -> String {
        let mut seen: Vec<&str> = Vec::new();
        for ap in &self.access_points {
            if !ap.security.is_empty() && !seen.contains(&ap.security.as_str()) {
                seen.push(&ap.security);
            }
        }
        seen.join(" / ")
    }

    /// The bands on offer, lowest first: "2.4 / 5 GHz".
    pub fn bands(&self) -> String {
        let mut bands: Vec<&str> = Vec::new();
        for b in ["2.4 GHz", "5 GHz", "6 GHz"] {
            if self.access_points.iter().any(|ap| band(ap.freq_mhz) == b) {
                bands.push(b);
            }
        }
        match bands.as_slice() {
            [one] => one.to_string(),
            many => {
                let numbers: Vec<&str> = many.iter().map(|b| b.trim_end_matches(" GHz")).collect();
                format!("{} GHz", numbers.join(" / "))
            }
        }
    }
}

/// Fold a scan into one group per network name, strongest first, with the
/// hidden networks gathered last.
pub fn group(networks: &[NetworkSummary]) -> Vec<NetworkGroup> {
    let mut groups: Vec<NetworkGroup> = Vec::new();
    for n in networks {
        match groups.iter_mut().find(|g| g.ssid == n.ssid) {
            Some(g) => {
                g.known |= n.known;
                g.access_points.push(n.clone());
            }
            None => groups.push(NetworkGroup {
                ssid: n.ssid.clone(),
                access_points: vec![n.clone()],
                known: n.known,
            }),
        }
    }
    for g in &mut groups {
        g.access_points
            .sort_by_key(|ap| std::cmp::Reverse(ap.signal_dbm));
    }
    groups.sort_by_key(|g| (g.hidden(), std::cmp::Reverse(g.signal_dbm())));
    groups
}

/// Signal strength as 0..=4 bars.
pub fn bars(dbm: i32) -> u8 {
    match dbm {
        d if d >= -55 => 4,
        d if d >= -65 => 3,
        d if d >= -75 => 2,
        d if d >= -85 => 1,
        _ => 0,
    }
}

pub fn band(freq_mhz: u32) -> &'static str {
    match freq_mhz {
        f if f >= 5925 => "6 GHz",
        f if f >= 4900 => "5 GHz",
        _ => "2.4 GHz",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_form_matches_cawd() {
        assert_eq!(
            serde_json::to_string(&Request::Connect {
                ssid: "HomeNet".into(),
                port: None
            })
            .unwrap(),
            r#"{"Connect":{"ssid":"HomeNet","port":null}}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::ListPorts).unwrap(),
            r#""ListPorts""#
        );
    }

    #[test]
    fn parses_bare_and_wrapped_events() {
        let a: ServerMessage = serde_json::from_str(r#""Scanning""#).unwrap();
        let b: ServerMessage = serde_json::from_str(r#"{"Scanning":null}"#).unwrap();
        assert!(matches!(a, ServerMessage::Event(Event::Scanning)));
        assert!(matches!(b, ServerMessage::Event(Event::Scanning)));
        let c: ServerMessage = serde_json::from_str(
            r#"{"NeedSecret":{"token":1,"prompt":"Passphrase","kind":"Passphrase"}}"#,
        )
        .unwrap();
        assert!(matches!(
            c,
            ServerMessage::Event(Event::NeedSecret { token: 1, .. })
        ));
        let d: ServerMessage = serde_json::from_str(r#"{"Networks":[]}"#).unwrap();
        assert!(matches!(d, ServerMessage::Response(Response::Networks(_))));
        let e: ServerMessage = serde_json::from_str(r#""Ok""#).unwrap();
        assert!(matches!(e, ServerMessage::Response(Response::Ok)));
    }

    fn ap(ssid: &str, freq_mhz: u32, signal_dbm: i32, security: &str, known: bool) -> NetworkSummary {
        NetworkSummary {
            ssid: ssid.into(),
            bssid: format!("{freq_mhz}"),
            signal_dbm,
            freq_mhz,
            security: security.into(),
            known,
        }
    }

    #[test]
    fn groups_one_row_per_name() {
        let scan = [
            ap("", 2437, -40, "WPA2-Personal", false),
            ap("Home", 2437, -60, "WPA2/WPA3-Personal", false),
            ap("Cafe", 2412, -50, "Open", false),
            ap("Home", 5180, -45, "WPA2/WPA3-Personal", true),
            ap("", 2462, -43, "WPA2-Personal", false),
        ];
        let groups = group(&scan);
        let names: Vec<&str> = groups.iter().map(|g| g.ssid.as_str()).collect();
        assert_eq!(names, ["Home", "Cafe", ""]);
        let home = &groups[0];
        assert!(home.known);
        assert_eq!(home.signal_dbm(), -45);
        assert_eq!(home.bands(), "2.4 / 5 GHz");
        assert_eq!(home.security(), "WPA2/WPA3-Personal");
        assert_eq!(groups[1].bands(), "2.4 GHz");
        assert_eq!(groups[2].access_points.len(), 2);
    }

    #[test]
    fn forget_wire_form() {
        assert_eq!(
            serde_json::to_string(&Request::Forget { ssid: "HomeNet".into() }).unwrap(),
            r#"{"Forget":{"ssid":"HomeNet"}}"#
        );
    }

    #[test]
    fn bars_and_bands() {
        assert_eq!(bars(-50), 4);
        assert_eq!(bars(-90), 0);
        assert_eq!(band(5180), "5 GHz");
        assert_eq!(band(2437), "2.4 GHz");
    }
}
