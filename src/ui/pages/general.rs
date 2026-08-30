//! General: terminal, clock, idle lock, power buttons, and power actions.

use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::{apps, system};
use crate::ui::{confirm, spawn, widgets, App};

const IDLE: [(&str, u32); 6] = [
    ("Never", 0),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("15 minutes", 15),
    ("30 minutes", 30),
    ("1 hour", 60),
];

const POWER_CHOICES: [(&str, &str); 4] = [
    ("Sleep", "suspend"),
    ("Power off", "poweroff"),
    ("Restart", "reboot"),
    ("Do nothing", "ignore"),
];

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("General", "Everyday behaviour of the desktop.");

    // Desktop
    let (desk_card, desk_body) = widgets::card("Desktop", "");
    let list = widgets::list();

    let terminals = apps::terminals();
    let names: Vec<String> = terminals.iter().map(|t| t.name.clone()).collect();
    let term_row = adw::ComboRow::builder()
        .title("Default terminal")
        .subtitle("Opened by Super+Shift+T and by other apps")
        .model(&gtk::StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    {
        let current = app.config.borrow().general.terminal.clone();
        if let Some(i) = terminals.iter().position(|t| {
            apps::executable(&t.id).as_deref() == Some(current.as_str())
                || t.id.trim_end_matches(".desktop") == current
        }) {
            term_row.set_selected(i as u32);
        }
        let app = app.clone();
        term_row.connect_selected_notify(move |r| {
            let Some(t) = terminals.get(r.selected() as usize) else {
                return;
            };
            let exe = apps::executable(&t.id)
                .unwrap_or_else(|| t.id.trim_end_matches(".desktop").to_string());
            if app.config.borrow().general.terminal == exe {
                return;
            }
            app.config.borrow_mut().general.terminal = exe;
            app.save();
        });
    }
    list.append(&term_row);

    let clock = adw::SwitchRow::builder()
        .title("24-hour clock")
        .active(app.config.borrow().general.clock_24h)
        .build();
    {
        let app = app.clone();
        clock.connect_active_notify(move |r| {
            app.config.borrow_mut().general.clock_24h = r.is_active();
            app.save();
        });
    }
    list.append(&clock);

    let date = adw::SwitchRow::builder()
        .title("Show the date in the bar")
        .active(app.config.borrow().general.show_date)
        .build();
    {
        let app = app.clone();
        date.connect_active_notify(move |r| {
            app.config.borrow_mut().general.show_date = r.is_active();
            app.save();
        });
    }
    list.append(&date);

    let idle = adw::ComboRow::builder()
        .title("Lock the screen when idle")
        .subtitle("Recorded in desktop.toml; the compositor applies it once it reads that file")
        .model(&gtk::StringList::new(&IDLE.map(|i| i.0)))
        .build();
    {
        let m = app.config.borrow().general.lock_after_minutes;
        idle.set_selected(IDLE.iter().position(|i| i.1 == m).unwrap_or(2) as u32);
        let app = app.clone();
        idle.connect_selected_notify(move |r| {
            let m = IDLE[r.selected() as usize].1;
            if app.config.borrow().general.lock_after_minutes == m {
                return;
            }
            app.config.borrow_mut().general.lock_after_minutes = m;
            app.save();
        });
    }
    list.append(&idle);

    let lang = adw::ActionRow::builder()
        .title("Language and region")
        .subtitle(system::locale())
        .build();
    lang.add_css_class("property");
    list.append(&lang);
    desk_body.append(&list);
    content.append(&desk_card);

    // Power buttons & lid
    let (pw_card, pw_body) = widgets::card(
        "Power buttons and lid",
        "From /etc/raven/power.toml. Changing these needs your password.",
    );
    let pw_list = widgets::list();
    match system::power_policy() {
        Ok(policy) => {
            for (title, table, key, current) in [
                (
                    "Power button",
                    "buttons",
                    "power",
                    policy.power_button.clone(),
                ),
                (
                    "Sleep button",
                    "buttons",
                    "sleep",
                    policy.sleep_button.clone(),
                ),
                ("Closing the lid", "lid", "close", policy.lid_close.clone()),
            ] {
                let row = adw::ComboRow::builder()
                    .title(title)
                    .model(&gtk::StringList::new(&POWER_CHOICES.map(|c| c.0)))
                    .build();
                row.set_selected(
                    POWER_CHOICES
                        .iter()
                        .position(|c| c.1 == current)
                        .unwrap_or(0) as u32,
                );
                let app = app.clone();
                let current = std::cell::RefCell::new(current);
                row.connect_selected_notify(move |r| {
                    let value = POWER_CHOICES[r.selected() as usize].1;
                    if *current.borrow() == value {
                        return;
                    }
                    *current.borrow_mut() = value.to_string();
                    let app = app.clone();
                    spawn(
                        move || system::set_power_policy(table, key, value),
                        move |res| match res {
                            Ok(()) => app.toast("Power policy updated"),
                            Err(e) => {
                                let d =
                                    adw::AlertDialog::new(Some("Needs root"), Some(&e.to_string()));
                                d.add_response("ok", "OK");
                                d.present(Some(&app.window()));
                            }
                        },
                    );
                });
                pw_list.append(&row);
            }
        }
        Err(e) => pw_list.append(
            &adw::ActionRow::builder()
                .title("Power policy unavailable")
                .subtitle(e.to_string())
                .build(),
        ),
    }
    pw_body.append(&pw_list);
    content.append(&pw_card);

    // Power actions
    let (act_card, act_body) = widgets::card("Power", "");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (label, icon, action, destructive) in [
        (
            "Sleep",
            "weather-clear-night-symbolic",
            system::PowerAction::Suspend,
            false,
        ),
        (
            "Restart",
            "system-reboot-symbolic",
            system::PowerAction::Reboot,
            true,
        ),
        (
            "Power off",
            "system-shutdown-symbolic",
            system::PowerAction::PowerOff,
            true,
        ),
    ] {
        let b = gtk::Button::builder().build();
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        inner.append(&gtk::Image::from_icon_name(icon));
        inner.append(&gtk::Label::new(Some(label)));
        b.set_child(Some(&inner));
        b.set_sensitive(system::power_available());
        let app = app.clone();
        b.connect_clicked(move |b| {
            let app = app.clone();
            let go = move |yes: bool| {
                if !yes {
                    return;
                }
                let app = app.clone();
                spawn(
                    move || system::power(action),
                    move |r| {
                        if let Err(e) = r {
                            app.error(label, &e);
                        }
                    },
                );
            };
            if destructive {
                confirm(
                    b,
                    &format!("{label} now?"),
                    "Unsaved work in open apps will be lost.",
                    label,
                    true,
                    go,
                );
            } else {
                go(true);
            }
        });
        actions.append(&b);
    }
    act_body.append(&actions);
    if !system::power_available() {
        act_body.append(&widgets::dim_label(
            "raven-powerd is not running, so power actions are unavailable.",
        ));
    }
    content.append(&act_card);

    root.upcast()
}
