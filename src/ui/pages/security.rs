//! Security: how you prove it is you. The fingerprint reader, which of your
//! fingers it knows, and where a finger may stand in for your password -- the
//! login screen, the lock screen, and sudo.
//!
//! Everything goes through `ravend` (see `backend::fingerprint`), which only
//! ever answers about the account running this window. Adding a finger and
//! switching a use on ask for your password; ravend checks it, not this page.
//!
//! Face unlock is the other half of this page and lives in
//! [`crate::ui::pages::face`]. Two modules rather than one, because the camera
//! and the reader are separate devices: they are asked about separately, they
//! fail separately, and one being busy must not grey the other out.

use std::cell::{Cell, RefCell};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::fingerprint::{self as fp, Enrolled, Finger, Outcome, Policy, Reader, Status};
use crate::backend::services as sv;
use crate::ui::{ask_text, confirm, offer_terminal, spawn, widgets, App};

/// The service raven-init runs for the fingerprint reader.
const SERVICE: &str = "fprintd";

struct Page {
    reader_row: adw::ActionRow,
    /// "Start" when the service is down.
    reader_action: gtk::Button,
    fingers: gtk::ListBox,
    add: gtk::Button,
    login: adw::SwitchRow,
    unlock: adw::SwitchRow,
    sudo: adw::SwitchRow,
    sudo_setup: adw::ActionRow,
    sudo_setup_button: gtk::Button,
    /// What ravend last said. The switches are drawn from this, and put back
    /// to it when a change is cancelled or refused.
    status: RefCell<Option<Status>>,
    /// Set while the switches are moved to match `status`, so their handlers
    /// can tell that from a click.
    syncing: Cell<bool>,
    /// A status request is out; see [`refresh`].
    refreshing: Cell<bool>,
    /// Another refresh was asked for while one was out.
    refresh_again: Cell<bool>,
    /// Seconds before the next automatic retry after a failed status, or 0
    /// after one that worked; see [`retry_later`].
    retry_delay: Cell<u64>,
    retry_pending: Cell<bool>,
}

