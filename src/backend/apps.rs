//! Default applications, through GIO's mimeapps.list handling so the result is
//! what every XDG-aware program (and `xdg-open`) will see.

use gio::prelude::*;

/// One row of the Default Apps page: a label and the MIME types it stands for.
/// Setting a default applies to every type in the list; the first is the one
/// the current default is read from.
pub struct Category {
    pub label: &'static str,
    pub icon: &'static str,
    pub mimes: &'static [&'static str],
}

pub const CATEGORIES: &[Category] = &[
    Category {
        label: "Web browser",
        icon: "web-browser-symbolic",
        mimes: &[
            "x-scheme-handler/http",
            "x-scheme-handler/https",
            "text/html",
        ],
    },
    Category {
        label: "Mail",
        icon: "mail-unread-symbolic",
        mimes: &["x-scheme-handler/mailto"],
    },
    Category {
        label: "Files",
        icon: "folder-symbolic",
        mimes: &["inode/directory"],
    },
    Category {
        label: "Text editor",
        icon: "accessories-text-editor-symbolic",
        mimes: &["text/plain", "text/markdown", "application/x-shellscript"],
    },
    Category {
        label: "Photos",
        icon: "image-x-generic-symbolic",
        mimes: &[
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/gif",
            "image/svg+xml",
        ],
    },
    Category {
        label: "Music",
        icon: "audio-x-generic-symbolic",
        mimes: &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav"],
    },
    Category {
        label: "Video",
        icon: "video-x-generic-symbolic",
        mimes: &["video/mp4", "video/x-matroska", "video/webm"],
    },
    Category {
        label: "PDF",
        icon: "x-office-document-symbolic",
        mimes: &["application/pdf"],
    },
    Category {
        label: "Archives",
        icon: "package-x-generic-symbolic",
        mimes: &[
            "application/zip",
            "application/x-tar",
            "application/gzip",
            "application/x-xz",
        ],
    },
];

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub name: String,
}

fn candidate(info: &gio::AppInfo) -> Option<Candidate> {
    let id = info.id()?.to_string();
    Some(Candidate {
        id,
        name: info.display_name().to_string(),
    })
}

/// The apps that declare support for any of the category's types.
pub fn candidates(cat: &Category) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for mime in cat.mimes {
        for info in gio::AppInfo::all_for_type(mime) {
            if let Some(c) = candidate(&info) {
                if !out.iter().any(|o| o.id == c.id) {
                    out.push(c);
                }
            }
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

pub fn current(cat: &Category) -> Option<Candidate> {
    let info = gio::AppInfo::default_for_type(cat.mimes[0], false)?;
    candidate(&info)
}

pub fn set_default(cat: &Category, id: &str) -> anyhow::Result<()> {
    let info =
        gio::DesktopAppInfo::new(id).ok_or_else(|| anyhow::anyhow!("no application {id}"))?;
    for mime in cat.mimes {
        info.set_as_default_for_type(mime)?;
    }
    Ok(())
}

/// Terminal emulators, for the General page's default-terminal picker. There
/// is no MIME type for "terminal", so this is every installed app in the
/// TerminalEmulator category.
pub fn terminals() -> Vec<Candidate> {
    let mut out: Vec<Candidate> = gio::AppInfo::all()
        .iter()
        .filter_map(|info| {
            let desktop = info.downcast_ref::<gio::DesktopAppInfo>()?;
            let cats = desktop.categories()?;
            if !cats.split(';').any(|c| c == "TerminalEmulator") {
                return None;
            }
            candidate(info)
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The executable for a desktop id, for writing into desktop.toml.
pub fn executable(id: &str) -> Option<String> {
    let info = gio::DesktopAppInfo::new(id)?;
    let exec = info.executable();
    Some(exec.file_name()?.to_string_lossy().to_string())
}
