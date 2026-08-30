//! Building blocks shared by the pages: cards, page headers, rows.

use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

/// A page: title, subtitle, and a vertical content box inside a scroller.
pub fn page(title: &str, subtitle: &str) -> (gtk::ScrolledWindow, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_start(26);
    content.set_margin_end(26);
    content.set_margin_top(20);
    content.set_margin_bottom(26);

    let head = gtk::Box::new(gtk::Orientation::Vertical, 4);
    head.add_css_class("page-head");
    let t = gtk::Label::new(Some(title));
    t.add_css_class("page-title");
    t.set_xalign(0.0);
    head.append(&t);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("page-subtitle");
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_margin_bottom(6);
    head.append(&s);
    content.append(&head);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&content)
        .build();
    (scroller, content)
}

/// A card with a title/subtitle header. Returns (card, body) where body is
/// the vertical box to fill.
pub fn card(title: &str, subtitle: &str) -> (gtk::Box, gtk::Box) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.add_css_class("raven-card");
    if !title.is_empty() {
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let t = gtk::Label::new(Some(title));
        t.add_css_class("card-title");
        t.set_xalign(0.0);
        head.append(&t);
        if !subtitle.is_empty() {
            let s = gtk::Label::new(Some(subtitle));
            s.add_css_class("card-subtitle");
            s.set_xalign(0.0);
            s.set_wrap(true);
            head.append(&s);
        }
        outer.append(&head);
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&body);
    (outer, body)
}

/// A card whose header sits left and a single control sits right.
pub fn card_with_control(title: &str, subtitle: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    outer.add_css_class("raven-card");
    let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
    head.set_hexpand(true);
    head.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("card-title");
    t.set_xalign(0.0);
    head.append(&t);
    if !subtitle.is_empty() {
        let s = gtk::Label::new(Some(subtitle));
        s.add_css_class("card-subtitle");
        s.set_xalign(0.0);
        s.set_wrap(true);
        head.append(&s);
    }
    outer.append(&head);
    control.set_valign(gtk::Align::Center);
    outer.append(control);
    outer
}

/// Icon + label + switch, for the stacked toggle cards.
pub fn toggle_row(icon: &str, label: &str, on: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("dim");
    row.append(&i);
    let l = gtk::Label::new(Some(label));
    l.set_xalign(0.0);
    l.set_hexpand(true);
    row.append(&l);
    let sw = gtk::Switch::builder()
        .active(on)
        .valign(gtk::Align::Center)
        .build();
    row.append(&sw);
    (row, sw)
}

/// A row of linked toggle buttons; `on_change` gets the chosen index.
pub fn segmented(
    options: &[&str],
    selected: usize,
    on_change: impl Fn(usize) + 'static,
) -> gtk::Box {
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bx.add_css_class("linked");
    bx.add_css_class("segmented");
    let on_change = std::rc::Rc::new(on_change);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, label) in options.iter().enumerate() {
        let b = gtk::ToggleButton::with_label(label);
        b.set_hexpand(true);
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.set_active(i == selected);
        let cb = on_change.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                cb(i);
            }
        });
        bx.append(&b);
    }
    bx
}

/// Two-column grid of cards, as in the mockup. Carries the `columns` class
/// so [`set_columns_stacked`] can turn it into one column in a narrow pane.
pub fn two_columns() -> (gtk::Box, gtk::Box, gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("columns");
    row.set_homogeneous(true);
    let left = gtk::Box::new(gtk::Orientation::Vertical, 14);
    let right = gtk::Box::new(gtk::Orientation::Vertical, 14);
    row.append(&left);
    row.append(&right);
    (row, left, right)
}

pub fn dim_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("dim");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}

pub fn banner(text: &str) -> adw::Banner {
    let b = adw::Banner::new(text);
    b.set_revealed(true);
    b
}

/// An `adw::ActionRow` with a trailing widget, inside a `ListBox` with the
/// boxed-list style.
pub fn list() -> gtk::ListBox {
    let l = gtk::ListBox::new();
    l.add_css_class("boxed-list");
    l.set_selection_mode(gtk::SelectionMode::None);
    l
}

pub fn clear(bx: &gtk::ListBox) {
    while let Some(child) = bx.first_child() {
        bx.remove(&child);
    }
}

pub fn clear_box(bx: &gtk::Box) {
    while let Some(child) = bx.first_child() {
        bx.remove(&child);
    }
}

pub fn signal_icon(bars: u8) -> &'static str {
    match bars {
        0 => "network-wireless-signal-none-symbolic",
        1 => "network-wireless-signal-weak-symbolic",
        2 => "network-wireless-signal-ok-symbolic",
        3 => "network-wireless-signal-good-symbolic",
        _ => "network-wireless-signal-excellent-symbolic",
    }
}

/// Stack (or unstack) every two-column row under `root`.
pub fn set_columns_stacked(root: &impl IsA<gtk::Widget>, stacked: bool) {
    fn walk(w: &gtk::Widget, stacked: bool) {
        if w.has_css_class("columns") {
            if let Some(b) = w.downcast_ref::<gtk::Box>() {
                b.set_orientation(if stacked {
                    gtk::Orientation::Vertical
                } else {
                    gtk::Orientation::Horizontal
                });
            }
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            walk(&ch, stacked);
            c = ch.next_sibling();
        }
    }
    walk(root.upcast_ref(), stacked);
}