fn fingerprint_icon() -> gtk::Image {
    let icon = gio::ThemedIcon::from_names(&[
        "auth-fingerprint-symbolic",
        "fingerprint-symbolic",
        "security-high-symbolic",
    ]);
    gtk::Image::from_gicon(&icon)
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page(
        "Security",
        "How you prove it's you: your fingerprint and your face, and where each can stand in for your password.",
    );

    // --- the reader and your fingers -----------------------------------------
    let (reader_card, reader_body) = widgets::card(
        "Fingerprint",
        "Fingerprints are stored on the reader itself. No image of your finger ever reaches this computer.",
    );
    let reader_list = widgets::list();
    let reader_row = adw::ActionRow::builder()
        .title("Fingerprint reader")
        .subtitle("Checking…")
        .build();
    reader_row.add_prefix(&fingerprint_icon());
    let reader_action = gtk::Button::with_label("Start");
    reader_action.add_css_class("flat");
    reader_action.set_valign(gtk::Align::Center);
    reader_action.set_visible(false);
    reader_row.add_suffix(&reader_action);
    let recheck = gtk::Button::from_icon_name("view-refresh-symbolic");
    recheck.add_css_class("flat");
    recheck.set_valign(gtk::Align::Center);
    recheck.set_tooltip_text(Some("Check again"));
    reader_row.add_suffix(&recheck);
    reader_list.append(&reader_row);
    reader_body.append(&reader_list);

    let fingers = widgets::list();
    fingers.set_visible(false);
    reader_body.append(&fingers);

    let add = gtk::Button::with_label("Add a Fingerprint…");
    add.add_css_class("suggested-action");
    add.set_halign(gtk::Align::Start);
    add.set_sensitive(false);
    reader_body.append(&add);
    content.append(&reader_card);

    // --- where a finger may be used ----------------------------------------
    let (use_card, use_body) = widgets::card(
        "Use your fingerprint to",
        "Your password always works as well. Screens that accept a finger keep the password field, and sudo falls back to asking for the password.",
    );
    let uses = widgets::list();
    let login = adw::SwitchRow::builder()
        .title("Log in")
        .subtitle("Touch the sensor at the login screen instead of typing your password.")
        .sensitive(false)
        .build();
    let unlock = adw::SwitchRow::builder()
        .title("Unlock the screen")
        .subtitle("Touch the sensor at the lock screen to get back in.")
        .sensitive(false)
        .build();
    let sudo = adw::SwitchRow::builder()
        .title("Approve sudo")
        .subtitle("In a terminal, touch the sensor instead of typing your password for sudo. Press Enter to use the password instead. Never offered over SSH.")
        .sensitive(false)
        .build();
    let sudo_setup = adw::ActionRow::builder()
        .title("sudo is not set up to ask for a finger yet")
        .subtitle("This needs one line in /etc/pam.d/sudo, which only root can add.")
        .visible(false)
        .build();
    sudo_setup.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
    let sudo_setup_button = gtk::Button::with_label("Set Up…");
    sudo_setup_button.set_valign(gtk::Align::Center);
    sudo_setup.add_suffix(&sudo_setup_button);
    uses.append(&login);
    uses.append(&unlock);
    uses.append(&sudo);
    uses.append(&sudo_setup);
    use_body.append(&uses);
    content.append(&use_card);

    let (about_card, about_body) = widgets::card("Why the password is asked for", "");
    about_body.append(&widgets::dim_label(
        "Anyone using your computer while it is unlocked could add their own finger. So adding a fingerprint, and turning on any of the switches above, asks for your password. Turning a switch off or removing a fingerprint never does. If the reader does not recognise a finger three times, it stops offering itself until you use your password.",
    ));
    content.append(&about_card);

    let page = Rc::new(Page {
        reader_row,
        reader_action,
        fingers,
        add,
        login,
        unlock,
        sudo,
        sudo_setup,
        sudo_setup_button,
        status: RefCell::new(None),
        syncing: Cell::new(false),
        refreshing: Cell::new(false),
        refresh_again: Cell::new(false),
        retry_delay: Cell::new(0),
        retry_pending: Cell::new(false),
    });

    {
        let (app, page2) = (app.clone(), page.clone());
        recheck.connect_clicked(move |_| refresh(&app, &page2));
    }
    {
        let (app, page2) = (app.clone(), page.clone());
        page.reader_action
            .connect_clicked(move |_| start_service(&app, &page2));
    }
    {
        let (app, page2) = (app.clone(), page.clone());
        page.add.connect_clicked(move |_| add_finger(&app, &page2));
    }
    {
        let (app, page2) = (app.clone(), page.clone());
        page.sudo_setup_button
            .connect_clicked(move |_| offer_sudo_setup(&app, &page2));
    }
    on_switch(app, &page, &page.login, |p| &mut p.login);
    on_switch(app, &page, &page.unlock, |p| &mut p.unlock);
    on_switch(app, &page, &page.sudo, |p| &mut p.sudo);

    // Face unlock, below the reader. Its own module and its own state: the
    // camera and the reader are separate devices that fail for unrelated
    // reasons, and a camera that is busy must not grey out the reader.
    content.append(&crate::ui::pages::face::section(app));

    // The login keyring, below both: not a way to log in, but the other side
    // of the question this page asks, so it belongs where somebody is already
    // looking rather than off on its own. Its own module and its own state
    // for the same reason face's is -- see keyring::section's own comment for
    // why it needs so much less of both.
    content.append(&crate::ui::pages::keyring::section(app));

    // Looked at again whenever the page is shown: a reader plugged in, or a
    // terminal command that finished, since it was last on screen.
    {
        let (app, page2) = (app.clone(), page.clone());
        root.connect_map(move |_| refresh(&app, &page2));
    }

    root.upcast()
}

