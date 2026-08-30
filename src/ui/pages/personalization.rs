//! Personalization: the dock, the bar, and default applications.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::{apps, integrations};
use crate::ui::{widgets, App};

const DOCK_POSITIONS: [&str; 5] = ["Centre", "Top", "Bottom", "Left", "Right"];
const DOCK_LAYOUTS: [&str; 3] = ["Grid", "Row", "Column"];

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Personalization", "Make the desktop yours.");
    let (row, left, right) = widgets::two_columns();
    content.append(&row);
    left.append(&dock_card(app));
    left.append(&bar_card(app));
    right.append(&default_apps_card(app));
    root.upcast()
}

fn dock_card(app: &Rc<App>) -> gtk::Box {
    let (card, body) = widgets::card(
        "Dock",
        "Pinned apps. Changes take effect at your next login: the desktop reads this at start.",
    );
    let (position, orientation, pins) = integrations::read_pins();
    let pins = Rc::new(RefCell::new(pins));
    let position = Rc::new(RefCell::new(position));
    let orientation = Rc::new(RefCell::new(orientation));

    let write = {
        let app = app.clone();
        let pins = pins.clone();
        let position = position.clone();
        let orientation = orientation.clone();
        Rc::new(move || {
            if let Err(e) =
                integrations::write_pins(&position.borrow(), &orientation.borrow(), &pins.borrow())
            {
                app.error("Could not save dock", &e);
            }
            {
                let mut c = app.config.borrow_mut();
                c.personalization.dock_position = position.borrow().to_lowercase();
                c.personalization.dock_layout = orientation.borrow().to_lowercase();
            }
            app.save();
        })
    };

    let list = widgets::list();
    let pos_row = adw::ComboRow::builder()
        .title("Position")
        .model(&gtk::StringList::new(&DOCK_POSITIONS))
        .build();
    pos_row.set_selected(
        DOCK_POSITIONS
            .iter()
            .position(|p| p.eq_ignore_ascii_case(&position.borrow()))
            .unwrap_or(0) as u32,
    );
    {
        let write = write.clone();
        let position = position.clone();
        pos_row.connect_selected_notify(move |r| {
            *position.borrow_mut() = DOCK_POSITIONS[r.selected() as usize].to_string();
            write();
        });
    }
    list.append(&pos_row);
    let lay_row = adw::ComboRow::builder()
        .title("Layout")
        .model(&gtk::StringList::new(&DOCK_LAYOUTS))
        .build();
    lay_row.set_selected(
        DOCK_LAYOUTS
            .iter()
            .position(|p| p.eq_ignore_ascii_case(&orientation.borrow()))
            .unwrap_or(0) as u32,
    );
    {
        let write = write.clone();
        let orientation = orientation.clone();
        lay_row.connect_selected_notify(move |r| {
            *orientation.borrow_mut() = DOCK_LAYOUTS[r.selected() as usize].to_string();
            write();
        });
    }
    list.append(&lay_row);
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
    card
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
    let (card, body) = widgets::card("Bar", "RoostBar. Saved to ~/.config/roostbar/config.toml.");
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
        .subtitle("Apply bar changes now")
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
