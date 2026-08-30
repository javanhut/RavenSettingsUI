//! Display: each screen's scale and position through the compositor's
//! raven_output_layout_v1, and backlight brightness through sysfs.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::backend::display::{self, Backlight, Change, Output};
use crate::ui::{spawn, widgets, App};

const SCALES: [(&str, f64); 6] = [
    ("Automatic", 0.0),
    ("100%", 1.0),
    ("125%", 1.25),
    ("150%", 1.5),
    ("175%", 1.75),
    ("200%", 2.0),
];

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Display", "Screens, arrangement and brightness.");

    let banner = widgets::banner("");
    banner.set_revealed(false);
    content.append(&banner);

    let screens = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.append(&screens);

    let (bl_card, bl_body) = widgets::card("Brightness", "Built-in display backlight");
    content.append(&bl_card);

    let outputs: Rc<RefCell<Vec<Output>>> = Rc::new(RefCell::new(vec![]));
    let changes: Rc<RefCell<Vec<Change>>> = Rc::new(RefCell::new(vec![]));

    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    apply.set_halign(gtk::Align::End);
    apply.set_sensitive(false);

    let load = {
        let app = app.clone();
        let screens = screens.clone();
        let banner = banner.clone();
        let outputs = outputs.clone();
        let changes = changes.clone();
        let apply = apply.clone();
        Rc::new(move || {
            let app = app.clone();
            let screens = screens.clone();
            let banner = banner.clone();
            let outputs = outputs.clone();
            let changes = changes.clone();
            let apply = apply.clone();
            spawn(display::outputs, move |r| match r {
                Ok(list) => {
                    banner.set_revealed(false);
                    *outputs.borrow_mut() = list.clone();
                    changes.borrow_mut().clear();
                    apply.set_sensitive(false);
                    widgets::clear_box(&screens);
                    for o in &list {
                        screens.append(&screen_card(o, &changes, &apply));
                    }
                    screens.append(&apply);
                    let _ = &app;
                }
                Err(e) => {
                    banner.set_title(&format!("Screens cannot be arranged here: {e}"));
                    banner.set_revealed(true);
                }
            });
        })
    };
    {
        let app = app.clone();
        let changes = changes.clone();
        let load = load.clone();
        apply.connect_clicked(move |b| {
            b.set_sensitive(false);
            let staged = changes.borrow().clone();
            let app = app.clone();
            let load = load.clone();
            spawn(
                move || display::apply(&staged),
                move |r| {
                    match r {
                        Ok(_) => app.toast("Display settings applied"),
                        Err(e) => app.error("Could not apply", &e),
                    }
                    load();
                },
            );
        });
    }
    {
        let load = load.clone();
        let app = app.clone();
        let bl_body = bl_body.clone();
        root.connect_map(move |_| {
            load();
            fill_backlight(&app, &bl_body);
        });
    }
    root.upcast()
}

fn screen_card(o: &Output, changes: &Rc<RefCell<Vec<Change>>>, apply: &gtk::Button) -> gtk::Box {
    let mut title = o.name.clone();
    if o.focused {
        title.push_str("  (focused)");
    }
    let mut sub = format!("{}×{} px", o.physical_width, o.physical_height);
    if let Some(inch) = o.diagonal_inches() {
        sub.push_str(&format!(" · {inch:.1}″"));
    }
    sub.push_str(&format!(
        " · {}×{} logical at {}×",
        o.width, o.height, o.scale
    ));
    let (card, body) = widgets::card(&title, &sub);
    let grid = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(8)
        .build();

    let scale_dd = gtk::DropDown::from_strings(&SCALES.map(|s| s.0));
    let current = SCALES
        .iter()
        .position(|(_, s)| (s - o.scale).abs() < 0.01)
        .unwrap_or(0);
    scale_dd.set_selected(current as u32);
    grid.attach(&label("Scale"), 0, 0, 1, 1);
    grid.attach(&scale_dd, 1, 0, 1, 1);

    let x = gtk::SpinButton::with_range(-16384.0, 16384.0, 1.0);
    x.set_value(o.x as f64);
    let y = gtk::SpinButton::with_range(-16384.0, 16384.0, 1.0);
    y.set_value(o.y as f64);
    grid.attach(&label("Position"), 0, 1, 1, 1);
    let pos = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    pos.append(&x);
    pos.append(&gtk::Label::new(Some("×")));
    pos.append(&y);
    grid.attach(&pos, 1, 1, 1, 1);
    body.append(&grid);

    let name = o.name.clone();
    let stage = {
        let changes = changes.clone();
        let apply = apply.clone();
        let scale_dd = scale_dd.clone();
        let x = x.clone();
        let y = y.clone();
        let orig = o.clone();
        Rc::new(move || {
            let mut list = changes.borrow_mut();
            list.retain(|c| c.name != name);
            let scale = SCALES[scale_dd.selected() as usize].1;
            let mut change = Change {
                name: name.clone(),
                position: None,
                scale: None,
            };
            if (scale - orig.scale).abs() > 0.01 || (scale == 0.0 && scale_dd.selected() == 0) {
                change.scale = Some(scale);
            }
            let (nx, ny) = (x.value() as i32, y.value() as i32);
            if nx != orig.x || ny != orig.y {
                change.position = Some((nx, ny));
            }
            if change.scale.is_some() || change.position.is_some() {
                list.push(change);
            }
            apply.set_sensitive(!list.is_empty());
        })
    };
    {
        let stage = stage.clone();
        scale_dd.connect_selected_notify(move |_| stage());
    }
    {
        let stage = stage.clone();
        x.connect_value_changed(move |_| stage());
    }
    y.connect_value_changed(move |_| stage());
    card
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.add_css_class("dim");
    l
}

fn fill_backlight(app: &Rc<App>, body: &gtk::Box) {
    widgets::clear_box(body);
    let lights = display::backlights();
    if lights.is_empty() {
        body.append(&widgets::dim_label(
            "No backlight device (an external monitor sets its own brightness).",
        ));
        return;
    }
    for bl in lights {
        if !bl.writable {
            let b = widgets::banner("Brightness is read-only for this account. Add the udev rule from the README to let the video group set it.");
            b.set_button_label(Some("Copy rule"));
            let app2 = app.clone();
            b.connect_button_clicked(move |_| {
                app2.window().clipboard().set_text(&format!(
                    "echo '{}' | sudo tee /etc/udev/rules.d/90-backlight.rules && sudo chgrp video /sys/class/backlight/*/brightness && sudo chmod g+w /sys/class/backlight/*/brightness",
                    display::UDEV_RULE
                ));
                app2.toast("Command copied");
            });
            body.append(&b);
        }
        body.append(&brightness_row(app, bl));
    }
}

fn brightness_row(app: &Rc<App>, bl: Backlight) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&gtk::Image::from_icon_name("display-brightness-symbolic"));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 100.0, 1.0);
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_format_value_func(|_, v| format!("{v:.0}%"));
    scale.set_value(bl.percent());
    scale.set_sensitive(bl.writable);
    let app = app.clone();
    scale.connect_value_changed(move |s| {
        let v = s.value();
        let bl = bl.clone();
        let app = app.clone();
        spawn(
            move || display::set_brightness(&bl, v),
            move |r| {
                if let Err(e) = r {
                    app.error("Brightness", &e);
                }
            },
        );
    });
    row.append(&scale);
    row
}
