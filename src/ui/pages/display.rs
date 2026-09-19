//! Display: each screen's scale, rotation and position through the compositor's
//! raven_output_layout_v1, and backlight brightness through sysfs.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

/// What the page holds while it is up: the screens as last reported, what
/// has been staged on top of them, and the preview's working state.
struct Page {
    outputs: RefCell<Vec<Output>>,
    changes: RefCell<Vec<Change>>,
    /// A main display picked and not yet applied: `Some(None)` is "none".
    primary: RefCell<Option<Option<String>>>,
    /// Each card's position boxes, by connector, so a drag in the preview
    /// can move them -- and through them stage the position, one path for
    /// both.
    spins: RefCell<HashMap<String, (gtk::SpinButton, gtk::SpinButton)>>,
    /// How the preview maps the desktop onto the widget: offset and scale.
    /// Held still for the length of a drag, or the picture would re-fit
    /// under the pointer as the screen being dragged changes its extent.
    view: Cell<Option<View>>,
    drag: RefCell<Option<Drag>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct View {
    ox: f64,
    oy: f64,
    fit: f64,
}

/// A screen being dragged in the preview.
#[derive(Debug, Clone)]
struct Drag {
    name: String,
    /// Where it was, in desktop pixels, when the drag began.
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// Every other screen, `(x, y, w, h)`.
    others: Vec<(f64, f64, f64, f64)>,
}

impl Page {
    /// The main display as it will be once applied.
    fn primary_name(&self) -> Option<String> {
        match &*self.primary.borrow() {
            Some(staged) => staged.clone(),
            None => self
                .outputs
                .borrow()
                .iter()
                .find(|o| o.primary == Some(true))
                .map(|o| o.name.clone()),
        }
    }

    fn preview(&self) -> Vec<PreviewScreen> {
        preview_screens(
            &self.outputs.borrow(),
            &self.changes.borrow(),
            self.primary_name().as_deref(),
        )
    }

    /// Move a card's position boxes, which stage the position.
    fn set_position(&self, name: &str, x: f64, y: f64) {
        let spins = self.spins.borrow().get(name).cloned();
        if let Some((sx, sy)) = spins {
            sx.set_value(x.round());
            sy.set_value(y.round());
        }
    }

    /// Whether anything is staged at all.
    fn dirty(&self) -> bool {
        !self.changes.borrow().is_empty() || self.primary.borrow().is_some()
    }
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Display", "Screens, arrangement and brightness.");

    let banner = widgets::banner("");
    banner.set_revealed(false);
    content.append(&banner);

    let page = Rc::new(Page {
        outputs: RefCell::new(vec![]),
        changes: RefCell::new(vec![]),
        primary: RefCell::new(None),
        spins: RefCell::new(HashMap::new()),
        view: Cell::new(None),
        drag: RefCell::new(None),
    });

    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    apply.set_halign(gtk::Align::End);
    apply.set_sensitive(false);

