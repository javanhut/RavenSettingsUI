//! Face unlock: the camera, the faces it knows, and where a face may stand in
//! for your password.
//!
//! A section of the Security page rather than a page of its own, because it is
//! the same question the fingerprint section asks and somebody deciding how
//! they log in should see both answers together. Its own module, and its own
//! state, because the camera and the reader are separate devices that fail for
//! unrelated reasons -- a camera that is busy must not grey out the reader.
//!
//! Everything goes through `ravend` (see `backend::face`), which only ever
//! answers about the account running this window. Adding a face and switching
//! a use on ask for your password; ravend checks it, not this page.
//!
//! There is no sudo switch here and there is one on the fingerprint section.
//! See `backend::face` -- it is a deliberate difference and not an omission,
//! and the card says so out loud rather than leaving somebody looking for it.

use std::cell::{Cell, RefCell};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::face::{self as fc, Camera, Enrolled, Outcome, Policy, Status, MAX_LOOKS};
use crate::ui::{confirm, spawn, widgets, App};

struct Page {
    camera_row: adw::ActionRow,
    looks: gtk::ListBox,
    add: gtk::Button,
    login: adw::SwitchRow,
    unlock: adw::SwitchRow,
    /// What ravend last said. The switches are drawn from this, and put back
    /// to it when a change is cancelled or refused.
    status: RefCell<Option<Status>>,
    /// Set while the switches are moved to match `status`, so their handlers
    /// can tell that from a click.
    syncing: Cell<bool>,
    /// A status request is out; see [`refresh`].
    refreshing: Cell<bool>,
    refresh_again: Cell<bool>,
    retry_delay: Cell<u64>,
    retry_pending: Cell<bool>,
}

fn face_icon() -> gtk::Image {
    let icon = gio::ThemedIcon::from_names(&[
        "face-smile-symbolic",
        "camera-web-symbolic",
        "avatar-default-symbolic",
    ]);
    gtk::Image::from_gicon(&icon)
}

/// The Face Unlock section, for the Security page to append.
pub fn section(app: &Rc<App>) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);

    // --- the camera and your faces -----------------------------------------
    let (camera_card, camera_body) = widgets::card(
        "Face",
        "Only a numeric description of your face is stored, never a picture, and it never leaves this computer.",
    );
    let camera_list = widgets::list();
    let camera_row = adw::ActionRow::builder()
        .title("Camera")
        .subtitle("Checking…")
        .build();
    camera_row.add_prefix(&face_icon());
    let recheck = gtk::Button::from_icon_name("view-refresh-symbolic");
    recheck.add_css_class("flat");
    recheck.set_valign(gtk::Align::Center);
    recheck.set_tooltip_text(Some("Check again"));
    camera_row.add_suffix(&recheck);
    camera_list.append(&camera_row);
    camera_body.append(&camera_list);

    let looks = widgets::list();
    looks.set_visible(false);
    camera_body.append(&looks);

    let add = gtk::Button::with_label("Add a Face…");
    add.add_css_class("suggested-action");
    add.set_halign(gtk::Align::Start);
    add.set_sensitive(false);
    camera_body.append(&add);
    content.append(&camera_card);

    // --- where a face may be used ------------------------------------------
    let (use_card, use_body) = widgets::card(
        "Use your face to",
        "Your password always works as well: both screens keep the password field while the camera is looking. \
         Your face cannot approve sudo — it is the weakest of the three, and sudo is where the password is doing real work.",
    );
    let uses = widgets::list();
    let login = adw::SwitchRow::builder()
        .title("Log in")
        .subtitle("Look at the camera at the login screen instead of typing your password.")
        .sensitive(false)
        .build();
    let unlock = adw::SwitchRow::builder()
        .title("Unlock the screen")
        .subtitle("Look at the camera at the lock screen to get back in.")
        .sensitive(false)
        .build();
    uses.append(&login);
    uses.append(&unlock);
    use_body.append(&uses);

    let note = adw::ActionRow::builder()
        .title("The screen flashes while it checks")
        .subtitle(
            "A camera cannot tell you from a photograph of you, so the screen throws a short \
             sequence of colours and checks that your face reflects them. It takes about a second.",
        )
        .build();
    note.add_prefix(&gtk::Image::from_icon_name("dialog-information-symbolic"));
    let notes = widgets::list();
    notes.append(&note);
    use_body.append(&notes);
    content.append(&use_card);

    let page = Rc::new(Page {
        camera_row,
        looks,
        add,
        login,
        unlock,
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
        page.add.connect_clicked(move |_| add_face(&app, &page2));
    }
    on_switch(app, &page, &page.login, |p| &mut p.login);
    on_switch(app, &page, &page.unlock, |p| &mut p.unlock);

    {
        let (app, page2) = (app.clone(), page.clone());
        content.connect_map(move |_| refresh(&app, &page2));
    }

    content.upcast()
}

