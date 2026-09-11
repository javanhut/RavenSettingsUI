//! The look: Raven Glass, the stylesheet shared with Raven Store and Raven
//! Power (`data/raven-glass.css`), plus the handful of classes only Settings
//! draws. Accent and light/dark are swapped at runtime by a second provider.

use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::config::ThemeMode;

/// The shared language first, then what only this app has: the theme and
/// accent pickers on the Appearance page and their live preview.
pub const BASE_CSS: &str = concat!(
    include_str!("../../data/raven-glass.css"),
    r#"
/* ── Settings-only ───────────────────────────────────────────────────── */
.theme-choice {
  border-radius: 12px;
  padding: 14px 10px;
  min-width: 90px;
  background-color: alpha(#ffffff, 0.06);
  border: 2px solid alpha(#ffffff, 0.08);
  box-shadow: inset 0 1px 0 alpha(#ffffff, 0.05);
  transition: background-color 120ms ease-out, border-color 120ms ease-out;
}
.theme-choice:hover { background-color: alpha(#ffffff, 0.09); }
.theme-choice:checked {
  border-color: @accent_bg_color;
  background-color: alpha(@accent_bg_color, 0.14);
  color: @window_fg_color;
}
.theme-choice image { -gtk-icon-size: 28px; margin-bottom: 6px; }
.accent-dot {
  min-width: 30px; min-height: 30px;
  border-radius: 999px;
  padding: 0;
  border: 3px solid transparent;
  background-clip: padding-box;
  box-shadow: inset 0 1px 0 alpha(#ffffff, 0.25), 0 1px 2px alpha(#000000, 0.30);
}
.accent-dot:checked { border-color: #ffffff; }
.accent-dot.a0 { background-color: #7AA2F7; }
.accent-dot.a1 { background-color: #3B9EFF; }
.accent-dot.a2 { background-color: #22C5DD; }
.accent-dot.a3 { background-color: #5FCF5F; }
.accent-dot.a4 { background-color: #F5A623; }
.accent-dot.a5 { background-color: #F7768E; }
.accent-dot.a6 { background-color: #B279F7; }
.preview-frame {
  border-radius: 12px;
  min-height: 190px;
}
.preview-window {
  background-color: alpha(#17171d, 0.90);
  border: 1px solid alpha(#ffffff, 0.10);
  box-shadow: inset 0 1px 0 alpha(#ffffff, 0.08), 0 10px 30px alpha(#000000, 0.40);
  border-radius: 10px;
  margin: 26px 34px;
  padding: 6px;
}
.preview-window.light {
  background-color: alpha(#f2f2f7, 0.94);
  border-color: alpha(#000000, 0.10);
  box-shadow: 0 10px 30px alpha(#000000, 0.20);
}
.preview-window .pv-row {
  border-radius: 4px; min-height: 8px; margin: 2px;
  background-color: alpha(currentColor, 0.15);
}
.preview-window .pv-row.active { background-color: @accent_bg_color; }
.preview-window .pv-side { min-width: 44px; }
.signal-bars { font-family: monospace; }
.wallpaper-thumb { border-radius: 10px; }
"#
);

pub fn load_base() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(BASE_CSS);
    let display = gtk::gdk::Display::default().expect("no display");
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

thread_local! {
    static ACCENT_PROVIDER: std::cell::RefCell<Option<gtk::CssProvider>> = const { std::cell::RefCell::new(None) };
}

/// Point every `@accent_bg_color` at the chosen hex, and set light/dark.
pub fn apply(mode: ThemeMode, accent: &str, glass: bool) {
    if let Some(w) = super::main_window() {
        if glass {
            w.add_css_class("glass");
        } else {
            w.remove_css_class("glass");
        }
    }
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(match mode {
        ThemeMode::Dark => adw::ColorScheme::ForceDark,
        ThemeMode::Light => adw::ColorScheme::ForceLight,
        ThemeMode::Auto => adw::ColorScheme::PreferDark,
    });
    let accent = if is_hex(accent) {
        accent
    } else {
        crate::config::DEFAULT_ACCENT
    };
    let light = matches!(mode, ThemeMode::Light);
    let css = format!(
        "@define-color accent_bg_color {accent};\n@define-color accent_color {accent};\n{}",
        if light {
            include_str!("../../data/raven-glass-light.css")
        } else {
            ""
        }
    );
    let display = gtk::gdk::Display::default().expect("no display");
    ACCENT_PROVIDER.with(|slot| {
        if let Some(old) = slot.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        *slot.borrow_mut() = Some(provider);
    });
}

pub fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}
