//! Personalization: the pinned application bar, RoostBar, and default
//! applications.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::{apps, integrations};
use crate::ui::{widgets, App};

thread_local! {
    /// Kept alive for the life of the process; a dropped monitor stops.
    /// Pages are built once, with the window, so this is filled once.
    static PINS_MONITOR: RefCell<Option<gio::FileMonitor>> = const { RefCell::new(None) };
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Personalization", "Make the desktop yours.");
    let (row, left, right) = widgets::two_columns();
    content.append(&row);
    left.append(&pinned_card(app));
    left.append(&bar_card(app));
    right.append(&default_apps_card(app));
    root.upcast()
}

/// The pinned application bar: the rail of pinned applications that rides one
/// edge of the screen.
///
/// Not the dock, which this card used to be called. The dock is the strip of
/// *running* applications the compositor reveals at the bottom edge; it is
/// the taskbar, it is not configured here, and calling this card "Dock" named
/// the wrong thing.
fn pinned_card(app: &Rc<App>) -> gtk::Box {
    // Whether the compositor takes `reload_pins`, asked once: it decides what
    // this card promises, and it cannot change while the page is open.
    let live = integrations::pins_apply_live();
    let (card, body) = widgets::card(
        "Pinned application bar",
        if live {
            "The rail of pinned applications along one edge of the screen. Changes show up straight away."
        } else {
            "The rail of pinned applications along one edge of the screen. This compositor is too old to be told about a change, so one takes effect at your next login. Update RavenGUI (imlazy install)."
        },
    );
    let (position, pins) = integrations::read_pins();
    let pins = Rc::new(RefCell::new(pins));
    let position = Rc::new(RefCell::new(position));

    let write = {
        let app = app.clone();
        let pins = pins.clone();
        let position = position.clone();
        Rc::new(move || {
            if let Err(e) = integrations::write_pins(&position.borrow(), &pins.borrow()) {
                app.error("Could not save the pinned application bar", &e);
                return;
            }
            // The compositor read this file at startup and has held it in
            // memory since, so writing it is only half the change: without
            // this it would show nothing new until the next login, and would
            // write its own copy back over the edit the moment anything else
            // touched the pins.
            if live {
                if let Err(e) = integrations::reload_pins() {
                    app.error("Could not update the pinned application bar", &e);
                }
            }
        })
    };

    let list = widgets::list();
    let pos_row = adw::ComboRow::builder()
        .title("Position")
        .subtitle("Which edge the bar rides. Left or right runs it down the screen, top or bottom across it.")
        .model(&gtk::StringList::new(&integrations::PIN_POSITIONS))
        .build();
    pos_row.set_selected(
        integrations::PIN_POSITIONS
            .iter()
            .position(|p| p.eq_ignore_ascii_case(&position.borrow()))
            .unwrap_or(0) as u32,
    );
    {
        let write = write.clone();
        let position = position.clone();
        pos_row.connect_selected_notify(move |r| {
            let chosen = integrations::PIN_POSITIONS[r.selected() as usize];
            if position.borrow().eq_ignore_ascii_case(chosen) {
                return;
            }
            *position.borrow_mut() = chosen.to_string();
            write();
        });
    }
    list.append(&pos_row);
    body.append(&list);

    let pinned = widgets::list();
    body.append(&widgets::dim_label("Pinned apps"));
    body.append(&pinned);
    let add = gtk::Button::with_label("Add app…");
    add.set_halign(gtk::Align::Start);
    body.append(&add);

    let render: Rc<dyn Fn()> = {
        let pinned = pinned.clone();
        let pins = pins.clone();
        let write = write.clone();
        type Render = Rc<dyn Fn()>;
        let render_cell: Rc<RefCell<Option<Render>>> = Rc::new(RefCell::new(None));
        let render_cell2 = render_cell.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            widgets::clear(&pinned);
            let list = pins.borrow().clone();
            if list.is_empty() {
                pinned.append(&adw::ActionRow::builder().title("Nothing pinned").build());
            }
            for (i, path) in list.iter().enumerate() {
                let info = gio::DesktopAppInfo::from_filename(path);
                let name = info
                    .as_ref()
                    .map(|i| i.display_name().to_string())
                    .unwrap_or_else(|| path.clone());
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&name))
                    .subtitle(path)
                    .build();
                if let Some(icon) = info.as_ref().and_then(|i| i.icon()) {
                    row.add_prefix(&gtk::Image::from_gicon(&icon));
                }
                let rm = gtk::Button::from_icon_name("list-remove-symbolic");
                rm.add_css_class("flat");
                rm.set_valign(gtk::Align::Center);
                let pins = pins.clone();
                let write = write.clone();
                let render_cell = render_cell2.clone();
                rm.connect_clicked(move |_| {
                    pins.borrow_mut().remove(i);
                    write();
                    if let Some(r) = render_cell.borrow().clone() {
                        r();
                    }
                });
                row.add_suffix(&rm);
                pinned.append(&row);
            }
        });
        *render_cell.borrow_mut() = Some(f.clone());
        f
    };
    render();
    {
        let app = app.clone();
        let pins = pins.clone();
        let write = write.clone();
        let render = render.clone();
        add.connect_clicked(move |_| {
            let pins = pins.clone();
            let write = write.clone();
            let render = render.clone();
            pick_app(&app, move |path| {
                if !pins.borrow().contains(&path) {
                    pins.borrow_mut().push(path);
                    write();
                    render();
                }
            });
        });
    }

    // The page is not the only one who changes this. Quick settings has a
    // row for the edge, and the dock's menu pins and unpins, and the
    // compositor writes the file on each. Without this the card would go on
    // showing what the file said when the window opened — and, worse, the
    // next change made here would write that stale picture back over
    // whatever quick settings had done.
    watch_pins({
        let pins = pins.clone();
        let position = position.clone();
        let pos_row = pos_row.clone();
        let render = render.clone();
        Rc::new(move || {
            let (edge, list) = integrations::read_pins();
            // Our own write lands here too, and reads back as what is
            // already on screen. Comparing rather than redrawing is what
            // keeps that from becoming a loop.
            if edge == *position.borrow() && list == *pins.borrow() {
                return;
            }
            *position.borrow_mut() = edge.clone();
            *pins.borrow_mut() = list;
            // Moving the row fires its handler, which writes. It returns
            // early when the edge it reads already matches, and it does:
            // the line above has just set it.
            if let Some(i) = integrations::PIN_POSITIONS
                .iter()
                .position(|p| p.eq_ignore_ascii_case(&edge))
            {
                pos_row.set_selected(i as u32);
            }
            render();
        })
    });
    card
}

