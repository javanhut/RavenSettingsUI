//! Notifications: whether they interrupt, and how long they stay.
//!
//! The desktop draws notifications itself — the compositor is the
//! notification server — and reads what this page writes to `[notifications]`
//! in desktop.toml. Nothing is signalled: the compositor watches the file.

use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::ui::{widgets, App};

const SHOW_FOR: [(&str, u32); 4] = [
    ("4 seconds", 4),
    ("6 seconds", 6),
    ("10 seconds", 10),
    ("20 seconds", 20),
];

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page(
        "Notifications",
        "Messages from applications appear in the top-right corner of the screen for a few seconds, then go.",
    );

    let quiet = gtk::Switch::builder()
        .active(app.config.borrow().notifications.do_not_disturb)
        .build();
    content.append(&widgets::card_with_control(
        "Do not disturb",
        "Only critical notifications appear. The rest wait quietly; the Notifications row in quick settings (Super+Ctrl+S) brings them back.",
        &quiet,
    ));
    {
        let app = app.clone();
        quiet.connect_active_notify(move |switch| {
            let on = switch.is_active();
            if app.config.borrow().notifications.do_not_disturb == on {
                return;
            }
            app.config.borrow_mut().notifications.do_not_disturb = on;
            app.save();
        });
    }

    let (options_card, options_body) = widgets::card("Options", "Changes apply straight away.");
    let options = widgets::list();
    let show_for = adw::ComboRow::builder()
        .title("Show for")
        .subtitle("How long a notification stays when its application leaves that to the desktop. Critical ones stay until dismissed.")
        .model(&gtk::StringList::new(&SHOW_FOR.map(|s| s.0)))
        .build();
    {
        let seconds = app.config.borrow().notifications.timeout_seconds;
        show_for.set_selected(SHOW_FOR.iter().position(|s| s.1 == seconds).unwrap_or(1) as u32);
        let app = app.clone();
        show_for.connect_selected_notify(move |row| {
            let seconds = SHOW_FOR[row.selected() as usize].1;
            if app.config.borrow().notifications.timeout_seconds == seconds {
                return;
            }
            app.config.borrow_mut().notifications.timeout_seconds = seconds;
            app.save();
        });
    }
    options.append(&show_for);
    options_body.append(&options);
    content.append(&options_card);

    let (note_card, note_body) = widgets::card("When they wait", "");
    note_body.append(&widgets::dim_label(
        "While a window is fullscreen or a video is playing, notifications other than critical ones wait quietly too. Nothing appears while the screen is locked; what arrived shows once you unlock.",
    ));
    note_body.append(&widgets::dim_label(
        "A notification stays while the pointer is on it, and while nobody is at the computer. Super+Ctrl+N dismisses the newest and Super+Ctrl+Shift+N dismisses them all; a right click dismisses one.",
    ));
    content.append(&note_card);

    root.upcast()
}
