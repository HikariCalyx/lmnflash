//! The serial ports a Qualcomm Firehose (EDL) device offers.
//!
//! A phone in EDL mode enumerates as USB `05C6:9008` and exposes a serial port
//! — `COM7` on Windows, `/dev/ttyUSB0` on Linux, `/dev/cu.usbmodem…` on macOS.
//! The Qualcomm Partition Manager lists exactly those ports, and looks them up
//! about once a second so that plugging the phone in or out shows up without a
//! manual refresh.

/// The USB vendor id of Qualcomm.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const VENDOR: u16 = 0x05C6;

/// The USB product id of a device in EDL (Firehose) mode.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const PRODUCT: u16 = 0x9008;

/// The ports a Firehose device offers right now, sorted by their number.
pub(crate) fn enumerate() -> Vec<String> {
    let mut ports = platform_ports();
    ports.sort_by_key(|port| port_number(port));
    ports.dedup();
    ports
}

/// The number a port name ends with, so `COM9` sorts before `COM10` and
/// `ttyUSB2` before `ttyUSB10`. A name that carries no number sorts last.
fn port_number(port: &str) -> u32 {
    let start = port.rfind(|character: char| !character.is_ascii_digit());

    start
        .map(|index| &port[index + 1..])
        .unwrap_or(port)
        .parse()
        .unwrap_or(u32::MAX)
}

/// Opens Windows' Device Manager, where the phone appears as a serial port.
///
/// Only Windows has one; `qualcomm` does not offer the button anywhere else.
#[cfg(windows)]
pub(crate) fn open_device_manager() -> Result<(), String> {
    // `mmc.exe` is an auto-elevating executable, so `CreateProcess` refuses it
    // from an unelevated application with ERROR_ELEVATION_REQUIRED. Going
    // through the shell is what double-clicking `devmgmt.msc` does: Windows
    // asks for the elevation the user needs to see the phone's ports.
    let msc = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("C:\\Windows"))
        .join("System32")
        .join("devmgmt.msc");

    shell_open(&msc)
}

/// Hands a document to the shell, which starts an auto-elevating program
/// (`mmc.exe`) the way Explorer does.
#[cfg(windows)]
fn shell_open(path: &std::path::Path) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// The UTF-16, NUL-terminated form the shell API takes.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let verb = wide("open");
    let file = wide(&path.to_string_lossy());

    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // The call reports a value above 32 when it succeeded; anything else is an
    // error code.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "ShellExecute could not open {} (code {})",
            path.display(),
            result as isize
        ))
    }
}

/// Windows-only (see the other implementation).
#[cfg(not(windows))]
pub(crate) fn open_device_manager() -> Result<(), String> {
    Err("Device Manager is only available on Windows".to_owned())
}

