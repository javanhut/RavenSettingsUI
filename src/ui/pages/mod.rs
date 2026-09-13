//! One module per page. `all()` is the sidebar order.

use std::rc::Rc;

use gtk4 as gtk;

use super::App;

pub mod about;
pub mod appearance;
pub mod bluetooth;
pub mod datetime;
pub mod display;
pub mod general;
pub mod keycast;
pub mod network;
pub mod personalization;
pub mod privacy;
pub mod sound;
pub mod storage;
pub mod updates;

#[derive(Clone)]
pub struct PageInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    /// Colour of the tile behind the icon in the sidebar; see `.nav-icon`.
    pub tint: &'static str,
    pub keywords: &'static [&'static str],
    pub build: fn(&Rc<App>) -> gtk::Widget,
}

pub fn all() -> Vec<PageInfo> {
    vec![
        PageInfo {
            id: "general",
            title: "General",
            icon: "emblem-system-symbolic",
            tint: "gray",
            keywords: &[
                "terminal",
                "clock",
                "power",
                "lid",
                "suspend",
                "reboot",
                "shutdown",
                "lock",
                "idle",
                "language",
                "hostname",
                "battery",
                "energy",
                "eco",
                "performance",
                "charge",
                "profile",
            ],
            build: general::build,
        },
        PageInfo {
            id: "datetime",
            title: "Date & Time",
            icon: "preferences-system-time-symbolic",
            tint: "blue",
            keywords: &[
                "time", "date", "timezone", "zone", "ntp", "sync", "clock", "utc", "region",
            ],
            build: datetime::build,
        },
        PageInfo {
            id: "appearance",
            title: "Appearance",
            icon: "preferences-desktop-appearance-symbolic",
            tint: "black",
            keywords: &[
                "theme",
                "dark",
                "light",
                "accent",
                "color",
                "scale",
                "wallpaper",
                "transparency",
                "blur",
                "shadow",
                "animation",
            ],
            build: appearance::build,
        },
        PageInfo {
            id: "personalization",
            title: "Personalization",
            icon: "preferences-desktop-wallpaper-symbolic",
            tint: "purple",
            keywords: &[
                "dock",
                "pinned",
                "bar",
                "roostbar",
                "panel",
                "default apps",
                "browser",
                "mail",
                "editor",
            ],
            build: personalization::build,
        },
        PageInfo {
            id: "network",
            title: "Network",
            icon: "network-wireless-symbolic",
            tint: "blue",
            keywords: &[
                "wifi", "wi-fi", "wireless", "ethernet", "wired", "ip", "dhcp", "caw", "ssid",
            ],
            build: network::build,
        },
        PageInfo {
            id: "bluetooth",
            title: "Bluetooth",
            icon: "bluetooth-symbolic",
            tint: "blue",
            keywords: &[
                "pair",
                "pairing",
                "headphones",
                "discoverable",
                "scan",
                "bluez",
            ],
            build: bluetooth::build,
        },
        PageInfo {
            id: "sound",
            title: "Sound",
            icon: "audio-speakers-symbolic",
            tint: "red",
            keywords: &[
                "volume",
                "mute",
                "output",
                "input",
                "microphone",
                "speaker",
                "pipewire",
            ],
            build: sound::build,
        },
        PageInfo {
            id: "display",
            title: "Display",
            icon: "video-display-symbolic",
            tint: "cyan",
            keywords: &[
                "monitor",
                "screen",
                "resolution",
                "scale",
                "brightness",
                "backlight",
                "arrange",
                "hidpi",
            ],
            build: display::build,
        },
        PageInfo {
            id: "keycast",
            title: "Key Overlay",
            icon: "input-keyboard-symbolic",
            tint: "orange",
            keywords: &[
                "keystrokes",
                "keys",
                "keyboard",
                "screencast",
                "screen recording",
                "record",
                "demo",
                "presentation",
                "overlay",
                "screenkey",
                "shortcuts",
                "mouse",
                "clicks",
            ],
            build: keycast::build,
        },
        PageInfo {
            id: "storage",
            title: "Storage",
            icon: "drive-harddisk-symbolic",
            tint: "graphite",
            keywords: &["disk", "partition", "space", "free", "mount"],
            build: storage::build,
        },
        PageInfo {
            id: "privacy",
            title: "Privacy",
            icon: "security-medium-symbolic",
            tint: "indigo",
            keywords: &["history", "frecency", "recent", "discoverable", "clear"],
            build: privacy::build,
        },
        PageInfo {
            id: "updates",
            title: "Updates",
            icon: "software-update-available-symbolic",
            tint: "gray",
            keywords: &["upgrade", "packages", "rvn", "install"],
            build: updates::build,
        },
        PageInfo {
            id: "about",
            title: "About",
            icon: "help-about-symbolic",
            tint: "gray",
            keywords: &["version", "kernel", "cpu", "memory", "hardware", "os"],
            build: about::build,
        },
    ]
}