/// Ask ravend how the camera is, and draw the answer.
///
/// One question at a time, as the fingerprint section does and for the same
/// reason: raven-faced serves one connection at a time, so a burst only queues
/// behind itself and turns a busy camera into one reported busy.
fn refresh(app: &Rc<App>, page: &Rc<Page>) {
    if page.refreshing.replace(true) {
        page.refresh_again.set(true);
        return;
    }
    let (app, page) = (app.clone(), page.clone());
    spawn(fc::status, move |result| {
        page.refreshing.set(false);
        match result {
            Ok(status) => {
                page.retry_delay.set(0);
                apply(&app, &page, status);
            }
            Err(e) => {
                tracing::warn!("face status: {e:#}");
                page.camera_row.set_subtitle(&e.to_string());
                page.looks.set_visible(false);
                page.add.set_sensitive(false);
                for row in [&page.login, &page.unlock] {
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

/// Try again after a failed status, waiting longer each time, and then leave
/// it to the button.
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
        if page.camera_row.is_mapped() {
            refresh(&app, &page);
        }
    });
}

/// Draw the section from what ravend said.
fn apply(app: &Rc<App>, page: &Rc<Page>, status: Status) {
    let present = status.camera.is_present();
    let subtitle = match &status.camera {
        Camera::Present { device, infrared } if *infrared => {
            format!("Ready. Infrared camera at {device}.")
        }
        Camera::Present { device, .. } => format!("Ready. {device}."),
        // Each of the three is a different thing to go and do, so each says
        // its own sentence rather than collapsing into "unavailable".
        other => other
            .why_not()
            .unwrap_or_else(|| "Not available.".to_string()),
    };
    page.camera_row.set_subtitle(&subtitle);

    // Your faces.
    widgets::clear(&page.looks);
    page.looks.set_visible(present);
    if status.looks.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No faces yet")
            .subtitle("Add one to log in or unlock the screen by looking at the camera.")
            .build();
        page.looks.append(&row);
    }
    for look in &status.looks {
        let row = adw::ActionRow::builder()
            .title(&look.display_name())
            .subtitle(&added_on(look.added))
            .build();
        row.add_prefix(&face_icon());
        let remove = gtk::Button::with_label("Remove");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        let (app2, page2) = (app.clone(), page.clone());
        let (id, name) = (look.id, look.display_name());
        remove.connect_clicked(move |b| {
            let (app, page) = (app2.clone(), page2.clone());
            let last = page
                .status
                .borrow()
                .as_ref()
                .is_some_and(|s| s.looks.len() == 1);
            let body = if last {
                "It is your only face, so logging in and unlocking will go back to asking for your password."
            } else {
                "This computer will forget it. You can add it again later."
            };
            let name = name.clone();
            confirm(
                b,
                &format!("Remove “{name}”?"),
                body,
                "Remove",
                true,
                move |yes| {
                    if !yes {
                        return;
                    }
                    let (app, page, name) = (app.clone(), page.clone(), name.clone());
                    spawn(
                        move || fc::forget(Some(id)),
                        move |result| match result {
                            Ok(status) => {
                                app.toast(&format!("{name} removed"));
                                apply(&app, &page, status);
                            }
                            Err(e) => app.error("Could not remove the face", &e),
                        },
                    );
                },
            );
        });
        row.add_suffix(&remove);
        page.looks.append(&row);
    }

    // Room for another? Several is the point -- glasses on, glasses off, a
    // room lit from one side -- so the button says which it is.
    let room = status.looks.len() < MAX_LOOKS;
    page.add.set_sensitive(present && room);
    page.add.set_label(if status.looks.is_empty() {
        "Add a Face…"
    } else {
        "Add Another Look…"
    });
    page.add.set_tooltip_text(if room {
        None
    } else {
        Some("Remove one first: this is as many faces as an account may store.")
    });

    // The switches, only once there is something to switch on.
    let usable = present && !status.looks.is_empty();
    page.syncing.set(true);
    page.login.set_active(status.policy.login);
    page.unlock.set_active(status.policy.unlock);
    page.syncing.set(false);
    for row in [&page.login, &page.unlock] {
        row.set_sensitive(usable);
    }

    *page.status.borrow_mut() = Some(status);
}

/// "Added 3 March 2026", or nothing for a template with no date.
///
/// Through glib rather than a date crate: this window already has one, and it
/// is the one that knows what the machine's locale calls a month.
fn added_on(added: i64) -> String {
    if added <= 0 {
        return String::new();
    }
    glib::DateTime::from_unix_local(added)
        .and_then(|d| d.format("%-e %B %Y"))
        .map(|when| format!("Added {}", when.trim()))
        .unwrap_or_default()
}

/// Put the switches back to what ravend last said, after a change that was
/// cancelled or refused.
fn resync(page: &Page) {
    let Some(status) = page.status.borrow().as_ref().map(|s| s.policy) else {
        return;
    };
    page.syncing.set(true);
    page.login.set_active(status.login);
    page.unlock.set_active(status.unlock);
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
        // Switching a use off never needs the password, so it happens at once:
        // somebody who no longer trusts this should not have to prove who they
        // are to turn it off.
        if !current.widened_by(next) {
            commit(&app, &page, current, next, None);
            return;
        }
        let d = adw::AlertDialog::new(
            Some("Enter your password"),
            Some("Turning face unlock on needs your password, because your face can be shown to the camera by anybody standing in front of it."),
        );
        let password = gtk::PasswordEntry::builder()
            .placeholder_text("Password")
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        d.set_extra_child(Some(&password));
        d.add_response("cancel", "Cancel");
        d.add_response("confirm", "Turn On");
        d.set_response_appearance("confirm", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("confirm"));
        d.set_close_response("cancel");
        let (app2, page2) = (app.clone(), page.clone());
        let entry = password.clone();
        d.connect_response(None, move |_, response| {
            let password = &entry;
            if response != "confirm" {
                resync(&page2);
                return;
            }
            let secret = password.text().to_string();
            password.set_text("");
            if secret.is_empty() {
                app2.toast("Enter your password to turn face unlock on");
                resync(&page2);
                return;
            }
            commit(&app2, &page2, current, next, Some(secret));
        });
        d.present(Some(&app.window()));
        password.grab_focus();
    });
}

