//! Lenovo tablet bootloader unlock (Mode 2, Smartphone Flash).
//!
//! Lenovo tablets are not Motorola devices and have no CID, so the unlock
//! works differently from the phone flow. The manual flow reads the serial
//! number and the `Bootloader_SN` and hands them to Lenovo's ZUI unlock
//! website, whose per-device `sn.img` token [`unlock`] then downloads and
//! flashes. The guided flow does everything on its own: [`guided_unlock`]
//! tries the bootloader's own unlock commands, a locally generated `sn.img`
//! (see `crate::lenovoubl`) and finally Lenovo's CDN token.
//!
//! Everything here talks to hardware or the network and is meant to run on a
//! background thread; the dialog that presents it lives in
//! `crate::tablet_bootloader`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::firmware::Platform;

/// Lenovo's ZUI bootloader-unlock website (Chinese only), where the serial
/// number and `Bootloader_SN` are submitted.
pub const UNLOCK_SITE_URL: &str = "https://www.zui.com/iunlock";

/// Token endpoint for tablets that report a `Bootloader_SN`.
const ENHANCED_BOOT_URL: &str = "http://cdn.zui.lenovomm.com/developer/enhancedboot";

/// Token endpoint for tablets without a `Bootloader_SN`.
const TABLET_BOOT_URL: &str = "http://cdn.zui.lenovomm.com/developer/tabletboot";

/// The token endpoint is served over plain HTTP and has no TLS, so the URL
/// must never be upgraded to `https`.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// How often `oem unlock-go` is re-sent while the tablet shows its on-device
/// unlock confirmation.
const UNLOCK_GO_ATTEMPTS: u32 = 5;

/// Pause between two `oem unlock-go` attempts.
const UNLOCK_GO_RETRY_DELAY: Duration = Duration::from_secs(2);

/// How long a Qualcomm tablet's on-device confirmation is waited for. Its
/// unlock command answers OKAY *before* the user confirms, so the reboot that
/// follows a confirmed unlock is what is watched for.
const QUALCOMM_CONFIRM_WINDOW: Duration = Duration::from_secs(30);

/// How often the tablet is polled while waiting for that reboot.
const CONFIRM_POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// The identity a ZUI unlock request needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabletUnlockInfo {
    /// Serial number, left-padded to eight characters.
    pub serial: String,
    /// `Bootloader_SN_Part1`, empty when the bootloader does not report it.
    pub bootloader_sn_part1: String,
    /// `Bootloader_SN_Part2`, empty when the bootloader does not report it.
    pub bootloader_sn_part2: String,
    /// The chipset the bootloader reports, which decides how the tablet's
    /// on-device unlock confirmation has to be accepted (`None` when it could
    /// not be told apart).
    pub platform: Option<Platform>,
}

impl TabletUnlockInfo {
    /// The `Bootloader_SN` shown to the user: both parts concatenated.
    pub fn bootloader_sn(&self) -> String {
        format!(
            "{}{}",
            self.bootloader_sn_part1.trim(),
            self.bootloader_sn_part2.trim()
        )
    }

    /// Whether a `Bootloader_SN` was read, in which case the enhanced token
    /// endpoint is used.
    pub fn has_bootloader_sn(&self) -> bool {
        !self.bootloader_sn().is_empty()
    }
}

/// Left-pads a serial number to at least eight characters with `0`
/// (`123456` → `00123456`), the form the ZUI endpoints expect.
pub fn pad_serial(serial: &str) -> String {
    let serial = serial.trim();

    if serial.len() >= 8 {
        serial.to_owned()
    } else {
        format!("{serial:0>8}")
    }
}

/// Reads the serial number and `Bootloader_SN` from one device.
///
/// Every variable is read best effort: a tablet that does not report the
/// `Bootloader_SN` (or a part of it) still yields the serial number.
pub fn read_info(device_serial: &str) -> Result<TabletUnlockInfo, String> {
    let device = fastboot::FastbootDevice::connect(device_serial)?;

    Ok(read_info_from(&device, device_serial).0)
}

