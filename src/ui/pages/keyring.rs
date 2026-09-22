//! Login keyring: whether logging in also unlocks HuginnKeyring, the store
//! behind Secret Service, the SSH agent and the rest -- see
//! `backend::keyring` and, in HuginnKeyring, `docs/pam.md`.
//!
//! A section of the Security page rather than a page of its own, for the same
//! reason the face section is: this is one more answer to "how you prove it
//! is you," and belongs beside the others rather than off on its own where
//! somebody would have to already know to look for it.
//!
//! Far smaller than the fingerprint and face sections beside it, and
//! deliberately so. Both of those ask `ravend` a question over a socket and
//! hold a policy per finger or per face; this asks the filesystem one
//! question -- are the two lines in `/etc/pam.d/system-login` -- and there is
//! nothing to enrol, nothing to deny and nothing that needs a password to
//! confirm. `docs/pam.md` already made the module safe to add
//! unconditionally (`optional`, never an error, never blocks a login); this
//! page's whole job is to offer the one privileged step that stays.
//!
//! On Raven, most machines never need this switched on at all: `ravend`
//! hands the login password to the keyring daemon itself, natively, with no
//! PAM involved -- see RavenLogin's `session::handoff_keyring`. What this
//! wires in is the fallback for everything that is not that: a plain console
//! login, `sudo`, `sshd`, anything else that goes through the system's own
//! PAM stack instead.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::keyring as kr;
use crate::ui::{offer_terminal, widgets, App};

struct Page {
    row: adw::SwitchRow,
    not_installed: adw::ActionRow,
    /// Set while `row` is moved to match the file on disk, so its handler
    /// can tell that from a click -- the same flag the fingerprint and face
    /// sections use for the same reason.
    syncing: Cell<bool>,
}

pub fn section(app: &Rc<App>) -> gtk::Widget {
    let (card, body) = widgets::card(
        "Login Keyring",
        "HuginnKeyring holds the passwords your browser, Wi-Fi and other applications save. Unlocking it at login means nothing has to ask for that password separately.",
    );
    let list = widgets::list();

    let row = adw::SwitchRow::builder()
        .title("Unlock automatically at login")
        .subtitle("Adds two lines to /etc/pam.d/system-login. Your password is read by the login prompt that already has it and goes nowhere else.")
        .sensitive(false)
        .build();

    let not_installed = adw::ActionRow::builder()
        .title("HuginnKeyring is not installed")
        .subtitle(format!(
            "{} was not found. Install huginn-keyring to use this.",
            kr::HELPER
        ))
        .visible(false)
        .build();
    not_installed.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));

    list.append(&row);
    list.append(&not_installed);
    body.append(&list);

    let page = Rc::new(Page {
        row,
        not_installed,
        syncing: Cell::new(false),
    });

    refresh(&page);

    let (app, page2) = (app.clone(), page.clone());
    page.row.connect_active_notify(move |switch| {
        if page2.syncing.get() {
            return;
        }
        let on = switch.is_active();
        let (heading, body, cmd): (_, _, Vec<String>) = if on {
            (
                "Unlock the keyring at login?",
                "This adds two lines to /etc/pam.d/system-login that hand your login password to the keyring daemon. Nothing this does can refuse you a login -- see docs/pam.md in HuginnKeyring.",
                kr::install_command(),
            )
        } else {
            (
                "Stop unlocking the keyring at login?",
                "This removes the two lines from /etc/pam.d/system-login. What is already stored stays where it is; the first application that wants it will ask to unlock it instead.",
                kr::remove_command(),
            )
        };
        let (app2, page3) = (app.clone(), page2.clone());
        offer_terminal(&app2, heading, body, &cmd, move |ran| {
            if ran {
                refresh_later(&page3);
            } else {
                resync(&page3);
            }
        });
    });

    // Looked at again whenever the section is shown, the same as the face
    // section's own `content.connect_map` does: the file may have changed
    // from a terminal command that finished after refresh_later gave up
    // waiting, or from outside this window entirely.
    {
        let page2 = page.clone();
        card.connect_map(move |_| refresh(&page2));
    }

    card.upcast()
}

/// Draw the switch from the file on disk.
fn refresh(page: &Rc<Page>) {
    let installed = kr::helper_installed();
    page.not_installed.set_visible(!installed);
    page.row.set_sensitive(installed);
    page.syncing.set(true);
    page.row.set_active(installed && kr::wired());
    page.syncing.set(false);
}

/// Put the switch back to what the file actually says, without a round trip:
/// there is no daemon here to ask, only the file `refresh` already reads.
fn resync(page: &Rc<Page>) {
    refresh(page);
}

/// Look again a few times after a terminal command was started, since there
/// is no telling when somebody finishes typing their sudo password there --
/// the same reasoning the fingerprint section's own `refresh_later` gives.
fn refresh_later(page: &Rc<Page>) {
    let page = page.clone();
    let mut tries = 0;
    glib::timeout_add_local(Duration::from_secs(2), move || {
        tries += 1;
        refresh(&page);
        if tries >= 15 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
