//! Bluetooth through BlueZ on the system bus.
//!
//! Blocking zbus on the caller's thread (the UI hands work to a worker), with
//! a separate connection for the pairing agent so a confirmation that waits on
//! the user cannot stall the call that asked for it.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use zbus::blocking::{fdo::ObjectManagerProxy, Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const BLUEZ: &str = "org.bluez";
const AGENT_PATH: &str = "/com/ravensettings/agent";
const ANSWER_WITHIN: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
#[allow(dead_code)] // full BlueZ picture; the UI shows what it needs
pub struct Adapter {
    pub path: OwnedObjectPath,
    pub name: String,
    pub address: String,
    pub powered: bool,
    pub discoverable: bool,
    pub pairable: bool,
    pub discovering: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Device {
    pub path: OwnedObjectPath,
    pub adapter: OwnedObjectPath,
    pub address: String,
    pub name: String,
    pub icon: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub rssi: Option<i16>,
    pub battery: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub adapters: Vec<Adapter>,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Ready,
    /// bluetoothd is not on the bus: BlueZ not installed or not started.
    NoDaemon,
    /// The daemon is up but there is no adapter.
    NoAdapter,
}

type Props = HashMap<String, OwnedValue>;

fn get_bool(p: &Props, k: &str) -> bool {
    p.get(k)
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false)
}
fn get_str(p: &Props, k: &str) -> String {
    p.get(k)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}
fn get_i16(p: &Props, k: &str) -> Option<i16> {
    p.get(k).and_then(|v| i16::try_from(v.clone()).ok())
}
fn get_path(p: &Props, k: &str) -> Option<OwnedObjectPath> {
    p.get(k)
        .and_then(|v| OwnedObjectPath::try_from(v.clone()).ok())
}

pub struct Bluetooth {
    conn: Connection,
}

impl Bluetooth {
    pub fn connect() -> Result<Self> {
        let conn = Connection::system().context("no system D-Bus")?;
        Ok(Self { conn })
    }

