//! Network: Wi-Fi through cawd, wired ports through the same socket.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::network::{self as net, Client, NetworkSummary, PortSummary, SecretKind};
use crate::ui::{ask_text, main_window, spawn, widgets, App};

struct Page {
    status_title: gtk::Label,
    status_sub: gtk::Label,
    disconnect: gtk::Button,
    scan: gtk::Button,
    spinner: gtk::Spinner,
    networks: gtk::ListBox,
    ports: gtk::ListBox,
    progress: gtk::Label,
    daemon: gtk::Switch,
    daemon_banner: adw::Banner,
    /// True while we set the switch programmatically to reflect cawd's real
    /// state, so the state-set handler doesn't act on it.
    toggling: std::cell::Cell<bool>,
    current_ssid: RefCell<Option<String>>,
    wifi_port: RefCell<Option<String>>,
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let (root, content) = widgets::page("Network", "Wi-Fi and wired connections, managed by cawd.");

    let daemon_banner = widgets::banner(
        "cawd is not running, so Wi-Fi is off. Use the switch below to start it.",
    );
    daemon_banner.set_visible(false);
    content.append(&daemon_banner);

    if Client::available() && !net::can_change() {
        let b = widgets::banner(
            "You can see networks but not join them: this account is not in the caw group.",
        );
        b.set_button_label(Some("Copy fix"));
        let app2 = app.clone();
        b.connect_button_clicked(move |_| {
            let user = std::env::var("USER").unwrap_or_default();
            app2.window().clipboard().set_text(&format!(
                "sudo usermod -aG caw {user}   # then log out and back in"
            ));
            app2.toast("Command copied");
        });
        content.append(&b);
    }

    // Wi-Fi status card
    let (status, status_body) = widgets::card("Wi-Fi", "");
    let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = gtk::Image::from_icon_name("network-wireless-symbolic");
    icon.set_pixel_size(28);
    status_row.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let status_title = gtk::Label::new(Some("Not connected"));
    status_title.add_css_class("card-title");
    status_title.set_xalign(0.0);
    let status_sub = widgets::dim_label("");
    text.append(&status_title);
    text.append(&status_sub);
    status_row.append(&text);
    let disconnect = gtk::Button::with_label("Disconnect");
    disconnect.set_valign(gtk::Align::Center);
    disconnect.set_visible(false);
    let daemon = gtk::Switch::new();
    daemon.set_valign(gtk::Align::Center);
    daemon.set_tooltip_text(Some("Start or stop the cawd Wi-Fi service"));
    daemon.set_active(Client::available());
    status_row.append(&daemon);
    status_row.append(&disconnect);
    status_body.append(&status_row);
    let progress = widgets::dim_label("");
    progress.set_visible(false);
    status_body.append(&progress);
    content.append(&status);

    // Networks card
    let (nets_card, nets_body) = widgets::card("Available networks", "Pick a network to connect");
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let spinner = gtk::Spinner::new();
    let scan = gtk::Button::from_icon_name("view-refresh-symbolic");
    scan.set_tooltip_text(Some("Scan again"));
    let hidden = gtk::Button::with_label("Join hidden network…");
    toolbar.append(&spinner);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    toolbar.append(&spacer);
    toolbar.append(&hidden);
    toolbar.append(&scan);
    nets_body.append(&toolbar);
    let networks = widgets::list();
    nets_body.append(&networks);
    content.append(&nets_card);

    // Wired card
    let (ports_card, ports_body) = widgets::card(
        "Wired",
        "Ethernet and other ports. Addresses come from DHCP.",
    );
    let ports = widgets::list();
    ports_body.append(&ports);
    content.append(&ports_card);

    let page = Rc::new(Page {
        status_title,
        status_sub,
        disconnect,
        scan,
        spinner,
        networks,
        ports,
        progress,
        daemon,
        daemon_banner,
        toggling: std::cell::Cell::new(false),
        current_ssid: RefCell::new(None),
        wifi_port: RefCell::new(None),
    });

