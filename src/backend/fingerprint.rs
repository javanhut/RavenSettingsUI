//! Fingerprints, through `ravend`.
//!
//! The reader belongs to root: `raven-fprintd` drives it on a root-only socket,
//! and the only way an unprivileged process reaches it is by asking `ravend`
//! on the lock screen's socket, `/run/raven-lock/verify.sock`. That socket
//! answers only about the account that owns the connection, which is exactly
//! the scope a settings panel should have -- this page can list, add and
//! remove *your* fingers and nobody else's, and there is no field in which to
//! name anybody else.
//!
//! The wire form is `raven-greet-proto`'s: a 4-byte big-endian length, then
//! JSON. It is mirrored here rather than depended on, as `network` mirrors
//! cawd's, so this repository builds on its own.
//!
//! Enrolling a finger and switching any use of one *on* take the password;
//! `ravend` checks it. Switching a use off and removing a finger do not.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const SOCKET_PATH: &str = "/run/raven-lock/verify.sock";

/// What `pam_exec` runs for sudo. Shipped with RavenLogin.
pub const HELPER: &str = "/usr/bin/raven-finger-auth";
const PAM_SUDO: &str = "/etc/pam.d/sudo";

/// The same cap `ravend` enforces, so a corrupted length is a closed
/// connection rather than a 4 GiB allocation.
const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Finger {
    LeftThumb,
    LeftIndex,
    LeftMiddle,
    LeftRing,
    LeftLittle,
    RightThumb,
    RightIndex,
    RightMiddle,
    RightRing,
    RightLittle,
}

impl Finger {
    /// In the order a picker lists them: the fingers people actually use
    /// first.
    pub const ALL: [Self; 10] = [
        Self::RightIndex,
        Self::LeftIndex,
        Self::RightThumb,
        Self::LeftThumb,
        Self::RightMiddle,
        Self::LeftMiddle,
        Self::RightRing,
        Self::LeftRing,
        Self::RightLittle,
        Self::LeftLittle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::LeftThumb => "Left thumb",
            Self::LeftIndex => "Left index finger",
            Self::LeftMiddle => "Left middle finger",
            Self::LeftRing => "Left ring finger",
            Self::LeftLittle => "Left little finger",
            Self::RightThumb => "Right thumb",
            Self::RightIndex => "Right index finger",
            Self::RightMiddle => "Right middle finger",
            Self::RightRing => "Right ring finger",
            Self::RightLittle => "Right little finger",
        }
    }
}

/// Where this account lets a finger stand in for its password.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub login: bool,
    pub unlock: bool,
    pub sudo: bool,
}

