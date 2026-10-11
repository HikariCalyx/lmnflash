//! Wireless ADB pairing ("Connect ADB Wireless Debugging", Mode 2).
//!
//! Android 11 and newer can be debugged over Wi-Fi. The phone is the one that
//! serves the pairing: it announces an mDNS "pairing" service, and the
//! computer, which is the client, proves it may pair by sending the pairing
//! code with `adb pair`. Two ways lead to that code:
//!
//! * The phone's *"Pair device with QR code"* screen scans a QR code that this
//!   computer shows. The code carries a service name and a password; the phone
//!   adopts both and announces its pairing service under that name, so the
//!   computer recognizes it, and answers `adb pair` with that password. No
//!   typing is needed — the password never has to be read off the screen.
//! * The phone's *"Pair device with pairing code"* screen shows a six-digit
//!   code, which the user types in. The service is found the same way, over
//!   mDNS.
//!
//! Once the pairing is done, `adb` connects on its own as soon as it sees the
//! phone's "connect" service (which wireless debugging announces while it is
//! on); this module also connects explicitly when that service was already
//! found, and then asks `adb devices` how the connection turned out. The
//! connection can be dropped again with `adb disconnect`.
//!
//! The dialog only *presents* this: the flow state and its update handlers live
//! in `crate` (`State::flash.wireless`), and the card in
//! [`crate::wireless_dialog`].

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};

use crate::debloat::{self, AdbDevice, AdbDeviceState};
use crate::platform_tools;

/// The mDNS service a phone announces while it waits to be paired.
///
/// Its instance name is built from the service name the QR code carried, so
/// the computer can tell its own pairing from another phone's.
pub const PAIRING_SERVICE: &str = "_adb-tls-pairing._tcp.local.";

/// The mDNS service a phone with wireless debugging on announces for the
/// connection itself.
pub const CONNECT_SERVICE: &str = "_adb-tls-connect._tcp.local.";

/// How often the device list is read after pairing, and how long between the
/// reads: `adb pair` and `adb connect` answer before the device shows up in it.
const LIST_ATTEMPTS: usize = 5;
const LIST_INTERVAL: Duration = Duration::from_millis(400);

/// The service name and password a QR code asks the phone to announce.
///
/// The phone takes both from the scanned code: it announces its pairing service
/// under this name and answers `adb pair` with this password. A fresh code is
/// therefore generated for every attempt — an old one is only of use to the
/// phone that scanned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub service: String,
    pub password: String,
}

impl PairingCode {
    /// A fresh code.
    ///
    /// The service name is prefixed so a phone that scanned our code is
    /// recognizable among the other pairing services on the network; the
    /// password is what the phone is asked to accept.
    pub fn new() -> Self {
        let random = uuid::Uuid::new_v4().simple().to_string();

        Self {
            service: format!("lmnflash-{}", &random[..4]),
            password: random[..6].to_uppercase(),
        }
    }

    /// The payload the phone's "Pair device with QR code" scanner reads.
    ///
    /// This is the Wi-Fi provisioning format with ADB as the service type,
    /// which is what Android's pairing screen parses.
    pub fn payload(&self) -> String {
        format!("WIFI:T:ADB;S:{};P:{};;", self.service, self.password)
    }
}

impl Default for PairingCode {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `fullname` is the pairing service a phone announced after scanning
/// `code`.
///
/// The phone prefixes the instance name it announces with the service name the
/// code carried (an mDNS instance name has to be unique on the network, so it
/// may add to it).
pub fn matches(code: &PairingCode, fullname: &str) -> bool {
    fullname.starts_with(code.service.as_str())
}

/// A QR code, encoded once: the module bits row by row, and the side length in
/// modules.
///
/// It is kept in the dialog's state so the view does not re-encode it on every
/// frame (the view runs on every message, mouse movement included).
#[derive(Debug, Clone)]
pub struct QrImage {
    pub modules: Arc<[bool]>,
    pub width: usize,
}

impl QrImage {
    /// Encodes `payload`, or `None` when it does not fit a QR code.
    pub fn encode(payload: &str) -> Option<Self> {
        let code =
            qrcode::QrCode::with_error_correction_level(payload, qrcode::EcLevel::L).ok()?;
        let width = code.width();
        let modules: Vec<bool> = code
            .to_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect();

        Some(Self {
            modules: modules.into(),
            width,
        })
    }

