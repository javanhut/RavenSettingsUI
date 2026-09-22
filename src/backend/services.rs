//! The machine's own daemons: whether one is running, and turning it on.
//!
//! # Why this module exists
//!
//! Several pages in this window can see a piece of hardware perfectly well
//! and still have nothing to offer. The face section can see the camera and
//! `/usr/bin/raven-faced`, and if `raven-faced` is not running it can only say
//! so. The fingerprint section is the same, and the Bluetooth page used to
//! print a banner with three chained `sudo` commands in it.
//!
//! None of that is a hardware problem. `raven-faced` is `enabled = true` in
//! the shipped init.toml and starts on every machine built from that tree;
//! what breaks is the machine that *installed* it later. raven-init reads
//! `/etc/raven/init.d` exactly once, at boot, so a definition that arrives
//! with a package is a service init has never heard of until something tells
//! it to reload. The fix was two commands in a terminal, and the hard part
//! was knowing it was those two.
//!
//! # The two halves, and why they are not symmetrical
//!
//! **Reading** takes no privilege at all. raven-init publishes what `raven-rc
//! list` and `raven-rc status NAME` would say to [`STATUS_DIR`], mode 0644,
//! rewritten only when the text changes. [`status`] reads one of those files.
//! There is no daemon in that path, nothing to time out, and nothing to ask
//! the user about, which is what makes it safe to call while drawing a page.
//!
//! **Changing** takes root, and this process never has it. raven-init's
//! control socket is mode 0600 and must stay that way -- PID 1 handing an
//! unprivileged session a channel into itself is the one thing its control
//! module says it must never do. So a change goes the way every other
//! privileged verb in this window goes, and the way Raven Store installs a
//! package: `rvn service`, which hands it to `rvnd` -- a root daemon that
//! checks the caller's group, asks the human through ravend if the policy
//! says to, and writes down what it did.
//!
//! This is why there is no `sudo` here and no terminal. `backend::terminal`
//! states the rule -- privilege is a verb on a daemon's socket, never sudo
//! spawned from a GUI -- and until `rvn service` existed, services were the
//! case with no daemon to grant the verb, so the terminal was the only way
//! left. Now there is one.
//!
//! # What can be named
//!
//! Whatever this machine already ships a service definition for. rvnd refuses
//! a name that has no root-owned definition under `/etc/raven/init.d` or
//! `/usr/share/raven/services`, so [`installed`] asks the same question
//! locally -- and one more, whether the program is actually on disk -- so a
//! page shows a switch only where one will work.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

/// Where raven-init publishes service status for readers without root.
pub const STATUS_DIR: &str = "/run/raven-init";

/// Where raven-init reads service definitions at boot.
pub const DROPIN_DIR: &str = "/etc/raven/init.d";

/// The inert definitions a package ships, promoted into [`DROPIN_DIR`] when
/// the binary they name is installed.
pub const TEMPLATE_DIR: &str = "/usr/share/raven/services";

/// Whether a daemon is running now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Stopped,
    /// raven-init has no service of that name. On a machine that has the
    /// definition on disk this means init has not read it yet, which is the
    /// state a reload fixes and [`enable`] fixes on the way past.
    Unknown,
}

/// Whether it starts at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boot {
    Enabled,
    Disabled,
    /// Defined, deliberately not started, and waiting for the device that
    /// starts it -- a printer being plugged in. Its own answer because it is
    /// neither of the others: somebody did enable it and it is correctly not
    /// running.
    OnDemand,
    Unknown,
}

/// What the machine says about one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    pub boot: Boot,
    /// The one-line description from the service definition, when init
    /// published one.
    pub description: Option<String>,
}

impl Status {
    /// Nothing known: no published file, which is also what a machine not
    /// running raven-init looks like.
    fn unknown() -> Status {
        Status {
            state: State::Unknown,
            boot: Boot::Unknown,
            description: None,
        }
    }

