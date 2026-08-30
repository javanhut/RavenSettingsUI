//! The GTK side. Everything here runs on the main thread; work that blocks
//! (a D-Bus call, a scan, a process) goes through [`spawn`] and comes back
//! through a closure on the main loop.

pub mod pages;
pub mod theme;
pub mod widgets;
pub mod window;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::config::DesktopConfig;

pub const APP_ID: &str = "com.ravensettings.Raven";

type Listener = Box<dyn Fn(&App)>;

pub struct App {
    pub config: RefCell<DesktopConfig>,
    pub toasts: adw::ToastOverlay,
    window: RefCell<Option<adw::ApplicationWindow>>,
    /// Pages that want a nudge when something global (theme, config) changes.
    listeners: RefCell<Vec<Listener>>,
}

impl App {
    pub fn window(&self) -> adw::ApplicationWindow {
        self.window.borrow().clone().expect("window not built yet")
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    pub fn error(&self, context: &str, err: &anyhow::Error) {
        tracing::warn!("{context}: {err:#}");
        let t = adw::Toast::new(&format!("{context}: {err}"));
        t.set_timeout(6);
        self.toasts.add_toast(t);
    }

    pub fn on_change(&self, f: impl Fn(&App) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    /// Tell listeners something changed without touching the config.
    pub fn save_status_only(&self) {
        for l in self.listeners.borrow().iter() {
            l(self);
        }
    }

    /// Persist desktop.toml, restyle this window, and push the choices out
    /// to RoostBar and GTK in the background.
    pub fn save(self: &Rc<Self>) {
        let cfg = self.config.borrow().clone();
        theme::apply(cfg.appearance.theme_mode, &cfg.appearance.accent, cfg.appearance.transparency);
        if let Err(e) = cfg.save() {
            self.error("Could not save settings", &e);
            return;
        }
        for l in self.listeners.borrow().iter() {
            l(self);
        }
        let app = self.clone();
        spawn(
            move || {
                use crate::backend::integrations as i;
                let mut errs = Vec::new();
                if let Err(e) = i::sync_roostbar(&cfg) {
                    errs.push(format!("RoostBar: {e}"));
                }
                if let Err(e) = i::sync_gtk(&cfg) {
                    errs.push(format!("GTK: {e}"));
                }
                errs
            },
            move |errs| {
                for e in errs {
                    app.toast(&e);
                }
            },
        );
    }
}

/// Run `work` off the main thread, then `done` with its result on it.
pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    glib::spawn_future_local(async move {
        match gio::spawn_blocking(work).await {
            Ok(v) => done(v),
            Err(_) => tracing::error!("background task panicked"),
        }
    });
}

thread_local! {
    static WINDOW: RefCell<Option<adw::ApplicationWindow>> = const { RefCell::new(None) };
}

/// The main window, for code on another thread that needs to raise a dialog
/// via `glib::idle_add_once`.
pub fn main_window() -> Option<adw::ApplicationWindow> {
    WINDOW.with(|w| w.borrow().clone())
}

pub fn run() -> glib::ExitCode {
    adw::init().expect("could not initialise GTK: is a Wayland display available?");
    let gtk_app = adw::Application::builder().application_id(APP_ID).build();
    let app: Rc<App> = Rc::new(App {
        config: RefCell::new(DesktopConfig::load()),
        toasts: adw::ToastOverlay::new(),
        window: RefCell::new(None),
        listeners: RefCell::new(Vec::new()),
    });

    gtk_app.connect_activate(move |gtk_app| {
        if let Some(w) = app.window.borrow().as_ref() {
            w.present();
            return;
        }
        theme::load_base();
        {
            let cfg = app.config.borrow();
            theme::apply(cfg.appearance.theme_mode, &cfg.appearance.accent, cfg.appearance.transparency);
        }
        let window = window::build(gtk_app, &app);
        *app.window.borrow_mut() = Some(window.clone());
        WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
        {
            let cfg = app.config.borrow();
            theme::apply(cfg.appearance.theme_mode, &cfg.appearance.accent, cfg.appearance.transparency);
        }
        window.present();
        if let Ok(size) = std::env::var("RAVEN_SETTINGS_SNAPSHOT_SIZE") {
            if let Some((w, h)) = size.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) {
                    window.set_default_size(w, h);
                }
            }
        }
        if let Ok(dir) = std::env::var("RAVEN_SETTINGS_SNAPSHOT") {
            snapshot_pages(&window, std::path::PathBuf::from(dir));
        }
    });

    gtk_app.run()
}