    {
        let app = app.clone();
        let page = page.clone();
        let page_for_cb = page.clone();
        page.daemon.connect_state_set(move |_, want_on| {
            if page_for_cb.toggling.get() {
                return glib::Propagation::Proceed;
            }
            // Leave the switch where it is until raven-rc succeeds; refresh
            // flips it once cawd's state is known.
            set_daemon(&app, &page_for_cb, want_on);
            glib::Propagation::Stop
        });
    }

    {
        let app = app.clone();
        let page = page.clone();
        let b = page.scan.clone();
        b.connect_clicked(move |_| refresh(&app, &page, true));
    }
    {
        let app = app.clone();
        let page = page.clone();
        hidden.connect_clicked(move |b| {
            let app2 = app.clone();
            let page = page.clone();
            ask_text(
                b,
                "Join a hidden network",
                "Enter the network name (SSID).",
                "Network name",
                false,
                "Join",
                move |ssid| {
                    if let Some(ssid) = ssid.filter(|s| !s.trim().is_empty()) {
                        connect_to(&app2, &page, ssid.trim());
                    }
                },
            );
        });
    }
    {
        let app = app.clone();
        let page = page.clone();
        let b = page.disconnect.clone();
        b.connect_clicked(move |_| {
            let Some(ssid) = page.current_ssid.borrow().clone() else {
                return;
            };
            let app2 = app.clone();
            let page = page.clone();
            spawn(
                move || Client::connect().and_then(|mut c| c.disconnect(&ssid)),
                move |r| {
                    if let Err(e) = r {
                        app2.error("Could not disconnect", &e);
                    }
                    refresh(&app2, &page, false);
                },
            );
        });
    }

    // Refresh when shown; rescan every 30 s while visible.
    {
        let app = app.clone();
        let page = page.clone();
        root.connect_map(move |_| refresh(&app, &page, true));
    }
    {
        let app = app.clone();
        let page = page.clone();
        let root2 = root.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(30), move || {
            if root2.is_mapped() {
                refresh(&app, &page, false);
            }
            glib::ControlFlow::Continue
        });
    }

    root.upcast()
}

struct Refresh {
    status: Option<net::ConnectionStatus>,
    networks: Vec<NetworkSummary>,
    ports: Vec<PortSummary>,
    error: Option<String>,
}

fn refresh(app: &Rc<App>, page: &Rc<Page>, rescan: bool) {
    if !Client::available() {
        page.spinner.stop();
        page.scan.set_sensitive(true);
        page.daemon_banner.set_visible(true);
        page.toggling.set(true);
        page.daemon.set_active(false);
        page.toggling.set(false);
        page.status_title.set_text("Wi-Fi is off");
        page.status_sub.set_text("cawd is not running");
        page.disconnect.set_visible(false);
        page.progress.set_visible(false);
        widgets::clear(&page.networks);
        page.networks
            .append(&adw::ActionRow::builder().title("Wi-Fi is off").build());
        widgets::clear(&page.ports);
        page.ports
            .append(&adw::ActionRow::builder().title("No wired ports").build());
        return;
    }
    page.daemon_banner.set_visible(false);
    page.toggling.set(true);
    page.daemon.set_active(true);
    page.toggling.set(false);
    page.spinner.start();
    page.scan.set_sensitive(false);
    let app = app.clone();
    let page = page.clone();
    spawn(
        move || {
            let mut out = Refresh {
                status: None,
                networks: vec![],
                ports: vec![],
                error: None,
            };
            match Client::connect() {
                Ok(mut c) => {
                    out.ports = c.ports().unwrap_or_default();
                    out.status = c.status().ok();
                    if rescan {
                        match c.scan(None) {
                            Ok(n) => out.networks = n,
                            Err(e) => out.error = Some(e.to_string()),
                        }
                    }
                }
                Err(e) => out.error = Some(e.to_string()),
            }
            out
        },
        move |r| {
            page.spinner.stop();
            page.scan.set_sensitive(true);
            if let Some(e) = r.error {
                app.toast(&e);
            }
            show_status(&page, r.status.as_ref(), &r.ports);
            show_ports(&app, &page, &r.ports);
            if rescan {
                show_networks(&app, &page, &r.networks);
            }
        },
    );
}

