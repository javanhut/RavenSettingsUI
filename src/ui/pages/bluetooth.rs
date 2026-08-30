//! Bluetooth: adapter power and discoverability, paired devices, and a live
//! scan of nearby devices with pairing that can answer PIN and passkey asks.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::bluetooth::{
    self as bt, Answer, Availability, Bluetooth, Device, Prompt, Reply, Snapshot,
};
use crate::ui::{ask_text, confirm, main_window, spawn, widgets, App};

struct Page {
    banner: adw::Banner,
    power: gtk::Switch,
    discoverable: gtk::Switch,
    adapter_name: gtk::Label,
    paired: gtk::ListBox,
    nearby: gtk::ListBox,
    scanning: gtk::Spinner,
    adapter: RefCell<Option<zbus::zvariant::OwnedObjectPath>>,
    busy: Cell<bool>,
    /// Set while a switch is being updated from a snapshot, so its handler
    /// does not write the value straight back.
    syncing: Cell<bool>,
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Bluetooth", "Pair and connect devices.");
    let banner = widgets::banner("");
    banner.set_revealed(false);
    content.append(&banner);

    let (adapter_card, body) = widgets::card("Adapter", "");
    let (row, power) = widgets::toggle_row("bluetooth-active-symbolic", "Bluetooth", false);
    body.append(&row);
    let (row2, discoverable) =
        widgets::toggle_row("view-reveal-symbolic", "Visible to other devices", false);
    body.append(&row2);
    let adapter_name = widgets::dim_label("");
    body.append(&adapter_name);
    content.append(&adapter_card);

    let (paired_card, paired_body) = widgets::card(
        "My devices",
        "Paired devices. Click to connect or disconnect.",
    );
    let paired = widgets::list();
    paired_body.append(&paired);
    content.append(&paired_card);

    let (nearby_card, nearby_body) = widgets::card(
        "Nearby devices",
        "Scanning while this page is open. Make the device discoverable, then pair.",
    );
    let scanning = gtk::Spinner::new();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&scanning);
    head.append(&widgets::dim_label("Scanning…"));
    nearby_body.append(&head);
    let nearby = widgets::list();
    nearby_body.append(&nearby);
    content.append(&nearby_card);

    let page = Rc::new(Page {
        banner,
        power,
        discoverable,
        adapter_name,
        paired,
        nearby,
        scanning,
        adapter: RefCell::new(None),
        busy: Cell::new(false),
        syncing: Cell::new(false),
    });

    {
        let app = app.clone();
        let page = page.clone();
        let sw = page.power.clone();
        sw.connect_state_set(move |_, on| {
            if page.syncing.get() {
                return glib::Propagation::Proceed;
            }
            let Some(adapter) = page.adapter.borrow().clone() else {
                return glib::Propagation::Proceed;
            };
            let app2 = app.clone();
            let page2 = page.clone();
            spawn(
                move || Bluetooth::connect().and_then(|b| b.set_powered(&adapter, on)),
                move |r| {
                    if let Err(e) = r {
                        app2.error("Could not change power", &e);
                    }
                    refresh(&app2, &page2);
                },
            );
            glib::Propagation::Proceed
        });
    }
    {
        let app = app.clone();
        let page = page.clone();
        let sw = page.discoverable.clone();
        sw.connect_state_set(move |_, on| {
            if page.syncing.get() {
                return glib::Propagation::Proceed;
            }
            let Some(adapter) = page.adapter.borrow().clone() else {
                return glib::Propagation::Proceed;
            };
            app.config.borrow_mut().privacy.bluetooth_discoverable = on;
            let _ = app.config.borrow().save();
            let app2 = app.clone();
            spawn(
                move || Bluetooth::connect().and_then(|b| b.set_discoverable(&adapter, on)),
                move |r| {
                    if let Err(e) = r {
                        app2.error("Could not change visibility", &e);
                    }
                },
            );
            glib::Propagation::Proceed
        });
    }

    // Scan while mapped, refresh every 2 s, stop on unmap.
    {
        let app = app.clone();
        let page = page.clone();
        root.connect_map(move |_| {
            refresh(&app, &page);
            set_discovery(&page, true);
        });
    }
    {
        let page = page.clone();
        root.connect_unmap(move |_| set_discovery(&page, false));
    }
    {
        let app = app.clone();
        let page = page.clone();
        let root2 = root.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
            if root2.is_mapped() && !page.busy.get() {
                refresh(&app, &page);
            }
            glib::ControlFlow::Continue
        });
    }

    root.upcast()
}

