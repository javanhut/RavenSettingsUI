//! About: the OS, the hardware, and the person signed in.

use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::system;
use crate::ui::{widgets, App};
use crate::util::human_bytes;

pub fn build(_app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("About", "This computer.");
    let os = system::os_release();
    let hw = system::hardware();
    let user = system::user();

    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    hero.add_css_class("raven-card");
    let theme = gtk::IconTheme::for_display(&gtk::gdk::Display::default().expect("display"));
    let logo = gtk::Image::from_icon_name(if theme.has_icon("com.ravensettings.Raven") {
        "com.ravensettings.Raven"
    } else {
        "computer-symbolic"
    });
    logo.set_pixel_size(64);
    hero.append(&logo);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    let n = gtk::Label::new(Some(&os.pretty_name));
    n.add_css_class("page-title");
    n.set_xalign(0.0);
    text.append(&n);
    text.append(&widgets::dim_label(&format!(
        "{} · {}",
        os.version, hw.hostname
    )));
    hero.append(&text);
    content.append(&hero);

    let (sys_card, sys_body) = widgets::card("System", "");
    let list = widgets::list();
    let hours = hw.uptime.as_secs() / 3600;
    let mins = (hw.uptime.as_secs() % 3600) / 60;
    for (k, v) in [
        ("Device name", hw.hostname.clone()),
        ("OS", format!("{} {}", os.name, os.version)),
        ("Build", os.build_id.clone()),
        ("Kernel", hw.kernel.clone()),
        (
            "Processor",
            format!("{} ({} threads)", hw.cpu, hw.cpu_threads),
        ),
        ("Memory", human_bytes(hw.memory_bytes)),
        (
            "Graphics",
            if hw.gpu.is_empty() {
                "Unknown".into()
            } else {
                hw.gpu.clone()
            },
        ),
        ("Desktop", "Huginn (RavenGUI)".into()),
        ("Uptime", format!("{hours}h {mins}m")),
        (
            "Signed in as",
            format!("{} ({})", user.full_name, user.login),
        ),
    ] {
        let row = adw::ActionRow::builder()
            .title(k)
            .subtitle(glib::markup_escape_text(&v))
            .build();
        row.add_css_class("property");
        list.append(&row);
    }
    sys_body.append(&list);
    content.append(&sys_card);

    if !os.home_url.is_empty() {
        let link = gtk::LinkButton::with_label(&os.home_url, "Raven Linux on GitHub");
        link.set_halign(gtk::Align::Start);
        content.append(&link);
    }
    root.upcast()
}
