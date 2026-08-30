//! The shell of the window: sidebar with navigation, header with search, and
//! a stack of pages.

use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use super::pages::{self, PageInfo};
use super::widgets;
use super::App;
use crate::backend::system;

pub fn build(gtk_app: &adw::Application, app: &Rc<App>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(gtk_app)
        .title("Settings")
        .default_width(1180)
        .default_height(780)
        .build();
    window.add_css_class("raven");

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hexpand(true)
        .vexpand(true)
        .build();

    // Sidebar
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 10);
    sidebar.add_css_class("sidebar");
    let title = gtk::Label::new(Some("Settings"));
    title.add_css_class("app-title");
    title.set_xalign(0.0);
    sidebar.append(&title);
    sidebar.append(&user_card());

    let nav = gtk::ListBox::new();
    nav.add_css_class("navigation-sidebar");
    nav.set_selection_mode(gtk::SelectionMode::Single);
    nav.set_vexpand(true);

    let infos = pages::all();
    for info in &infos {
        let row = nav_row(info);
        nav.append(&row);
        let page = (info.build)(app);
        stack.add_named(&page, Some(info.id));
    }
    sidebar.append(&nav);

    let status = status_card(app, &stack, &nav);
    sidebar.append(&status);

    // Search filters the sidebar by title and keywords.
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search settings…")
        .hexpand(true)
        .build();
    {
        let nav = nav.clone();
        let infos2: Vec<PageInfo> = infos.clone();
        search.connect_search_changed(move |e| {
            let q = e.text().to_lowercase();
            let mut first: Option<gtk::ListBoxRow> = None;
            let mut i = 0;
            while let Some(row) = nav.row_at_index(i) {
                let info = &infos2[i as usize];
                let hit = q.is_empty()
                    || info.title.to_lowercase().contains(&q)
                    || info.keywords.iter().any(|k| k.contains(&q));
                row.set_visible(hit);
                if hit && first.is_none() {
                    first = Some(row.clone());
                }
                i += 1;
            }
            if !q.is_empty() {
                if let Some(r) = first {
                    nav.select_row(Some(&r));
                }
            }
        });
    }
    {
        let stack = stack.clone();
        let ids: Vec<&'static str> = infos.iter().map(|i| i.id).collect();
        nav.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                stack.set_visible_child_name(ids[row.index() as usize]);
            }
        });
    }
    nav.select_row(nav.row_at_index(0).as_ref());

    let header = adw::HeaderBar::builder()
        .title_widget(&search)
        .show_title(true)
        .build();
    // A sidebar button, shown only when the pane is too narrow for the
    // sidebar to stay open — the window is tiled by Huginn and may be given
    // half or a quarter of the screen.
    let show_sidebar = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Sections")
        .visible(false)
        .build();
    header.pack_start(&show_sidebar);
    let toolbar = adw::ToolbarView::new();
    toolbar.set_top_bar_style(adw::ToolbarStyle::Raised);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    // The sidebar scrolls rather than dictating the window's height: its
    // natural height (title + cards + every nav row) is taller than a
    // laptop panel, and GTK opens a window at its natural size.
    let sidebar_scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(false)
        .child(&sidebar)
        .build();
    let split = adw::OverlaySplitView::builder()
        .sidebar(&sidebar_scroller)
        .content(&toolbar)
        .sidebar_width_fraction(0.25)
        .min_sidebar_width(230.0)
        .max_sidebar_width(280.0)
        .build();
    split
        .bind_property("show-sidebar", &show_sidebar, "active")
        .bidirectional()
        .sync_create()
        .build();

    // Narrow: the sidebar becomes an overlay behind the button, and every
    // two-column page stacks to one column. Wide: the mockup's layout.
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        860.0,
        adw::LengthUnit::Px,
    ));
    narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
    narrow.add_setter(&show_sidebar, "visible", Some(&true.to_value()));
    {
        let stack = stack.clone();
        narrow.connect_apply(move |_| widgets::set_columns_stacked(&stack, true));
    }
    {
        let stack = stack.clone();
        narrow.connect_unapply(move |_| widgets::set_columns_stacked(&stack, false));
    }
    window.add_breakpoint(narrow);
    // Choosing a section closes an overlaid sidebar.
    {
        let split = split.clone();
        nav.connect_row_activated(move |_, _| {
            if split.is_collapsed() {
                split.set_show_sidebar(false);
            }
        });
    }

    app.toasts.set_child(Some(&split));
    window.set_content(Some(&app.toasts));
    // Small enough to live in a quarter-screen tile; the pages scroll.
    window.set_size_request(480, 360);
    // Open at a size that fits a laptop panel; without this GTK would use the
    // natural size of the content instead.
    window.set_default_size(960, 640);

    // Ctrl+F focuses search.
    let ctrl = gtk::ShortcutController::new();
    ctrl.set_scope(gtk::ShortcutScope::Global);
    let s2 = search.clone();
    ctrl.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("<Control>f"),
        Some(gtk::CallbackAction::new(move |_, _| {
            s2.grab_focus();
            glib::Propagation::Stop
        })),
    ));
    window.add_controller(ctrl);

    window
}

