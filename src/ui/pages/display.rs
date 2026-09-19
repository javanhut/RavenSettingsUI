//! Display: each screen's scale, rotation and position through the compositor's
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

/// Quarter turns counter-clockwise, as the protocol numbers them. Named by
/// the angle rather than "portrait left/right": which of 90 and 270 suits a
/// monitor depends on which way it was stood up, and trying one shows it.
const ROTATIONS: [(&str, u32); 4] = [
    ("Normal", 0),
    ("90°", 1),
    ("180° (upside down)", 2),
    ("270°", 3),
];

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Display", "Screens, arrangement and brightness.");

    let banner = widgets::banner("");
    banner.set_revealed(false);
    content.append(&banner);

    let outputs: Rc<RefCell<Vec<Output>>> = Rc::new(RefCell::new(vec![]));
    let changes: Rc<RefCell<Vec<Change>>> = Rc::new(RefCell::new(vec![]));

    // The arrangement as it will be once applied: every staged position,
    // scale and rotation drawn before anything is sent, numbered the way
    // Huginn numbers the screens themselves.
    let identify = gtk::Button::with_label("Identify displays");
    identify.set_tooltip_text(Some("Show each screen's number on it for a few seconds"));
    let arrangement = widgets::card_with_control(
        "Arrangement",
        "Numbered left to right, as the overview and Identify show them",
        &identify,
    );
    let preview = gtk::DrawingArea::new();
    preview.set_content_height(220);
    preview.set_hexpand(true);
    {
        let outputs = outputs.clone();
        let changes = changes.clone();
        preview.set_draw_func(move |area, cr, width, height| {
            let screens = preview_screens(&outputs.borrow(), &changes.borrow());
            draw_preview(cr, width as f64, height as f64, &area.color(), &screens);
        });
    }
    arrangement.append(&preview);
    content.append(&arrangement);
    {
        let app = app.clone();
        identify.connect_clicked(move |b| {
            b.set_sensitive(false);
            let app = app.clone();
            let b = b.clone();
            spawn(display::identify, move |r| {
                if let Err(e) = r {
                    app.error("Could not identify displays", &e);
                }
                b.set_sensitive(true);
            });
        });
    }

    let screens = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.append(&screens);

    let (bl_card, bl_body) = widgets::card("Brightness", "Built-in display backlight");
    content.append(&bl_card);

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
        let preview = preview.clone();
        Rc::new(move || {
            let app = app.clone();
            let screens = screens.clone();
            let banner = banner.clone();
            let outputs = outputs.clone();
            let changes = changes.clone();
            let apply = apply.clone();
            let preview = preview.clone();
            spawn(display::outputs, move |r| match r {
                Ok(list) => {
                    banner.set_revealed(false);
                    *outputs.borrow_mut() = list.clone();
                    changes.borrow_mut().clear();
                    apply.set_sensitive(false);
                    widgets::clear_box(&screens);
                    let numbers = screen_numbers(&list);
                    for (o, number) in list.iter().zip(numbers) {
                        screens.append(&screen_card(o, number, &changes, &apply, &preview));
                    }
                    preview.queue_draw();
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

/// Each screen's number, as Huginn badges it in the overview: left to
/// right, then top to bottom, from 1. Keep in step with
/// `huginn-comp/src/overview.rs`'s `screen_numbers`.
fn screen_numbers(outputs: &[Output]) -> Vec<u32> {
    let mut order: Vec<usize> = (0..outputs.len()).collect();
    order.sort_by_key(|&i| (outputs[i].x, outputs[i].y));
    let mut numbers = vec![0; outputs.len()];
    for (rank, i) in order.into_iter().enumerate() {
        numbers[i] = rank as u32 + 1;
    }
    numbers
}

fn screen_card(
    o: &Output,
    number: u32,
    changes: &Rc<RefCell<Vec<Change>>>,
    apply: &gtk::Button,
    preview: &gtk::DrawingArea,
) -> gtk::Box {
    let mut title = format!("Display {number}  ({})", o.name);
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

    let rotate_dd = gtk::DropDown::from_strings(&ROTATIONS.map(|r| r.0));
    rotate_dd.set_selected(o.rotation.unwrap_or(0).min(3));
    if o.rotation.is_none() {
        rotate_dd.set_sensitive(false);
        rotate_dd.set_tooltip_text(Some(
            "This compositor cannot rotate screens. Update RavenGUI and log in again.",
        ));
    }
    grid.attach(&label("Rotation"), 0, 2, 1, 1);
    grid.attach(&rotate_dd, 1, 2, 1, 1);

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
        let rotate_dd = rotate_dd.clone();
        let preview = preview.clone();
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
                rotation: None,
            };
            let rotation = ROTATIONS[rotate_dd.selected() as usize].1;
            if orig.rotation.is_some_and(|r| r != rotation) {
                change.rotation = Some(rotation);
            }
            if (scale - orig.scale).abs() > 0.01 || (scale == 0.0 && scale_dd.selected() == 0) {
                change.scale = Some(scale);
            }
            let (nx, ny) = (x.value() as i32, y.value() as i32);
            if nx != orig.x || ny != orig.y {
                change.position = Some((nx, ny));
            }
            if change.scale.is_some() || change.position.is_some() || change.rotation.is_some() {
                list.push(change);
            }
            apply.set_sensitive(!list.is_empty());
            drop(list);
            preview.queue_draw();
        })
    };
    {
        let stage = stage.clone();
        scale_dd.connect_selected_notify(move |_| stage());
    }
    {
        let stage = stage.clone();
        rotate_dd.connect_selected_notify(move |_| stage());
    }
    {
        let stage = stage.clone();
        x.connect_value_changed(move |_| stage());
    }
    y.connect_value_changed(move |_| stage());
    card
}

/// One screen as the preview draws it: where it will be and what shape,
/// once the staged changes are applied.
#[derive(Debug, Clone, PartialEq)]
struct PreviewScreen {
    number: u32,
    name: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// Quarter turns counter-clockwise, 0–3.
    rotation: u32,
    focused: bool,
}

/// The screens with every staged change folded in. A quarter turn that
/// differs from the current one trades width and height; a new scale sizes
/// the screen by how much denser or sparser it gets. "Automatic" is left at
/// the size it has now, since only the compositor knows what that works out
/// to -- Apply shows the truth.
fn preview_screens(outputs: &[Output], changes: &[Change]) -> Vec<PreviewScreen> {
    let numbers = screen_numbers(outputs);
    outputs
        .iter()
        .zip(numbers)
        .map(|(o, number)| {
            let change = changes.iter().find(|c| c.name == o.name);
            let now = o.rotation.unwrap_or(0);
            let rotation = change.and_then(|c| c.rotation).unwrap_or(now);
            let (mut w, mut h) = (o.width as f64, o.height as f64);
            if rotation % 2 != now % 2 {
                std::mem::swap(&mut w, &mut h);
            }
            if let Some(scale) = change.and_then(|c| c.scale).filter(|s| *s > 0.0) {
                let factor = o.scale / scale;
                w *= factor;
                h *= factor;
            }
            let (x, y) = change.and_then(|c| c.position).unwrap_or((o.x, o.y));
            PreviewScreen {
                number,
                name: o.name.clone(),
                x: x as f64,
                y: y as f64,
                w,
                h,
                rotation,
                focused: o.focused,
            }
        })
        .collect()
}

/// What the screen's shape is and how far it is turned, in words.
fn orientation_text(screen: &PreviewScreen) -> String {
    let shape = if screen.w >= screen.h { "Landscape" } else { "Portrait" };
    match screen.rotation {
        0 => shape.to_owned(),
        r => format!("{shape} · {}°", r * 90),
    }
}

/// Huginn's accent and panel colours: the badge here is the badge on the
/// screen, so the two can be matched by eye.
const ACCENT: (f64, f64, f64) = (0x7A as f64 / 255.0, 0xA2 as f64 / 255.0, 0xF7 as f64 / 255.0);
const BADGE_TEXT: (f64, f64, f64) = (0x16 as f64 / 255.0, 0x16 as f64 / 255.0, 0x1F as f64 / 255.0);

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// Text centred on `cx`, with its middle at `cy`.
fn centred_text(cr: &gtk::cairo::Context, text: &str, cx: f64, cy: f64) {
    if let Ok(ext) = cr.text_extents(text) {
        cr.move_to(
            cx - ext.width() / 2.0 - ext.x_bearing(),
            cy - ext.height() / 2.0 - ext.y_bearing(),
        );
        let _ = cr.show_text(text);
    }
}

fn draw_preview(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    fg: &gtk::gdk::RGBA,
    screens: &[PreviewScreen],
) {
    use gtk::cairo::{FontSlant, FontWeight};
    let (r, g, b) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
    if screens.is_empty() {
        return;
    }
    // Fit the whole desktop, with room round the edge, and centre it.
    let left = screens.iter().map(|s| s.x).fold(f64::INFINITY, f64::min);
    let top = screens.iter().map(|s| s.y).fold(f64::INFINITY, f64::min);
    let right = screens.iter().map(|s| s.x + s.w).fold(f64::NEG_INFINITY, f64::max);
    let bottom = screens.iter().map(|s| s.y + s.h).fold(f64::NEG_INFINITY, f64::max);
    let pad = 16.0;
    let fit = ((width - pad * 2.0) / (right - left).max(1.0))
        .min((height - pad * 2.0) / (bottom - top).max(1.0));
    let ox = (width - (right - left) * fit) / 2.0 - left * fit;
    let oy = (height - (bottom - top) * fit) / 2.0 - top * fit;
    // A hairline apart, so screens that touch still read as two.
    let gap = 3.0;

    for s in screens {
        let (x, y) = (ox + s.x * fit + gap, oy + s.y * fit + gap);
        let (w, h) = ((s.w * fit - gap * 2.0).max(4.0), (s.h * fit - gap * 2.0).max(4.0));

        rounded(cr, x, y, w, h, 8.0);
        cr.set_source_rgba(r, g, b, 0.08);
        let _ = cr.fill_preserve();
        if s.focused {
            cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
            cr.set_line_width(2.0);
        } else {
            cr.set_source_rgba(r, g, b, 0.3);
            cr.set_line_width(1.0);
        }
        let _ = cr.stroke();

        // The number, in the square the screen shows in its corner.
        let side = (w.min(h) * 0.38).clamp(18.0, 56.0);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0 - side * 0.3);
        rounded(cr, cx - side / 2.0, cy - side / 2.0, side, side, side * 0.22);
        cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
        let _ = cr.fill();
        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Bold);
        cr.set_font_size(side * 0.6);
        cr.set_source_rgb(BADGE_TEXT.0, BADGE_TEXT.1, BADGE_TEXT.2);
        centred_text(cr, &s.number.to_string(), cx, cy);

        // The connector and the orientation under it, when there is room.
        let below = cy + side / 2.0;
        if h > side * 2.2 && w > 60.0 {
            cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
            cr.set_font_size(11.0);
            cr.set_source_rgba(r, g, b, 0.85);
            centred_text(cr, &s.name, cx, below + 14.0);
            cr.set_source_rgba(r, g, b, 0.6);
            centred_text(cr, &orientation_text(s), cx, below + 29.0);
        }
    }
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
            let b = widgets::banner("Brightness is read-only for this account. A udev rule lets the video group set it.");
            b.set_button_label(Some("Install rule…"));
            let app2 = app.clone();
            b.connect_button_clicked(move |_| {
                // /etc/udev/rules.d and the sysfs node are root's, and
                // raven-controlsd's socket covers the keyboard backlight
                // and fans, not the display, so the rule is installed in a
                // terminal the user authorises (see backend::terminal).
                let cmd = display::udev_rule_command();
                let app3 = app2.clone();
                crate::ui::offer_terminal(
                    &app2,
                    "Let this account set brightness?",
                    "The rule hands /sys/class/backlight/*/brightness to the video group, which the session already holds for the screen. It is applied to the current device too, so no reboot is needed.",
                    &cmd,
                    move |ran| {
                        if ran {
                            app3.toast("Reopen Display once it finishes");
                        }
                    },
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, x: i32, w: i32, h: i32, rotation: u32) -> Output {
        Output {
            name: name.into(),
            x,
            y: 0,
            width: w,
            height: h,
            scale: 1.0,
            physical_width: w,
            physical_height: h,
            mm_width: 0,
            mm_height: 0,
            focused: false,
            rotation: Some(rotation),
        }
    }

    #[test]
    fn screens_are_numbered_left_to_right_whatever_order_they_come_in() {
        let list = [output("HDMI-A-1", 1920, 2560, 1440, 0), output("eDP-1", 0, 1920, 1080, 0)];
        assert_eq!(screen_numbers(&list), vec![2, 1]);
    }

    #[test]
    fn a_staged_quarter_turn_stands_the_preview_on_its_side() {
        let list = [output("DP-1", 0, 2560, 1440, 0)];
        let turned = [Change {
            name: "DP-1".into(),
            rotation: Some(1),
            ..Change::default()
        }];
        let p = &preview_screens(&list, &turned)[0];
        assert_eq!((p.w, p.h), (1440.0, 2560.0));
        assert_eq!(orientation_text(p), "Portrait · 90°");
        // A half turn keeps the shape.
        let flipped = [Change {
            name: "DP-1".into(),
            rotation: Some(2),
            ..Change::default()
        }];
        let p = &preview_screens(&list, &flipped)[0];
        assert_eq!((p.w, p.h), (2560.0, 1440.0));
        assert_eq!(orientation_text(p), "Landscape · 180°");
    }

    #[test]
    fn turning_a_portrait_screen_back_makes_it_wide_again() {
        // Reported already turned: 1440 wide, 2560 tall.
        let list = [output("DP-1", 0, 1440, 2560, 1)];
        let upright = [Change {
            name: "DP-1".into(),
            rotation: Some(0),
            ..Change::default()
        }];
        let p = &preview_screens(&list, &upright)[0];
        assert_eq!((p.w, p.h), (2560.0, 1440.0));
    }

    #[test]
    fn a_staged_scale_and_position_move_and_resize_the_preview() {
        let list = [output("DP-1", 0, 2560, 1440, 0)];
        let staged = [Change {
            name: "DP-1".into(),
            scale: Some(2.0),
            position: Some((100, 50)),
            ..Change::default()
        }];
        let p = &preview_screens(&list, &staged)[0];
        assert_eq!((p.x, p.y, p.w, p.h), (100.0, 50.0, 1280.0, 720.0));
    }
}
