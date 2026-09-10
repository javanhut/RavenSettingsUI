//! Appearance: theme mode, accent, scale, transparency, preview, animation
//! speed, wallpaper, and the effect toggles — the page in the mockup.

use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::backend::integrations;
use crate::config::{AnimationSpeed, ThemeMode, ACCENTS};
use crate::ui::{spawn, widgets, App};

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page(
        "Appearance",
        "Customize how Raven looks and feels on your desktop.",
    );
    let (row, left, right) = widgets::two_columns();
    content.append(&row);
    let note = widgets::dim_label(
        "Saved to ~/.config/raven/desktop.toml and applied to RoostBar and GTK apps right away. The compositor picks them up once it reads that file.",
    );
    note.add_css_class("note");
    content.append(&note);

    // Preview is built first so the other cards can update it.
    let preview = Preview::new(app);

    left.append(&theme_card(app, &preview));
    left.append(&accent_card(app, &preview));
    left.append(&scale_card(app));
    left.append(&transparency_card(app));

    right.append(&preview.card);
    right.append(&animation_card(app));
    right.append(&wallpaper_card(app, &preview));
    right.append(&effects_card(app));

    root.upcast()
}

/// A miniature desktop that follows the theme, accent and wallpaper.
struct Preview {
    card: gtk::Box,
    picture: gtk::Picture,
    window: gtk::Box,
}

impl Preview {
    fn new(app: &Rc<App>) -> Rc<Self> {
        let (card, body) = widgets::card("Preview", "See how changes look");
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .height_request(190)
            .build();
        picture.add_css_class("preview-frame");
        let overlay = gtk::Overlay::builder().child(&picture).build();
        overlay.add_css_class("preview-frame");

        let window = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        window.add_css_class("preview-window");
        let side = gtk::Box::new(gtk::Orientation::Vertical, 2);
        side.add_css_class("pv-side");
        for i in 0..6 {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            r.add_css_class("pv-row");
            if i == 1 {
                r.add_css_class("active");
            }
            side.append(&r);
        }
        window.append(&side);
        let main = gtk::Box::new(gtk::Orientation::Vertical, 2);
        main.set_hexpand(true);
        for _ in 0..7 {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            r.add_css_class("pv-row");
            main.append(&r);
        }
        window.append(&main);
        overlay.add_overlay(&window);
        body.append(&overlay);

        let p = Rc::new(Self {
            card,
            picture,
            window,
        });
        p.refresh(app);
        p
    }

    fn refresh(&self, app: &Rc<App>) {
        let cfg = app.config.borrow();
        if matches!(cfg.appearance.theme_mode, ThemeMode::Light) {
            self.window.add_css_class("light");
        } else {
            self.window.remove_css_class("light");
        }
        let path = if cfg.appearance.wallpaper.is_empty() {
            integrations::current_system_wallpaper()
        } else {
            Some(std::path::PathBuf::from(&cfg.appearance.wallpaper))
        };
        match path {
            Some(p) => self.picture.set_filename(Some(&p)),
            None => self.picture.set_filename(None::<&std::path::Path>),
        }
    }
}

fn theme_card(app: &Rc<App>, preview: &Rc<Preview>) -> gtk::Box {
    let (card, body) = widgets::card("Theme mode", "Choose how Raven looks");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_homogeneous(true);
    let current = app.config.borrow().appearance.theme_mode;
    let mut first: Option<gtk::ToggleButton> = None;
    for (mode, label, icon) in [
        (ThemeMode::Light, "Light", "weather-clear-symbolic"),
        (ThemeMode::Dark, "Dark", "weather-clear-night-symbolic"),
        (ThemeMode::Auto, "Auto", "display-brightness-symbolic"),
    ] {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.append(&gtk::Image::from_icon_name(icon));
        content.append(&gtk::Label::new(Some(label)));
        let b = gtk::ToggleButton::builder().child(&content).build();
        b.add_css_class("theme-choice");
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.set_active(mode == current);
        let app = app.clone();
        let preview = preview.clone();
        b.connect_toggled(move |b| {
            if !b.is_active() || app.config.borrow().appearance.theme_mode == mode {
                return;
            }
            app.config.borrow_mut().appearance.theme_mode = mode;
            app.save();
            preview.refresh(&app);
        });
        row.append(&b);
    }
    body.append(&row);
    card
}