fn set_discovery(page: &Rc<Page>, on: bool) {
    let Some(adapter) = page.adapter.borrow().clone() else {
        return;
    };
    if on {
        page.scanning.start();
    } else {
        page.scanning.stop();
    }
    spawn(
        move || {
            Bluetooth::connect().and_then(|b| {
                if on {
                    b.start_discovery(&adapter)
                } else {
                    b.stop_discovery(&adapter)
                }
            })
        },
        |_| {},
    );
}

fn refresh(app: &Rc<App>, page: &Rc<Page>) {
    let app = app.clone();
    let page = page.clone();
    spawn(
        move || {
            let b = Bluetooth::connect()?;
            let avail = b.availability();
            let snap = if avail == Availability::Ready {
                b.snapshot()?
            } else {
                Snapshot::default()
            };
            Ok::<_, anyhow::Error>((avail, snap))
        },
        move |r| match r {
            Ok((avail, snap)) => show(&app, &page, avail, snap),
            Err(e) => {
                page.banner
                    .set_title(&format!("Bluetooth is unavailable: {e}"));
                page.banner.set_revealed(true);
            }
        },
    );
}

fn show(app: &Rc<App>, page: &Rc<Page>, avail: Availability, snap: Snapshot) {
    match avail {
        Availability::NoDaemon => {
            page.banner.set_title("bluetoothd is not running. Install BlueZ (sudo rvn install -y bluez), copy /usr/share/raven/services/bluetoothd.toml to /etc/raven/init.d/, then: sudo raven-rc reload && sudo raven-rc start bluetoothd");
            page.banner.set_revealed(true);
            return;
        }
        Availability::NoAdapter => {
            page.banner
                .set_title("No Bluetooth adapter was found on this machine.");
            page.banner.set_revealed(true);
            return;
        }
        Availability::Ready => page.banner.set_revealed(false),
    }
    let Some(adapter) = snap.adapters.first() else {
        return;
    };
    let first_time = page.adapter.borrow().is_none();
    *page.adapter.borrow_mut() = Some(adapter.path.clone());
    if first_time {
        set_discovery(page, true);
    }
    page.syncing.set(true);
    page.power.set_active(adapter.powered);
    page.discoverable.set_active(adapter.discoverable);
    page.syncing.set(false);
    page.discoverable.set_sensitive(adapter.powered);
    page.adapter_name
        .set_text(&format!("{} · {}", adapter.name, adapter.address));
    if adapter.powered && !adapter.discovering {
        set_discovery(page, true);
    }

    widgets::clear(&page.paired);
    widgets::clear(&page.nearby);
    let mut any_paired = false;
    let mut any_nearby = false;
    for d in &snap.devices {
        if d.paired {
            any_paired = true;
            page.paired.append(&paired_row(app, page, d));
        } else if !d.name.is_empty() && (d.rssi.is_some() || d.name != d.address) {
            any_nearby = true;
            page.nearby.append(&nearby_row(app, page, d));
        }
    }
    if !any_paired {
        page.paired.append(
            &adw::ActionRow::builder()
                .title("No paired devices yet")
                .build(),
        );
    }
    if !any_nearby {
        page.nearby.append(
            &adw::ActionRow::builder()
                .title(if adapter.powered {
                    "Nothing found yet"
                } else {
                    "Turn Bluetooth on to scan"
                })
                .build(),
        );
    }
}

fn device_row(d: &Device) -> adw::ActionRow {
    let mut sub = Vec::new();
    if d.connected {
        sub.push("Connected".to_string());
    }
    if let Some(b) = d.battery {
        sub.push(format!("Battery {b}%"));
    }
    if let Some(r) = d.rssi {
        sub.push(format!("{r} dBm"));
    }
    sub.push(d.address.clone());
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&d.name))
        .subtitle(sub.join(" · "))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(bt::icon_name(&d.icon)));
    row
}

fn run_op(
    app: &Rc<App>,
    page: &Rc<Page>,
    what: &'static str,
    op: impl FnOnce(&Bluetooth) -> anyhow::Result<()> + Send + 'static,
) {
    page.busy.set(true);
    let app2 = app.clone();
    let page2 = page.clone();
    spawn(
        move || Bluetooth::connect().and_then(|b| op(&b)),
        move |r| {
            page2.busy.set(false);
            match r {
                Ok(()) => app2.toast(what),
                Err(e) => app2.error(what, &e),
            }
            refresh(&app2, &page2);
        },
    );
}