/// Reads the identity from an open device; `fallback_serial` (the USB serial)
/// stands in when the bootloader does not answer `getvar serialno`.
///
/// The second value is the **raw** serial number: the manual flow left-pads it
/// for ZUI's endpoints, while the `sn.img` generator pads it its own way and
/// therefore needs it unchanged.
fn read_info_from(
    device: &fastboot::FastbootDevice<'_>,
    fallback_serial: &str,
) -> (TabletUnlockInfo, String) {
    let read = |name: &str| -> String {
        crate::fastboot_info::getvar_value(device, name)
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    };

    let serial = read("serialno");
    let serial = if serial.is_empty() {
        fallback_serial.to_owned()
    } else {
        serial
    };

    let info = TabletUnlockInfo {
        serial: pad_serial(&serial),
        bootloader_sn_part1: read("Bootloader_SN_Part1"),
        bootloader_sn_part2: read("Bootloader_SN_Part2"),
        // The chipset is read with the identity: both cards tell the user how
        // to accept the confirmation the tablet shows during an unlock, and
        // that differs per chipset.
        platform: crate::fastboot_info::platform_from_device(device),
    };

    (info, serial)
}

/// The URL the device's unlock token is downloaded from.
///
/// A device that reports a `Bootloader_SN` uses the enhanced endpoint, where
/// the two parts are concatenated (the `+` ZUI documentation writes between
/// them is only a notation for the join and is not part of the URI);
/// otherwise the (padded) serial number is used.
pub fn token_url(info: &TabletUnlockInfo) -> String {
    if info.has_bootloader_sn() {
        format!(
            "{ENHANCED_BOOT_URL}/{}{}/sn.img",
            info.bootloader_sn_part1.trim(),
            info.bootloader_sn_part2.trim()
        )
    } else {
        format!("{TABLET_BOOT_URL}/{}/sn.img", info.serial.trim())
    }
}

/// Where the downloaded token is written.
pub fn token_path() -> PathBuf {
    std::env::temp_dir()
        .join("lmnflash-tablet-unlock")
        .join("sn.img")
}

/// Downloads `url` to `path`. Any failure means the token is not available —
/// it has not been generated yet, or the server cannot be reached.
fn download_token(url: &str, path: &Path) -> Result<(), String> {
    let response = ureq::AgentBuilder::new()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .get(url)
        .call()
        .map_err(|error| error.to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    std::io::copy(&mut response.into_reader(), &mut file).map_err(|error| error.to_string())?;

    Ok(())
}

/// What happened during a ZUI unlock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockOutcome {
    /// The token was flashed and `oem unlock-go` succeeded; the tablet shows
    /// its own confirmation prompt.
    Unlocked,
    /// The token has not been generated (or could not be downloaded).
    NotGenerated,
    /// Flashing the token failed; carries the bootloader's own message.
    FlashFailed(String),
    /// `oem unlock-go` was refused (OEM unlocking must be enabled first).
    OemUnlockingDisabled,
    /// "Cancel & Restart" asked the job to stop; the tablet has been rebooted
    /// back into the normal system.
    Cancelled,
    /// The device could not be reached.
    Failed(String),
}

/// Runs the whole unlock: download the token for `info`, flash it to the
/// `unlock` partition, then send `oem unlock-go`.
///
/// `cancel` is polled between the steps, so the dialog's "Cancel & Restart"
/// stops the job at its next checkpoint (a running download cannot be
/// interrupted and is waited out).
pub fn unlock(
    device_serial: &str,
    info: &TabletUnlockInfo,
    token: &Path,
    cancel: &AtomicBool,
) -> UnlockOutcome {
    let url = token_url(info);

    if let Err(error) = download_token(&url, token) {
        #[cfg(debug_assertions)]
        eprintln!("[tablet] token download failed ({url}): {error}");
        #[cfg(not(debug_assertions))]
        let _ = &error;

        return UnlockOutcome::NotGenerated;
    }

    if cancelled(cancel) {
        trace(|| "cancelled by the user before the token was flashed".to_owned());

        // Nothing has been written yet, but the tablet is still in fastboot:
        // cancelling sends it back to the system all the same.
        return match fastboot::FastbootDevice::connect(device_serial) {
            Ok(device) => cancelled_unlock_outcome(&device),
            Err(_) => UnlockOutcome::Cancelled,
        };
    }

    let device = match fastboot::FastbootDevice::connect(device_serial) {
        Ok(device) => device,
        Err(error) => return UnlockOutcome::Failed(error),
    };

    if cancelled(cancel) {
        trace(|| "cancelled by the user".to_owned());
        return cancelled_unlock_outcome(&device);
    }

    let platform = crate::fastboot_info::platform_from_device(&device);

    if let Err(error) = device.flash_file("unlock", token, None) {
        return UnlockOutcome::FlashFailed(raw_failure(&error));
    }

    if cancelled(cancel) {
        trace(|| "cancelled by the user after the token was flashed".to_owned());
        return cancelled_unlock_outcome(&device);
    }

    if unlock_go_with_retries(&device, platform, cancel) {
        UnlockOutcome::Unlocked
    } else if cancelled(cancel) {
        trace(|| "cancelled by the user".to_owned());
        cancelled_unlock_outcome(&device)
    } else {
        UnlockOutcome::OemUnlockingDisabled
    }
}

