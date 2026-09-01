//! Date & Time: the clock, its zone, and who keeps it.
//!
//! Everything privileged here goes through raven-timed's socket -- see
//! `backend::time`. The page itself only formats and asks.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::time;
use crate::ui::{spawn, widgets, App};

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Date & Time", "The clock, its zone, and who keeps it.");

    if !time::available() {
        let (card, body) = widgets::card("Date & Time", "");
        body.append(&widgets::dim_label(
            "raven-timed is not running, so the clock cannot be managed from here.",
        ));
        content.append(&card);
        return root.upcast();
    }

    let status = match time::status() {
        Ok(status) => status,
        Err(e) => {
            let (card, body) = widgets::card("Date & Time", "");
            body.append(&widgets::dim_label(&e.to_string()));
            content.append(&card);
            return root.upcast();
        }
    };

    // The zone the clock row formats in. Kept here rather than asked of glib,
    // because glib caches the local zone per process: after a change this
    // cell is right immediately and the cache is not.
    let zone: Rc<RefCell<String>> = Rc::new(RefCell::new(status.zone.clone()));

    // Clock
    let (clock_card, clock_body) = widgets::card("Clock", "");
    let clock_list = widgets::list();

    let now_row = adw::ActionRow::builder().title("Current time").build();
    now_row.add_css_class("property");
    {
        let now_row = now_row.clone();
        let zone = zone.clone();
        let twenty_four = app.config.borrow().general.clock_24h;
        let tick = move || {
            let tz = glib::TimeZone::new(Some(zone.borrow().as_str()));
            let format = if twenty_four {
                "%A %e %B %Y, %H:%M:%S"
            } else {
                "%A %e %B %Y, %I:%M:%S %p"
            };
            if let Some(text) = glib::DateTime::now(&tz)
                .ok()
                .and_then(|dt| dt.format(format).ok())
            {
                now_row.set_subtitle(text.trim());
            }
        };
        tick();
        glib::timeout_add_seconds_local(1, move || {
            tick();
            glib::ControlFlow::Continue
        });
    }
    clock_list.append(&now_row);

    let zones = time::zones().unwrap_or_default();
    let zone_row = adw::ComboRow::builder()
        .title("Time zone")
        .subtitle("Moves /etc/localtime; every app sees the change at once")
        .enable_search(true)
        .model(&gtk::StringList::new(
            &zones.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    if let Some(i) = zones.iter().position(|z| *z == status.zone) {
        zone_row.set_selected(i as u32);
    }
    {
        let app = app.clone();
        let zone = zone.clone();
        let zones = zones.clone();
        zone_row.connect_selected_notify(move |r| {
            let Some(picked) = zones.get(r.selected() as usize) else {
                return;
            };
            if *picked == *zone.borrow() {
                return;
            }
            let app = app.clone();
            let zone = zone.clone();
            let picked = picked.clone();
            let row = r.clone();
            let zones = zones.clone();
            spawn(
                {
                    let picked = picked.clone();
                    move || time::set_zone(&picked)
                },
                move |res| match res {
                    Ok(()) => {
                        *zone.borrow_mut() = picked.clone();
                        app.toast(&format!("Time zone set to {picked}"));
                    }
                    Err(e) => {
                        // Put the row back on the zone that is still real;
                        // re-entering the handler is stopped by the equality
                        // check above.
                        if let Some(i) = zones.iter().position(|z| *z == *zone.borrow()) {
                            row.set_selected(i as u32);
                        }
                        app.error("Could not set the time zone", &e);
                    }
                },
            );
        });
    }
    clock_list.append(&zone_row);
    clock_body.append(&clock_list);
    content.append(&clock_card);

    // Synchronization
    let (sync_card, sync_body) = widgets::card(
        "Automatic time",
        "From /etc/raven/time.toml. raven-timed asks pool.ntp.org unless that file says otherwise.",
    );
    let sync_list = widgets::list();

    let last_row = adw::ActionRow::builder().title("Last synchronized").build();
    last_row.add_css_class("property");
    let describe_last = |last: &Option<time::LastSync>| match last {
        Some(l) => format!("{} — clock was {} off, per {}", l.at, l.offset, l.server),
        None => "Never".to_string(),
    };
    last_row.set_subtitle(&describe_last(&status.last));

    let auto = adw::SwitchRow::builder()
        .title("Set time automatically")
        .subtitle("Checks every hour, and moments after the network comes up")
        .active(status.sync_on)
        .build();
    {
        let app = app.clone();
        auto.connect_active_notify(move |r| {
            let on = r.is_active();
            let app = app.clone();
            spawn(
                move || time::set_sync(on),
                move |res| match res {
                    Ok(()) => app.toast(if on {
                        "Automatic time on"
                    } else {
                        "Automatic time off"
                    }),
                    Err(e) => app.error("Could not change automatic time", &e),
                },
            );
        });
    }
    sync_list.append(&auto);

    let sync_row = adw::ActionRow::builder()
        .title("Synchronize now")
        .subtitle("Ask the servers this minute")
        .build();
    let sync_button = gtk::Button::builder()
        .label("Sync now")
        .valign(gtk::Align::Center)
        .build();
    {
        let app = app.clone();
        let last_row = last_row.clone();
        sync_button.connect_clicked(move |b| {
            b.set_sensitive(false);
            let app = app.clone();
            let b = b.clone();
            let last_row = last_row.clone();
            spawn(
                || {
                    let reply = time::sync_now()?;
                    Ok((reply, time::status().ok()))
                },
                move |res: anyhow::Result<_>| {
                    b.set_sensitive(true);
                    match res {
                        Ok((reply, status)) => {
                            app.toast(&reply);
                            if let Some(last) = status.map(|s| s.last) {
                                last_row.set_subtitle(&match &last {
                                    Some(l) => format!(
                                        "{} — clock was {} off, per {}",
                                        l.at, l.offset, l.server
                                    ),
                                    None => "Never".to_string(),
                                });
                            }
                        }
                        Err(e) => app.error("Could not synchronize", &e),
                    }
                },
            );
        });
    }
    sync_row.add_suffix(&sync_button);
    sync_list.append(&sync_row);
    sync_list.append(&last_row);

    sync_body.append(&sync_list);
    content.append(&sync_card);

    root.upcast()
}