    /// Whether the module at `(x, y)` is dark.
    pub fn dark(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.width + x]
    }
}

/// Which of the two services an answer belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// A phone that is waiting for a pairing code.
    Pairing,
    /// A phone that wireless debugging can connect to.
    Connect,
}

/// A phone's wireless-debugging service, as mDNS resolved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbService {
    pub kind: ServiceKind,
    /// The mDNS full name, which identifies the service.
    pub fullname: String,
    /// The IPv4 address to reach it at.
    pub address: String,
    pub port: u16,
}

impl AdbService {
    /// The `host:port` the tools take.
    pub fn address_port(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }

    /// What the device list shows for this service.
    pub fn label(&self) -> String {
        self.address_port()
    }
}

/// What a running [`Discovery`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverEvent {
    /// A service was resolved, including where to reach it.
    Found(AdbService),
    /// A service was withdrawn.
    Lost(ServiceKind, String),
    /// The discovery could not be started (no multicast, no socket, …).
    Failed(String),
}

/// A running network discovery.
///
/// Dropping it only drops the handle; [`Discovery::stop`] is what ends the
/// browse and, with it, the event stream the UI drains.
pub struct Discovery {
    daemon: ServiceDaemon,
}

impl Discovery {
    /// Stops the browse. The daemon thread ends, the workers reading from it
    /// with it, and the event stream closes.
    pub fn stop(self) {
        let _ = self.daemon.shutdown();
    }
}

/// Starts looking for the phones' pairing and connect services, forwarding
/// what it finds to `emit`.
///
/// Both service types are browsed through one mDNS daemon: the pairing service
/// exists while a phone waits for a code, the connect service while wireless
/// debugging is on, so either can be the first to answer.
///
/// The returned handle keeps the browse alive; [`Discovery::stop`] ends it. A
/// failure to start is reported as [`DiscoverEvent::Failed`] and `None`.
pub fn start(
    emit: impl Fn(DiscoverEvent) + Send + Sync + Clone + 'static,
) -> Option<Discovery> {
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            emit(DiscoverEvent::Failed(error.to_string()));
            return None;
        }
    };

    for (kind, service) in [
        (ServiceKind::Pairing, PAIRING_SERVICE),
        (ServiceKind::Connect, CONNECT_SERVICE),
    ] {
        let receiver = match daemon.browse(service) {
            Ok(receiver) => receiver,
            Err(error) => {
                emit(DiscoverEvent::Failed(error.to_string()));
                continue;
            }
        };

        let emit = emit.clone();

        // The browse delivers its events on a channel that closes when the
        // daemon shuts down, which is what ends this thread.
        std::thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        if let Some(service) = resolved(kind, &info) {
                            emit(DiscoverEvent::Found(service));
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        emit(DiscoverEvent::Lost(kind, fullname));
                    }
                    _ => {}
                }
            }
        });
    }

    Some(Discovery { daemon })
}

/// The [`AdbService`] a resolved mDNS answer describes.
///
/// A service without an IPv4 address is of no use here: the tools connect over
/// IPv4, and the phone always announces one.
fn resolved(kind: ServiceKind, info: &ResolvedService) -> Option<AdbService> {
    let address: Ipv4Addr = info.get_addresses_v4().into_iter().next()?;

    Some(AdbService {
        kind,
        fullname: info.get_fullname().to_owned(),
        address: address.to_string(),
        port: info.get_port(),
    })
}

/// How a pairing attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairOutcome {
    /// The phone is paired and `adb devices` lists it.
    Connected(AdbDevice),
    /// The phone answered the pairing, but is not listed as connected yet —
    /// `adb` connects on its own once it sees the phone's connect service.
    Paired,
}

/// Pairs with `pair_address` using `code`, then tries to connect.
///
/// `connect_address` is the phone's connect service, when it was already
/// announced; it is optional because `adb` also connects on its own through
/// mDNS.
pub fn pair_and_connect(
    pair_address: &str,
    code: &str,
    connect_address: Option<&str>,
) -> Result<PairOutcome, String> {
    pair(pair_address, code)?;

    if let Some(address) = connect_address {
        let _ = connect(address);
    }

    let host = host(pair_address);

    for attempt in 0..LIST_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(LIST_INTERVAL);
        }

        let devices = debloat::list_devices().unwrap_or_default();

        if let Some(device) = ready_device(&devices, host) {
            return Ok(PairOutcome::Connected(device.clone()));
        }
    }

    Ok(PairOutcome::Paired)
}