fn nav_row(info: &PageInfo) -> gtk::ListBoxRow {
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bx.append(&gtk::Image::from_icon_name(info.icon));
    let l = gtk::Label::new(Some(info.title));
    l.set_xalign(0.0);
    bx.append(&l);
    gtk::ListBoxRow::builder().child(&bx).build()
}

fn user_card() -> gtk::Box {
    let u = system::user();
    let hw = system::hardware();
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    card.add_css_class("raven-card");
    card.add_css_class("user-card");
    card.add_css_class("status-card");
    let avatar = adw::Avatar::new(44, Some(&u.full_name), true);
    if let Some(face) = &u.avatar {
        if let Ok(tex) = gtk::gdk::Texture::from_filename(face) {
            avatar.set_custom_image(Some(&tex));
        }
    }
    card.append(&avatar);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&u.full_name));
    name.add_css_class("name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&name);
    let sub = gtk::Label::new(Some(&format!("{}@{}", u.login, hw.hostname)));
    sub.add_css_class("dim");
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&sub);
    card.append(&text);
    card
}

/// "System is up to date" at the foot of the sidebar; clicking goes to
/// Updates. The Updates page rewrites the text after a check.
fn status_card(app: &Rc<App>, stack: &gtk::Stack, nav: &gtk::ListBox) -> gtk::Button {
    let os = system::os_release();
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = gtk::Image::from_icon_name("object-select-symbolic");
    icon.add_css_class("success");
    bx.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some("Checking for updates…"));
    t.set_xalign(0.0);
    t.add_css_class("name");
    text.append(&t);
    let s = gtk::Label::new(Some(&format!("{} {}", os.name, os.version_id)));
    s.set_xalign(0.0);
    s.add_css_class("dim");
    text.append(&s);
    bx.append(&text);
    bx.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    let button = gtk::Button::builder().child(&bx).build();
    button.add_css_class("raven-card");
    button.add_css_class("status-card");
    button.add_css_class("flat");
    let stack = stack.clone();
    let nav = nav.clone();
    button.connect_clicked(move |_| {
        let ids: Vec<&str> = pages::all().iter().map(|p| p.id).collect();
        if let Some(i) = ids.iter().position(|id| *id == "updates") {
            nav.select_row(nav.row_at_index(i as i32).as_ref());
            stack.set_visible_child_name("updates");
        }
    });
    // The Updates page reports back through the shared status.
    let label = t.clone();
    let icon2 = icon.clone();
    app.on_change(move |app| {
        let st = pages::updates::status();
        match st {
            Some(0) => {
                label.set_text("System is up to date");
                icon2.set_icon_name(Some("object-select-symbolic"));
            }
            Some(n) => {
                label.set_text(&format!("{n} updates available"));
                icon2.set_icon_name(Some("software-update-available-symbolic"));
            }
            None => {}
        }
        let _ = app;
    });
    button
}