/// The bootloader's own FAIL text, without the description the fastboot crate
/// wraps it in (e.g. `Flash failed: <payload>`).
fn raw_failure(message: &str) -> String {
    const WRAPPERS: &[&str] = &["Flash failed: ", "Download rejected: "];

    for wrapper in WRAPPERS {
        if let Some(rest) = message.strip_prefix(wrapper) {
            return rest.to_owned();
        }
    }

    message.to_owned()
}

/// A fastboot device offered by the guided flow: its USB serial and the
/// `product` name its bootloader reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabletDevice {
    pub serial: String,
    pub product: String,
}

impl TabletDevice {
    /// `serial (product)`, or just the serial when no product is reported.
    pub fn label(&self) -> String {
        let product = self.product.trim();

        if product.is_empty() {
            self.serial.clone()
        } else {
            format!("{} ({})", self.serial, product)
        }
    }
}

/// Lists every connected fastboot device with the `product` it reports.
///
/// Unlike the Motorola flows this keeps devices from any vendor: a tablet does
/// not answer the `cid` variable that identifies a Motorola phone.
pub fn list_devices() -> Result<Vec<TabletDevice>, String> {
    let serials = fastboot::FastbootDevice::list_devices()?;
    let mut devices = Vec::with_capacity(serials.len());

    for serial in serials {
        let product = fastboot::FastbootDevice::connect(&serial)
            .ok()
            .and_then(|device| crate::fastboot_info::getvar_value(&device, "product").ok())
            .map(|product| product.trim().to_owned())
            .unwrap_or_default();

        devices.push(TabletDevice { serial, product });
    }

    Ok(devices)
}

/// What happened during a guided (automatic) tablet unlock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuidedOutcome {
    /// One of the attempts unlocked the bootloader.
    Unlocked,
    /// OEM unlocking is disabled in Android's Developer Options.
    OemUnlockingDisabled,
    /// Nothing worked automatically; the user should continue with the manual
    /// flow (the report carries what was read so it can be pre-filled).
    ManualRequired,
    /// Every method failed; carries the last bootloader message.
    Failed(String),
    /// "Cancel & Restart" asked the job to stop; the tablet has been rebooted
    /// back into the normal system.
    Cancelled,
}

/// Result of a guided attempt, including the identity read from the device so
/// the manual flow can be pre-filled when it takes over.
#[derive(Debug, Clone)]
pub struct GuidedReport {
    /// USB serial of the device that was worked on.
    pub device: String,
    /// The identity read from it (best effort).
    pub info: TabletUnlockInfo,
    pub outcome: GuidedOutcome,
}

/// Progress the guided unlock reports to the dialog while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidedEvent {
    /// The chipset the tablet reports, published before the first unlock
    /// attempt so the dialog can show how to accept the on-device
    /// confirmation. `None` when the chipset could not be told apart.
    Platform(Option<Platform>),
    /// An unlock command is about to be sent, so the tablet is (or is about
    /// to be) showing the confirmation the user has to accept. Published
    /// before every command that asks for one.
    AwaitingConfirm,
}

/// Writes a line of the guided workflow to the debug console (`cargo run`).
///
/// The message closure is only called in debug builds, so a release build pays
/// nothing for the traces while the values they mention still count as used.
fn trace(message: impl FnOnce() -> String) {
    #[cfg(debug_assertions)]
    eprintln!("[tablet] guided: {}", message());

    #[cfg(not(debug_assertions))]
    let _ = message;
}