/// Pairs with the phone at `address` (its pairing service) using `code`.
pub fn pair(address: &str, code: &str) -> Result<String, String> {
    debloat::ensure_adb()?;

    let output = platform_tools::run_adb(&["pair", address, code])?;

    Ok(output.trim().to_owned())
}

/// Connects to the phone at `address` (its connect service).
pub fn connect(address: &str) -> Result<String, String> {
    debloat::ensure_adb()?;

    let output = platform_tools::run_adb(&["connect", address])?;

    // `adb connect` reports a failure on standard output and still exits 0, so
    // the text is what tells the two apart.
    let message = output.trim().to_owned();

    if message.to_ascii_lowercase().contains("failed") {
        return Err(message);
    }

    Ok(message)
}

/// What "Disconnect" should drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectTarget {
    /// One phone, at the `host:port` it is connected as.
    Device(String),
    /// Every device connected over the network.
    Everything,
}

impl DisconnectTarget {
    /// The address `adb disconnect` is given; `None` drops all of them.
    pub fn address(&self) -> Option<&str> {
        match self {
            Self::Device(address) => Some(address),
            Self::Everything => None,
        }
    }
}

/// Disconnects from `target` — one phone, or every wirelessly connected one.
pub fn disconnect(target: &DisconnectTarget) -> Result<(), String> {
    debloat::ensure_adb()?;

    let output = match target.address() {
        Some(address) => platform_tools::run_adb(&["disconnect", address])?,
        None => platform_tools::run_adb(&["disconnect"])?,
    };

    // `adb disconnect` answers on standard output, like `adb connect` does.
    let message = output.trim().to_owned();

    if message.to_ascii_lowercase().contains("failed") {
        return Err(message);
    }

    Ok(())
}

/// The wireless (TCP/IP) devices `adb` already has connected.
///
/// This is how a phone that was connected before the dialog was opened (or by
/// another tool) is known. An empty list is also what an `adb` that is not
/// installed yet answers, so merely opening the dialog never starts the
/// platform-tools download.
pub fn connected_devices() -> Vec<String> {
    if !platform_tools::adb_present() {
        return Vec::new();
    }

    network_devices(&debloat::list_devices().unwrap_or_default())
}

/// The serials of the wireless devices in an `adb devices` listing, the ones
/// `adb` can already talk to first.
pub fn network_devices(devices: &[AdbDevice]) -> Vec<String> {
    let mut network: Vec<&AdbDevice> = devices
        .iter()
        .filter(|device| is_network(&device.serial))
        .collect();

    network.sort_by_key(|device| device.state != AdbDeviceState::Ready);

    network
        .into_iter()
        .map(|device| device.serial.clone())
        .collect()
}

/// Whether an `adb` serial names a device reached over the network: a TCP/IP
/// device is spelled `<host>:<port>`, while a USB device and an emulator are
/// not.
fn is_network(serial: &str) -> bool {
    serial.contains(':')
}

/// Whether `serial` is a device reached over the network (wireless debugging)
/// rather than over the cable.
///
/// A feature that takes the network away — the captive-portal fix cycles
/// airplane mode — drops such a connection itself instead of waiting for it to
/// time out.
pub(crate) fn is_wireless(serial: &str) -> bool {
    is_network(serial)
}

/// The host part of a `host:port` address.
fn host(address: &str) -> &str {
    address.split(':').next().unwrap_or(address)
}