/// Ask ravend how the reader is, and draw the answer.
///
/// One question at a time. raven-fprintd serves one connection at a time, so
/// a burst of them -- the page shown again and again, the button pressed while
/// waiting -- only queues behind itself, and turns a slow reader into one that
/// is reported busy. A refresh asked for meanwhile runs once this one is back.
fn refresh(app: &Rc<App>, page: &Rc<Page>) {
    if page.refreshing.replace(true) {
        page.refresh_again.set(true);
        return;
    }
    let (app, page) = (app.clone(), page.clone());
    spawn(fp::status, move |result| {
        page.refreshing.set(false);
        match result {
            Ok(status) => {
                page.retry_delay.set(0);
                apply(&app, &page, status);
            }
            Err(e) => {
                tracing::warn!("fingerprint status: {e:#}");
                page.reader_row.set_subtitle(&e.to_string());
                page.reader_action.set_visible(false);
                page.fingers.set_visible(false);
                page.add.set_sensitive(false);
                for row in [&page.login, &page.unlock, &page.sudo] {
                    row.set_sensitive(false);
                }
                retry_later(&app, &page);
            }
        }
        if page.refresh_again.replace(false) {
            refresh(&app, &page);
        }
    });
}

/// Try again after a failed status, waiting longer each time -- 5, 10, 20 and
/// 40 seconds -- and then leave it to the button. Only while the page is on
/// screen; showing it again asks anyway.
fn retry_later(app: &Rc<App>, page: &Rc<Page>) {
    if page.retry_pending.get() {
        return;
    }
    let delay = match page.retry_delay.get() {
        0 => 5,
        d if d < 40 => d * 2,
        _ => return,
    };
    page.retry_delay.set(delay);
    page.retry_pending.set(true);
    let (app, page) = (app.clone(), page.clone());
    glib::timeout_add_local_once(Duration::from_secs(delay), move || {
        page.retry_pending.set(false);
        if page.reader_row.is_mapped() {
            refresh(&app, &page);
        }
    });
}