/// Runs the whole guided unlock on one device.
///
/// A device in userspace fastboot (`fastbootd`) is sent back to the bootloader
/// first. When OEM unlocking is enabled the methods are tried in order until
/// one succeeds: `flashing unlock`, `oem unlock-go`, a locally generated
/// `sn.img` (from the `Bootloader_SN`, then from the serial number) and
/// finally the token Lenovo's CDN serves for the device. When nothing can be
/// downloaded the manual flow has to take over.
///
/// `events` is called with the chipset once it is known, so the dialog can show
/// the tablet's confirmation instructions, and `cancel` is polled between the
/// steps so "Cancel & Restart" stops the job and reboots the tablet back into
/// the normal system.
pub fn guided_unlock(
    device_serial: &str,
    token: &Path,
    events: &dyn Fn(GuidedEvent),
    cancel: &AtomicBool,
) -> GuidedReport {
    let mut info = TabletUnlockInfo::default();
    let outcome = run_guided(device_serial, token, events, cancel, &mut info);

    trace(|| format!("finished with {outcome:?}"));

    GuidedReport {
        device: device_serial.to_owned(),
        info,
        outcome,
    }
}

/// The body of [`guided_unlock`], writing the identity it reads into `info`.
fn run_guided(
    device_serial: &str,
    token: &Path,
    events: &dyn Fn(GuidedEvent),
    cancel: &AtomicBool,
    info: &mut TabletUnlockInfo,
) -> GuidedOutcome {
    trace(|| format!("connecting to `{device_serial}`"));

    let mut device = match fastboot::FastbootDevice::connect(device_serial) {
        Ok(device) => device,
        Err(error) => {
            trace(|| format!("could not open the device: {error}"));
            return GuidedOutcome::Failed(error);
        }
    };

    let (read, mut raw_serial) = read_info_from(&device, device_serial);
    *info = read;
    trace(|| {
        format!(
            "read serial number `{}` and Bootloader_SN `{}`",
            info.serial,
            info.bootloader_sn()
        )
    });

    // `fastbootd` has a different command set, so leave it first.
    if is_userspace(&device) {
        trace(|| "device is in fastbootd; rebooting into the bootloader".to_owned());

        let _ = device.send_reboot(Some("bootloader"));
        drop(device);

        let Some(reconnected) = reconnect_after_reboot(device_serial) else {
            trace(|| "the device did not come back in bootloader mode".to_owned());
            return GuidedOutcome::Failed(
                "the device did not come back in bootloader mode".to_owned(),
            );
        };
        device = reconnected;

        let (read, serial) = read_info_from(&device, device_serial);
        *info = read;
        raw_serial = serial;
        trace(|| "device is back in bootloader mode".to_owned());
    } else {
        trace(|| "device is in bootloader mode".to_owned());
    }

    // The chipset decides how the tablet's on-device confirmation has to be
    // accepted, so it is published before the first unlock command is sent.
    // It was read together with the identity (again, when the device had to
    // be taken out of `fastbootd` first).
    let platform = info.platform;
    trace(|| match platform {
        Some(platform) => format!("chipset: {platform:?}"),
        None => "chipset: not recognised; every confirmation instruction is shown".to_owned(),
    });
    events(GuidedEvent::Platform(platform));

    if cancelled(cancel) {
        trace(|| "cancelled by the user".to_owned());
        return cancelled_outcome(&device);
    }

    let ability = unlock_ability(&device);
    trace(|| format!("`flashing get_unlock_ability` reported OEM unlocking = {ability:?}"));

    if ability == Some(false) {
        trace(|| "OEM unlocking is disabled; stopping".to_owned());
        return GuidedOutcome::OemUnlockingDisabled;
    }

    // 1. Ask the bootloader to unlock directly.
    trace(|| "attempt 1: `flashing unlock`".to_owned());
    events(GuidedEvent::AwaitingConfirm);

    if attempt_unlock(&device, "flashing unlock", platform, cancel) {
        trace(|| "`flashing unlock` succeeded".to_owned());
        return GuidedOutcome::Unlocked;
    }
    if cancelled(cancel) {
        trace(|| "cancelled by the user".to_owned());
        return cancelled_outcome(&device);
    }

    // 2. The classic fastboot unlock.
    trace(|| "attempt 2: `oem unlock-go`".to_owned());
    events(GuidedEvent::AwaitingConfirm);

    if unlock_go_with_retries(&device, platform, cancel) {
        trace(|| "`oem unlock-go` succeeded".to_owned());
        return GuidedOutcome::Unlocked;
    }
    if cancelled(cancel) {
        trace(|| "cancelled by the user".to_owned());
        return cancelled_outcome(&device);
    }

    // 3. A locally generated token: the Bootloader_SN (ZUX) first, then the
    //    raw serial number (ZUI).
    for (source, serial) in [
        ("Bootloader_SN", info.bootloader_sn()),
        ("serial number", raw_serial),
    ] {
        let serial = serial.trim();

        if serial.is_empty() {
            trace(|| format!("attempt 3: no {source} to build a token from"));
            continue;
        }

        match crate::lenovoubl::generate(serial) {
            Some(image) => {
                trace(|| {
                    format!(
                        "attempt 3: flashing a generated sn.img for the {source} `{}` ({} bytes)",
                        serial,
                        image.len()
                    )
                });

                events(GuidedEvent::AwaitingConfirm);

                if write_and_flash(&device, token, &image, platform, cancel) {
                    trace(|| "the generated token unlocked the bootloader".to_owned());
                    return GuidedOutcome::Unlocked;
                }

                if cancelled(cancel) {
                    trace(|| "cancelled by the user".to_owned());
                    return cancelled_outcome(&device);
                }

                trace(|| format!("the generated token for the {source} did not unlock"));
            }
            None => trace(|| format!("attempt 3: the {source} `{serial}` fits no token layout")),
        }
    }

    if cancelled(cancel) {
        trace(|| "cancelled by the user".to_owned());
        return cancelled_outcome(&device);
    }

    // 4. The token Lenovo's CDN serves for the device.
    let url = token_url(info);
    trace(|| format!("attempt 4: downloading {url}"));

    match download_token(&url, token) {
        Ok(()) => {
            trace(|| "the CDN token was downloaded; flashing it".to_owned());
            events(GuidedEvent::AwaitingConfirm);

            if flash_and_unlock(&device, token, platform, cancel) {
                GuidedOutcome::Unlocked
            } else if cancelled(cancel) {
                trace(|| "cancelled by the user".to_owned());
                cancelled_outcome(&device)
            } else {
                GuidedOutcome::Failed(
                    "the unlock token was flashed but the unlock was refused".to_owned(),
                )
            }
        }
        Err(error) => {
            trace(|| format!("the CDN token could not be downloaded: {error}"));
            GuidedOutcome::ManualRequired
        }
    }
}