/// Call `reload` whenever the pins file changes underneath the page.
///
/// The directory is watched rather than the file: the file is replaced by a
/// rename — both writers are careful never to leave a half-written one — and
/// a watch on the old inode would go deaf the first time that happened. The
/// launch history is written into the same directory on every application
/// launch, so events are filtered by name rather than acted on wholesale.
///
/// A file watch rather than something on `raven_shell_v1`: the file is
/// already the contract between the two processes, and the compositor
/// already writes it on every change, so there is nothing to add on either
/// side. Reading it back costs a few hundred bytes.
fn watch_pins(reload: Rc<dyn Fn()>) {
    let path = integrations::pins_path();
    let Some(dir) = path.parent().map(std::path::Path::to_path_buf) else {
        return;
    };
    let Some(name) = path.file_name().map(std::ffi::OsString::from) else {
        return;
    };
    // It may not exist until something first pins something; creating it
    // early costs nothing and lets the page be right from the start.
    let _ = std::fs::create_dir_all(&dir);
    let monitor = match gio::File::for_path(&dir)
        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
    {
        Ok(m) => m,
        Err(e) => {
            // Not fatal: the card still shows and still writes, it just
            // stops noticing changes made elsewhere.
            tracing::debug!("not watching {}: {e}", dir.display());
            return;
        }
    };
    monitor.connect_changed(move |_, file, other, _| {
        // A rename reports the old path as `file` and the new one as
        // `other`, so either may be the one we care about.
        let ours = |f: Option<&gio::File>| {
            f.and_then(gio::prelude::FileExt::basename)
                .is_some_and(|b| b == std::path::Path::new(&name))
        };
        if ours(Some(file)) || ours(other) {
            reload();
        }
    });
    PINS_MONITOR.with(|m| *m.borrow_mut() = Some(monitor));
}

