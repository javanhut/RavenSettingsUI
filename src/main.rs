mod backend;
mod config;
mod ui;
mod util;

fn main() -> glib::ExitCode {
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