/// True when "Cancel & Restart" asked the running job to stop.
fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

/// The outcome for a run the user cancelled.
///
/// The tablet is rebooted out of fastboot first, so cancelling leaves it
/// booting the normal system instead of sitting in the bootloader.
fn cancelled_outcome(device: &fastboot::FastbootDevice<'_>) -> GuidedOutcome {
    reboot_to_system(device);
    GuidedOutcome::Cancelled
}

/// The manual-flow counterpart of [`cancelled_outcome`].
fn cancelled_unlock_outcome(device: &fastboot::FastbootDevice<'_>) -> UnlockOutcome {
    reboot_to_system(device);
    UnlockOutcome::Cancelled
}

/// Reboots the tablet back into the normal system.
///
/// Send-and-forget: the tablet leaves fastboot while it answers — or without
/// answering at all — so the reply is never waited for.
fn reboot_to_system(device: &fastboot::FastbootDevice<'_>) {
    trace(|| "rebooting the tablet back to the system".to_owned());
    let _ = device.send_reboot(None);
}

/// True when the device runs userspace fastboot (`fastbootd`).
fn is_userspace(device: &fastboot::FastbootDevice<'_>) -> bool {
    crate::fastboot_info::getvar_value(device, "is-userspace")
        .map(|value| value.trim().eq_ignore_ascii_case("yes"))
        .unwrap_or(false)
}

/// Whether OEM unlocking is enabled, from `flashing get_unlock_ability`.
/// `None` when the bootloader does not answer.
fn unlock_ability(device: &fastboot::FastbootDevice<'_>) -> Option<bool> {
    let lines = device
        .command_info_with_fail_details("flashing get_unlock_ability")
        .ok()?;

    parse_unlock_ability(&lines)
}

/// Parses the `flashing get_unlock_ability` reply: the bootloader answers
/// either `get_unlock_ability: 0`/`1` or `unlock_ability is false`/`true`.
fn parse_unlock_ability(lines: &[String]) -> Option<bool> {
    let text = lines.join("\n").to_ascii_lowercase();

    if text.contains("false") {
        return Some(false);
    }
    if text.contains("true") {
        return Some(true);
    }

    text.split(|c: char| !c.is_ascii_digit())
        .filter(|token| !token.is_empty())
        .next_back()
        .and_then(|token| token.parse::<u32>().ok())
        .map(|value| value != 0)
}