fn accent_card(app: &Rc<App>, preview: &Rc<Preview>) -> gtk::Box {
    let (card, body) = widgets::card("Accent color", "Choose your favorite accent");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let current = app.config.borrow().appearance.accent.to_uppercase();
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, (name, hex)) in ACCENTS.iter().enumerate() {
        let b = gtk::ToggleButton::new();
        b.add_css_class("accent-dot");
        b.add_css_class(&format!("a{i}"));
        b.set_tooltip_text(Some(name));
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.set_active(hex.eq_ignore_ascii_case(&current));
        let app = app.clone();
        let preview = preview.clone();
        b.connect_toggled(move |b| {
            if !b.is_active()
                || app
                    .config
                    .borrow()
                    .appearance
                    .accent
                    .eq_ignore_ascii_case(hex)
            {
                return;
            }
            app.config.borrow_mut().appearance.accent = hex.to_string();
            app.save();
            preview.refresh(&app);
        });
        row.append(&b);
    }
    body.append(&row);
    card
}

fn scale_card(app: &Rc<App>) -> gtk::Box {
    let (card, body) = widgets::card("Interface scale", "Adjust the size of UI elements");
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 80.0, 120.0, 10.0);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Top);
    scale.set_format_value_func(|_, v| format!("{v:.0}%"));
    for v in [80, 90, 100, 110, 120] {
        scale.add_mark(v as f64, gtk::PositionType::Bottom, Some(&format!("{v}%")));
    }
    scale.set_round_digits(0);
    scale.set_value(app.config.borrow().appearance.scale * 100.0);
    let app = app.clone();
    scale.connect_value_changed(move |s| {
        let v = (s.value() / 10.0).round() * 10.0;
        if (app.config.borrow().appearance.scale * 100.0 - v).abs() < 0.5 {
            return;
        }
        app.config.borrow_mut().appearance.scale = v / 100.0;
        app.save();
    });
    body.append(&scale);
    body.append(&widgets::dim_label(
        "Applies to newly started apps via desktop.toml; per-screen scale is under Display.",
    ));
    card
}

fn transparency_card(app: &Rc<App>) -> gtk::Box {
    let sw = gtk::Switch::builder()
        .active(app.config.borrow().appearance.transparency)
        .build();
    let app = app.clone();
    sw.connect_state_set(move |_, on| {
        app.config.borrow_mut().appearance.transparency = on;
        app.save();
        glib::Propagation::Proceed
    });
    widgets::card_with_control(
        "Window transparency",
        "Let the background shine through",
        &sw,
    )
}

fn animation_card(app: &Rc<App>) -> gtk::Box {
    let (card, body) = widgets::card("Animation speed", "Control how fast animations play");
    let current = match app.config.borrow().appearance.animation_speed {
        AnimationSpeed::Slow => 0,
        AnimationSpeed::Normal => 1,
        AnimationSpeed::Fast => 2,
    };
    let app = app.clone();
    body.append(&widgets::segmented(
        &["Slow", "Normal", "Fast"],
        current,
        move |i| {
            let speed = [
                AnimationSpeed::Slow,
                AnimationSpeed::Normal,
                AnimationSpeed::Fast,
            ][i];
            if app.config.borrow().appearance.animation_speed == speed {
                return;
            }
            app.config.borrow_mut().appearance.animation_speed = speed;
            app.save();
        },
    ));
    card
}