/// The 05C6:9008 serial ports of this system.
#[cfg(windows)]
fn platform_ports() -> Vec<String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

    const USB: &str = "SYSTEM\\CurrentControlSet\\Enum\\USB";

    let mut ports = Vec::new();

    // The device shows up as `VID_05C6&PID_9008`, or as one of its composite
    // functions (`VID_05C6&PID_9008&MI_00`), each with an instance key per
    // connected unit.
    let usb = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(USB, KEY_READ | KEY_WOW64_64KEY);

    let Ok(usb) = usb else {
        return ports;
    };

    let wanted = format!("vid_{VENDOR:04x}&pid_{PRODUCT:04x}");
    // The `Enum` keys are a device's history, not its state: a phone that was
    // unplugged keeps its instance — and the `COM` name it was given — so only
    // the instances Windows has right now may be offered.
    let present = present_devices();

    for class in usb.enum_keys().flatten() {
        let name = class.to_ascii_lowercase();

        if !name.starts_with(&wanted) {
            continue;
        }

        // The device itself, or one of its composite functions
        // (`…&MI_00`). A longer product id that merely starts with these
        // digits is not a 9008 device.
        let rest = &name[wanted.len()..];

        if !rest.is_empty() && !rest.starts_with('&') {
            continue;
        }

        let Ok(devices) = usb.open_subkey_with_flags(&class, KEY_READ | KEY_WOW64_64KEY)
        else {
            continue;
        };

        for instance in devices.enum_keys().flatten() {
            if !is_present(present.as_ref(), &class, &instance) {
                continue;
            }

            let Ok(device) =
                devices.open_subkey_with_flags(&instance, KEY_READ | KEY_WOW64_64KEY)
            else {
                continue;
            };

            // The port name lives in the device parameters; a few drivers
            // keep it directly on the instance key instead.
            let port = device
                .open_subkey_with_flags("Device Parameters", KEY_READ | KEY_WOW64_64KEY)
                .and_then(|parameters| parameters.get_value::<String, _>("PortName"))
                .or_else(|_| device.get_value::<String, _>("PortName"));

            if let Ok(port) = port {
                let port = port.trim().to_ascii_uppercase();

                if !port.is_empty() && !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
    }

    ports
}

/// The two cfgmgr32 calls the port list needs.
///
/// They are declared here rather than taken from `windows-sys`: the page asks
/// the device manager one narrow question, and the crate's whole installation
/// module is a large addition to a binary that already links close to the MSVC
/// linker's program-database limit (`LNK1140`).
#[cfg(windows)]
mod cfgmgr {
    use std::ffi::c_ushort;

    #[link(name = "cfgmgr32")]
    unsafe extern "system" {
        pub(crate) fn CM_Get_Device_ID_List_SizeW(
            size: *mut u32,
            filter: *const c_ushort,
            flags: u32,
        ) -> u32;
        pub(crate) fn CM_Get_Device_ID_ListW(
            filter: *const c_ushort,
            buffer: *mut c_ushort,
            length: u32,
            flags: u32,
        ) -> u32;
    }

    /// `CM_GETIDLIST_FILTER_PRESENT`: only the devices that are plugged in.
    pub(crate) const PRESENT: u32 = 0x0000_0100;
}

/// The device instances Windows has plugged in right now, spelled the way
/// cfgmgr32 spells them, or `None` when the list cannot be read.
///
/// `CM_Get_Device_ID_List` with [`cfgmgr::PRESENT`] answers with the devices
/// that exist *now* — the one question the registry cannot answer, since its
/// `Enum` keys keep the instance (and the `COM` name) of a device that was
/// unplugged long ago.
#[cfg(windows)]
fn present_devices() -> Option<std::collections::HashSet<String>> {
    let mut size = 0_u32;
    let status = unsafe {
        cfgmgr::CM_Get_Device_ID_List_SizeW(&mut size, std::ptr::null(), cfgmgr::PRESENT)
    };
    if status != 0 || size == 0 {
        return None;
    }

    // A multi-string: every instance id, each NUL-terminated, then a final NUL.
    let mut buffer = vec![0_u16; size as usize];
    let status = unsafe {
        cfgmgr::CM_Get_Device_ID_ListW(
            std::ptr::null(),
            buffer.as_mut_ptr(),
            size,
            cfgmgr::PRESENT,
        )
    };
    if status != 0 {
        return None;
    }

    Some(
        buffer
            .split(|unit| *unit == 0)
            .filter(|id| !id.is_empty())
            .filter_map(|id| String::from_utf16(id).ok())
            .map(|id| id.to_ascii_uppercase())
            .collect(),
    )
}

/// The registry path of one device instance, as cfgmgr32 spells it.
///
/// The registry keys keep the case they were written with, so both sides are
/// compared upper-cased.
#[cfg_attr(not(windows), allow(dead_code))]
fn device_id(class: &str, instance: &str) -> String {
    format!("USB\\{class}\\{instance}").to_ascii_uppercase()
}

/// Whether a device instance is plugged in.
///
/// Without a list (`None`) everything counts as present: an answer that could
/// not be read must not empty the dropdown.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_present(
    present: Option<&std::collections::HashSet<String>>,
    class: &str,
    instance: &str,
) -> bool {
    match present {
        Some(present) => present.contains(&device_id(class, instance)),
        None => true,
    }
}

/// The 05C6:9008 serial ports of this system.
#[cfg(target_os = "linux")]
fn platform_ports() -> Vec<String> {
    let mut ports = Vec::new();

    let Ok(devices) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return ports;
    };

    for device in devices.flatten() {
        let directory = device.path();

        if read_hex(&directory.join("idVendor")) != Some(VENDOR)
            || read_hex(&directory.join("idProduct")) != Some(PRODUCT)
        {
            continue;
        }

        // The tty belongs to one of the device's interfaces, one level down
        // (`<device>:<config>.<interface>/ttyUSB0`).
        let Ok(interfaces) = std::fs::read_dir(&directory) else {
            continue;
        };

        for interface in interfaces.flatten() {
            let Ok(nodes) = std::fs::read_dir(interface.path()) else {
                continue;
            };

            for node in nodes.flatten() {
                let name = node.file_name().to_string_lossy().into_owned();

                if !name.starts_with("tty") {
                    continue;
                }

                let path = dev_path(&name);

                if !ports.contains(&path) {
                    ports.push(path);
                }
            }
        }
    }

    ports
}

/// Reads a USB id (`05c6`) from a sysfs attribute.
#[cfg(target_os = "linux")]
fn read_hex(path: &std::path::Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    u16::from_str_radix(text.trim(), 16).ok()
}

/// The 05C6:9008 serial ports of this system.
///
/// macOS does not publish the USB ids beside a device node, so the node's own
/// name is what identifies the phone: an EDL phone is a CDC-ACM device, which
/// Apple's driver gives a `usbmodem` node — a USB-serial adapter appears as
/// `usbserial-…` and is offered as well, since the page fails loudly enough
/// when the port it was given is not a phone.
#[cfg(target_os = "macos")]
fn platform_ports() -> Vec<String> {
    use std::os::unix::fs::FileTypeExt;

    let mut ports = Vec::new();

    let Ok(devices) = std::fs::read_dir("/dev") else {
        return ports;
    };

    for device in devices.flatten() {
        let name = device.file_name().to_string_lossy().into_owned();
        let Some(path) = macos_port(&name) else {
            continue;
        };

        // A port is a character device; anything else under that name is not
        // something that can be opened as one (and `tty.` nodes are symlinks
        // to the same driver).
        if !matches!(device.file_type(), Ok(kind) if kind.is_char_device()) {
            continue;
        }

        if !ports.contains(&path) {
            ports.push(path);
        }
    }

    ports
}