/// Sends a raw command and reports whether the bootloader accepted it.
fn command_ok(device: &fastboot::FastbootDevice<'_>, command: &str) -> bool {
    match device.command_info_with_fail_details(command) {
        Ok(_) => true,
        Err(error) => {
            #[cfg(debug_assertions)]
            eprintln!("[tablet] guided: `{command}` failed: {error}");
            #[cfg(not(debug_assertions))]
            let _ = &error;

            false
        }
    }
}

/// Sends `oem unlock-go`, retrying while the tablet waits for the user to
/// accept its on-device unlock confirmation.
///
/// The unlock token has already been flashed when this is called, so a retry
/// only sends the unlock command again — nothing is written to the device. Each
/// attempt makes the tablet show its confirmation prompt afresh, which gives
/// the user another chance to accept it before the loop gives up.
fn unlock_go_with_retries(
    device: &fastboot::FastbootDevice<'_>,
    platform: Option<Platform>,
    cancel: &AtomicBool,
) -> bool {
    for attempt in 1..=UNLOCK_GO_ATTEMPTS {
        if cancelled(cancel) {
            return false;
        }

        trace(|| format!("`oem unlock-go` attempt {attempt}/{UNLOCK_GO_ATTEMPTS}"));

        if attempt_unlock(device, "oem unlock-go", platform, cancel) {
            return true;
        }

        if cancelled(cancel) {
            return false;
        }

        if attempt < UNLOCK_GO_ATTEMPTS {
            trace(|| "the tablet did not confirm the unlock; trying again".to_owned());
            std::thread::sleep(UNLOCK_GO_RETRY_DELAY);
        }
    }

    false
}

/// Sends one unlock command and waits for the tablet to actually unlock.
///
/// A MediaTek tablet blocks the command until the user accepts its confirmation
/// (Volume Up), so an OKAY already means the unlock happened. A Qualcomm tablet
/// answers OKAY *before* the user confirms and then keeps the confirmation on
/// screen, so the reboot that follows a confirmed unlock is what is waited for.
fn attempt_unlock(
    device: &fastboot::FastbootDevice<'_>,
    command: &str,
    platform: Option<Platform>,
    cancel: &AtomicBool,
) -> bool {
    if !command_ok(device, command) {
        return false;
    }

    if platform == Some(Platform::Qualcomm) {
        return wait_for_reboot(device, cancel);
    }

    true
}

/// Waits for a Qualcomm tablet to finish the unlock it is prompting for.
///
/// A confirmed unlock reboots the tablet, so its fastboot interface stops
/// answering. A reply that is not a `FAIL` (the bootloader simply does not know
/// the variable) therefore means the tablet is gone and the unlock went
/// through.
fn wait_for_reboot(device: &fastboot::FastbootDevice<'_>, cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + QUALCOMM_CONFIRM_WINDOW;

    loop {
        if cancelled(cancel) || Instant::now() >= deadline {
            return false;
        }

        std::thread::sleep(CONFIRM_POLL_INTERVAL);

        match crate::fastboot_info::getvar_value(device, "serialno") {
            Ok(_) => {}
            Err(error) if error.starts_with("FAIL:") || error.starts_with('?') => {}
            Err(_) => return true,
        }
    }
}

/// Flashes `token` to the `unlock` partition, then runs `oem unlock-go`.
fn flash_and_unlock(
    device: &fastboot::FastbootDevice<'_>,
    token: &Path,
    platform: Option<Platform>,
    cancel: &AtomicBool,
) -> bool {
    if let Err(error) = device.flash_file("unlock", token, None) {
        #[cfg(debug_assertions)]
        eprintln!("[tablet] guided: flash unlock failed: {}", raw_failure(&error));
        #[cfg(not(debug_assertions))]
        let _ = &error;

        return false;
    }

    unlock_go_with_retries(device, platform, cancel)
}

/// Writes a generated `sn.img` and flashes it.
fn write_and_flash(
    device: &fastboot::FastbootDevice<'_>,
    token: &Path,
    image: &[u8],
    platform: Option<Platform>,
    cancel: &AtomicBool,
) -> bool {
    if let Some(parent) = token.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            #[cfg(debug_assertions)]
            eprintln!(
                "[tablet] guided: could not create {}: {error}",
                parent.display()
            );
            #[cfg(not(debug_assertions))]
            let _ = error;

            return false;
        }
    }

    if let Err(error) = std::fs::write(token, image) {
        #[cfg(debug_assertions)]
        eprintln!("[tablet] guided: could not write the token: {error}");
        #[cfg(not(debug_assertions))]
        let _ = error;

        return false;
    }

    flash_and_unlock(device, token, platform, cancel)
}