fn paired_row(app: &Rc<App>, page: &Rc<Page>, d: &Device) -> adw::ActionRow {
    let row = device_row(d);
    let toggle = gtk::Button::with_label(if d.connected { "Disconnect" } else { "Connect" });
    toggle.add_css_class("flat");
    toggle.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        let page = page.clone();
        let path = d.path.clone();
        let connected = d.connected;
        toggle.connect_clicked(move |_| {
            let path = path.clone();
            if connected {
                run_op(&app, &page, "Disconnected", move |b| {
                    b.disconnect_device(&path)
                });
            } else {
                run_op(&app, &page, "Connected", move |b| b.connect_device(&path));
            }
        });
    }
    row.add_suffix(&toggle);
    row.set_activatable_widget(Some(&toggle));

    let forget = gtk::Button::from_icon_name("user-trash-symbolic");
    forget.add_css_class("flat");
    forget.set_tooltip_text(Some("Forget this device"));
    forget.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        let page = page.clone();
        let dev = d.clone();
        forget.connect_clicked(move |b| {
            let app = app.clone();
            let page = page.clone();
            let dev = dev.clone();
            confirm(
                b,
                &format!("Forget {}?", dev.name),
                "You will need to pair it again to use it.",
                "Forget",
                true,
                move |yes| {
                    if yes {
                        let dev = dev.clone();
                        run_op(&app, &page, "Device forgotten", move |b| b.forget(&dev));
                    }
                },
            );
        });
    }
    row.add_suffix(&forget);
    row
}

fn nearby_row(app: &Rc<App>, page: &Rc<Page>, d: &Device) -> adw::ActionRow {
    let row = device_row(d);
    let icon = gtk::Image::from_icon_name(widgets::signal_icon(bt::bars(d.rssi)));
    icon.add_css_class("dim");
    row.add_suffix(&icon);
    let pair = gtk::Button::with_label("Pair");
    pair.add_css_class("flat");
    pair.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        let page = page.clone();
        let path = d.path.clone();
        let name = d.name.clone();
        pair.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Pairing…");
            let path = path.clone();
            let prompter: bt::Prompter = Arc::new(prompt_on_main);
            app.toast(&format!("Pairing with {name}…"));
            run_op(&app, &page, "Paired", move |b| b.pair(&path, prompter));
        });
    }
    row.add_suffix(&pair);
    row.set_activatable_widget(Some(&pair));
    row
}

/// Runs on the agent's D-Bus thread: hop to the main loop, show the right
/// dialog, and hand the answer back through `reply`.
fn prompt_on_main(prompt: Prompt, reply: Reply) {
    glib::idle_add_once(move || {
        let Some(window) = main_window() else {
            reply.give(Answer::No);
            return;
        };
        match prompt {
            Prompt::Confirm { device, passkey } => {
                let r = reply.clone();
                confirm(
                    &window,
                    &format!("Pair with {device}?"),
                    &format!("Confirm that the device shows this code:\n\n{passkey:06}"),
                    "Pair",
                    false,
                    move |yes| r.give(if yes { Answer::Yes } else { Answer::No }),
                );
            }
            Prompt::Display { device, passkey } => {
                let d = adw::AlertDialog::new(
                    Some(&format!("Pairing with {device}")),
                    Some(&format!("Type this code on the device:\n\n{passkey:06}")),
                );
                d.add_response("ok", "Done");
                d.present(Some(&window));
                reply.give(Answer::Yes);
            }
            Prompt::Pin { device } => {
                let r = reply.clone();
                ask_text(
                    &window,
                    &format!("PIN for {device}"),
                    "Enter the PIN shown on the device.",
                    "0000",
                    false,
                    "Pair",
                    move |t| {
                        r.give(t.map(Answer::Text).unwrap_or(Answer::No));
                    },
                );
            }
            Prompt::Passkey { device } => {
                let r = reply.clone();
                ask_text(
                    &window,
                    &format!("Passkey for {device}"),
                    "Enter the six-digit passkey shown on the device.",
                    "123456",
                    false,
                    "Pair",
                    move |t| {
                        r.give(t.map(Answer::Text).unwrap_or(Answer::No));
                    },
                );
            }
        }
    });
}