/// Ask a yes/no question. `on_answer(true)` when confirmed.
pub fn confirm(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    action: &str,
    destructive: bool,
    on_answer: impl Fn(bool) + 'static,
) {
    let d = adw::AlertDialog::new(Some(heading), Some(body));
    d.add_response("cancel", "Cancel");
    d.add_response("ok", action);
    if destructive {
        d.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
    } else {
        d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    }
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    d.connect_response(None, move |_, r| on_answer(r == "ok"));
    d.present(Some(parent));
}

/// Ask for a line of text. `on_answer(None)` when cancelled.
pub fn ask_text(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    placeholder: &str,
    secret: bool,
    action: &str,
    on_answer: impl Fn(Option<String>) + 'static,
) {
    let d = adw::AlertDialog::new(Some(heading), Some(body));
    let entry: gtk::Widget = if secret {
        let e = gtk::PasswordEntry::builder()
            .placeholder_text(placeholder)
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        e.upcast()
    } else {
        let e = gtk::Entry::builder()
            .placeholder_text(placeholder)
            .activates_default(true)
            .build();
        e.upcast()
    };
    d.set_extra_child(Some(&entry));
    d.add_response("cancel", "Cancel");
    d.add_response("ok", action);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    let on_answer = Rc::new(on_answer);
    let entry2 = entry.clone();
    d.connect_response(None, move |_, r| {
        let entry = &entry2;
        if r == "ok" {
            let text = if let Some(p) = entry.downcast_ref::<gtk::PasswordEntry>() {
                p.text().to_string()
            } else if let Some(e) = entry.downcast_ref::<gtk::Entry>() {
                e.text().to_string()
            } else {
                String::new()
            };
            on_answer(Some(text));
        } else {
            on_answer(None);
        }
    });
    d.present(Some(parent));
    entry.grab_focus();
}

/// Development aid: with `RAVEN_SETTINGS_SNAPSHOT=<dir>`, render every page
/// to a PNG in that directory and quit. Lets the UI be checked from a shell.
fn snapshot_pages(window: &adw::ApplicationWindow, dir: std::path::PathBuf) {
    let window = window.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
        let ids: Vec<&'static str> = pages::all().iter().map(|p| p.id).collect();
        let (min, nat) = window.preferred_size();
        tracing::info!("window min {}x{} natural {}x{}", min.width(), min.height(), nat.width(), nat.height());
        let stack = find_stack(window.upcast_ref()).expect("no stack");
        let nav = find_nav(window.upcast_ref());
        let mut i = 0usize;
        glib::timeout_add_local(std::time::Duration::from_millis(1200), move || {
            if i > 0 {
                let id = ids[i - 1];
                let content = window.content().unwrap();
                let paintable = gtk::WidgetPaintable::new(Some(&content));
                let snap = gtk::Snapshot::new();
                paintable.snapshot(&snap, content.width() as f64, content.height() as f64);
                if let (Some(node), Some(renderer)) = (snap.to_node(), window.renderer()) {
                    let tex = renderer.render_texture(node, None);
                    let _ = std::fs::create_dir_all(&dir);
                    tex.save_to_png(dir.join(format!("{id}.png"))).ok();
                }
            }
            if i >= ids.len() {
                window.close();
                return glib::ControlFlow::Break;
            }
            if let Some(nav) = &nav {
                nav.select_row(nav.row_at_index(i as i32).as_ref());
            }
            stack.set_visible_child_name(ids[i]);
            i += 1;
            glib::ControlFlow::Continue
        });
    });
}

fn find_stack(w: &gtk::Widget) -> Option<gtk::Stack> {
    if let Some(s) = w.downcast_ref::<gtk::Stack>() {
        return Some(s.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(s) = find_stack(&c) {
            return Some(s);
        }
        child = c.next_sibling();
    }
    None
}

fn find_nav(w: &gtk::Widget) -> Option<gtk::ListBox> {
    if let Some(l) = w.downcast_ref::<gtk::ListBox>() {
        if l.has_css_class("navigation-sidebar") {
            return Some(l.clone());
        }
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(s) = find_nav(&c) {
            return Some(s);
        }
        child = c.next_sibling();
    }
    None
}