impl Policy {
    /// Whether moving to `next` switches anything on, which is what needs
    /// the password.
    pub fn widened_by(self, next: Self) -> bool {
        (next.login && !self.login) || (next.unlock && !self.unlock) || (next.sudo && !self.sudo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Reader {
    NoService,
    Absent,
    Present {
        stages: u8,
        stored: u8,
        firmware: String,
    },
}

#[derive(Debug, Clone)]
pub struct Status {
    pub reader: Reader,
    pub enrolled: Vec<Finger>,
    pub policy: Policy,
}

#[derive(Serialize)]
#[serde(tag = "request", rename_all = "snake_case")]
enum Request {
    FingerStatus,
    EnrolFinger {
        finger: Finger,
        secret: String,
    },
    ForgetFinger {
        finger: Option<Finger>,
    },
    SetFingerPolicy {
        policy: Policy,
        secret: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
enum Response {
    Denied {
        message: String,
    },
    Failed {
        message: String,
    },
    Finger {
        message: String,
        progress: Option<(u8, u8)>,
    },
    FingerStatus {
        reader: Reader,
        enrolled: Vec<Finger>,
        policy: Policy,
    },
    FingerEnrolled {
        #[allow(dead_code)] // the finger asked for; the page already knows it
        finger: Finger,
    },
    #[serde(other)]
    Other,
}

/// How a request that needs the password went.
#[derive(Debug)]
pub enum Outcome<T> {
    Done(T),
    /// The password was wrong, or throttled. The message is ravend's, already
    /// fit to show.
    Denied(String),
}

fn connect() -> Result<UnixStream> {
    let stream = UnixStream::connect(SOCKET_PATH).with_context(|| {
        format!("cannot reach the login service at {SOCKET_PATH}; is ravend running?")
    })?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    Ok(stream)
}

fn send(stream: &mut UnixStream, request: &Request) -> Result<()> {
    let body = serde_json::to_vec(request)?;
    let len = u32::try_from(body.len()).context("request too large")?;
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(&body);
    stream.write_all(&framed)?;
    Ok(())
}

/// The next message; `None` if the connection closed cleanly.
fn receive(stream: &mut UnixStream) -> Result<Option<Response>> {
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_MESSAGE {
        bail!("ravend sent a message of {size} bytes");
    }
    let mut body = vec![0u8; size];
    stream.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

/// One request, one answer, for the requests that answer at once.
fn exchange(request: &Request) -> Result<Response> {
    let mut stream = connect()?;
    // ravend asks raven-fprintd, which serves one caller at a time; a lock
    // screen elsewhere holding the reader makes this wait, never for long.
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    send(&mut stream, request)?;
    // A ravend from before fingerprints cannot parse the request and hangs up
    // without a word, which is the one case worth naming.
    receive(&mut stream)?.context(
        "The login service does not support fingerprints yet. Install the current RavenLogin.",
    )
}

fn into_status(response: Response) -> Result<Outcome<Status>> {
    match response {
        Response::FingerStatus {
            reader,
            enrolled,
            policy,
        } => Ok(Outcome::Done(Status {
            reader,
            enrolled,
            policy,
        })),
        Response::Denied { message } => Ok(Outcome::Denied(message)),
        Response::Failed { message } => bail!("{message}"),
        other => bail!("unexpected answer from the login service: {other:?}"),
    }
}

fn done(outcome: Outcome<Status>) -> Result<Status> {
    match outcome {
        Outcome::Done(status) => Ok(status),
        Outcome::Denied(message) => bail!("{message}"),
    }
}

/// Is there a reader, what is enrolled, and what is switched on.
pub fn status() -> Result<Status> {
    done(into_status(exchange(&Request::FingerStatus)?)?)
}

/// Remove one of your fingers, or all of them with `None`. Removing the last
/// one switches every use off.
pub fn forget(finger: Option<Finger>) -> Result<Status> {
    done(into_status(exchange(&Request::ForgetFinger { finger })?)?)
}

/// Change where a finger may be used. `secret` is needed when `policy`
/// switches something on.
pub fn set_policy(policy: Policy, secret: Option<String>) -> Result<Outcome<Status>> {
    into_status(exchange(&Request::SetFingerPolicy { policy, secret })?)
}

/// How an enrolment ended.
#[derive(Debug)]
pub enum Enrolled {
    Yes,
    /// The window was closed first.
    Cancelled,
}

/// Enrol `finger`, calling `on` with each message and `(done, of)` progress.
///
/// Blocking, for a worker thread. The connection is put in `cancel` as soon
/// as it is open; shutting it down from the main thread is how the dialog's
/// Cancel reaches `ravend`, which then puts the reader down.
pub fn enrol(
    finger: Finger,
    secret: String,
    cancel: &Arc<Mutex<Option<UnixStream>>>,
    mut on: impl FnMut(String, Option<(u8, u8)>),
) -> Result<Outcome<Enrolled>> {
    let mut stream = connect()?;
    *cancel.lock().unwrap_or_else(|e| e.into_inner()) = Some(stream.try_clone()?);
    send(&mut stream, &Request::EnrolFinger { finger, secret })?;
    loop {
        let response = match receive(&mut stream) {
            Ok(Some(response)) => response,
            // Closed under us: the Cancel button, most likely. If it was the
            // daemon instead, the page's refresh will show what is left.
            Ok(None) | Err(_) => return Ok(Outcome::Done(Enrolled::Cancelled)),
        };
        match response {
            Response::Finger { message, progress } => on(message, progress),
            Response::FingerEnrolled { .. } => return Ok(Outcome::Done(Enrolled::Yes)),
            Response::Denied { message } => return Ok(Outcome::Denied(message)),
            Response::Failed { message } => bail!("{message}"),
            other => bail!("unexpected answer from the login service: {other:?}"),
        }
    }
}

/// Whether `raven-finger-auth` is installed at all.
pub fn helper_installed() -> bool {
    Path::new(HELPER).exists()
}

/// Whether `/etc/pam.d/sudo` asks it for a finger. Without that line the sudo
/// switch is a policy nothing reads.
pub fn sudo_wired() -> bool {
    std::fs::read_to_string(PAM_SUDO).is_ok_and(|text| {
        text.lines()
            .any(|l| !l.trim_start().starts_with('#') && l.contains("raven-finger-auth"))
    })
}

/// The command that puts the line in, for the user's terminal.
pub fn wire_sudo_command() -> Vec<String> {
    vec!["sudo".into(), HELPER.into(), "--install-pam".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words must be the ones ravend and the sensor use.
    #[test]
    fn fingers_serialize_as_ravend_names_them() {
        assert_eq!(
            serde_json::to_string(&Finger::RightIndex).unwrap(),
            "\"right-index\""
        );
        assert_eq!(
            serde_json::to_string(&Request::ForgetFinger { finger: None }).unwrap(),
            r#"{"request":"forget_finger","finger":null}"#
        );
    }

    #[test]
    fn a_status_answer_parses() {
        let json = r#"{"response":"finger_status","reader":{"state":"present","stages":9,"stored":1,"firmware":"0104"},"enrolled":["right-index"],"policy":{"login":false,"unlock":true,"sudo":false}}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match into_status(response).unwrap() {
            Outcome::Done(status) => {
                assert!(matches!(status.reader, Reader::Present { stages: 9, .. }));
                assert_eq!(status.enrolled, vec![Finger::RightIndex]);
                assert!(status.policy.unlock && !status.policy.sudo);
            }
            Outcome::Denied(_) => panic!("not denied"),
        }
    }

    #[test]
    fn widening_needs_the_password_narrowing_does_not() {
        let on = Policy {
            unlock: true,
            ..Policy::default()
        };
        assert!(Policy::default().widened_by(on));
        assert!(!on.widened_by(Policy::default()));
    }
}
