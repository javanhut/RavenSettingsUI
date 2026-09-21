//! Faces, through `ravend`.
//!
//! [`crate::backend::fingerprint`]'s shape exactly, and for the same reasons.
//! The camera belongs to root: `raven-faced` drives it on a root-only socket,
//! and the only way an unprivileged process reaches it is by asking `ravend`
//! on the lock screen's socket, which answers only about the account that owns
//! the connection. This page can add and remove *your* faces and nobody
//! else's, and there is no field in which to name anybody else.
//!
//! The wire form is mirrored here rather than depended on, as `fingerprint`
//! and `network` mirror theirs, so this repository builds on its own.
//!
//! # What is different from fingerprints
//!
//! Two things, and both are visible on the page.
//!
//! **There is no sudo switch.** `ravend` has no field for one. A face is the
//! weakest of the three proofs the machine takes -- it is presented
//! continuously, to a sensor across the room, by somebody who may be asleep --
//! and `sudo` is the one prompt where the password is doing real work. So a
//! face may open a machine that is already shut, and may not become root.
//!
//! **Enrolment here does not run the liveness check.** At the login and lock
//! screens the whole screen flashes a random sequence of colours and
//! `raven-faced` checks that the face reflects them; a settings window cannot
//! do that without taking over the display. The check that matters is the one
//! at the door, and this is not it: the password has already been given to get
//! this far, and somebody enrolling a photograph of themselves is only making
//! their own face unlock worse.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub use crate::backend::fingerprint::{Outcome, SOCKET_PATH};

/// The same cap `ravend` enforces, so a corrupted length is a closed
/// connection rather than a 4 GiB allocation.
const MAX_MESSAGE: usize = 64 * 1024;

/// Face templates one account may store. `ravend`'s number; the page greys the
/// Add button out at it rather than letting somebody find out by being refused.
pub const MAX_LOOKS: usize = 5;

/// One stored set of templates: what somebody looks like in one set of
/// circumstances.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Look {
    pub id: u8,
    pub label: String,
    /// Unix seconds.
    pub added: i64,
}

impl Look {
    /// What to put on the row, named or not.
    pub fn display_name(&self) -> String {
        if self.label.is_empty() {
            format!("Face {}", self.id)
        } else {
            self.label.clone()
        }
    }
}

/// Where a face may be used. Two switches, and see the module note for why
/// there is no third.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub login: bool,
    pub unlock: bool,
}

impl Policy {
    /// Whether moving to `next` switches anything on, which is what needs the
    /// password.
    pub fn widened_by(self, next: Self) -> bool {
        (next.login && !self.login) || (next.unlock && !self.unlock)
    }
}

/// What the machine has in the way of a camera.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Camera {
    /// `raven-faced` is not running.
    NoService,
    /// No camera it can capture from.
    Absent,
    /// A camera, and nothing to run on what it sees. Its own state, because
    /// the fix is installing a file and not buying hardware.
    NoModel { why: String },
    Present { device: String, infrared: bool },
}