    /// Whether a switch for this service should read as on.
    ///
    /// Running *or* enabled, deliberately. A service that is enabled and
    /// stopped has usually just been asked for and is still starting, and a
    /// switch that snapped back to off in between would look like a refusal;
    /// a service that is running and not enabled was started by hand and the
    /// switch should say so rather than inviting a second start.
    pub fn is_on(&self) -> bool {
        self.state == State::Running || self.boot == Boot::Enabled
    }
}

/// A service name as raven-init and rvnd will accept it.
///
/// The same rule `rvn::initctl::valid_service_name` applies, mirrored here
/// rather than depended on -- as `backend::fingerprint` mirrors ravend's wire
/// form -- so this repository builds on its own. It is not a security check
/// in this process: rvnd applies its own to whatever arrives. It is here so
/// that a page never builds a request that is going to be refused.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && name != "."
        && name != ".."
}

/// Whether `name` is a service this machine could actually run: it has a
/// definition, and the program that definition names is on disk.
///
/// This is the question every page asks before it offers a switch, and both
/// halves are load-bearing. A definition alone is not enough:
/// `/usr/share/raven/services/bluetoothd.toml` ships on every machine,
/// including the ones with no BlueZ installed, so a page that asked only
/// whether a definition existed would offer to start a daemon that is not
/// there -- and report init's complaint about a missing binary to somebody
/// whose actual problem is a missing package.
pub fn installed(name: &str) -> bool {
    definition(name)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| exec_of(&text))
        .is_some_and(|exec| Path::new(&exec).is_file())
}

/// The program a raven-init service definition runs.
///
/// A deliberately small scan rather than a TOML parser, and the boundaries
/// are what keep it honest: it starts after the first `[[services]]` header,
/// so a comment above it that mentions `exec = "..."` -- and the shipped
/// files are heavily commented -- is never read; it stops at the next section
/// header, so a `[services.limits]` block cannot contribute one; and it skips
/// comment lines. It reads one key out of a file this process cannot write,
/// to decide whether to draw a button. A wrong answer greys a button out or
/// shows one that fails with init's own message, and neither is worth a
/// dependency.
fn exec_of(text: &str) -> Option<String> {
    let mut in_service = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // `[[services]]` opens the block; any other header closes it.
            in_service = line == "[[services]]";
            continue;
        }
        if !in_service {
            continue;
        }
        let Some(value) = line.strip_prefix("exec") else {
            continue;
        };
        let value = value.trim_start();
        let Some(value) = value.strip_prefix('=') else {
            continue;
        };
        // Quoted, always: it is a path, and raven-init's own files quote it.
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// The file that defines `name`, drop-in first because that is the one init
/// reads.
pub fn definition(name: &str) -> Option<PathBuf> {
    if !valid_name(name) {
        return None;
    }
    let dropin = Path::new(DROPIN_DIR).join(format!("{name}.toml"));
    if dropin.is_file() {
        return Some(dropin);
    }
    let template = Path::new(TEMPLATE_DIR).join(format!("{name}.toml"));
    template.is_file().then_some(template)
}

/// What raven-init last published about `name`.
///
/// Reads one small file. Never fails: a machine with no `/run/raven-init` is
/// one not running raven-init, and "unknown" is the honest answer rather than
/// an error for a page to render.
pub fn status(name: &str) -> Status {
    if !valid_name(name) {
        return Status::unknown();
    }
    let path = Path::new(STATUS_DIR).join("services").join(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_status(&text),
        Err(_) => Status::unknown(),
    }
}

/// Reads raven-init's published block for one service.
///
/// The format is the name on the first line and then `  key   value` lines,
/// which is what `raven-rc status NAME` prints -- it is meant to be read by a
/// person, and this reads it the way a person would: find the line whose
/// first word is the key, take the rest. Unrecognised keys and extra lines
/// are stepped over rather than refused, because init adds a line to that
/// block whenever it learns to report something new (`pid`, `started`,
/// memory) and a parser that insisted on the exact set would break on the
/// next one.
fn parse_status(text: &str) -> Status {
    let mut status = Status::unknown();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim();
        match key {
            "state" => {
                status.state = match value {
                    // `running`, and also the forms that carry a detail after
                    // the word -- a restarting service reports how many times.
                    v if v.starts_with("running") => State::Running,
                    _ => State::Stopped,
                }
            }
            "boot" => {
                status.boot = match value {
                    "enabled" => Boot::Enabled,
                    "disabled" => Boot::Disabled,
                    "on demand" => Boot::OnDemand,
                    _ => Boot::Unknown,
                }
            }
            "description" if !value.is_empty() => status.description = Some(value.to_string()),
            _ => {}
        }
    }
    status
}

