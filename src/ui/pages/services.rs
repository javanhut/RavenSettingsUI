//! Services: the background programs this machine runs, and a switch for
//! each.
//!
//! # What this page is for
//!
//! Every daemon on a Raven machine is defined by a `[[services]]` block that
//! raven-init reads at boot. Turning one on or off meant `raven-rc`, which
//! needs root, which meant a terminal -- so the answer to "my laptop has a
//! fingerprint reader and nothing is using it" was two commands and knowing
//! which two. The Security and Bluetooth pages each grew a button for their
//! own daemon; this page is the general case, and it is the reason those
//! buttons could be written at all: `backend::services` turns a service on
//! through rvnd, the same root daemon Raven Store installs packages through.
//!
//! # What it lists, and what it refuses to list
//!
//! Only services this machine ships a definition for *and* has the program
//! for -- `backend::services::installed`. That is a deliberately short list.
//! It is not `raven-rc list`: this is not a service manager, and a settings
//! window is the wrong place to stop `udev` or the compositor's seat daemon.
//! The names below are the ones a person has a reason to want on or off,
//! spelled the way they would think of them rather than the way init does.
//!
//! A machine that does not have one of these simply does not show that row,
//! which is why there is no "not installed" state to render: a row that is
//! here is a row whose switch works.
//!
//! # Why the switch can lag
//!
//! Turning one on goes out to rvnd and may put a password prompt in front of
//! the person, and `raven-faced` takes a few seconds to load its models
//! before it binds anything. So a switch is made insensitive while its
//! request is out and the page re-reads the machine's state afterwards,
//! rather than assuming the switch it drew is the truth. State is read from
//! the files raven-init publishes under `/run/raven-init`, which need no
//! privilege at all.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::services::{self as sv, Boot, State};
use crate::ui::{spawn, widgets, App};

/// One service worth offering, and how to say what it is.
///
/// `title` and `detail` rather than the description init publishes: init's
/// is written for whoever reads `raven-rc list` ("Face unlock", "Bluetooth
/// service") and this is written for whoever is deciding. The published
/// description is still shown when a service has no entry here -- see
/// [`describe`] -- but every service this page lists has one.
struct Known {
    service: &'static str,
    title: &'static str,
    detail: &'static str,
}

/// The services this page offers, in the order they are shown.
///
/// Ordered by how likely somebody is to be looking for it rather than
/// alphabetically: the two that send people to a terminal most often are
/// first.
const KNOWN: &[Known] = &[
    Known {
        service: "fprintd",
        title: "Fingerprint reader",
        detail: "Lets the login screen, the lock screen and sudo accept a fingerprint. \
                 Your fingerprints stay on the reader.",
    },
    Known {
        service: "faced",
        title: "Face unlock",
        detail: "Lets the login and lock screens recognise your face. Takes a few seconds \
                 to start, because it loads two small neural networks first.",
    },
    Known {
        service: "bluetoothd",
        title: "Bluetooth",
        detail: "Pairing and connecting Bluetooth devices.",
    },
    Known {
        service: "obexd",
        title: "Bluetooth file transfer",
        detail: "Sending and receiving files over Bluetooth. Needs Bluetooth above.",
    },
    Known {
        service: "cupsd",
        title: "Printing",
        detail: "The print spooler. Needed for any printer, including one on the network.",
    },
    Known {
        service: "avahi-daemon",
        title: "Find devices on the network",
        detail: "Discovers network printers and other machines that announce themselves.",
    },
    Known {
        service: "ipp-usb",
        title: "USB printers",
        detail: "Drives modern printers over USB. Starts by itself when one is plugged in.",
    },
    Known {
        service: "sshd",
        title: "Remote login",
        detail: "Lets other machines log in to this one over SSH. Off unless you want it.",
    },
    Known {
        service: "fwupd",
        title: "Firmware updates",
        detail: "Lets Updates offer firmware for this machine's hardware.",
    },
];

/// How long to leave a switch insensitive after a change, so the published
/// state has a tick to catch up before it is read back.
///
/// raven-init republishes on its main loop rather than the moment a service
/// changes, so reading immediately can see the state from before the change
/// and put the switch back where it was -- which looks exactly like a
/// refusal. One second clears it comfortably.
const SETTLE: Duration = Duration::from_millis(1000);

struct Row {
    service: &'static str,
    switch: adw::SwitchRow,
    /// Set while the switch is moved to match the machine, so its handler can
    /// tell that from somebody clicking it.
    syncing: Cell<bool>,
    /// A request is out for this row.
    busy: Cell<bool>,
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page(
        "Services",
        "Background programs this computer runs. Turning one off stops it now and leaves it off \
         at the next start-up.",
    );

    let available: Vec<&Known> = KNOWN
        .iter()
        .filter(|k| sv::installed(k.service))
        .collect();

    if available.is_empty() {
        // Every machine built from this tree has at least fprintd, so landing
        // here means something unusual -- a container, or an install with no
        // service definitions at all. Saying so beats an empty page.
        let (card, body) = widgets::card("Nothing to show", "");
        body.append(&widgets::dim_label(
            "This machine has no optional services installed. Install one from Raven Store \
             and it will appear here.",
        ));
        content.append(&card);
        return root.upcast();
    }

    let (card, body) = widgets::card(
        "Hardware and sharing",
        "Each of these runs as the system rather than as you, so turning one on may ask for \
         your password.",
    );
    let list = widgets::list();
    let mut rows: Vec<Rc<Row>> = Vec::new();

    for known in available {
        let switch = adw::SwitchRow::builder()
            .title(known.title)
            .subtitle(known.detail)
            .build();
        list.append(&switch);
        let row = Rc::new(Row {
            service: known.service,
            switch,
            syncing: Cell::new(false),
            busy: Cell::new(false),
        });
        rows.push(row);
    }
    body.append(&list);
    content.append(&card);