fn show_status(page: &Page, status: Option<&net::ConnectionStatus>, ports: &[PortSummary]) {
    *page.wifi_port.borrow_mut() = ports.iter().find(|p| p.wireless).map(|p| p.name.clone());
    match status {
        Some(s) if s.ssid.is_some() && s.state.eq_ignore_ascii_case("connected") => {
            let ssid = s.ssid.clone().unwrap();
            page.status_title.set_text(&format!("Connected to {ssid}"));
            page.status_sub
                .set_text(&format!("{} · {}", s.port, s.addrs.join(", ")));
            page.disconnect.set_visible(true);
            *page.current_ssid.borrow_mut() = Some(ssid);
        }
        Some(s) => {
            page.status_title.set_text(&s.state);
            page.status_sub.set_text(&s.port.to_string());
            page.disconnect.set_visible(false);
            *page.current_ssid.borrow_mut() = None;
        }
        None => {
            page.status_title.set_text("No wireless port");
            page.status_sub.set_text("");
            page.disconnect.set_visible(false);
            *page.current_ssid.borrow_mut() = None;
        }
    }
}

fn show_networks(app: &Rc<App>, page: &Rc<Page>, nets: &[NetworkSummary]) {
    widgets::clear(&page.networks);
    if nets.is_empty() {
        let row = adw::ActionRow::builder().title("No networks found").build();
        page.networks.append(&row);
        return;
    }
    let current = page.current_ssid.borrow().clone();
    for n in nets {
        let hidden = n.ssid.is_empty();
        let title = if hidden {
            "Hidden network".to_string()
        } else {
            n.ssid.clone()
        };
        let mut subtitle = format!(
            "{} · {} · {} dBm",
            n.security,
            net::band(n.freq_mhz),
            n.signal_dbm
        );
        if n.known {
            subtitle.push_str(" · Saved");
        }
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&title))
            .subtitle(&subtitle)
            .build();
        let icon = gtk::Image::from_icon_name(widgets::signal_icon(net::bars(n.signal_dbm)));
        row.add_prefix(&icon);
        if n.security != "Open" && !n.security.is_empty() {
            let lock = gtk::Image::from_icon_name("channel-secure-symbolic");
            lock.add_css_class("dim");
            row.add_suffix(&lock);
        }
        if current.as_deref() == Some(n.ssid.as_str()) {
            let l = gtk::Label::new(Some("Connected"));
            l.add_css_class("badge");
            row.add_suffix(&l);
        } else if !hidden {
            let b = gtk::Button::with_label("Connect");
            b.set_valign(gtk::Align::Center);
            b.add_css_class("flat");
            let app = app.clone();
            let page = page.clone();
            let ssid = n.ssid.clone();
            b.connect_clicked(move |_| connect_to(&app, &page, &ssid));
            row.add_suffix(&b);
            row.set_activatable_widget(Some(&b));
        }
        page.networks.append(&row);
    }
}

fn show_ports(app: &Rc<App>, page: &Rc<Page>, ports: &[PortSummary]) {
    widgets::clear(&page.ports);
    let mut any = false;
    for p in ports.iter().filter(|p| !p.wireless && p.name != "lo") {
        any = true;
        let state = match (p.up, p.carrier) {
            (true, true) => "Connected",
            (true, false) => "No cable",
            (false, _) => "Down",
        };
        let mut sub = format!("{state} · {}", p.mac);
        if !p.addrs.is_empty() {
            sub.push_str(&format!(" · {}", p.addrs.join(", ")));
        }
        let row = adw::ActionRow::builder()
            .title(&p.name)
            .subtitle(&sub)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("network-wired-symbolic"));
        let b = gtk::Button::with_label(if p.up { "Bring down" } else { "Bring up" });
        b.add_css_class("flat");
        b.set_valign(gtk::Align::Center);
        let app = app.clone();
        let page_for_click = page.clone();
        let name = p.name.clone();
        let up = !p.up;
        b.connect_clicked(move |_| {
            let app2 = app.clone();
            let page2 = page_for_click.clone();
            let name = name.clone();
            spawn(
                move || Client::connect().and_then(|mut c| c.port_up(&name, up)),
                move |r| {
                    if let Err(e) = r {
                        app2.error("Could not change port", &e);
                    }
                    refresh(&app2, &page2, false);
                },
            );
        });
        row.add_suffix(&b);
        page.ports.append(&row);
    }
    if !any {
        page.ports
            .append(&adw::ActionRow::builder().title("No wired ports").build());
    }
}

