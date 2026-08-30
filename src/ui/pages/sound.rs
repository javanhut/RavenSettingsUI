//! Sound: default output and input, volume and mute, through wpctl.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::backend::sound::{self, Device};
use crate::ui::{spawn, widgets, App};

struct Section {
    dropdown: gtk::DropDown,
    scale: gtk::Scale,
    mute: gtk::ToggleButton,
    devices: RefCell<Vec<Device>>,
    syncing: Cell<bool>,
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Sound", "Output and input devices, through PipeWire.");
    if !sound::available() {
        content.append(&widgets::banner("wpctl was not found. Install PipeWire and WirePlumber: sudo rvn install -y pipewire-audio wireplumber pipewire-pulse"));
        return root.upcast();
    }

    let output = section(
        app,
        &content,
        "Output",
        "Speakers and headphones",
        "audio-volume-high-symbolic",
        true,
    );
    let input = section(
        app,
        &content,
        "Input",
        "Microphones",
        "audio-input-microphone-symbolic",
        false,
    );

    let refresh_btn = gtk::Button::with_label("Refresh devices");
    refresh_btn.set_halign(gtk::Align::Start);
    content.append(&refresh_btn);

    let refresh = {
        let app = app.clone();
        let output = output.clone();
        let input = input.clone();
        Rc::new(move || {
            let app = app.clone();
            let output = output.clone();
            let input = input.clone();
            spawn(sound::snapshot, move |r| match r {
                Ok(snap) => {
                    fill(&output, &snap.sinks);
                    fill(&input, &snap.sources);
                }
                Err(e) => app.error("Could not read audio devices", &e),
            });
        })
    };
    {
        let refresh = refresh.clone();
        refresh_btn.connect_clicked(move |_| refresh());
    }
    {
        let refresh = refresh.clone();
        root.connect_map(move |_| refresh());
    }
    {
        let root2 = root.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(5), move || {
            if root2.is_mapped() {
                refresh();
            }
            glib::ControlFlow::Continue
        });
    }
    root.upcast()
}

fn section(
    app: &Rc<App>,
    content: &gtk::Box,
    title: &str,
    subtitle: &str,
    icon: &str,
    is_sink: bool,
) -> Rc<Section> {
    let (card, body) = widgets::card(title, subtitle);
    let dropdown = gtk::DropDown::from_strings(&[]);
    dropdown.set_hexpand(true);
    body.append(&dropdown);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let mute = gtk::ToggleButton::new();
    mute.set_icon_name(icon);
    mute.set_tooltip_text(Some("Mute"));
    row.append(&mute);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_format_value_func(|_, v| format!("{v:.0}%"));
    row.append(&scale);
    body.append(&row);
    content.append(&card);

    let s = Rc::new(Section {
        dropdown,
        scale,
        mute,
        devices: RefCell::new(vec![]),
        syncing: Cell::new(false),
    });
    let _ = is_sink;

    {
        let s2 = s.clone();
        let app = app.clone();
        s.dropdown.connect_selected_notify(move |d| {
            if s2.syncing.get() {
                return;
            }
            let Some(dev) = s2.devices.borrow().get(d.selected() as usize).cloned() else {
                return;
            };
            let app = app.clone();
            let s3 = s2.clone();
            spawn(
                move || sound::set_default(dev.id).map(|_| dev),
                move |r| match r {
                    Ok(dev) => {
                        s3.syncing.set(true);
                        s3.scale.set_value(dev.volume * 100.0);
                        s3.mute.set_active(dev.muted);
                        s3.syncing.set(false);
                    }
                    Err(e) => app.error("Could not change device", &e),
                },
            );
        });
    }
    {
        let s2 = s.clone();
        let app = app.clone();
        s.scale.connect_value_changed(move |sc| {
            if s2.syncing.get() {
                return;
            }
            let Some(dev) = s2
                .devices
                .borrow()
                .get(s2.dropdown.selected() as usize)
                .cloned()
            else {
                return;
            };
            let v = sc.value() / 100.0;
            let app = app.clone();
            spawn(
                move || sound::set_volume(dev.id, v),
                move |r| {
                    if let Err(e) = r {
                        app.error("Could not set volume", &e);
                    }
                },
            );
        });
    }
    {
        let s2 = s.clone();
        let app = app.clone();
        s.mute.connect_toggled(move |b| {
            if s2.syncing.get() {
                return;
            }
            let Some(dev) = s2
                .devices
                .borrow()
                .get(s2.dropdown.selected() as usize)
                .cloned()
            else {
                return;
            };
            let on = b.is_active();
            let app = app.clone();
            spawn(
                move || sound::set_mute(dev.id, on),
                move |r| {
                    if let Err(e) = r {
                        app.error("Could not mute", &e);
                    }
                },
            );
        });
    }
    s
}

fn fill(s: &Rc<Section>, devices: &[Device]) {
    let same = {
        let old = s.devices.borrow();
        old.len() == devices.len()
            && old
                .iter()
                .zip(devices)
                .all(|(a, b)| a.id == b.id && a.name == b.name)
    };
    s.syncing.set(true);
    if !same {
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        s.dropdown.set_model(Some(&gtk::StringList::new(&names)));
    }
    *s.devices.borrow_mut() = devices.to_vec();
    if let Some(i) = devices.iter().position(|d| d.is_default) {
        if s.dropdown.selected() != i as u32 {
            s.dropdown.set_selected(i as u32);
        }
        let d = &devices[i];
        // Don't fight the user mid-drag.
        if !s.scale.has_focus() {
            s.scale.set_value(d.volume * 100.0);
        }
        s.mute.set_active(d.muted);
    }
    s.dropdown.set_sensitive(!devices.is_empty());
    s.scale.set_sensitive(!devices.is_empty());
    s.syncing.set(false);
}