    // The arrangement as it will be once applied: every staged position,
    // scale, rotation and main display drawn before anything is sent,
    // numbered the way Huginn numbers the screens themselves. Screens are
    // dragged into place here.
    let identify = gtk::Button::with_label("Identify displays");
    identify.set_tooltip_text(Some("Show each screen's number on it for a few seconds"));
    let arrangement = widgets::card_with_control(
        "Arrangement",
        "Drag screens to match your desk. The main display is 1, the rest left to right",
        &identify,
    );
    let preview = gtk::DrawingArea::new();
    preview.set_content_height(240);
    preview.set_hexpand(true);
    {
        let page = page.clone();
        preview.set_draw_func(move |area, cr, width, height| {
            let screens = page.preview();
            let view = match (page.view.get(), page.drag.borrow().is_some()) {
                (Some(view), true) => view,
                _ => fit_view(&screens, width as f64, height as f64),
            };
            page.view.set(Some(view));
            draw_preview(cr, view, &area.color(), &screens);
        });
    }
    {
        let drag = gtk::GestureDrag::new();
        {
            let page = page.clone();
            drag.connect_drag_begin(move |gesture, wx, wy| {
                let Some(view) = page.view.get() else {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                };
                let (px, py) = ((wx - view.ox) / view.fit, (wy - view.oy) / view.fit);
                let screens = page.preview();
                let Some(hit) = screens
                    .iter()
                    .find(|s| px >= s.x && px < s.x + s.w && py >= s.y && py < s.y + s.h)
                else {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                };
                let others = screens
                    .iter()
                    .filter(|s| s.name != hit.name)
                    .map(|s| (s.x, s.y, s.w, s.h))
                    .collect();
                *page.drag.borrow_mut() = Some(Drag {
                    name: hit.name.clone(),
                    x: hit.x,
                    y: hit.y,
                    w: hit.w,
                    h: hit.h,
                    others,
                });
            });
        }
        {
            let page = page.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                let (Some(d), Some(view)) = (page.drag.borrow().clone(), page.view.get()) else {
                    return;
                };
                let wanted = (d.x + dx / view.fit, d.y + dy / view.fit);
                // Ten widget pixels of pull, whatever the zoom.
                let (x, y) = snap(wanted, (d.w, d.h), &d.others, 10.0 / view.fit);
                page.set_position(&d.name, x, y);
            });
        }
        {
            let page = page.clone();
            let preview = preview.clone();
            drag.connect_drag_end(move |_, dx, dy| {
                let (Some(d), Some(view)) = (page.drag.borrow_mut().take(), page.view.get()) else {
                    return;
                };
                let wanted = (d.x + dx / view.fit, d.y + dy / view.fit);
                let snapped = snap(wanted, (d.w, d.h), &d.others, 10.0 / view.fit);
                // Let go, it joins the desktop: an edge against another
                // screen, and over none of them.
                let (x, y) = attach(snapped, (d.w, d.h), &d.others);
                page.set_position(&d.name, x, y);
                preview.queue_draw();
            });
        }
        preview.add_controller(drag);
    }
    arrangement.append(&preview);
    let main_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    arrangement.append(&main_row);
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

    let load = {
        let screens = screens.clone();
        let banner = banner.clone();
        let page = page.clone();
        let apply = apply.clone();
        let preview = preview.clone();
        let main_row = main_row.clone();
        Rc::new(move || {
            let screens = screens.clone();
            let banner = banner.clone();
            let page = page.clone();
            let apply = apply.clone();
            let preview = preview.clone();
            let main_row = main_row.clone();
            spawn(display::outputs, move |r| match r {
                Ok(list) => {
                    banner.set_revealed(false);
                    *page.outputs.borrow_mut() = list.clone();
                    page.changes.borrow_mut().clear();
                    *page.primary.borrow_mut() = None;
                    page.spins.borrow_mut().clear();
                    apply.set_sensitive(false);
                    widgets::clear_box(&screens);
                    let numbers = screen_numbers(&list, primary_index(&list));
                    let mut order: Vec<usize> = (0..list.len()).collect();
                    order.sort_by_key(|&i| numbers[i]);
                    for &i in &order {
                        screens.append(&screen_card(&list[i], numbers[i], &page, &apply, &preview));
                    }
                    fill_main_row(&main_row, &list, &numbers, &order, &page, &apply, &preview);
                    preview.queue_draw();
                    screens.append(&apply);
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
        let page = page.clone();
        let load = load.clone();
        apply.connect_clicked(move |b| {
            b.set_sensitive(false);
            let staged = with_every_position(&page.outputs.borrow(), &page.changes.borrow());
            let primary = page.primary.borrow().clone();
            let app = app.clone();
            let load = load.clone();
            spawn(
                move || display::apply(&staged, primary),
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

/// The "Main display" picker under the preview.
fn fill_main_row(
    row: &gtk::Box,
    list: &[Output],
    numbers: &[u32],
    order: &[usize],
    page: &Rc<Page>,
    apply: &gtk::Button,
    preview: &gtk::DrawingArea,
) {
    widgets::clear_box(row);
    row.append(&label("Main display"));
    let mut names: Vec<String> = vec!["None (panels follow the pointer)".into()];
    let mut choices: Vec<Option<String>> = vec![None];
    for &i in order {
        names.push(format!("Display {} ({})", numbers[i], list[i].name));
        choices.push(Some(list[i].name.clone()));
    }
    let strings: Vec<&str> = names.iter().map(String::as_str).collect();
    let dd = gtk::DropDown::from_strings(&strings);
    let current = list.iter().find(|o| o.primary == Some(true)).map(|o| o.name.clone());
    let selected = choices.iter().position(|c| *c == current).unwrap_or(0);
    dd.set_selected(selected as u32);
    dd.set_tooltip_text(Some(
        "The dock and panels stay on it, new windows open there, and focus starts there",
    ));
    if list.iter().all(|o| o.primary.is_none()) {
        dd.set_sensitive(false);
        dd.set_tooltip_text(Some(
            "This compositor cannot set a main display. Update RavenGUI and log in again.",
        ));
    }
    {
        let page = page.clone();
        let apply = apply.clone();
        let preview = preview.clone();
        dd.connect_selected_notify(move |dd| {
            let pick = choices.get(dd.selected() as usize).cloned().flatten();
            *page.primary.borrow_mut() = (pick != current).then_some(pick);
            apply.set_sensitive(page.dirty());
            preview.queue_draw();
        });
    }
    row.append(&dd);
}

/// The index of the main screen in `list`, as the compositor reported it.
fn primary_index(list: &[Output]) -> Option<usize> {
    list.iter().position(|o| o.primary == Some(true))
}

/// Each screen's number, as Huginn badges it: the main screen 1, then the
/// rest left to right and top to bottom. Keep in step with
/// `huginn-comp/src/overview.rs`'s `screen_numbers`.
fn screen_numbers(outputs: &[Output], primary: Option<usize>) -> Vec<u32> {
    let mut order: Vec<usize> = (0..outputs.len()).collect();
    order.sort_by_key(|&i| (Some(i) != primary, outputs[i].x, outputs[i].y));
    let mut numbers = vec![0; outputs.len()];
    for (rank, i) in order.into_iter().enumerate() {
        numbers[i] = rank as u32 + 1;
    }
    numbers
}

/// The staged changes, with every screen's position pinned whenever any
/// position is being set.
///
/// Huginn places a screen with no saved position to the right of the
/// others. Dragging one screen and sending only its position would leave
/// the rest to that rule, which is not the arrangement on the page -- so the
/// whole arrangement is sent, exactly as drawn.
fn with_every_position(outputs: &[Output], changes: &[Change]) -> Vec<Change> {
    let mut out = changes.to_vec();
    if !out.iter().any(|c| c.position.is_some()) {
        return out;
    }
    for o in outputs {
        match out.iter_mut().find(|c| c.name == o.name) {
            Some(c) if c.position.is_none() => c.position = Some((o.x, o.y)),
            Some(_) => {}
            None => out.push(Change {
                name: o.name.clone(),
                position: Some((o.x, o.y)),
                ..Change::default()
            }),
        }
    }
    out
}

/// Pull a dragged screen's edges onto the edges of the screens around it
/// when they come within `threshold` desktop pixels: side by side, or lined
/// up along the same edge. Each axis is snapped on its own, to the nearest.
fn snap(
    (x, y): (f64, f64),
    (w, h): (f64, f64),
    others: &[(f64, f64, f64, f64)],
    threshold: f64,
) -> (f64, f64) {
    let nearest = |at: f64, candidates: Vec<f64>| {
        candidates
            .into_iter()
            .map(|c| (c, (c - at).abs()))
            .filter(|&(_, d)| d <= threshold)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(at, |(c, _)| c)
    };
    let xs = others
        .iter()
        .flat_map(|&(ox, _, ow, _)| [ox - w, ox + ow, ox, ox + ow - w])
        .collect();
    let ys = others
        .iter()
        .flat_map(|&(_, oy, _, oh)| [oy - h, oy + oh, oy, oy + oh - h])
        .collect();
    (nearest(x, xs), nearest(y, ys))
}

/// Where a dropped screen goes: where it was let go if it already sits edge
/// to edge with another screen and over none, and otherwise the nearest
/// place that does -- beside, above or below one of the others. A screen on
/// its own, or cut off by a gap, is one the pointer cannot reach.
fn attach(
    (x, y): (f64, f64),
    (w, h): (f64, f64),
    others: &[(f64, f64, f64, f64)],
) -> (f64, f64) {
    let (x, y) = (x.round(), y.round());
    if others.is_empty() {
        return (x, y);
    }
    let overlaps = |x: f64, y: f64| {
        others
            .iter()
            .any(|&(ox, oy, ow, oh)| x < ox + ow && ox < x + w && y < oy + oh && oy < y + h)
    };
    let touches = |x: f64, y: f64| {
        others.iter().any(|&(ox, oy, ow, oh)| {
            let across = y < oy + oh && oy < y + h;
            let along = x < ox + ow && ox < x + w;
            (across && (x + w == ox || ox + ow == x)) || (along && (y + h == oy || oy + oh == y))
        })
    };
    if !overlaps(x, y) && touches(x, y) {
        return (x, y);
    }
    others
        .iter()
        .flat_map(|&(ox, oy, ow, oh)| {
            // Kept overlapping along the shared edge by at least a pixel, so
            // the two really do meet.
            let cy = y.clamp(oy - h + 1.0, oy + oh - 1.0);
            let cx = x.clamp(ox - w + 1.0, ox + ow - 1.0);
            [(ox + ow, cy), (ox - w, cy), (cx, oy - h), (cx, oy + oh)]
        })
        .filter(|&(cx, cy)| !overlaps(cx, cy))
        .min_by(|a, b| {
            let d = |p: &(f64, f64)| (p.0 - x).hypot(p.1 - y);
            d(a).total_cmp(&d(b))
        })
        .unwrap_or((x, y))
}

fn screen_card(
    o: &Output,
    number: u32,
    page: &Rc<Page>,
    apply: &gtk::Button,
    preview: &gtk::DrawingArea,
) -> gtk::Box {
    let mut title = format!("Display {number}  ({})", o.name);
    if o.primary == Some(true) {
        title.push_str("  · Main");
    }
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
    page
        .spins
        .borrow_mut()
        .insert(o.name.clone(), (x.clone(), y.clone()));

    let name = o.name.clone();
    let stage = {
        let page = page.clone();
        let apply = apply.clone();
        let scale_dd = scale_dd.clone();
        let rotate_dd = rotate_dd.clone();
        let preview = preview.clone();
        let x = x.clone();
        let y = y.clone();
        let orig = o.clone();
        Rc::new(move || {
            let mut list = page.changes.borrow_mut();
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
            drop(list);
            apply.set_sensitive(page.dirty());
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
    primary: bool,
}

/// The screens with every staged change folded in. A quarter turn that
/// differs from the current one trades width and height; a new scale sizes
/// the screen by how much denser or sparser it gets. "Automatic" is left at
/// the size it has now, since only the compositor knows what that works out
/// to -- Apply shows the truth.
fn preview_screens(
    outputs: &[Output],
    changes: &[Change],
    primary: Option<&str>,
) -> Vec<PreviewScreen> {
    // Numbered as they are now, so the preview agrees with the cards and
    // with the badges on the screens; Apply renumbers all three together.
    let numbers = screen_numbers(outputs, primary_index(outputs));
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
                primary: primary == Some(o.name.as_str()),
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

/// Fit the whole desktop into the widget, with room round the edge, centred.
fn fit_view(screens: &[PreviewScreen], width: f64, height: f64) -> View {
    if screens.is_empty() {
        return View { ox: 0.0, oy: 0.0, fit: 1.0 };
    }
    let left = screens.iter().map(|s| s.x).fold(f64::INFINITY, f64::min);
    let top = screens.iter().map(|s| s.y).fold(f64::INFINITY, f64::min);
    let right = screens.iter().map(|s| s.x + s.w).fold(f64::NEG_INFINITY, f64::max);
    let bottom = screens.iter().map(|s| s.y + s.h).fold(f64::NEG_INFINITY, f64::max);
    // Room to drag a screen out past the others without leaving the widget.
    let pad = 28.0;
    let fit = ((width - pad * 2.0) / (right - left).max(1.0))
        .min((height - pad * 2.0) / (bottom - top).max(1.0))
        .max(1e-4);
    View {
        ox: (width - (right - left) * fit) / 2.0 - left * fit,
        oy: (height - (bottom - top) * fit) / 2.0 - top * fit,
        fit,
    }
}

fn draw_preview(
    cr: &gtk::cairo::Context,
    View { ox, oy, fit }: View,
    fg: &gtk::gdk::RGBA,
    screens: &[PreviewScreen],
) {
    use gtk::cairo::{FontSlant, FontWeight};
    let (r, g, b) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
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

        // The main display says so, in the top-left corner.
        if s.primary && w > 50.0 && h > 40.0 {
            cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Bold);
            cr.set_font_size(10.0);
            cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
            cr.move_to(x + 8.0, y + 16.0);
            let _ = cr.show_text("MAIN");
        }

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
            primary: Some(false),
        }
    }

    #[test]
    fn screens_are_numbered_left_to_right_whatever_order_they_come_in() {
        let list = [output("HDMI-A-1", 1920, 2560, 1440, 0), output("eDP-1", 0, 1920, 1080, 0)];
        assert_eq!(screen_numbers(&list, None), vec![2, 1]);
        assert_eq!(screen_numbers(&list, Some(0)), vec![1, 2], "the main one is 1");
    }

    const LAPTOP: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1080.0);

    #[test]
    fn a_screen_dragged_near_an_edge_snaps_onto_it() {
        // Dropped a little right of the laptop and a little low: pulled flush
        // to its right edge and level with its top.
        let at = snap((1935.0, 8.0), (2560.0, 1440.0), &[LAPTOP], 20.0);
        assert_eq!(at, (1920.0, 0.0));
        // Too far to pull: left where it was.
        assert_eq!(snap((2100.0, 300.0), (2560.0, 1440.0), &[LAPTOP], 20.0), (2100.0, 300.0));
    }

    #[test]
    fn a_dropped_screen_touching_another_stays_put() {
        assert_eq!(attach((1920.0, 300.0), (2560.0, 1440.0), &[LAPTOP]), (1920.0, 300.0));
        // Above, offset to the right, still sharing some of the edge.
        assert_eq!(attach((500.0, -1440.0), (2560.0, 1440.0), &[LAPTOP]), (500.0, -1440.0));
    }

    #[test]
    fn a_screen_left_floating_or_overlapping_is_moved_to_the_nearest_edge() {
        // A gap to the right: pulled back against the laptop.
        assert_eq!(attach((2200.0, 100.0), (1920.0, 1080.0), &[LAPTOP]), (1920.0, 100.0));
        // Dropped mostly on top of it, towards the bottom: out below.
        assert_eq!(attach((200.0, 900.0), (1920.0, 1080.0), &[LAPTOP]), (200.0, 1080.0));
        // Only corner to corner: slid until they share an edge.
        let (x, y) = attach((1920.0, 1080.0), (1920.0, 1080.0), &[LAPTOP]);
        assert!(x < 1920.0 || y < 1080.0, "({x}, {y}) still only meets at a corner");
    }

    #[test]
    fn a_drop_between_two_screens_lands_on_neither() {
        let right = (1920.0, 0.0, 1920.0, 1080.0);
        let (x, y) = attach((1000.0, 200.0), (1920.0, 1080.0), &[LAPTOP, right]);
        for (ox, oy, ow, oh) in [LAPTOP, right] {
            assert!(!(x < ox + ow && ox < x + 1920.0 && y < oy + oh && oy < y + 1080.0));
        }
    }

    #[test]
    fn moving_one_screen_pins_every_other_where_it_is() {
        let list = [output("eDP-1", 0, 1920, 1080, 0), output("DP-1", 1920, 2560, 1440, 0)];
        let moved = [Change {
            name: "DP-1".into(),
            position: Some((0, -1440)),
            ..Change::default()
        }];
        let sent = with_every_position(&list, &moved);
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().any(|c| c.name == "eDP-1" && c.position == Some((0, 0))));
        assert!(sent.iter().any(|c| c.name == "DP-1" && c.position == Some((0, -1440))));
        // Nothing moved: nothing pinned.
        let scaled = [Change {
            name: "DP-1".into(),
            scale: Some(1.5),
            ..Change::default()
        }];
        assert_eq!(with_every_position(&list, &scaled).len(), 1);
    }

    #[test]
    fn the_staged_main_display_is_marked_in_the_preview() {
        let list = [output("eDP-1", 0, 1920, 1080, 0), output("DP-1", 1920, 2560, 1440, 0)];
        let p = preview_screens(&list, &[], Some("DP-1"));
        assert!(!p[0].primary && p[1].primary);
    }

    #[test]
    fn a_staged_quarter_turn_stands_the_preview_on_its_side() {
        let list = [output("DP-1", 0, 2560, 1440, 0)];
        let turned = [Change {
            name: "DP-1".into(),
            rotation: Some(1),
            ..Change::default()
        }];
        let p = &preview_screens(&list, &turned, None)[0];
        assert_eq!((p.w, p.h), (1440.0, 2560.0));
        assert_eq!(orientation_text(p), "Portrait · 90°");
        // A half turn keeps the shape.
        let flipped = [Change {
            name: "DP-1".into(),
            rotation: Some(2),
            ..Change::default()
        }];
        let p = &preview_screens(&list, &flipped, None)[0];
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
        let p = &preview_screens(&list, &upright, None)[0];
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
        let p = &preview_screens(&list, &staged, None)[0];
        assert_eq!((p.x, p.y, p.w, p.h), (100.0, 50.0, 1280.0, 720.0));
    }
}