/// Waits for a device that was just rebooted out of `fastbootd` to come back
/// in bootloader mode.
fn reconnect_after_reboot<'a>(serial: &str) -> Option<fastboot::FastbootDevice<'a>> {
    let deadline = Instant::now() + Duration::from_secs(60);

    std::thread::sleep(Duration::from_secs(2));

    loop {
        if let Ok(device) = fastboot::FastbootDevice::connect(serial) {
            if !is_userspace(&device) {
                return Some(device);
            }
        }

        if Instant::now() >= deadline {
            return None;
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_serials_are_left_padded() {
        assert_eq!(pad_serial("123456"), "00123456");
        assert_eq!(pad_serial("12345678"), "12345678");
        assert_eq!(pad_serial("123456789"), "123456789");
        assert_eq!(pad_serial("  42  "), "00000042");
    }

    #[test]
    fn enhanced_endpoint_uses_the_bootloader_sn() {
        let info = TabletUnlockInfo {
            serial: "00123456".to_owned(),
            bootloader_sn_part1: "ABCD".to_owned(),
            bootloader_sn_part2: "EFGH".to_owned(),
            ..TabletUnlockInfo::default()
        };

        assert_eq!(
            token_url(&info),
            "http://cdn.zui.lenovomm.com/developer/enhancedboot/ABCDEFGH/sn.img"
        );
    }

    #[test]
    fn plain_endpoint_uses_the_serial() {
        let info = TabletUnlockInfo {
            serial: "00123456".to_owned(),
            ..TabletUnlockInfo::default()
        };

        assert_eq!(
            token_url(&info),
            "http://cdn.zui.lenovomm.com/developer/tabletboot/00123456/sn.img"
        );
    }

    #[test]
    fn the_token_url_is_never_https() {
        // The endpoint has no TLS, so an accidental upgrade would break it.
        assert!(token_url(&TabletUnlockInfo::default()).starts_with("http://"));
    }

    #[test]
    fn the_bootloader_sn_is_the_two_parts_concatenated() {
        let info = TabletUnlockInfo {
            serial: "x".to_owned(),
            bootloader_sn_part1: "AAA".to_owned(),
            bootloader_sn_part2: "BBB".to_owned(),
            ..TabletUnlockInfo::default()
        };

        assert_eq!(info.bootloader_sn(), "AAABBB");
        assert!(info.has_bootloader_sn());
        assert!(!TabletUnlockInfo::default().has_bootloader_sn());
    }

    #[test]
    fn wrapper_prefixes_are_stripped_from_failures() {
        assert_eq!(
            raw_failure("Flash failed: Partition not found"),
            "Partition not found"
        );
        assert_eq!(raw_failure("W: timeout"), "W: timeout");
    }

    #[test]
    fn unlock_ability_is_read_from_either_reply() {
        assert_eq!(
            parse_unlock_ability(&["(bootloader) get_unlock_ability: 1".to_owned()]),
            Some(true)
        );
        assert_eq!(
            parse_unlock_ability(&["(bootloader) get_unlock_ability: 0".to_owned()]),
            Some(false)
        );
        assert_eq!(
            parse_unlock_ability(&["unlock_ability is false".to_owned()]),
            Some(false)
        );
        assert_eq!(
            parse_unlock_ability(&["unlock_ability is true".to_owned()]),
            Some(true)
        );
        assert_eq!(parse_unlock_ability(&[]), None);
        assert_eq!(
            parse_unlock_ability(&["(bootloader) nothing useful".to_owned()]),
            None
        );
    }

    #[test]
    fn device_labels_show_the_product() {
        let with_product = TabletDevice {
            serial: "ZY22ABC".to_owned(),
            product: "TB320FC".to_owned(),
        };
        assert_eq!(with_product.label(), "ZY22ABC (TB320FC)");

        let without = TabletDevice {
            serial: "ZY22ABC".to_owned(),
            product: "  ".to_owned(),
        };
        assert_eq!(without.label(), "ZY22ABC");
    }
}