/// The binary that brokers a change. Named once so a test and the error
/// message agree with the code.
const RVN: &str = "rvn";

/// Turn `name` on: running now, and running at every boot.
///
/// Blocking, and it can take a while -- `raven-faced` optimises two ONNX
/// graphs before it binds its socket, which is why its own readiness timeout
/// is 30 seconds. Every caller runs it through `ui::spawn`.
pub fn enable(name: &str) -> Result<()> {
    act("enable", name)
}

/// Turn it off: stopped now, and left stopped at the next boot.
pub fn disable(name: &str) -> Result<()> {
    act("disable", name)
}

fn act(action: &str, name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("refusing service name {name:?}");
    }
    if !crate::util::have(RVN) {
        bail!("rvn is not installed, so services cannot be changed from here");
    }

    // `--json` for the same reason Raven Store uses it: the event stream is
    // rvn's stable interface, and the failure arrives as a message written
    // for a person to read rather than as an exit code to guess at.
    let (ok, text) = crate::util::run_all(RVN, &["--json", "service", action, name])?;
    let message = last_failure(&text);
    if ok && message.is_none() {
        return Ok(());
    }
    match message {
        Some(message) => bail!(message),
        // A non-zero exit with nothing to say. Rare, and the name of the
        // service is the only useful thing left to put in front of somebody.
        None => bail!("could not {action} {name}"),
    }
}