    let rows = Rc::new(rows);
    for row in rows.iter() {
        let (app, row2, rows2) = (app.clone(), row.clone(), rows.clone());
        row.switch.connect_active_notify(move |sw| {
            if row2.syncing.get() || row2.busy.get() {
                return;
            }
            toggle(&app, &row2, &rows2, sw.is_active());
        });
    }

    refresh(&rows);

    // The machine changes underneath this page: a service can be started from
    // a terminal, or stopped because it crashed, and a demand-started one
    // comes up when a printer is plugged in. Re-reading while the page is on
    // screen costs one small file per row.
    {
        let rows = rows.clone();
        let root2 = root.clone();
        glib::timeout_add_local(Duration::from_secs(3), move || {
            if !root2.is_mapped() {
                // Not the page being shown. Keep the timer -- the page is
                // rebuilt per visit in some navigations and kept in others,
                // and a timer that stopped on the first hidden tick would
                // never come back in the second case.
                return glib::ControlFlow::Continue;
            }
            refresh(&rows);
            glib::ControlFlow::Continue
        });
    }

    root.upcast()
}

/// Draw every switch from what raven-init publishes.
///
/// Rows with a request out are left alone: their switch is where the person
/// put it, and the machine has not caught up yet.
fn refresh(rows: &[Rc<Row>]) {
    for row in rows {
        if row.busy.get() {
            continue;
        }
        let status = sv::status(row.service);
        row.syncing.set(true);
        row.switch.set_active(status.is_on());
        row.syncing.set(false);
        row.switch.set_subtitle(&describe(row.service, &status));
    }
}

/// The subtitle for a row: what the service is for, and anything about its
/// current state worth saying out loud.
///
/// Most rows say only the first. The two that say more are the ones where a
/// switch that reads "on" would otherwise be misleading: a service that is
/// meant to run and is not running, and one that is deliberately waiting for
/// a device.
fn describe(service: &str, status: &sv::Status) -> String {
    let base = KNOWN
        .iter()
        .find(|k| k.service == service)
        .map(|k| k.detail.to_string())
        .or_else(|| status.description.clone())
        .unwrap_or_default();

    match (status.state, status.boot) {
        (_, Boot::OnDemand) => format!("{base} Starts by itself when it is needed."),
        (State::Stopped, Boot::Enabled) => {
            format!("{base} It is meant to be running and is not — turn it off and on again, \
                     or restart the computer.")
        }
        _ => base,
    }
}

/// Carry one switch out to rvnd.
fn toggle(app: &Rc<App>, row: &Rc<Row>, rows: &Rc<Vec<Rc<Row>>>, on: bool) {
    row.busy.set(true);
    row.switch.set_sensitive(false);

    let service = row.service;
    let (app, row2, rows) = (app.clone(), row.clone(), rows.clone());
    spawn(
        move || {
            if on {
                sv::enable(service)
            } else {
                sv::disable(service)
            }
        },
        move |result| {
            row2.switch.set_sensitive(true);
            match result {
                Ok(()) => {
                    let title = row2.switch.title();
                    app.toast(&format!(
                        "{title} is {}",
                        if on { "on" } else { "off" }
                    ));
                }
                Err(e) => {
                    // The switch goes back where it was, which `refresh`
                    // below does from the machine rather than from an
                    // assumption about what failed.
                    let title = row2.switch.title();
                    app.error(&format!("Could not change {title}"), &e);
                }
            }
            // Cleared before the settle so a second click is possible, but
            // the read is delayed: init republishes on its own tick.
            row2.busy.set(false);
            glib::timeout_add_local_once(SETTLE, move || refresh(&rows));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name here has to be one rvnd will accept, or the row is a switch
    /// that always fails. They are string literals, so this is the only place
    /// a typo in one can be caught.
    #[test]
    fn every_offered_service_has_a_usable_name() {
        for known in KNOWN {
            assert!(
                sv::valid_name(known.service),
                "{:?} is not a name raven-init will take",
                known.service
            );
            assert!(!known.title.is_empty());
            assert!(!known.detail.is_empty());
        }
    }

    /// Two rows for one service would be two switches disagreeing.
    #[test]
    fn no_service_is_listed_twice() {
        let mut seen: Vec<&str> = KNOWN.iter().map(|k| k.service).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "a service is listed more than once");
    }

    /// A stopped service that is meant to be running is the state this whole
    /// feature exists for, and the row has to say so rather than showing a
    /// switch that reads "on" beside a daemon that is not there.
    #[test]
    fn a_service_that_should_be_running_and_is_not_says_so() {
        let status = sv::Status {
            state: State::Stopped,
            boot: Boot::Enabled,
            description: None,
        };
        let text = describe("faced", &status);
        assert!(text.contains("meant to be running"), "{text}");

        // And a demand-started one is not a fault.
        let status = sv::Status {
            state: State::Stopped,
            boot: Boot::OnDemand,
            description: None,
        };
        let text = describe("ipp-usb", &status);
        assert!(text.contains("by itself"), "{text}");
        assert!(!text.contains("meant to be running"), "{text}");
    }

    /// A service with no entry in `KNOWN` falls back to what init published,
    /// so the function is safe to call for a row this page did not write.
    #[test]
    fn an_unknown_service_uses_what_init_said() {
        let status = sv::Status {
            state: State::Running,
            boot: Boot::Enabled,
            description: Some("Some other daemon".into()),
        };
        assert_eq!(describe("not-in-the-list", &status), "Some other daemon");
    }
}
