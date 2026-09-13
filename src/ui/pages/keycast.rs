//! Key Overlay: the keys and buttons being pressed, shown on screen by
//! `raven-keycast`, for screen recordings and demonstrations.

use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::keycast::{self, InputAccess};
use crate::config::Keycast;
use crate::ui::{offer_terminal, spawn, widgets, App};

type Choices = &'static [(&'static str, &'static str)];

const POSITIONS: Choices = &[
    ("Bottom centre", "bottom-centre"),
    ("Bottom left", "bottom-left"),
    ("Bottom right", "bottom-right"),
    ("Top centre", "top-centre"),
];
const SIZES: Choices = &[("Small", "small"), ("Medium", "medium"), ("Large", "large")];
const MODES: Choices = &[("Every key", "all"), ("Shortcuts only", "shortcuts")];
const HIDE_AFTER: [(&str, u32); 4] = [
    ("1 second", 1000),
    ("2 seconds", 2000),
    ("3 seconds", 3000),
    ("5 seconds", 5000),
];

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page(
        "Key Overlay",
        "Show the keys and buttons you press on screen, for screen recordings, demonstrations and teaching.",
    );

    let switch = gtk::Switch::builder()
        .active(app.config.borrow().keycast.enabled)
        .build();
    content.append(&widgets::card_with_control(
        "Show keystrokes on screen",
        "Keycaps appear at the edge of the screen and press down with your keys. They stay above every window, and clicks pass straight through them.",
        &switch,
    ));

    let (status_card, status_body) = widgets::card("Status", "");
    let status = widgets::list();
    status_body.append(&status);
    content.append(&status_card);

    {
        let app = app.clone();
        let status = status.clone();
        switch.connect_active_notify(move |sw| {
            let on = sw.is_active();
            if app.config.borrow().keycast.enabled == on {
                return;
            }
            // Saved first: the daemon reads `enabled` from the file, so it
            // starts already on, and switching off hides it even if the
            // process outlives the stop.
            app.config.borrow_mut().keycast.enabled = on;
            app.save();
            let app = app.clone();
            let status = status.clone();
            spawn(
                move || {
                    let result = if on {
                        keycast::start()
                    } else {
                        keycast::stop()
                    };
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    result
                },
                move |result| {
                    if let Err(e) = result {
                        let context = if on {
                            "Could not start the key overlay"
                        } else {
                            "Could not stop the key overlay"
                        };
                        app.error(context, &e);
                    }
                    refresh_status(&app, &status);
                },
            );
        });
    }

    let (opt_card, opt_body) = widgets::card("Options", "Changes show on screen straight away.");
    let options = widgets::list();
    options.append(&choice_row(
        app,
        "Position",
        "",
        POSITIONS,
        |k| &k.position,
        |k, v| k.position = v.into(),
    ));
    options.append(&choice_row(
        app,
        "Size",
        "",
        SIZES,
        |k| &k.size,
        |k, v| k.size = v.into(),
    ));
    options.append(&choice_row(
        app,
        "Show",
        "Shortcuts only shows keys pressed with Ctrl, Alt or Super, and keys that type nothing, so text you type stays off screen",
        MODES,
        |k| &k.mode,
        |k, v| k.mode = v.into(),
    ));

    let mouse = adw::SwitchRow::builder()
        .title("Show mouse clicks")
        .active(app.config.borrow().keycast.show_mouse)
        .build();
    {
        let app = app.clone();
        mouse.connect_active_notify(move |r| {
            app.config.borrow_mut().keycast.show_mouse = r.is_active();
            app.save();
        });
    }
    options.append(&mouse);

    let hide = adw::ComboRow::builder()
        .title("Hide after")
        .subtitle("How long keys stay on screen once released")
        .model(&gtk::StringList::new(&HIDE_AFTER.map(|h| h.0)))
        .build();
    {
        let ms = app.config.borrow().keycast.hide_after_ms;
        hide.set_selected(HIDE_AFTER.iter().position(|h| h.1 == ms).unwrap_or(1) as u32);
        let app = app.clone();
        hide.connect_selected_notify(move |r| {
            let ms = HIDE_AFTER[r.selected() as usize].1;
            if app.config.borrow().keycast.hide_after_ms == ms {
                return;
            }
            app.config.borrow_mut().keycast.hide_after_ms = ms;
            app.save();
        });
    }
    options.append(&hide);
    opt_body.append(&options);
    content.append(&opt_card);

    let (note_card, note_body) = widgets::card("Before you record", "");
    note_body.append(&widgets::dim_label(
        "With Every key, everything you type is on screen, passwords included. Switch to Shortcuts only before typing one. Nothing shows while the screen is locked, and keys are never written anywhere.",
    ));
    note_body.append(&widgets::dim_label(
        "Keys are labelled as they are on a US keyboard. The overlay follows the accent colour, and moves less when smooth animations are off in Appearance.",
    ));
    content.append(&note_card);

    {
        let app = app.clone();
        root.connect_map(move |_| refresh_status(&app, &status));
    }
    root.upcast()
}

