//! Updates: check with `rvn update --dry-run`, apply in a terminal.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::{system, updates};
use crate::ui::{spawn, widgets, App};

thread_local! {
    static STATUS: Cell<Option<usize>> = const { Cell::new(None) };
}

/// How many updates the last check found; `None` before any check.
pub fn status() -> Option<usize> {
    STATUS.with(|s| s.get())
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Updates", "Keep Raven current with rvn.");
    if !updates::available() {
        content.append(&widgets::banner("rvn was not found on this system."));
        return root.upcast();
    }
    let os = system::os_release();

    let (status_card, status_body) = widgets::card("", "");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let icon = gtk::Image::from_icon_name("software-update-available-symbolic");
    icon.set_pixel_size(36);
    row.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let headline = gtk::Label::new(Some("Not checked yet"));
    headline.add_css_class("card-title");
    headline.set_xalign(0.0);
    let detail = widgets::dim_label(&format!("{} {} · rolling", os.name, os.version_id));
    text.append(&headline);
    text.append(&detail);
    row.append(&text);
    let spinner = gtk::Spinner::new();
    row.append(&spinner);
    let check = gtk::Button::with_label("Check for updates");
    check.set_valign(gtk::Align::Center);
    row.append(&check);
    let install = gtk::Button::with_label("Install updates");
    install.add_css_class("suggested-action");
    install.set_valign(gtk::Align::Center);
    install.set_visible(false);
    row.append(&install);
    status_body.append(&row);
    content.append(&status_card);

    let (list_card, list_body) = widgets::card("Available updates", "");
    let list = widgets::list();
    list_body.append(&list);
    list_card.set_visible(false);
    content.append(&list_card);

    content.append(&widgets::dim_label(if updates::store_available() {
        "Installing opens Raven Store, which shows progress and asks for your password. Packages from the AUR are built on this machine."
    } else {
        "Installing runs `sudo rvn update` in your terminal so you can watch it and answer prompts. Packages from the AUR are built on this machine."
    }));

    let do_check = {
        let app = app.clone();
        let headline = headline.clone();
        let detail = detail.clone();
        let spinner = spinner.clone();
        let check = check.clone();
        let install = install.clone();
        let list = list.clone();
        let list_card = list_card.clone();
        Rc::new(move |refresh: bool| {
            spinner.start();
            check.set_sensitive(false);
            headline.set_text("Checking…");
            let app = app.clone();
            let headline = headline.clone();
            let detail = detail.clone();
            let spinner = spinner.clone();
            let check = check.clone();
            let install = install.clone();
            let list = list.clone();
            let list_card = list_card.clone();
            spawn(
                move || updates::check(refresh),
                move |r| {
                    spinner.stop();
                    check.set_sensitive(true);
                    match r {
                        Ok(c) => {
                            let n = c.updates.len();
                            STATUS.with(|s| s.set(Some(n)));
                            if n == 0 {
                                headline.set_text("System is up to date");
                                detail.set_text("Everything installed is at its latest version.");
                            } else {
                                headline.set_text(&format!(
                                    "{n} update{} available",
                                    if n == 1 { "" } else { "s" }
                                ));
                                detail.set_text(
                                    &c.download
                                        .map(|d| format!("Download {d}"))
                                        .unwrap_or_default(),
                                );
                            }
                            install.set_visible(n > 0);
                            list_card.set_visible(n > 0);
                            widgets::clear(&list);
                            for u in c.updates {
                                let row = adw::ActionRow::builder()
                                    .title(glib::markup_escape_text(&u.name))
                                    .subtitle(format!("{}  →  {}   ({})", u.from, u.to, u.repo))
                                    .build();
                                list.append(&row);
                            }
                            // Nudge the sidebar's status card.
                            app.save_status_only();
                        }
                        Err(e) => {
                            headline.set_text("Could not check for updates");
                            detail.set_text(&e.to_string());
                        }
                    }
                },
            );
        })
    };
    {
        let do_check = do_check.clone();
        check.connect_clicked(move |_| do_check(true));
    }
    {
        let app = app.clone();
        install.connect_clicked(move |_| {
            // Raven Store shows progress in a window and handles the
            // password prompt; a terminal is the fallback for images
            // without it, and for people who prefer one.
            if updates::store_available() {
                match updates::open_store() {
                    Ok(()) => app.toast("Opening Raven Store"),
                    Err(e) => app.error("Could not open Raven Store", &e),
                }
                return;
            }
            let terminal = app.config.borrow().general.terminal.clone();
            match launch_in_terminal(&terminal, &updates::apply_command()) {
                Ok(()) => app.toast("Updating in a terminal window"),
                Err(e) => app.error("Could not open a terminal", &e),
            }
        });
    }
    // One check shortly after start, without a repository refresh, so the
    // sidebar badge is right without hammering mirrors at every launch.
    {
        let do_check = do_check.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || do_check(false));
    }
    root.upcast()
}

/// Run a command in the user's terminal. `-e` is what nearly every emulator
/// accepts; GIO's own terminal lookup is the fallback.
fn launch_in_terminal(terminal: &str, cmd: &[String]) -> anyhow::Result<()> {
    let script = format!(
        "{}; echo; echo 'Done. Press Enter to close.'; read _",
        cmd.iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if crate::util::have(terminal) {
        let spawned = std::process::Command::new(terminal)
            .args(["-e", "sh", "-c", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if spawned.is_ok() {
            return Ok(());
        }
    }
    let info = gio::AppInfo::create_from_commandline(
        format!("sh -c \"{}\"", script.replace('"', "\\\"")),
        None,
        gio::AppInfoCreateFlags::NEEDS_TERMINAL,
    )?;
    info.launch(&[], None::<&gio::AppLaunchContext>)?;
    Ok(())
}
