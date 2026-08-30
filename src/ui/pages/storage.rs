//! Storage: filesystems with usage bars, and the block devices behind them.

use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::storage::{self, BlockDevice};
use crate::ui::{spawn, widgets, App};
use crate::util::human_bytes;

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Storage", "Disks and how full they are.");
    let (fs_card, fs_body) = widgets::card("Filesystems", "");
    content.append(&fs_card);
    let (dev_card, dev_body) = widgets::card("Devices", "");
    let devices = widgets::list();
    dev_body.append(&devices);
    content.append(&dev_card);

    let app = app.clone();
    root.connect_map(move |_| {
        let app = app.clone();
        let fs_body = fs_body.clone();
        let devices = devices.clone();
        spawn(
            || (storage::filesystems(), storage::devices()),
            move |(fs, devs)| {
                widgets::clear_box(&fs_body);
                match fs {
                    Ok(list) => {
                        for f in list {
                            fs_body.append(&fs_row(&f));
                        }
                    }
                    Err(e) => app.error("Filesystems", &e),
                }
                widgets::clear(&devices);
                match devs {
                    Ok(list) => {
                        for d in list {
                            add_device(&devices, &d, 0);
                        }
                    }
                    Err(e) => app.error("Devices", &e),
                }
            },
        );
    });
    root.upcast()
}

fn fs_row(f: &storage::Filesystem) -> gtk::Box {
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::new(Some(&f.target));
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.add_css_class("card-title");
    head.append(&name);
    let pct = (f.used * 100).checked_div(f.size).unwrap_or(0);
    let detail = widgets::dim_label(&format!(
        "{} free of {} · {pct}% used · {}",
        human_bytes(f.avail),
        human_bytes(f.size),
        f.source
    ));
    head.append(&detail);
    bx.append(&head);
    let bar = gtk::LevelBar::builder()
        .min_value(0.0)
        .max_value(1.0)
        .value(f.used as f64 / f.size.max(1) as f64)
        .build();
    bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_LOW, 0.9);
    bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_HIGH, 0.97);
    bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_FULL, 1.0);
    bx.append(&bar);
    bx
}

fn add_device(list: &gtk::ListBox, d: &BlockDevice, depth: usize) {
    let mut sub = Vec::new();
    if let Some(s) = d.size {
        sub.push(human_bytes(s));
    }
    sub.push(d.kind.clone());
    if let Some(f) = &d.fstype {
        sub.push(f.clone());
    }
    if let Some(m) = &d.mountpoint {
        sub.push(format!("mounted at {m}"));
    }
    let title = match (&d.model, &d.label) {
        (Some(m), _) if !m.trim().is_empty() => format!("{}  ({})", d.name, m.trim()),
        (_, Some(l)) if !l.is_empty() => format!("{}  ({l})", d.name),
        _ => d.name.clone(),
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(sub.join(" · "))
        .build();
    let icon = match d.kind.as_str() {
        "disk" => "drive-harddisk-symbolic",
        "rom" => "media-optical-symbolic",
        _ => "drive-harddisk-symbolic",
    };
    let img = gtk::Image::from_icon_name(icon);
    img.set_margin_start((depth * 18) as i32);
    row.add_prefix(&img);
    list.append(&row);
    for c in &d.children {
        add_device(list, c, depth + 1);
    }
}