    pub fn availability(&self) -> Availability {
        match self.snapshot() {
            Ok(s) if s.adapters.is_empty() => Availability::NoAdapter,
            Ok(_) => Availability::Ready,
            Err(_) => Availability::NoDaemon,
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let om = ObjectManagerProxy::builder(&self.conn)
            .destination(BLUEZ)?
            .path("/")?
            .build()?;
        let objects = om
            .get_managed_objects()
            .context("bluetoothd is not running")?;
        let mut snap = Snapshot::default();
        for (path, ifaces) in objects {
            let ifaces: HashMap<String, Props> = ifaces
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect();
            if let Some(a) = ifaces.get("org.bluez.Adapter1") {
                snap.adapters.push(Adapter {
                    path: path.clone(),
                    name: get_str(a, "Alias"),
                    address: get_str(a, "Address"),
                    powered: get_bool(a, "Powered"),
                    discoverable: get_bool(a, "Discoverable"),
                    pairable: get_bool(a, "Pairable"),
                    discovering: get_bool(a, "Discovering"),
                });
            }
            if let Some(d) = ifaces.get("org.bluez.Device1") {
                let mut name = get_str(d, "Alias");
                if name.is_empty() {
                    name = get_str(d, "Name");
                }
                let address = get_str(d, "Address");
                if name.is_empty() {
                    name = address.clone();
                }
                let battery = ifaces
                    .get("org.bluez.Battery1")
                    .and_then(|b| b.get("Percentage"))
                    .and_then(|v| u8::try_from(v.clone()).ok());
                snap.devices.push(Device {
                    adapter: get_path(d, "Adapter").unwrap_or_else(|| path.clone()),
                    path: path.clone(),
                    address,
                    name,
                    icon: get_str(d, "Icon"),
                    paired: get_bool(d, "Paired"),
                    trusted: get_bool(d, "Trusted"),
                    connected: get_bool(d, "Connected"),
                    rssi: get_i16(d, "RSSI"),
                    battery,
                });
            }
        }
        snap.adapters.sort_by(|a, b| a.path.cmp(&b.path));
        snap.devices.sort_by(|a, b| {
            (b.connected, b.paired, a.name.to_lowercase()).cmp(&(
                a.connected,
                a.paired,
                b.name.to_lowercase(),
            ))
        });
        Ok(snap)
    }

    fn adapter(&self, path: &ObjectPath<'_>) -> Result<Proxy<'_>> {
        Ok(Proxy::new(
            &self.conn,
            BLUEZ,
            path.to_owned(),
            "org.bluez.Adapter1",
        )?)
    }

    fn device(&self, path: &ObjectPath<'_>) -> Result<Proxy<'_>> {
        Ok(Proxy::new(
            &self.conn,
            BLUEZ,
            path.to_owned(),
            "org.bluez.Device1",
        )?)
    }

    pub fn set_powered(&self, adapter: &ObjectPath<'_>, on: bool) -> Result<()> {
        self.adapter(adapter)?
            .set_property("Powered", on)
            .map_err(short_fdo)
    }

    pub fn set_discoverable(&self, adapter: &ObjectPath<'_>, on: bool) -> Result<()> {
        let a = self.adapter(adapter)?;
        if on {
            // Stay visible until turned off here, rather than BlueZ's
            // default three minutes, so the switch means what it says.
            a.set_property("DiscoverableTimeout", 0u32)
                .map_err(short_fdo)?;
        }
        a.set_property("Discoverable", on).map_err(short_fdo)
    }

    pub fn start_discovery(&self, adapter: &ObjectPath<'_>) -> Result<()> {
        let a = self.adapter(adapter)?;
        match a.call_method("StartDiscovery", &()) {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("InProgress") => Ok(()),
            Err(e) => Err(short(e)),
        }
    }

    pub fn stop_discovery(&self, adapter: &ObjectPath<'_>) -> Result<()> {
        let a = self.adapter(adapter)?;
        match a.call_method("StopDiscovery", &()) {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("NotReady") || e.to_string().contains("Failed") => {
                Ok(())
            }
            Err(e) => Err(short(e)),
        }
    }

    pub fn connect_device(&self, dev: &ObjectPath<'_>) -> Result<()> {
        self.device(dev)?
            .call_method("Connect", &())
            .map(|_| ())
            .map_err(short)
    }

    pub fn disconnect_device(&self, dev: &ObjectPath<'_>) -> Result<()> {
        self.device(dev)?
            .call_method("Disconnect", &())
            .map(|_| ())
            .map_err(short)
    }

    /// Pair, trust and connect, with `prompts` answering whatever the device
    /// asks (a passkey to confirm, a PIN to type).
    pub fn pair(&self, dev: &ObjectPath<'_>, prompts: Prompter) -> Result<()> {
        let _agent = AgentGuard::register(prompts)?;
        let d = self.device(dev)?;
        let already: bool = d.get_property("Paired").unwrap_or(false);
        if !already {
            d.call_method("Pair", &()).map_err(short)?;
        }
        let _ = d.set_property("Trusted", true);
        match d.call_method("Connect", &()) {
            Ok(_) => Ok(()),
            // Paired is the point; a device with no connectable profile
            // (a phone, say) still counts as success.
            Err(e) if e.to_string().contains("NotAvailable") => Ok(()),
            Err(e) => Err(short(e)),
        }
    }

    pub fn forget(&self, dev: &Device) -> Result<()> {
        let a = self.adapter(&dev.adapter)?;
        a.call_method("RemoveDevice", &(&dev.path,))
            .map(|_| ())
            .map_err(short)
    }
}

fn short_fdo(e: zbus::fdo::Error) -> anyhow::Error {
    match e {
        zbus::fdo::Error::ZBus(z) => short(z),
        other => anyhow!("{other}"),
    }
}

/// Trim `org.bluez.Error.Failed: br-connection-canceled` to the useful half.
fn short(e: zbus::Error) -> anyhow::Error {
    match e {
        zbus::Error::MethodError(name, desc, _) => {
            let kind = name
                .as_str()
                .rsplit('.')
                .next()
                .unwrap_or("Error")
                .to_string();
            match desc {
                Some(d) if !d.is_empty() => anyhow!("{kind}: {d}"),
                _ => anyhow!("{kind}"),
            }
        }
        other => anyhow!("{other}"),
    }
}

// ---- pairing agent -----------------------------------------------------

/// What the device is asking for.
#[derive(Debug, Clone)]
pub enum Prompt {
    /// Show this passkey; the user confirms it matches the device's.
    Confirm { device: String, passkey: u32 },
    /// Show this passkey for typing on the device. Informational.
    Display { device: String, passkey: u32 },
    /// Type the PIN the device shows.
    Pin { device: String },
    /// Type the passkey the device shows.
    Passkey { device: String },
}

#[derive(Debug, Clone)]
pub enum Answer {
    Yes,
    No,
    Text(String),
}

/// Supplied by the UI: called on the agent's thread with a prompt, must
/// arrange for `Reply::give` to be called from anywhere, and return.
pub type Prompter = Arc<dyn Fn(Prompt, Reply) + Send + Sync>;

#[derive(Clone, Default)]
pub struct Reply {
    inner: Arc<(Mutex<Option<Answer>>, Condvar)>,
}

impl std::fmt::Debug for Reply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reply")
    }
}

impl Reply {
    pub fn give(&self, answer: Answer) {
        let (lock, cv) = &*self.inner;
        *lock.lock().unwrap() = Some(answer);
        cv.notify_all();
    }

