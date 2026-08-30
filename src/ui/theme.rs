//! The look: Raven's palette as libadwaita named colours, plus the card
//! layout from the mockup. Accent is swapped at runtime by a second provider.

use gtk4 as gtk;
use libadwaita as adw;
use gtk::prelude::*;

use crate::config::ThemeMode;

pub const BASE_CSS: &str = r#"
/* Raven dark palette (matches Huginn's theme.rs and RoostBar's defaults),
   expressed as libadwaita's named colours so every stock widget follows. */
@define-color window_bg_color #16161f;
@define-color window_fg_color #d0d0e0;
@define-color headerbar_bg_color #16161f;
@define-color headerbar_fg_color #d0d0e0;
@define-color headerbar_border_color #2a2a3a;
@define-color headerbar_shade_color rgba(0,0,0,0.36);
@define-color view_bg_color #1a1a26;
@define-color view_fg_color #d0d0e0;
@define-color card_bg_color #1e1e2b;
@define-color card_fg_color #d0d0e0;
@define-color dialog_bg_color #1e1e2b;
@define-color dialog_fg_color #d0d0e0;
@define-color popover_bg_color #1e1e2b;
@define-color popover_fg_color #d0d0e0;
@define-color sidebar_bg_color #141420;
@define-color borders #2a2a3a;
window.raven {
  background-color: @window_bg_color;
}
headerbar { box-shadow: none; border-bottom: 1px solid #2a2a3a; background-color: transparent; }
toolbarview, stack { background-color: transparent; }

/* Glass: the mockup's translucent window over the wallpaper. Alpha only —
   the blur behind it is the compositor's to draw. Toggled by the
   "Window transparency" switch via the .glass class on the window. */
window.raven.glass {
  background-color: alpha(#16161f, 0.72);
}
window.raven.glass .sidebar {
  background-color: alpha(#0e0e16, 0.45);
  border-right-color: alpha(#ffffff, 0.07);
}
window.raven.glass .card, window.raven.glass .raven-card {
  background-color: alpha(#ffffff, 0.085);
  border-color: alpha(#ffffff, 0.11);
}
window.raven.glass headerbar { border-bottom-color: alpha(#ffffff, 0.07); }
window.raven.glass .theme-choice { background-color: alpha(#ffffff, 0.06); }
window.raven.glass list.boxed-list, window.raven.glass list.boxed-list row {
  background-color: alpha(#ffffff, 0.04);
}
window.raven.glass entry, window.raven.glass .segmented button, window.raven.glass dropdown > button {
  background-color: alpha(#ffffff, 0.07);
}
.sidebar {
  background-color: #141420;
  border-right: 1px solid #2a2a3a;
  padding: 18px 14px;
}
.sidebar-pane { background-color: transparent; }
.sidebar .app-title {
  font-size: 20px;
  font-weight: 700;
  margin: 0 8px 14px 8px;
}
.sidebar list.navigation-sidebar {
  background: transparent;
}
.sidebar list.navigation-sidebar row {
  border-radius: 10px;
  padding: 9px 10px;
  margin: 2px 0;
}
.sidebar list.navigation-sidebar row:selected {
  background-color: alpha(@accent_bg_color, 0.22);
  color: @window_fg_color;
  box-shadow: inset 0 0 0 1px alpha(@accent_bg_color, 0.55);
}
.sidebar list.navigation-sidebar row image {
  color: @accent_bg_color;
}
.card, .raven-card {
  background-color: #1e1e2b;
  border: 1px solid #2a2a3a;
  border-radius: 14px;
  padding: 16px;
}
.raven-card .card-title {
  font-weight: 600;
  font-size: 15px;
}
.raven-card .card-subtitle, .page-subtitle, .dim {
  color: alpha(@window_fg_color, 0.6);
}
.page-title {
  font-size: 24px;
  font-weight: 700;
  color: @window_fg_color;
}
.status-card {
  padding: 12px 14px;
}
.user-card .name { font-weight: 600; }
.badge {
  background-color: alpha(@accent_bg_color, 0.25);
  color: @accent_bg_color;
  border-radius: 999px;
  padding: 2px 8px;
  font-size: 11px;
  font-weight: 700;
}
.theme-choice {
  border-radius: 12px;
  padding: 14px 10px;
  min-width: 90px;
  background-color: #262636;
  border: 2px solid transparent;
}
.theme-choice:checked {
  border-color: @accent_bg_color;
  background-color: alpha(@accent_bg_color, 0.14);
}
.theme-choice image { -gtk-icon-size: 28px; margin-bottom: 6px; }
.accent-dot {
  min-width: 30px; min-height: 30px;
  border-radius: 999px;
  padding: 0;
  border: 3px solid transparent;
  background-clip: padding-box;
}
.accent-dot:checked { border-color: @window_fg_color; }
.accent-dot.a0 { background-color: #7AA2F7; }
.accent-dot.a1 { background-color: #3B9EFF; }
.accent-dot.a2 { background-color: #22C5DD; }
.accent-dot.a3 { background-color: #5FCF5F; }
.accent-dot.a4 { background-color: #F5A623; }
.accent-dot.a5 { background-color: #F7768E; }
.accent-dot.a6 { background-color: #B279F7; }
.segmented button { min-width: 70px; }
.segmented button:checked {
  background-color: alpha(@accent_bg_color, 0.22);
  color: @accent_bg_color;
  box-shadow: inset 0 0 0 1px alpha(@accent_bg_color, 0.6);
}
.note { color: alpha(@window_fg_color, 0.55); font-size: 12px; }
.preview-frame {
  border-radius: 10px;
  min-height: 190px;
}
.preview-window {
  background-color: alpha(#16161f, 0.92);
  border: 1px solid #2a2a3a;
  border-radius: 8px;
  margin: 26px 34px;
  padding: 6px;
}
.preview-window.light { background-color: alpha(#f2f3f8, 0.94); border-color: #d0d3e0; }
.preview-window .pv-row {
  border-radius: 4px; min-height: 8px; margin: 2px;
  background-color: alpha(currentColor, 0.15);
}
.preview-window .pv-row.active { background-color: @accent_bg_color; }
.preview-window .pv-side { min-width: 44px; }
.signal-bars { font-family: monospace; }
.wallpaper-thumb { border-radius: 8px; }
.mono { font-family: monospace; }
levelbar block.filled { background-color: @accent_bg_color; }
"#;

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
        if glass { w.add_css_class("glass"); } else { w.remove_css_class("glass"); }
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
            "@define-color window_bg_color #eef0f6;\n@define-color window_fg_color #1a1b26;\n@define-color headerbar_bg_color #eef0f6;\n@define-color headerbar_fg_color #1a1b26;\n@define-color headerbar_border_color #d0d3e0;\n@define-color view_bg_color #f7f8fc;\n@define-color view_fg_color #1a1b26;\n@define-color card_bg_color #f7f8fc;\n@define-color card_fg_color #1a1b26;\n@define-color dialog_bg_color #f7f8fc;\n@define-color dialog_fg_color #1a1b26;\n@define-color popover_bg_color #f7f8fc;\n@define-color popover_fg_color #1a1b26;\n@define-color sidebar_bg_color #e6e8f0;\n@define-color borders #d0d3e0;\nheaderbar { border-bottom-color: #d0d3e0; }\n.sidebar { background-color: #e6e8f0; border-right-color: #d0d3e0; }\n.card, .raven-card { background-color: #f7f8fc; border-color: #d0d3e0; }\n.theme-choice { background-color: #e2e4ee; }\n.preview-window { background-color: alpha(#f2f3f8, 0.94); border-color: #d0d3e0; }\nwindow.raven.glass { background-color: alpha(#eef0f6, 0.8); }\nwindow.raven.glass .sidebar { background-color: alpha(#ffffff, 0.35); }\nwindow.raven.glass .card, window.raven.glass .raven-card { background-color: alpha(#ffffff, 0.45); border-color: alpha(#000000, 0.08); }\n"
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