/// The path of the device node macOS gives a USB serial port, or `None` for a
/// node that is not one.
///
/// Only the `cu.` (callout) node is taken: opening a `tty.` node blocks until
/// the modem raises its carrier, which a phone in EDL mode never does, so the
/// open would never return.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_port(name: &str) -> Option<String> {
    let serial = name.starts_with("cu.usbmodem") || name.starts_with("cu.usbserial");

    serial.then(|| dev_path(name))
}

/// The path a device node under `/dev` is opened through.
///
/// The name has to be absolute: the Firehose transport opens exactly the string
/// the page offers, and the working directory of a launched application is not
/// `/dev`.
#[cfg_attr(not(unix), allow(dead_code))]
fn dev_path(name: &str) -> String {
    format!("/dev/{name}")
}

/// The 05C6:9008 serial ports of this system.
///
/// No other platform is supported: the page lists nothing there rather than
/// guessing at device nodes it cannot check.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn platform_ports() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ports sort by their number, not by the characters of their name.
    #[test]
    fn the_ports_are_sorted_by_their_number() {
        let mut ports = vec![
            "COM10".to_owned(),
            "ttyUSB10".to_owned(),
            "COM9".to_owned(),
            "COM1".to_owned(),
            "ttyUSB2".to_owned(),
        ];

        ports.sort_by_key(|port| port_number(port));

        assert_eq!(
            ports,
            ["COM1", "ttyUSB2", "COM9", "COM10", "ttyUSB10"]
        );
    }

    /// A name without a number sorts after the numbered ones.
    #[test]
    fn a_name_without_a_number_sorts_last() {
        assert_eq!(port_number("COM7"), 7);
        assert_eq!(port_number("ttyUSB12"), 12);
        assert_eq!(port_number("usbmodem"), u32::MAX);
    }

    /// Only the nodes macOS gives a USB serial port are offered, and only the
    /// `cu.` one: a `tty.` node waits for a carrier a phone never raises.
    #[test]
    fn a_macos_port_is_the_callout_node_of_a_usb_serial_device() {
        assert_eq!(
            macos_port("cu.usbmodem14201").as_deref(),
            Some("/dev/cu.usbmodem14201")
        );
        assert_eq!(
            macos_port("cu.usbserial-1420").as_deref(),
            Some("/dev/cu.usbserial-1420")
        );

        // The same device's blocking node, and the ports every Mac has.
        assert_eq!(macos_port("tty.usbmodem14201"), None);
        assert_eq!(macos_port("cu.Bluetooth-Incoming-Port"), None);
        assert_eq!(macos_port("disk0"), None);
    }

    /// A device node is opened through its absolute path: an application's
    /// working directory is not `/dev`.
    #[test]
    fn a_device_node_is_an_absolute_path() {
        assert_eq!(dev_path("ttyUSB0"), "/dev/ttyUSB0");
    }

    /// Looking the ports up must never panic, whatever this machine holds.
    #[test]
    fn enumerating_does_not_panic() {
        let _ = enumerate();
    }

    /// The registry keeps a device it has ever seen, so the instance it names
    /// has to be matched against the devices that are plugged in now — that is
    /// what keeps an unplugged phone out of the dropdown.
    #[test]
    fn a_device_that_is_not_plugged_in_is_not_offered() {
        let present: std::collections::HashSet<String> = ["USB\\VID_05C6&PID_9008\\7&ABC&0&1"]
            .iter()
            .map(|id| id.to_ascii_uppercase())
            .collect();

        assert!(is_present(
            Some(&present),
            "VID_05C6&PID_9008",
            "7&abc&0&1"
        ));
        assert!(!is_present(
            Some(&present),
            "VID_05C6&PID_9008",
            "6&2a0573b5&0&7"
        ));
        // Another device's port is not the one that was read from this one.
        assert!(!is_present(
            Some(&present),
            "VID_05C6&PID_9008&MI_00",
            "7&abc&0&1"
        ));

        // An unreadable list keeps everything rather than hiding the phone.
        assert!(is_present(
            None,
            "VID_05C6&PID_9008",
            "6&2a0573b5&0&7"
        ));
    }

    /// The path is spelled the way cfgmgr32 spells it, whatever the registry
    /// key's own case is.
    #[test]
    fn a_device_id_is_compared_upper_cased() {
        assert_eq!(
            device_id("vid_05c6&pid_9008", "6&2a0573b5&0&7"),
            "USB\\VID_05C6&PID_9008\\6&2A0573B5&0&7"
        );
    }

    /// A document the shell cannot find is reported as a failure instead of
    /// being taken for success (and nothing is opened).
    #[cfg(windows)]
    #[test]
    fn a_missing_document_is_reported() {
        let missing = std::path::Path::new("C:\\lmnflash-no-such-document.msc");

        assert!(shell_open(missing).is_err());
    }
}