/// A dialog listing installed apps; `chosen` gets the .desktop path.
fn pick_app(app: &Rc<App>, chosen: impl Fn(String) + 'static) {
    let dialog = adw::Dialog::builder()
        .title("Pin an app")
        .content_width(420)
        .content_height(560)
        .build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    let mut infos: Vec<gio::AppInfo> = gio::AppInfo::all()
        .into_iter()
        .filter(|i| i.should_show())
        .collect();
    infos.sort_by_key(|i| i.display_name().to_lowercase());
    let chosen = Rc::new(chosen);
    for info in infos {
        let Some(desktop) = info.downcast_ref::<gio::DesktopAppInfo>() else {
            continue;
        };
        let Some(path) = desktop.filename() else {
            continue;
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&info.display_name()))
            .activatable(true)
            .build();
        if let Some(icon) = info.icon() {
            row.add_prefix(&gtk::Image::from_gicon(&icon));
        }
        let chosen = chosen.clone();
        let dialog = dialog.clone();
        let path = path.to_string_lossy().to_string();
        row.connect_activated(move |_| {
            chosen(path.clone());
            dialog.close();
        });
        list.append(&row);
    }
    let sc = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();
    tv.set_content(Some(&sc));
    dialog.set_child(Some(&tv));
    dialog.present(Some(&app.window()));
}

fn bar_card(app: &Rc<App>) -> gtk::Box {
    let (card, body) = widgets::card(
        "Bar",
        "RoostBar. Saved to ~/.config/roostbar/config.toml, which the bar rereads within a few seconds. Its height and text size follow Interface scale, under Appearance.",
    );
    let list = widgets::list();
    let pos = adw::ComboRow::builder()
        .title("Position")
        .model(&gtk::StringList::new(&["Top", "Bottom"]))
        .build();
    pos.set_selected(
        if app.config.borrow().personalization.bar_position == "bottom" {
            1
        } else {
            0
        },
    );
    {
        let app = app.clone();
        pos.connect_selected_notify(move |r| {
            let v = if r.selected() == 1 { "bottom" } else { "top" };
            if app.config.borrow().personalization.bar_position == v {
                return;
            }
            app.config.borrow_mut().personalization.bar_position = v.into();
            app.save();
        });
    }
    list.append(&pos);
    let restart = adw::ActionRow::builder()
        .title("Restart the bar")
        .subtitle("Only needed for a bar too old to reread its config, or one that is not running")
        .build();
    let b = gtk::Button::with_label("Restart");
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        b.connect_clicked(move |_| match integrations::restart_roostbar() {
            Ok(()) => app.toast("Bar restarted"),
            Err(e) => app.error("Bar", &e),
        });
    }
    restart.add_suffix(&b);
    list.append(&restart);
    body.append(&list);
    card
}

fn default_apps_card(app: &Rc<App>) -> gtk::Box {
    let (card, body) = widgets::card(
        "Default applications",
        "Which app opens what. Written to ~/.config/mimeapps.list.",
    );
    let list = widgets::list();
    for cat in apps::CATEGORIES {
        let candidates = apps::candidates(cat);
        let current = apps::current(cat);
        let current_idx = current
            .as_ref()
            .and_then(|c| candidates.iter().position(|x| x.id == c.id));
        // A leading "Not set" entry when nothing is chosen, so the row never
        // pretends the first installed app is the default.
        let offset = usize::from(current_idx.is_none());
        let mut names: Vec<String> = Vec::new();
        if offset == 1 {
            names.push("Not set".into());
        }
        names.extend(candidates.iter().map(|c| c.name.clone()));
        let row = adw::ComboRow::builder()
            .title(cat.label)
            .model(&gtk::StringList::new(
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
            ))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(cat.icon));
        if let Some(i) = current_idx {
            row.set_selected(i as u32);
        }
        if candidates.is_empty() {
            row.set_sensitive(false);
            row.set_subtitle("No installed app handles this");
        }
        let app = app.clone();
        row.connect_selected_notify(move |r| {
            let i = r.selected() as usize;
            if i < offset {
                return;
            }
            let Some(c) = candidates.get(i - offset) else {
                return;
            };
            match apps::set_default(cat, &c.id) {
                Ok(()) => app.toast(&format!(
                    "{} is now the default {}",
                    c.name,
                    cat.label.to_lowercase()
                )),
                Err(e) => app.error("Could not set default", &e),
            }
        });
        list.append(&row);
    }
    body.append(&list);
    card
}