/// Look again a few times after a terminal command was started, since there is
/// no telling when somebody finishes typing their sudo password there.
fn refresh_later(app: &Rc<App>, page: &Rc<Page>) {
    let (app, page) = (app.clone(), page.clone());
    let mut tries = 0;
    glib::timeout_add_local(Duration::from_secs(2), move || {
        tries += 1;
        refresh(&app, &page);
        if tries >= 15 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// Start the fingerprint daemon.
///
/// This button used to open a dialog offering to run `sudo raven-rc start
/// fprintd` in a terminal, because there was no daemon on this machine that
/// would start a service for an unprivileged caller and
/// `backend::terminal`'s rule is that a GUI never runs sudo itself. rvnd now
/// grants that verb -- see `backend::services` -- so the button does the
/// thing it is named after.
///
/// `enable` rather than `start`: somebody turning their fingerprint reader on
/// in a settings window means it, and a reader that worked until the next
/// reboot would be a bug report. It also covers the machine whose drop-in
/// raven-init has never read, which `start` alone cannot.
fn start_service(app: &Rc<App>, page: &Rc<Page>) {
    page.reader_action.set_sensitive(false);
    page.reader_action.set_label("Starting…");

    let (app, page) = (app.clone(), page.clone());
    spawn(
        || sv::enable(SERVICE),
        move |result| {
            page.reader_action.set_sensitive(true);
            page.reader_action.set_label("Start");
            match result {
                Ok(()) => {
                    app.toast("The fingerprint service is running");
                    refresh(&app, &page);
                }
                Err(e) => app.error("Could not start the fingerprint service", &e),
            }
        },
    );
}

/// Draw the page from what ravend said.
fn apply(app: &Rc<App>, page: &Rc<Page>, status: Status) {
    let present = matches!(status.reader, Reader::Present { .. });
    let subtitle = match &status.reader {
        Reader::NoService => {
            "The fingerprint service is not running. Start it to use the reader.".to_string()
        }
        Reader::Absent => "No fingerprint reader was found on this computer.".to_string(),
        Reader::Present { firmware, .. } if firmware.is_empty() => "Ready.".to_string(),
        Reader::Present { firmware, .. } => format!("Ready. Firmware {firmware}."),
    };
    page.reader_row.set_subtitle(&subtitle);
    // Only when there is a service to start. A machine that never installed
    // raven-fprintd has no definition for one, and the sentence above is the
    // whole of the answer there.
    page.reader_action
        .set_visible(status.reader == Reader::NoService && sv::installed(SERVICE));

    // Your fingers.
    widgets::clear(&page.fingers);
    page.fingers.set_visible(present);
    if status.enrolled.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No fingerprints yet")
            .subtitle("Add one to log in, unlock, or approve sudo with it.")
            .build();
        page.fingers.append(&row);
    }
    for &finger in &status.enrolled {
        let row = adw::ActionRow::builder().title(finger.label()).build();
        row.add_prefix(&fingerprint_icon());
        let remove = gtk::Button::with_label("Remove");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        let (app2, page2) = (app.clone(), page.clone());
        remove.connect_clicked(move |b| {
            let (app, page) = (&app2, &page2);
            let last = page
                .status
                .borrow()
                .as_ref()
                .is_some_and(|s| s.enrolled.len() == 1);
            let body = if last {
                "It is your only fingerprint, so logging in, unlocking and sudo will go back to asking for your password."
            } else {
                "The reader will forget it. You can add it again later."
            };
            let (app, page) = (app.clone(), page.clone());
            confirm(
                b,
                &format!("Remove your {}?", finger.label().to_lowercase()),
                body,
                "Remove",
                true,
                move |yes| {
                    if !yes {
                        return;
                    }
                    let (app, page) = (app.clone(), page.clone());
                    spawn(
                        move || fp::forget(Some(finger)),
                        move |result| match result {
                            Ok(status) => {
                                app.toast(&format!("{} removed", finger.label()));
                                apply(&app, &page, status);
                            }
                            Err(e) => app.error("Could not remove the fingerprint", &e),
                        },
                    );
                },
            );
        });
        row.add_suffix(&remove);
        page.fingers.append(&row);
    }

    // Some readers cannot remove one finger at a time -- the sensor stores
    // fingers and not accounts, and the only delete they honour takes the
    // whole store. On those, removing one above fails with the reader's own
    // reason and this is the way through; on the others it is the quick way to
    // start again. Shown only with more than one stored, because removing the
    // single one already clears the sensor.
    if status.enrolled.len() > 1 {
        let row = adw::ActionRow::builder()
            .title("Remove all fingerprints")
            .subtitle("Some readers can only forget every fingerprint at once.")
            .build();
        let remove_all = gtk::Button::with_label("Remove All");
        remove_all.add_css_class("flat");
        remove_all.add_css_class("destructive-action");
        remove_all.set_valign(gtk::Align::Center);
        let (app2, page2) = (app.clone(), page.clone());
        remove_all.connect_clicked(move |b| {
            let (app, page) = (app2.clone(), page2.clone());
            confirm(
                b,
                "Remove all fingerprints?",
                "This clears every fingerprint on the reader, including any belonging to \
                 other people who use this computer. Logging in, unlocking and sudo will go \
                 back to asking for your password.",
                "Remove All",
                true,
                move |yes| {
                    if !yes {
                        return;
                    }
                    let (app, page) = (app.clone(), page.clone());
                    spawn(
                        move || fp::forget(None),
                        move |result| match result {
                            Ok(status) => {
                                app.toast("All fingerprints removed");
                                apply(&app, &page, status);
                            }
                            Err(e) => app.error("Could not remove the fingerprints", &e),
                        },
                    );
                },
            );
        });
        row.add_suffix(&remove_all);
        page.fingers.append(&row);
    }

    page.add.set_sensitive(present);
    page.add.set_label(if status.enrolled.is_empty() {
        "Add a Fingerprint…"
    } else {
        "Add Another Fingerprint…"
    });

    // The switches. Usable with a reader and a finger to present; one that is
    // on stays usable regardless, so it can always be turned off.
    let usable = present && !status.enrolled.is_empty();
    page.syncing.set(true);
    for (row, on) in [
        (&page.login, status.policy.login),
        (&page.unlock, status.policy.unlock),
        (&page.sudo, status.policy.sudo),
    ] {
        row.set_active(on);
        row.set_sensitive(usable || on);
    }
    page.syncing.set(false);

    let wired = fp::sudo_wired();
    page.sudo_setup.set_visible(status.policy.sudo && !wired);
    if fp::helper_installed() {
        page.sudo_setup_button.set_visible(true);
        page.sudo_setup
            .set_subtitle("This needs one line in /etc/pam.d/sudo, which only root can add.");
    } else {
        page.sudo_setup_button.set_visible(false);
        page.sudo_setup.set_subtitle(&format!(
            "{} is not installed. It comes with RavenLogin; reinstall it to use this.",
            fp::HELPER
        ));
    }

    *page.status.borrow_mut() = Some(status);
}

/// Put the switches back to what ravend last said.
fn resync(page: &Page) {
    let Some(policy) = page.status.borrow().as_ref().map(|s| s.policy) else {
        return;
    };
    page.syncing.set(true);
    page.login.set_active(policy.login);
    page.unlock.set_active(policy.unlock);
    page.sudo.set_active(policy.sudo);
    page.syncing.set(false);
}

fn on_switch(
    app: &Rc<App>,
    page: &Rc<Page>,
    row: &adw::SwitchRow,
    field: fn(&mut Policy) -> &mut bool,
) {
    let (app, page) = (app.clone(), page.clone());
    row.connect_active_notify(move |row| {
        if page.syncing.get() {
            return;
        }
        let Some(current) = page.status.borrow().as_ref().map(|s| s.policy) else {
            return;
        };
        let mut next = current;
        *field(&mut next) = row.is_active();
        if next == current {
            return;
        }
        if !current.widened_by(next) {
            commit(&app, &page, current, next, None);
            return;
        }
        let (app2, page2) = (app.clone(), page.clone());
        ask_text(
            &app.window(),
            "Enter Your Password",
            "Turning this on lets your fingerprint stand in for your password.",
            "Password",
            true,
            "Turn On",
            move |answer| match answer {
                Some(password) if !password.is_empty() => {
                    commit(&app2, &page2, current, next, Some(password));
                }
                _ => resync(&page2),
            },
        );
    });
}

fn commit(app: &Rc<App>, page: &Rc<Page>, current: Policy, next: Policy, secret: Option<String>) {
    let (app, page) = (app.clone(), page.clone());
    spawn(
        move || fp::set_policy(next, secret),
        move |result| match result {
            Ok(Outcome::Done(status)) => {
                apply(&app, &page, status);
                if next.sudo && !current.sudo && !fp::sudo_wired() && fp::helper_installed() {
                    offer_sudo_setup(&app, &page);
                }
            }
            Ok(Outcome::Denied(message)) => {
                app.toast(&message);
                resync(&page);
            }
            Err(e) => {
                app.error("Could not change the fingerprint settings", &e);
                resync(&page);
            }
        },
    );
}

fn offer_sudo_setup(app: &Rc<App>, page: &Rc<Page>) {
    let (app2, page2) = (app.clone(), page.clone());
    offer_terminal(
        app,
        "Let sudo ask for your fingerprint?",
        "sudo reads its rules from /etc/pam.d/sudo, which only root can change. This adds one line there that asks the reader first and falls back to your password. It only applies to accounts that have turned fingerprint sudo on.",
        &fp::wire_sudo_command(),
        move |ran| {
            if ran {
                refresh_later(&app2, &page2);
            }
        },
    );
}

/// Choose a finger and give the password, then enrol it.
fn add_finger(app: &Rc<App>, page: &Rc<Page>) {
    let enrolled: Vec<Finger> = page
        .status
        .borrow()
        .as_ref()
        .map(|s| s.enrolled.clone())
        .unwrap_or_default();

    let d = adw::AlertDialog::new(
        Some("Add a Fingerprint"),
        Some("Choose a finger and enter your password. Then you will touch the sensor several times."),
    );
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let names: Vec<String> = Finger::ALL
        .iter()
        .map(|f| {
            if enrolled.contains(f) {
                // Not "(replace)": a reader that cannot remove one finger
                // cannot replace one either, and the enrolment is refused
                // rather than quietly leaving two templates under one name.
                format!("{} (already added)", f.label())
            } else {
                f.label().to_string()
            }
        })
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let choice = gtk::DropDown::from_strings(&names);
    let first_free = Finger::ALL
        .iter()
        .position(|f| !enrolled.contains(f))
        .unwrap_or(0);
    choice.set_selected(first_free as u32);
    let password = gtk::PasswordEntry::builder()
        .placeholder_text("Password")
        .show_peek_icon(true)
        .activates_default(true)
        .build();
    bx.append(&choice);
    bx.append(&password);
    d.set_extra_child(Some(&bx));
    d.add_response("cancel", "Cancel");
    d.add_response("continue", "Continue");
    d.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("continue"));
    d.set_close_response("cancel");
    {
        let (app, page) = (app.clone(), page.clone());
        let password = password.clone();
        d.connect_response(None, move |_, response| {
            if response != "continue" {
                return;
            }
            let Some(&finger) = Finger::ALL.get(choice.selected() as usize) else {
                return;
            };
            if enrolled.contains(&finger) {
                app.toast(&format!(
                    "Your {} is already added. Remove it first.",
                    finger.label().to_lowercase()
                ));
                return;
            }
            let secret = password.text().to_string();
            // Cleared now it has been read, so it does not sit in a widget
            // that outlives the dialog.
            password.set_text("");
            if secret.is_empty() {
                app.toast("Enter your password to add a fingerprint");
                return;
            }
            enrol(&app, &page, finger, secret);
        });
    }
    d.present(Some(&app.window()));
    password.grab_focus();
}

/// The enrolment itself: a dialog that follows the reader until it has the
/// finger, or until Cancel hangs up on it.
fn enrol(app: &Rc<App>, page: &Rc<Page>, finger: Finger, secret: String) {
    let d = adw::AlertDialog::new(
        Some(&format!("Adding your {}", finger.label().to_lowercase())),
        None,
    );
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let bar = gtk::ProgressBar::new();
    let message = gtk::Label::new(Some("Waiting for the reader…"));
    message.set_wrap(true);
    message.set_justify(gtk::Justification::Center);
    let icon = fingerprint_icon();
    icon.set_pixel_size(64);
    bx.append(&icon);
    bx.append(&message);
    bx.append(&bar);
    d.set_extra_child(Some(&bx));
    d.add_response("cancel", "Cancel");
    d.set_close_response("cancel");

    let cancel: Arc<Mutex<Option<UnixStream>>> = Arc::default();
    let closed = Rc::new(Cell::new(false));
    {
        let (cancel, closed) = (cancel.clone(), closed.clone());
        d.connect_response(None, move |_, _| {
            closed.set(true);
            // ravend notices the hang-up and puts the reader down; the
            // half-built template is thrown away rather than stored.
            if let Some(stream) = cancel.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        });
    }
    d.present(Some(&app.window()));

    let (tx, rx) = mpsc::channel::<(String, Option<(u8, u8)>)>();
    {
        let (bar, message) = (bar.clone(), message.clone());
        glib::timeout_add_local(Duration::from_millis(100), move || loop {
            match rx.try_recv() {
                Ok((text, progress)) => {
                    message.set_text(&text);
                    if let Some((done, of)) = progress {
                        if of > 0 {
                            bar.set_fraction(f64::from(done) / f64::from(of));
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
            }
        });
    }

    let (app, page) = (app.clone(), page.clone());
    spawn(
        move || {
            fp::enrol(finger, secret, &cancel, |text, progress| {
                let _ = tx.send((text, progress));
            })
        },
        move |result| {
            if !closed.get() {
                closed.set(true);
                d.force_close();
            }
            match result {
                Ok(Outcome::Done(Enrolled::Yes)) => {
                    app.toast(&format!("{} added", finger.label()));
                }
                Ok(Outcome::Done(Enrolled::Cancelled)) => {}
                Ok(Outcome::Denied(message)) => app.toast(&message),
                Err(e) => app.error("Could not add the fingerprint", &e),
            }
            refresh(&app, &page);
        },
    );
}
