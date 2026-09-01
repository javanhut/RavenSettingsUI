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
    pub keywords: &'static [&'static str],
    pub build: fn(&Rc<App>) -> gtk::Widget,
}

pub fn all() -> Vec<PageInfo> {
    vec![
        PageInfo {
            id: "general",
            title: "General",
            icon: "emblem-system-symbolic",
            keywords: &[
                "terminal", "clock", "power", "lid", "suspend", "reboot", "shutdown", "lock",
                "idle", "language", "hostname", "battery", "energy", "eco", "performance",
                "charge", "profile",
            ],
            build: general::build,
        },
        PageInfo {
            id: "datetime",
            title: "Date & Time",
            icon: "preferences-system-time-symbolic",
            keywords: &[
                "time", "date", "timezone", "zone", "ntp", "sync", "clock", "utc", "region",
            ],
            build: datetime::build,
        },
        PageInfo {
            id: "appearance",
            title: "Appearance",
            icon: "preferences-desktop-appearance-symbolic",
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
            keywords: &[
                "wifi", "wi-fi", "wireless", "ethernet", "wired", "ip", "dhcp", "caw", "ssid",
            ],
            build: network::build,
        },
        PageInfo {
            id: "bluetooth",
            title: "Bluetooth",
            icon: "bluetooth-symbolic",
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
            id: "storage",
            title: "Storage",
            icon: "drive-harddisk-symbolic",
            keywords: &["disk", "partition", "space", "free", "mount"],
            build: storage::build,
        },
        PageInfo {
            id: "privacy",
            title: "Privacy",
            icon: "security-medium-symbolic",
            keywords: &["history", "frecency", "recent", "discoverable", "clear"],
            build: privacy::build,
        },
        PageInfo {
            id: "updates",
            title: "Updates",
            icon: "software-update-available-symbolic",
            keywords: &["upgrade", "packages", "rvn", "install"],
            build: updates::build,
        },
        PageInfo {
            id: "about",
            title: "About",
            icon: "help-about-symbolic",
            keywords: &["version", "kernel", "cpu", "memory", "hardware", "os"],
            build: about::build,
        },
    ]
}