fn wallpaper_card(app: &Rc<App>, preview: &Rc<Preview>) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    outer.add_css_class("raven-card");
    let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
    head.set_hexpand(true);
    head.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some("Wallpaper"));
    t.add_css_class("card-title");
    t.set_xalign(0.0);
    head.append(&t);
    let s = gtk::Label::new(Some("Choose your desktop background"));
    s.add_css_class("card-subtitle");
    s.set_xalign(0.0);
    s.set_wrap(true);
    head.append(&s);
    outer.append(&head);

    let thumb = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .width_request(96)
        .height_request(58)
        .build();
    thumb.add_css_class("wallpaper-thumb");
    let set_thumb = {
        let thumb = thumb.clone();
        move |path: Option<std::path::PathBuf>| match path {
            Some(p) => thumb.set_filename(Some(&p)),
            None => thumb.set_filename(None::<&std::path::Path>),
        }
    };
    {
        let cfg = app.config.borrow();
        set_thumb(if cfg.appearance.wallpaper.is_empty() {
            integrations::current_system_wallpaper()
        } else {
            Some(cfg.appearance.wallpaper.clone().into())
        });
    }
    outer.append(&thumb);

    let browse = gtk::Button::with_label("Browse…");
    browse.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        let preview = preview.clone();
        browse.connect_clicked(move |b| {
            // Stills the desktop draws itself, animated WebP the daemon
            // plays, and videos that get converted to one on the way in.
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Wallpapers"));
            filter.add_mime_type("image/png");
            filter.add_mime_type("image/jpeg");
            filter.add_mime_type("image/webp");
            if integrations::can_convert_video() {
                filter.set_name(Some("Wallpapers and videos"));
                filter.add_mime_type("video/mp4");
                filter.add_mime_type("video/webm");
                filter.add_mime_type("video/x-matroska");
                filter.add_mime_type("video/quicktime");
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title("Choose a wallpaper")
                .filters(&filters)
                .modal(true)
                .build();
            let window = app.window();
            let app = app.clone();
            let preview = preview.clone();
            let set_thumb = set_thumb.clone();
            let b = b.clone();
            dialog.open(Some(&window), None::<&gio::Cancellable>, move |res| {
                let Ok(file) = res else { return };
                let Some(src) = file.path() else { return };
                b.set_sensitive(false);
                if integrations::WallpaperKind::of(&src) == Some(integrations::WallpaperKind::Video) {
                    // ffmpeg on a long clip is minutes, not seconds; say so
                    // rather than leave a greyed button as the only sign.
                    app.toast("Converting the video to a live wallpaper…");
                }
                let app2 = app.clone();
                let preview = preview.clone();
                let set_thumb = set_thumb.clone();
                let b = b.clone();
                spawn(
                    move || {
                        let w = integrations::install_user_wallpaper(&src)?;
                        let via = integrations::set_wallpaper_via_canvas(&w);
                        Ok::<_, anyhow::Error>((w, via))
                    },
                    move |res| {
                        b.set_sensitive(true);
                        match res {
                            Ok((w, via_canvas)) => {
                                // desktop.toml gets the still: Huginn draws
                                // that field itself when the daemon is not
                                // running, and it draws pictures only.
                                let recorded = w.still.clone().unwrap_or_else(|| w.path.clone());
                                app2.config.borrow_mut().appearance.wallpaper =
                                    recorded.to_string_lossy().to_string();
                                app2.save();
                                set_thumb(Some(recorded.clone()));
                                preview.refresh(&app2);
                                let motion = w.kind != integrations::WallpaperKind::Image;
                                match via_canvas {
                                    Ok(true) if motion => app2.toast("Live wallpaper set"),
                                    Ok(true) => app2.toast("Wallpaper set"),
                                    Ok(false) if motion => app2.toast(
                                        "Saved, but a live wallpaper needs RavenCanvas, which is not installed",
                                    ),
                                    Ok(false) => offer_system_install(&app2, &recorded),
                                    Err(e) => app2.error("RavenCanvas", &e),
                                }
                            }
                            Err(e) => app2.error("Could not set wallpaper", &e),
                        }
                    },
                );
            });
        });
    }
    outer.append(&browse);
    outer
}

/// Without RavenCanvas the compositor draws the root-owned system wallpaper
/// (or desktop.toml's, once it reloads). The per-user copy is already saved;
/// installing it system-wide is root's work with no daemon to ask, so it is
/// offered as a command for the user's terminal (see backend::terminal).
fn offer_system_install(app: &Rc<App>, dest: &std::path::Path) {
    let cmd = integrations::system_wallpaper_command(dest);
    crate::ui::offer_terminal(
        app,
        "Apply to the desktop background?",
        "Saved for your account and used by the desktop. To make it the machine's wallpaper (the login screen too), it has to be copied into /usr/share/wallpaper/set, which only root can write.",
        &cmd,
        |_| {},
    );
}

fn effects_card(app: &Rc<App>) -> gtk::Box {
    let (card, body) = widgets::card("", "");
    let cfg = app.config.borrow().appearance.clone();
    for (icon, label, on, set) in [
        (
            "view-paged-symbolic",
            "Show window shadows",
            cfg.shadows,
            0usize,
        ),
        ("view-reveal-symbolic", "Enable blur effects", cfg.blur, 1),
        (
            "media-playlist-repeat-symbolic",
            "Smooth animations",
            cfg.smooth_animations,
            2,
        ),
    ] {
        let (row, sw) = widgets::toggle_row(icon, label, on);
        let app = app.clone();
        sw.connect_state_set(move |_, v| {
            {
                let mut c = app.config.borrow_mut();
                match set {
                    0 => c.appearance.shadows = v,
                    1 => c.appearance.blur = v,
                    _ => c.appearance.smooth_animations = v,
                }
            }
            app.save();
            glib::Propagation::Proceed
        });
        body.append(&row);
    }
    card
}