fn commit(app: &Rc<App>, page: &Rc<Page>, current: Policy, next: Policy, secret: Option<String>) {
    let (app, page) = (app.clone(), page.clone());
    spawn(
        move || fc::set_policy(next, secret),
        move |result| match result {
            Ok(Outcome::Done(status)) => apply(&app, &page, status),
            Ok(Outcome::Denied(message)) => {
                app.toast(&message);
                resync(&page);
            }
            Err(e) => {
                app.error("Could not change the face unlock settings", &e);
                let _ = current;
                resync(&page);
            }
        },
    );
}

fn add_face(app: &Rc<App>, page: &Rc<Page>) {
    let d = adw::AlertDialog::new(
        Some("Add a Face"),
        Some(
            "Name this look and enter your password. Then look at the camera while it takes a few \
             pictures — it keeps the numbers, not the pictures.",
        ),
    );
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let label = gtk::Entry::builder()
        .placeholder_text("With glasses (optional)")
        .activates_default(true)
        .build();
    let password = gtk::PasswordEntry::builder()
        .placeholder_text("Password")
        .show_peek_icon(true)
        .activates_default(true)
        .build();
    bx.append(&label);
    bx.append(&password);
    d.set_extra_child(Some(&bx));
    d.add_response("cancel", "Cancel");
    d.add_response("continue", "Continue");
    d.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("continue"));
    d.set_close_response("cancel");
    {
        let (app, page) = (app.clone(), page.clone());
        let (label, password) = (label.clone(), password.clone());
        d.connect_response(None, move |_, response| {
            if response != "continue" {
                return;
            }
            let name = label.text().to_string();
            let secret = password.text().to_string();
            // Cleared now it has been read, so it does not sit in a widget
            // that outlives the dialog.
            password.set_text("");
            if secret.is_empty() {
                app.toast("Enter your password to add a face");
                return;
            }
            enrol(&app, &page, name, secret);
        });
    }
    d.present(Some(&app.window()));
    password.grab_focus();
}

/// The enrolment itself: a dialog that follows the camera until it has enough
/// good looks, or until Cancel hangs up on it.
fn enrol(app: &Rc<App>, page: &Rc<Page>, name: String, secret: String) {
    let title = if name.is_empty() {
        "Adding your face".to_string()
    } else {
        format!("Adding “{name}”")
    };
    let d = adw::AlertDialog::new(Some(&title), None);
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let bar = gtk::ProgressBar::new();
    let message = gtk::Label::new(Some("Waiting for the camera…"));
    message.set_wrap(true);
    message.set_justify(gtk::Justification::Center);
    let icon = face_icon();
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
            // ravend notices the hang-up and puts the camera down; nothing
            // half-captured is stored.
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
            fc::enrol(name, secret, &cancel, |text, progress| {
                let _ = tx.send((text, progress));
            })
        },
        move |result| {
            if !closed.get() {
                closed.set(true);
                d.force_close();
            }
            match result {
                Ok(Outcome::Done(Enrolled::Yes)) => app.toast("Face added"),
                Ok(Outcome::Done(Enrolled::Cancelled)) => {}
                Ok(Outcome::Denied(message)) => app.toast(&message),
                Err(e) => app.error("Could not add the face", &e),
            }
            refresh(&app, &page);
        },
    );
}