/// A row choosing one of `choices` for a string key of `[keycast]`.
fn choice_row(
    app: &Rc<App>,
    title: &str,
    subtitle: &str,
    choices: Choices,
    get: fn(&Keycast) -> &String,
    set: fn(&mut Keycast, &str),
) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(
            &choices.iter().map(|c| c.0).collect::<Vec<_>>(),
        ))
        .build();
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    {
        let config = app.config.borrow();
        let current = get(&config.keycast);
        if let Some(i) = choices.iter().position(|c| c.1 == current) {
            row.set_selected(i as u32);
        }
    }
    let app = app.clone();
    row.connect_selected_notify(move |r| {
        let Some(&(_, value)) = choices.get(r.selected() as usize) else {
            return;
        };
        if get(&app.config.borrow().keycast) == value {
            return;
        }
        set(&mut app.config.borrow_mut().keycast, value);
        app.save();
    });
    row
}

fn refresh_status(app: &Rc<App>, list: &gtk::ListBox) {
    let app = app.clone();
    let list = list.clone();
    spawn(
        || {
            (
                keycast::binary().is_some(),
                keycast::running(),
                keycast::input_access(),
            )
        },
        move |(installed, running, access)| {
            widgets::clear(&list);
            let enabled = app.config.borrow().keycast.enabled;

            let overlay = adw::ActionRow::builder().title("Overlay").build();
            overlay.add_prefix(&gtk::Image::from_icon_name("input-keyboard-symbolic"));
            match (installed, enabled, running) {
                (false, _, _) => overlay.set_subtitle(
                    "raven-keycast is not installed. sudo make install puts it beside raven-settings.",
                ),
                (true, false, _) => overlay.set_subtitle("Off"),
                (true, true, true) => overlay.set_subtitle("Showing keystrokes"),
                (true, true, false) => {
                    overlay.set_subtitle("Switched on, but raven-keycast is not running");
                    let start = gtk::Button::builder()
                        .label("Start")
                        .valign(gtk::Align::Center)
                        .build();
                    let app = app.clone();
                    let list = list.clone();
                    start.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        let app = app.clone();
                        let list = list.clone();
                        spawn(
                            || {
                                let result = keycast::start();
                                std::thread::sleep(std::time::Duration::from_millis(400));
                                result
                            },
                            move |result| {
                                if let Err(e) = result {
                                    app.error("Could not start the key overlay", &e);
                                }
                                refresh_status(&app, &list);
                            },
                        );
                    });
                    overlay.add_suffix(&start);
                }
            }
            list.append(&overlay);

            let input = adw::ActionRow::builder().title("Keyboard access").build();
            match access {
                InputAccess::Readable => {
                    input.set_subtitle("This account can read the keyboard and mouse")
                }
                InputAccess::NoDevices => input.set_subtitle("No keyboard or mouse found"),
                InputAccess::Denied => {
                    input.set_subtitle(&format!(
                        "Keys are read from /dev/input, which needs the {} group",
                        keycast::INPUT_GROUP
                    ));
                    let fix = gtk::Button::builder()
                        .label("Add Me")
                        .valign(gtk::Align::Center)
                        .build();
                    fix.add_css_class("suggested-action");
                    let app = app.clone();
                    fix.connect_clicked(move |_| {
                        // Group membership is root's to grant and no daemon
                        // offers it; see backend::terminal.
                        let user = std::env::var("USER").unwrap_or_default();
                        let cmd: Vec<String> =
                            ["sudo", "usermod", "-aG", keycast::INPUT_GROUP, &user]
                                .iter()
                                .map(|s| s.to_string())
                                .collect();
                        let app2 = app.clone();
                        offer_terminal(
                            &app,
                            "Join the input group?",
                            &format!(
                                "The key overlay reads the keyboard and mouse directly, which members of the {} group may do. Adding {user} to it needs root; it applies after you log out and back in.",
                                keycast::INPUT_GROUP
                            ),
                            &cmd,
                            move |ran| {
                                if ran {
                                    app2.toast("Log out and back in once it finishes");
                                }
                            },
                        );
                    });
                    input.add_suffix(&fix);
                }
            }
            list.append(&input);
        },
    );
}