/// The message from the last `failed` event in an rvn event stream, if there
/// is one.
///
/// rvn emits one JSON object per line. Parsing is deliberately shallow -- the
/// only field this window needs is the message on a failure -- and a line
/// that is not JSON at all is skipped, because rvn's stderr is merged into
/// this text and a crash is not JSON.
fn last_failure(text: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if value.get("event").and_then(|e| e.as_str()) != Some("failed") {
            continue;
        }
        if let Some(message) = value.get("message").and_then(|m| m.as_str())
            .filter(|m| !m.is_empty())
        {
            found = Some(message.to_string());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact text raven-init publishes for a service that is up, taken
    /// from /run/raven-init/services/fprintd on a running machine.
    const RUNNING: &str = "fprintd\n  \
        state        running\n  \
        boot         enabled\n  \
        description  Fingerprint reader\n  \
        pid          228\n  \
        started      0.9s after boot\n";

    /// And for one that is defined, meant to run, and is not running -- which
    /// is the state this whole module exists for.
    const STOPPED: &str = "faced\n  \
        state        stopped\n  \
        boot         enabled\n  \
        description  Face unlock\n  \
        exec         /usr/bin/raven-faced\n";

    #[test]
    fn a_published_block_reads_back() {
        let s = parse_status(RUNNING);
        assert_eq!(s.state, State::Running);
        assert_eq!(s.boot, Boot::Enabled);
        assert_eq!(s.description.as_deref(), Some("Fingerprint reader"));
        assert!(s.is_on());

        let s = parse_status(STOPPED);
        assert_eq!(s.state, State::Stopped);
        assert_eq!(s.boot, Boot::Enabled);
        // Enabled and not running: asked for, still coming up. The switch
        // stays where the person put it.
        assert!(s.is_on());
    }

    #[test]
    fn a_service_that_is_off_reads_as_off() {
        let s = parse_status("sshd\n  state        stopped\n  boot         disabled\n");
        assert_eq!(s.state, State::Stopped);
        assert_eq!(s.boot, Boot::Disabled);
        assert!(!s.is_on());
    }

    /// `on demand` is a third answer beside enabled and disabled, and it has
    /// a space in it -- the one value in this format that a `split_once` on
    /// whitespace could have cut in half.
    #[test]
    fn on_demand_survives_the_space_in_it() {
        let s = parse_status("ipp-usb\n  state        stopped\n  boot         on demand\n");
        assert_eq!(s.boot, Boot::OnDemand);
        // Not "on": nothing is wrong, and offering to start it would be
        // offering to undo the point of a demand-started service.
        assert!(!s.is_on());
    }

    /// Init adds lines to this block as it learns to report more. A parser
    /// that refused an unknown key would break on the next release.
    #[test]
    fn a_line_this_parser_does_not_know_is_stepped_over() {
        let s = parse_status(
            "faced\n  state        running\n  memory       41.2 MiB\n  \
             something-new  whatever it says\n  boot         enabled\n",
        );
        assert_eq!(s.state, State::Running);
        assert_eq!(s.boot, Boot::Enabled);
    }

    #[test]
    fn nothing_published_is_unknown_and_not_a_failure() {
        let s = parse_status("");
        assert_eq!(s.state, State::Unknown);
        assert_eq!(s.boot, Boot::Unknown);
        assert!(!s.is_on());
        // A name no machine has, so this exercises the missing-file path.
        assert_eq!(status("definitely-not-a-raven-service").state, State::Unknown);
    }

    /// The shape of a shipped definition: a long comment block that contains
    /// the very key being looked for, then the block itself, then a section
    /// that is not it.
    const DEFINITION: &str = r#"
# faced -- face unlock.
#
# Started with exec = "/not/this/one", and a line about raven-rc start faced.

[[services]]
name = "faced"
description = "Face unlock"
exec = "/usr/bin/raven-faced"
after = ["udev"]
enabled = true

[services.limits]
core = "0"
"#;

    #[test]
    fn the_program_comes_out_of_the_block_and_not_the_comments() {
        assert_eq!(exec_of(DEFINITION).as_deref(), Some("/usr/bin/raven-faced"));
        // Nothing before the first header counts.
        assert_eq!(exec_of("exec = \"/bin/early\"\n"), None);
        // And nothing after the block ends.
        assert_eq!(
            exec_of("[[services]]\nname = \"x\"\n[services.limits]\nexec = \"/bin/late\"\n"),
            None
        );
        assert_eq!(exec_of(""), None);
    }

    /// A definition for a daemon nobody installed is not something to offer a
    /// button for. bluetoothd.toml ships on machines with no BlueZ.
    #[test]
    fn a_definition_whose_program_is_absent_is_not_installed() {
        assert!(!installed("definitely-not-a-raven-service"));
        assert!(!installed("../../etc/shadow"));
    }

    #[test]
    fn a_name_that_could_leave_the_directory_is_refused() {
        assert!(valid_name("faced"));
        assert!(valid_name("avahi-daemon"));
        assert!(!valid_name("../../etc/shadow"));
        assert!(!valid_name("faced stop"));
        assert!(!valid_name(""));
        assert!(!valid_name("-rf"));
        assert!(definition("../../etc/shadow").is_none());
    }

    /// The failure a person needs is the one rvn wrote for them, not an exit
    /// code. It is also not necessarily the last line: rvn emits `exit` after
    /// it.
    #[test]
    fn the_message_comes_off_the_event_stream() {
        let stream = "\
{\"event\":\"stage\",\"message\":\"asking rvnd\"}\n\
{\"event\":\"failed\",\"message\":\"raven-init is not answering\"}\n\
{\"event\":\"exit\",\"code\":1}\n";
        assert_eq!(
            last_failure(stream).as_deref(),
            Some("raven-init is not answering")
        );

        // Success says nothing, and neither does rvn's own stderr, which is
        // merged into the same text and is not JSON.
        assert_eq!(last_failure("{\"event\":\"ok\",\"message\":\"done\"}\n"), None);
        assert_eq!(last_failure("rvn: warning: something\n"), None);
        assert_eq!(last_failure(""), None);
    }
}