    fn wait(&self, within: Duration) -> Answer {
        let (lock, cv) = &*self.inner;
        let guard = lock.lock().unwrap();
        let (guard, _) = cv
            .wait_timeout_while(guard, within, |a| a.is_none())
            .unwrap();
        guard.clone().unwrap_or(Answer::No)
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

struct Agent {
    prompts: Prompter,
    names: Arc<Mutex<HashMap<String, String>>>,
}

impl Agent {
    fn name_of(&self, device: &OwnedObjectPath) -> String {
        self.names
            .lock()
            .unwrap()
            .get(device.as_str())
            .cloned()
            .unwrap_or_else(|| "device".to_string())
    }

    fn ask(&self, prompt: Prompt) -> Answer {
        let reply = Reply::default();
        (self.prompts)(prompt, reply.clone());
        reply.wait(ANSWER_WITHIN)
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    fn cancel(&self) {}

    fn request_pin_code(&self, device: OwnedObjectPath) -> Result<String, AgentError> {
        match self.ask(Prompt::Pin {
            device: self.name_of(&device),
        }) {
            Answer::Text(t) if !t.is_empty() => Ok(t),
            _ => Err(AgentError::Canceled("no PIN entered".into())),
        }
    }

    fn display_pin_code(&self, device: OwnedObjectPath, pincode: String) {
        let passkey = pincode.parse().unwrap_or(0);
        let reply = Reply::default();
        (self.prompts)(
            Prompt::Display {
                device: self.name_of(&device),
                passkey,
            },
            reply,
        );
    }

    fn request_passkey(&self, device: OwnedObjectPath) -> Result<u32, AgentError> {
        match self.ask(Prompt::Passkey {
            device: self.name_of(&device),
        }) {
            Answer::Text(t) => t
                .trim()
                .parse()
                .map_err(|_| AgentError::Rejected("passkey must be a number".into())),
            _ => Err(AgentError::Canceled("no passkey entered".into())),
        }
    }

    fn display_passkey(&self, device: OwnedObjectPath, passkey: u32, _entered: u16) {
        let reply = Reply::default();
        (self.prompts)(
            Prompt::Display {
                device: self.name_of(&device),
                passkey,
            },
            reply,
        );
    }

    fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> Result<(), AgentError> {
        match self.ask(Prompt::Confirm {
            device: self.name_of(&device),
            passkey,
        }) {
            Answer::Yes => Ok(()),
            _ => Err(AgentError::Rejected("not confirmed".into())),
        }
    }

    fn request_authorization(&self, _device: OwnedObjectPath) -> Result<(), AgentError> {
        // The user picked this device a moment ago; that is the authorization.
        Ok(())
    }

    fn authorize_service(&self, _device: OwnedObjectPath, _uuid: String) -> Result<(), AgentError> {
        Ok(())
    }
}

struct AgentGuard {
    conn: Connection,
}

impl AgentGuard {
    fn register(prompts: Prompter) -> Result<Self> {
        let conn = Connection::system()?;
        // Names for the prompts, so a dialog can say "Confirm 123456 on
        // Pixel 8" rather than showing an object path.
        let names = Arc::new(Mutex::new(HashMap::new()));
        if let Ok(snap) = (Bluetooth { conn: conn.clone() }).snapshot() {
            let mut n = names.lock().unwrap();
            for d in snap.devices {
                n.insert(d.path.to_string(), d.name);
            }
        }
        conn.object_server()
            .at(AGENT_PATH, Agent { prompts, names })?;
        let path = ObjectPath::try_from(AGENT_PATH)?;
        let manager = Proxy::new(&conn, BLUEZ, "/org/bluez", "org.bluez.AgentManager1")?;
        manager
            .call_method("RegisterAgent", &(&path, "KeyboardDisplay"))
            .map_err(short)?;
        let _ = manager.call_method("RequestDefaultAgent", &(&path,));
        Ok(Self { conn })
    }
}

impl Drop for AgentGuard {
    fn drop(&mut self) {
        if let (Ok(path), Ok(manager)) = (
            ObjectPath::try_from(AGENT_PATH),
            Proxy::new(&self.conn, BLUEZ, "/org/bluez", "org.bluez.AgentManager1"),
        ) {
            let _ = manager.call_method("UnregisterAgent", &(&path,));
        }
        let _ = self.conn.object_server().remove::<Agent, _>(AGENT_PATH);
    }
}

/// Signal strength as 0..=4 bars from RSSI.
pub fn bars(rssi: Option<i16>) -> u8 {
    match rssi {
        Some(r) if r >= -60 => 4,
        Some(r) if r >= -70 => 3,
        Some(r) if r >= -80 => 2,
        Some(_) => 1,
        None => 0,
    }
}

/// A symbolic icon name for BlueZ's `Icon` hint.
pub fn icon_name(icon: &str) -> &'static str {
    match icon {
        "audio-headset" | "audio-headphones" => "audio-headphones-symbolic",
        "audio-card" => "audio-speakers-symbolic",
        "input-keyboard" => "input-keyboard-symbolic",
        "input-mouse" => "input-mouse-symbolic",
        "input-gaming" => "input-gaming-symbolic",
        "phone" => "phone-symbolic",
        "computer" => "computer-symbolic",
        "video-display" => "video-display-symbolic",
        _ => "bluetooth-symbolic",
    }
}
