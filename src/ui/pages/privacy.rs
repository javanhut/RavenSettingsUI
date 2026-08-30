//! Privacy: what the desktop remembers about you, and how to clear it.

use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::integrations;
use crate::ui::{confirm, widgets, App};

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Privacy", "What Raven keeps about how you use it.");

    let (hist_card, hist_body) = widgets::card("History", "");
    let list = widgets::list();

    let frecency = integrations::frecency_path();
    let row = adw::ActionRow::builder()
        .title("Launcher history")
        .subtitle(format!(
            "Apps you open most are ranked first in the launcher. Stored in {}",
            frecency.display()
        ))
        .build();
    let clear = gtk::Button::with_label("Clear");
    clear.add_css_class("flat");
    clear.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        clear.connect_clicked(move |b| {
            let app = app.clone();
            let path = frecency.clone();
            confirm(
                b,
                "Clear launcher history?",
                "The launcher will forget which apps you use most.",
                "Clear",
                true,
                move |yes| {
                    if yes {
                        match std::fs::remove_file(&path) {
                            Ok(()) => app.toast("Launcher history cleared"),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                app.toast("Nothing to clear")
                            }
                            Err(e) => app.error("Could not clear", &e.into()),
                        }
                    }
                },
            );
        });
    }
    row.add_suffix(&clear);
    list.append(&row);

    let remember = adw::SwitchRow::builder()
        .title("Remember app usage")
        .subtitle("Turning this off records your preference; the launcher honours it once it reads desktop.toml.")
        .active(app.config.borrow().privacy.remember_app_usage)
        .build();
    {
        let app = app.clone();
        remember.connect_active_notify(move |r| {
            app.config.borrow_mut().privacy.remember_app_usage = r.is_active();
            app.save();
        });
    }
    list.append(&remember);

    for (title, subtitle, path) in [
        (
            "File manager search history",
            "Recent searches in Raven File Manager.",
            crate::config::config_dir().join("search_history.json"),
        ),
        (
            "Terminal search history",
            "Recent searches in Raven Terminal.",
            crate::config::config_dir()
                .with_file_name("raven-terminal")
                .join("search_history.json"),
        ),
    ] {
        if !path.exists() {
            continue;
        }
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .build();
        let b = gtk::Button::with_label("Clear");
        b.add_css_class("flat");
        b.set_valign(gtk::Align::Center);
        let app = app.clone();
        b.connect_clicked(move |_| match std::fs::remove_file(&path) {
            Ok(()) => app.toast("Cleared"),
            Err(e) => app.error("Could not clear", &e.into()),
        });
        row.add_suffix(&b);
        list.append(&row);
    }
    hist_body.append(&list);
    content.append(&hist_card);

    let (vis_card, vis_body) = widgets::card("Visibility", "");
    vis_body.append(&widgets::dim_label(
        "Bluetooth visibility to other devices is controlled on the Bluetooth page. Wi-Fi passphrases are stored by cawd in /var/lib/caw/profiles, readable by root only.",
    ));
    content.append(&vis_card);

    root.upcast()
}