/// The first device `adb` lists as ready at `host`.
///
/// Matching the address keeps a phone that was already connected over USB from
/// passing for the one that was just paired.
fn ready_device<'a>(devices: &'a [AdbDevice], host: &str) -> Option<&'a AdbDevice> {
    devices
        .iter()
        .find(|device| device.state == AdbDeviceState::Ready && device.serial.starts_with(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(service: &str, password: &str) -> PairingCode {
        PairingCode {
            service: service.to_owned(),
            password: password.to_owned(),
        }
    }

    /// The payload is what Android's "Pair device with QR code" screen parses,
    /// so its spelling is the whole point of the code.
    #[test]
    fn the_payload_is_the_wifi_format_a_phone_scans() {
        assert_eq!(
            code("lmnflash-abcd", "12AB34").payload(),
            "WIFI:T:ADB;S:lmnflash-abcd;P:12AB34;;"
        );
    }

    /// A fresh code has to be a usable service name and password, and must not
    /// contain the separator the payload is built from.
    #[test]
    fn a_fresh_code_is_a_service_name_and_a_password() {
        let code = PairingCode::new();

        assert!(code.service.starts_with("lmnflash-"), "{code:?}");
        assert_eq!(code.password.len(), 6, "{code:?}");
        assert!(!code.service.contains(';'), "{code:?}");
        assert!(!code.password.contains(';'), "{code:?}");
    }

    /// The phone announces the service the code named, so a name match is what
    /// tells "our phone scanned the code" from another phone's pairing.
    #[test]
    fn a_phone_announces_the_service_the_code_named() {
        let code = code("lmnflash-abcd", "123456");

        assert!(matches(
            &code,
            "lmnflash-abcd._adb-tls-pairing._tcp.local."
        ));
        // The instance name has to be unique on the network, so the phone may
        // add to it.
        assert!(matches(
            &code,
            "lmnflash-abcd-1._adb-tls-pairing._tcp.local."
        ));
        assert!(!matches(
            &code,
            "adb-35121FDJH000R8-xyMD0H._adb-tls-pairing._tcp.local."
        ));
    }

    /// The drawing needs a square matrix of module bits, and the finder
    /// patterns make the three corners dark.
    #[test]
    fn the_qr_code_encodes_the_payload() {
        let image = QrImage::encode(&PairingCode::new().payload()).expect("the payload fits");

        assert!(image.width >= 21, "{}", image.width);
        assert_eq!(image.modules.len(), image.width * image.width);
        assert!(image.dark(0, 0));
        assert!(image.dark(image.width - 1, 0));
        assert!(image.dark(0, image.width - 1));
    }

    /// Only the phone that was just paired counts: a device that was already
    /// connected over USB must not pass for it.
    #[test]
    fn only_the_phone_at_the_paired_address_counts_as_connected() {
        let device = |serial: &str, state| AdbDevice {
            serial: serial.to_owned(),
            state,
            model: None,
        };
        let devices = vec![
            device("0A1B2C3D", AdbDeviceState::Ready),
            device("192.168.1.5:5555", AdbDeviceState::Ready),
            device("192.168.1.9:37000", AdbDeviceState::Unauthorized),
        ];

        assert_eq!(
            ready_device(&devices, "192.168.1.5").map(|device| device.serial.as_str()),
            Some("192.168.1.5:5555")
        );
        assert!(ready_device(&devices, "192.168.1.9").is_none());
        assert!(ready_device(&devices, "192.168.1.7").is_none());
    }

    #[test]
    fn the_host_of_an_address_is_what_comes_before_the_port() {
        assert_eq!(host("192.168.1.5:37000"), "192.168.1.5");
        assert_eq!(host("192.168.1.5"), "192.168.1.5");
    }

    /// Only a single phone carries the address `adb disconnect` is given;
    /// dropping every one of them is what omitting it means.
    #[test]
    fn only_a_single_device_carries_an_address() {
        assert_eq!(
            DisconnectTarget::Device("192.168.1.5:5555".to_owned()).address(),
            Some("192.168.1.5:5555")
        );
        assert_eq!(DisconnectTarget::Everything.address(), None);
    }

    /// Only devices reached over the network are offered for disconnecting:
    /// a phone on the cable or an emulator is not what `adb disconnect` is
    /// for. The ones `adb` can talk to come first.
    #[test]
    fn only_network_devices_are_offered_for_disconnecting() {
        let device = |serial: &str, state| AdbDevice {
            serial: serial.to_owned(),
            state,
            model: None,
        };
        let devices = vec![
            device("0A1B2C3D", AdbDeviceState::Ready),
            device("emulator-5554", AdbDeviceState::Ready),
            device("192.168.1.9:37000", AdbDeviceState::Unauthorized),
            device("192.168.1.5:5555", AdbDeviceState::Ready),
        ];

        assert_eq!(
            network_devices(&devices),
            ["192.168.1.5:5555", "192.168.1.9:37000"]
        );
    }
}