impl Camera {
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present { .. })
    }

    /// One sentence for the page when face unlock cannot be offered, or `None`
    /// when it can.
    pub fn why_not(&self) -> Option<String> {
        match self {
            Self::Present { .. } => None,
            Self::NoService => {
                Some("The face unlock service is not running on this machine.".into())
            }
            Self::Absent => Some("This machine has no camera.".into()),
            Self::NoModel { .. } => Some(
                "The face recognition models are not installed. \
                 Run /usr/share/raven-face/fetch-models.sh as root, then restart \
                 the face unlock service."
                    .into(),
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub camera: Camera,
    pub looks: Vec<Look>,
    pub policy: Policy,
}

#[derive(Serialize)]
#[serde(tag = "request", rename_all = "snake_case")]
enum Request {
    FaceStatus,
    EnrolFace {
        label: String,
        flash: bool,
        secret: String,
    },
    ForgetFace {
        look: Option<u8>,
    },
    SetFacePolicy {
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
    Face {
        message: String,
        progress: Option<(u8, u8)>,
    },
    FaceStatus {
        camera: Camera,
        looks: Vec<Look>,
        policy: Policy,
    },
    FaceEnrolled {
        #[allow(dead_code)] // the page refreshes rather than reading it
        look: Look,
    },
    #[serde(other)]
    Other,
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
    // ravend asks raven-faced, which serves one caller at a time; a lock
    // screen elsewhere holding the camera makes this wait, never for long.
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    send(&mut stream, request)?;
    // A ravend from before face unlock cannot parse the request and hangs up
    // without a word, which is the one case worth naming.
    receive(&mut stream)?.context(
        "The login service does not support face unlock yet. Install the current RavenLogin.",
    )
}

fn into_status(response: Response) -> Result<Outcome<Status>> {
    match response {
        Response::FaceStatus {
            camera,
            looks,
            policy,
        } => Ok(Outcome::Done(Status {
            camera,
            looks,
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

/// Is there a camera, what is stored, and what is switched on.
pub fn status() -> Result<Status> {
    done(into_status(exchange(&Request::FaceStatus)?)?)
}

/// Remove one of your faces, or all of them with `None`. Removing the last one
/// switches every use off.
pub fn forget(look: Option<u8>) -> Result<Status> {
    done(into_status(exchange(&Request::ForgetFace { look })?)?)
}

/// Change where a face may be used. `secret` is needed when `policy` switches
/// something on.
pub fn set_policy(policy: Policy, secret: Option<String>) -> Result<Outcome<Status>> {
    into_status(exchange(&Request::SetFacePolicy { policy, secret })?)
}

/// How an enrolment ended.
#[derive(Debug)]
pub enum Enrolled {
    Yes,
    /// The window was closed first.
    Cancelled,
}

/// Store another look, calling `on` with each message and `(done, of)`
/// progress.
///
/// Blocking, for a worker thread. The connection is put in `cancel` as soon as
/// it is open; shutting it down from the main thread is how the dialog's
/// Cancel reaches `ravend`, which then puts the camera down.
///
/// `flash: false` is passed, and the module note says why.
pub fn enrol(
    label: String,
    secret: String,
    cancel: &Arc<Mutex<Option<UnixStream>>>,
    mut on: impl FnMut(String, Option<(u8, u8)>),
) -> Result<Outcome<Enrolled>> {
    let mut stream = connect()?;
    *cancel.lock().unwrap_or_else(|e| e.into_inner()) = Some(stream.try_clone()?);
    send(
        &mut stream,
        &Request::EnrolFace {
            label,
            flash: false,
            secret,
        },
    )?;
    loop {
        let response = match receive(&mut stream) {
            Ok(Some(response)) => response,
            // Closed under us: the Cancel button, most likely. If it was the
            // daemon instead, the page's refresh will show what is left.
            Ok(None) | Err(_) => return Ok(Outcome::Done(Enrolled::Cancelled)),
        };
        match response {
            Response::Face { message, progress } => on(message, progress),
            Response::FaceEnrolled { .. } => return Ok(Outcome::Done(Enrolled::Yes)),
            Response::Denied { message } => return Ok(Outcome::Denied(message)),
            Response::Failed { message } => bail!("{message}"),
            other => bail!("unexpected answer from the login service: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request names must be the ones ravend matches on.
    #[test]
    fn requests_serialize_as_ravend_names_them() {
        assert_eq!(
            serde_json::to_string(&Request::ForgetFace { look: None }).unwrap(),
            r#"{"request":"forget_face","look":null}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::FaceStatus).unwrap(),
            r#"{"request":"face_status"}"#
        );
    }

    /// A policy with a `sudo` in it is not a thing this can send, and that is
    /// a compile-time property asserted here so that adding the field to be
    /// helpful fails a test rather than quietly widening what a face can do.
    #[test]
    fn a_face_policy_has_no_sudo_field() {
        let json = serde_json::to_string(&Policy {
            login: true,
            unlock: true,
        })
        .unwrap();
        assert_eq!(json, r#"{"login":true,"unlock":true}"#);
    }

    #[test]
    fn a_status_answer_parses() {
        let json = r#"{"response":"face_status","camera":{"state":"present","device":"/dev/video0","infrared":false},"looks":[{"id":1,"label":"with glasses","added":1774000000}],"policy":{"login":true,"unlock":false}}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match into_status(response).unwrap() {
            Outcome::Done(status) => {
                assert!(status.camera.is_present());
                assert!(status.camera.why_not().is_none());
                assert_eq!(status.looks[0].display_name(), "with glasses");
                assert!(status.policy.login && !status.policy.unlock);
            }
            Outcome::Denied(_) => panic!("not denied"),
        }
    }

    /// A camera with no models has to be told apart from a machine with none,
    /// because the page sends somebody somewhere different for each.
    #[test]
    fn a_camera_without_models_says_what_to_do() {
        let json = r#"{"response":"face_status","camera":{"state":"no_model","why":"not installed"},"looks":[],"policy":{}}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        let Outcome::Done(status) = into_status(response).unwrap() else {
            panic!("not denied");
        };
        let why = status.camera.why_not().expect("there is a reason");
        assert!(why.contains("fetch-models"), "{why}");
        assert_ne!(why, Camera::Absent.why_not().unwrap());
    }

    /// An unnamed look still has something to put on its row.
    #[test]
    fn an_unnamed_look_is_still_nameable() {
        let look = Look {
            id: 3,
            label: String::new(),
            added: 0,
        };
        assert_eq!(look.display_name(), "Face 3");
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