/// Flip cawd on or off via raven-rc (asking for the sudo password when
/// needed), then rescan once the socket is up.
fn set_daemon(app: &Rc<App>, page: &Rc<Page>, want_on: bool) {
    page.daemon.set_sensitive(false);
    let app = app.clone();
    let page = page.clone();
    spawn(
        move || {
            let r = net::set_daemon(want_on);
            let ready = r.is_ok() && (!want_on || net::wait_ready(std::time::Duration::from_secs(10)));
            (r, ready)
        },
        move |(r, ready)| {
            page.daemon.set_sensitive(true);
            match (&r, ready) {
                (Err(e), _) => app.error(
                    if want_on {
                        "Could not start cawd"
                    } else {
                        "Could not stop cawd"
                    },
                    e,
                ),
                (Ok(()), false) => app
                    .toast("cawd started but is not answering yet; try Scan again in a moment"),
                _ => {}
            }
            refresh(&app, &page, r.is_ok() && ready && want_on);
        },
    );
}

/// Join a network. The daemon asks for credentials over the socket while the
/// worker blocks; each request is bounced to the main thread as a dialog and
/// the answer comes back over a channel.
fn connect_to(app: &Rc<App>, page: &Rc<Page>, ssid: &str) {
    if !net::can_change() {
        app.toast("This account is not in the caw group, so it cannot join networks.");
        return;
    }
    let ssid = ssid.to_string();
    let port = page.wifi_port.borrow().clone();
    page.progress.set_visible(true);
    page.progress.set_text(&format!("Connecting to {ssid}…"));
    let (ptx, prx) = mpsc::channel::<net::Progress>();
    let progress = page.progress.clone();
    let ssid_for_label = ssid.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        let mut alive = true;
        loop {
            match prx.try_recv() {
                Ok(p) => progress.set_text(&format!("{}: {}", ssid_for_label, p.label())),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    alive = false;
                    break;
                }
            }
        }
        if alive {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });

    let app2 = app.clone();
    let page2 = page.clone();
    spawn(
        move || {
            let mut c = Client::connect()?;
            let ssid2 = ssid.clone();
            c.connect_network(
                &ssid,
                port.as_deref(),
                move |kind, prompt| ask_secret_blocking(&ssid2, kind, prompt),
                move |p| {
                    let _ = ptx.send(p);
                },
            )
        },
        move |r| {
            page2.progress.set_visible(false);
            match r {
                Ok(()) => app2.toast("Connected"),
                Err(e) if e.to_string() == "cancelled" => {}
                Err(e) => app2.error("Could not connect", &e),
            }
            refresh(&app2, &page2, false);
        },
    );
}

/// Called on the worker thread. Shows a dialog on the main thread and waits.
fn ask_secret_blocking(ssid: &str, kind: SecretKind, prompt: &str) -> Option<String> {
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let heading = match kind {
        SecretKind::Passphrase => format!("Password for {ssid}"),
        SecretKind::Username => format!("Username for {ssid}"),
        SecretKind::Password => format!("Password for {ssid}"),
    };
    let body = prompt.to_string();
    let secret = kind != SecretKind::Username;
    glib::idle_add_once(move || {
        let Some(window) = main_window() else {
            let _ = tx.send(None);
            return;
        };
        ask_text(
            &window,
            &heading,
            &body,
            "",
            secret,
            "Connect",
            move |answer| {
                let _ = tx.send(answer);
            },
        );
    });
    rx.recv_timeout(std::time::Duration::from_secs(180))
        .ok()
        .flatten()
}
