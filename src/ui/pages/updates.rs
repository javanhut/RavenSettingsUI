//! Updates: check with `rvn update --dry-run`, apply in Raven Store or a
//! terminal.
//!
//! A check here runs as the user, so rvn syncs the databases into a
//! per-user copy when it cannot write the system one. Later checks — here,
//! in Raven Store, or `rvn update --dry-run --no-refresh` — read whichever
//! copy is fresher, so the two apps always agree. The page also watches the
//! database directories and re-checks when something else changes them.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::{system, updates};
use crate::ui::{spawn, widgets, App};

thread_local! {
    static STATUS: Cell<Option<usize>> = const { Cell::new(None) };
    /// Kept alive for the life of the process; a dropped monitor stops.
    static MONITORS: RefCell<Vec<gio::FileMonitor>> = const { RefCell::new(Vec::new()) };
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

    // When the last check started, so the database watcher can tell a
    // change that check already saw from one that arrived after it.
    let checked_at: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let do_check = {
        let app = app.clone();
        let headline = headline.clone();
        let detail = detail.clone();
        let spinner = spinner.clone();
        let check = check.clone();
        let install = install.clone();
        let list = list.clone();
        let list_card = list_card.clone();
        let checked_at = checked_at.clone();
        Rc::new(move |refresh: bool| {
            // A check is already running; it will report what it finds.
            if !check.is_sensitive() {
                return;
            }
            checked_at.set(Some(Instant::now()));
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
    watch_databases(do_check, checked_at);
    root.upcast()
}

/// The directories rvn writes when the picture of the system changes: the
/// system sync databases, the per-user copy an unprivileged check syncs
/// into, and the local database of what is installed.
fn watched_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/var/lib/pacman/sync"),
        PathBuf::from("/var/lib/pacman/local"),
    ];
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
    if let Some(cache) = cache {
        dirs.push(cache.join("rvn").join("sync"));
    }
    dirs
}

/// Re-checks (without refreshing) shortly after any of [`watched_dirs`]
/// changes, so a check made in Raven Store, or an update applied there or
/// from a terminal, is reflected here and in the sidebar badge without
/// pressing the button again. Bursts of events collapse into one check; a
/// change our own check already covered is skipped.
fn watch_databases(do_check: Rc<dyn Fn(bool)>, checked_at: Rc<Cell<Option<Instant>>>) {
    let changed_at: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    for dir in watched_dirs() {
        // The per-user copy may not exist until the first check; creating
        // it early costs nothing and lets it be watched from the start.
        if dir.starts_with(std::env::var_os("HOME").unwrap_or_default()) {
            let _ = std::fs::create_dir_all(&dir);
        }
        let monitor = match gio::File::for_path(&dir)
            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            Ok(m) => {
                tracing::debug!("watching {}", dir.display());
                m
            }
            Err(e) => {
                tracing::debug!("not watching {}: {e}", dir.display());
                continue;
            }
        };
        let do_check = do_check.clone();
        let checked_at = checked_at.clone();
        let changed_at = changed_at.clone();
        let pending = pending.clone();
        monitor.connect_changed(move |_, _, _, _| {
            changed_at.set(Some(Instant::now()));
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let do_check = do_check.clone();
            let checked_at = checked_at.clone();
            let changed_at = changed_at.clone();
            let pending2 = pending.clone();
            let id = glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                pending2.borrow_mut().take();
                let stale = match (changed_at.get(), checked_at.get()) {
                    (Some(changed), Some(checked)) => changed > checked,
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                if stale {
                    tracing::info!("package databases changed on disk; re-checking");
                    do_check(false);
                }
            });
            *pending.borrow_mut() = Some(id);
        });
        MONITORS.with(|m| m.borrow_mut().push(monitor));
    }
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
