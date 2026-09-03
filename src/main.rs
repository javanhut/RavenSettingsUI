mod backend;
mod config;
mod ui;
mod util;

fn main() -> glib::ExitCode {
    if std::env::args().any(|a| a == "--askpass") {
        return askpass();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    if std::env::args().any(|a| a == "--probe") {
        probe();
        return glib::ExitCode::SUCCESS;
    }
    ui::run()
}

/// `raven-settings --askpass`: the graphical password prompt for `sudo -A`.
/// Only ever invoked by sudo (see `backend::network::set_daemon`); prints the
/// password on stdout, nothing else. Tracing is not initialised here so the
/// log lines cannot pollute the password.
fn askpass() -> glib::ExitCode {
    use gtk4 as gtk;
    use libadwaita::prelude::*;

    libadwaita::init().expect("could not initialise GTK: is a Wayland display available?");
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let dialog = libadwaita::AlertDialog::new(
        Some("Authentication required"),
        Some("Enter your password to manage the Wi-Fi service."),
    );
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .activates_default(true)
        .build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", "Authenticate");
    dialog.set_response_appearance("ok", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    let main_loop = glib::MainLoop::new(None, false);
    {
        let entry = entry.clone();
        let main_loop = main_loop.clone();
        dialog.connect_response(None, move |_, r| {
            let _ = tx.send(if r == "ok" {
                Some(entry.text().to_string())
            } else {
                None
            });
            main_loop.quit();
        });
    }
    dialog.present(None::<&gtk::Window>);
    entry.grab_focus();
    main_loop.run();
    match rx.recv() {
        Ok(Some(password)) => {
            println!("{password}");
            glib::ExitCode::SUCCESS
        }
        _ => glib::ExitCode::FAILURE,
    }
}

/// `raven-settings --probe`: exercise every backend and print what it sees.
/// For checking a machine from a shell, and for bug reports.
fn probe() {
    use backend::*;
    println!("== config: {}", config::path().display());
    println!("{:#?}", config::DesktopConfig::load());
    println!("== system");
    println!("{:#?}", system::os_release());
    println!("{:#?}", system::hardware());
    println!("power socket: {}", system::power_available());
    println!("power policy: {:?}", system::power_policy());
    println!(
        "== network (cawd at {}, can change: {})",
        network::SOCKET_PATH,
        network::can_change()
    );
    match network::Client::connect() {
        Ok(mut c) => {
            println!("ports: {:#?}", c.ports());
            println!("status: {:#?}", c.status());
            println!("scan: {:#?}", c.scan(None).map(|n| n.len()));
        }
        Err(e) => println!("unavailable: {e}"),
    }
    println!("== bluetooth");
    match bluetooth::Bluetooth::connect() {
        Ok(b) => {
            println!("availability: {:?}", b.availability());
            if let Ok(s) = b.snapshot() {
                println!("adapters: {:#?}", s.adapters);
                println!("devices: {}", s.devices.len());
            }
        }
        Err(e) => println!("unavailable: {e}"),
    }
    println!("== sound (wpctl: {})", sound::available());
    println!("{:#?}", sound::snapshot());
    println!("== display");
    println!("outputs: {:?}", display::outputs());
    println!("backlights: {:#?}", display::backlights());
    println!("== storage");
    println!("{:#?}", storage::filesystems().map(|v| v.len()));
    println!("== updates (rvn: {})", updates::available());
    println!("{:?}", updates::check(false).map(|c| c.updates.len()));
}
