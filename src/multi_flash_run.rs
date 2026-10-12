//! The run of the "Multi Flashing" page (Mode 2).
//!
//! A *session* watches the USB bus: every device that appears is identified,
//! the first preset whose conditions accept it is flashed to it, and what
//! happens is written to a log file of its own under
//! `Documents\lmnflash\logs\multiflash_<moment the run started>`.
//!
//! One thread watches the bus, and every device is served by a thread of its
//! own: a flash takes minutes and several phones are flashed at the same time.
//! Stopping a session only stops the *watching* — a device that is being
//! written right now runs to the end, because pulling the cable out of a phone
//! mid-flash is not what the user's "Stop" button should do.
//!
//! Like the other feature modules this one holds no UI state: it reports what
//! it does through [`SessionEvent`]s.

use std::collections::HashSet;
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::fastboot_info::{self, DeviceFacts, SecureState};
use crate::flash_engine::{self, Engine, FlashEvent, FlashJob, Mfastboot};
use crate::flash_log::{self, FlashLog};
use crate::flashfile;
use crate::{FlashConditions, MultiFlashPreset, PresetTool};

/// How often the session looks for devices that just appeared.
///
/// A device is only looked at when it is new to the session, so the interval is
/// about how long a phone waits to be noticed, not about how much work each
/// look costs.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// What a running session reports to the page.
#[derive(Debug, Clone)]
pub(crate) enum SessionEvent {
    /// The directory this run's logs are written to.
    LogDirectory(PathBuf),
    /// The session could not run at all (its log directory could not be made).
    Failed(String),
    /// A device was read, and the session decided what to do with it:
    /// `steps` is the number of procedures the preset that was chosen for it
    /// has, or `None` when no preset matches the device.
    Identified {
        serial: String,
        xt_code: Option<String>,
        cid: Option<String>,
        steps: Option<usize>,
    },
    /// A procedure of the preset started (`index` is 0-based).
    Step {
        serial: String,
        index: usize,
        total: usize,
        label: String,
    },
    /// Byte progress inside that procedure.
    Progress {
        serial: String,
        done: u64,
        total: u64,
    },
    /// The device is done: flashed (and rebooted, when the preset asks for
    /// that), or left as it was with the reason.
    Finished {
        serial: String,
        result: Result<(), String>,
    },
    /// The session stopped watching the bus. The devices that are still being
    /// flashed keep going.
    Stopped,
}

/// The directory a run's logs go into:
/// `Documents\lmnflash\logs\multiflash_yyyyMMdd_HHmmss`, named after the
/// moment the run started, in local time (see [`crate::flash_log`]).
pub(crate) fn log_directory() -> PathBuf {
    flash_log::logs_root().join(format!("multiflash_{}", flash_log::timestamp()))
}

/// Watches for devices and flashes them, until `stop` is set.
///
/// Meant to run on a blocking worker thread; the devices it serves get a thread
/// each and are *not* stopped by `stop`.
pub(crate) fn run(
    presets: Vec<MultiFlashPreset>,
    tools: Vec<Mfastboot>,
    platform_tools: Option<Mfastboot>,
    directory: PathBuf,
    emit: Arc<dyn Fn(SessionEvent) + Send + Sync>,
    stop: Arc<AtomicBool>,
) {
    if let Err(error) = fs::create_dir_all(&directory) {
        emit(SessionEvent::Failed(format!(
            "could not create {}: {error}",
            directory.display()
        )));
        return;
    }

    emit(SessionEvent::LogDirectory(directory.clone()));

    // One attempt per serial: a device is flashed once per run, and a phone
    // that comes back (rebooting into fastboot after its flash, say) is left
    // alone. A device that was refused counts as served too — it would be
    // refused again.
    let mut seen: HashSet<String> = HashSet::new();

    while !stop.load(Ordering::Relaxed) {
        // Only the list of serials is read here (a USB descriptor walk); a
        // device's variables are read when it is served, on its own thread.
        let serials = fastboot::FastbootDevice::list_devices().unwrap_or_default();

        for serial in serials {
            // Stop is answered between devices: what is being flashed keeps
            // being flashed, nothing new starts.
            if stop.load(Ordering::Relaxed) {
                break;
            }

            if !seen.insert(serial.clone()) {
                continue;
            }

            let presets = presets.clone();
            let tools = tools.clone();
            let platform_tools = platform_tools.clone();
            let directory = directory.clone();
            let emit = Arc::clone(&emit);

            std::thread::spawn(move || {
                let reporter = Arc::clone(&emit);
                let reported = serial.clone();
                let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    serve(
                        &serial,
                        &presets,
                        &tools,
                        platform_tools.as_ref(),
                        &directory,
                        &*emit,
                    )
                }));

                // A device that dies here would never be reported as done, and
                // a debug build would leave the page believing it is still
                // flashing — which keeps the window from being closed.
                if outcome.is_err() {
                    reporter(SessionEvent::Finished {
                        serial: reported,
                        result: Err("the device worker stopped unexpectedly".to_owned()),
                    });
                }
            });
        }

        std::thread::sleep(POLL_INTERVAL);
    }

    emit(SessionEvent::Stopped);
}

/// Serves one device: reads what it is, picks a preset, flashes it.
///
/// A device that is not a Motorola phone is left alone and not even shown: the
/// table is about the phones this run has something to say about.
fn serve(
    serial: &str,
    presets: &[MultiFlashPreset],
    tools: &[Mfastboot],
    platform_tools: Option<&Mfastboot>,
    directory: &Path,
    emit: &(dyn Fn(SessionEvent) + Send + Sync),
) {
    let facts = match fastboot_info::read_device_facts(serial) {
        Ok(facts) => facts,
        Err(error) => {
            // The device answered when the bus was listed and does not now: it
            // was unplugged, or another tool has it open.
            emit(SessionEvent::Finished {
                serial: serial.to_owned(),
                result: Err(error),
            });
            return;
        }
    };

    if !facts.is_motorola() {
        return;
    }

    let log = Mutex::new(open_device_log(directory, facts.xt_code.as_deref(), serial));
    log_line(&log, &describe_device(serial, &facts));

    // The first preset that accepts the device: the presets are the user's list
    // of firmwares, and each one says which phones it is for.
    let Some(preset) = presets.iter().find(|preset| accepts(preset, &facts).is_ok()) else {
        // Why every firmware was refused is written out: "no preset matches"
        // alone would leave the user guessing which of their presets was close.
        for preset in presets {
            if let Err(reason) = accepts(preset, &facts) {
                log_line(
                    &log,
                    &format!("preset {}: skipped, {reason}", preset_name(preset)),
                );
            }
        }

        log_line(&log, "no preset matches this device");
        emit(SessionEvent::Identified {
            serial: serial.to_owned(),
            xt_code: facts.xt_code.clone(),
            cid: facts.cid.clone(),
            steps: None,
        });
        return;
    };

    log_line(&log, &describe_preset(preset));

    let (engine, note) = preset_engine(&preset.tool, tools, platform_tools);

    if let Some(note) = note {
        log_line(&log, &note);
    }

    log_line(
        &log,
        &format!(
            "tool: {}",
            engine
                .tool()
                .map(Mfastboot::label)
                .unwrap_or_else(|| "the built-in fastboot".to_owned())
        ),
    );
    log_line(
        &log,
        &format!(
            "checksums {}; {}",
            if preset.verify_checksums {
                "verified before every write"
            } else {
                "not checked"
            },
            if preset.reboot {
                "the device is rebooted into the system afterwards"
            } else {
                "the device stays where it is"
            },
        ),
    );

    // The row appears here, once what the device is and which preset it gets are
    // known; the first procedure follows as soon as the firmware is read.
    emit(SessionEvent::Identified {
        serial: serial.to_owned(),
        xt_code: facts.xt_code.clone(),
        cid: facts.cid.clone(),
        steps: Some(preset.identity.steps.len()),
    });

    // Read from the files the preset was set up with. A ZIP package was
    // unpacked when the preset was added, so nothing is extracted here.
    let package = match preset_package(preset) {
        Ok(package) => package,
        Err(error) => {
            log_line(&log, &format!("failed: {error}"));
            emit(SessionEvent::Finished {
                serial: serial.to_owned(),
                result: Err(error),
            });
            return;
        }
    };

    let job = FlashJob {
        package,
        // The extra erases of the single-device dialog are not part of a
        // preset: what a run erases is what the package and its checklist say.
        extra_erases: Vec::new(),
        engine: engine.clone(),
        serial: serial.to_owned(),
        verify_checksums: preset.verify_checksums,
    };

    let result = flash_to(job, preset, &engine, serial, &log, emit);
    emit(SessionEvent::Finished {
        serial: serial.to_owned(),
        result,
    });
}

/// Runs the job, writes the log, and reports the steps to the page.
///
/// The preset's reboot is part of the run: a phone that was left in the
/// bootloader after its firmware was written is not a finished job.
fn flash_to(
    job: FlashJob,
    preset: &MultiFlashPreset,
    engine: &Engine,
    serial: &str,
    log: &Mutex<FlashLog>,
    emit: &(dyn Fn(SessionEvent) + Send + Sync),
) -> Result<(), String> {
    // The engine wants a `Fn` and the log is written from inside it, so the file
    // sits behind a mutex (the tool's reader thread may report through it too).
    let outcome = Mutex::new(None);
    let on_event = |event: FlashEvent| match event {
        FlashEvent::StepStarted {
            index,
            total,
            label,
        } => {
            log_line(log, &format!("[{}/{}] {}", index + 1, total, label));
            emit(SessionEvent::Step {
                serial: serial.to_owned(),
                index,
                total,
                label,
            });
        }
        FlashEvent::Progress { done, total } => emit(SessionEvent::Progress {
            serial: serial.to_owned(),
            done,
            total,
        }),
        // What the tool printed, one line per reply.
        FlashEvent::Log(line) => log_line(log, &line),
        // The run reports its outcome as an event rather than returning it.
        FlashEvent::Finished(result) => {
            if let Ok(mut outcome) = outcome.lock() {
                *outcome = Some(result);
            }
        }
    };

    flash_engine::run(job, &on_event);

    let mut result = reported_outcome(&outcome);

    if result.is_ok() && preset.reboot {
        log_line(log, &format!("rebooting {serial} into the system"));

        if let Err(error) =
            flash_engine::reboot(engine, serial, flash_engine::RebootMode::System, &on_event)
        {
            result = Err(error);
        }
    }

    match &result {
        Ok(()) => log_line(log, "finished"),
        Err(error) => log_line(log, &format!("failed: {error}")),
    }

    result
}

/// What the flash reported, or a failure when it ended without saying.
fn reported_outcome(outcome: &Mutex<Option<Result<(), String>>>) -> Result<(), String> {
    let Ok(guard) = outcome.lock() else {
        return Err("the outcome of the flash was lost".to_owned());
    };

    guard
        .clone()
        .unwrap_or_else(|| Err("the flash ended without a result".to_owned()))
}

/// The package a preset flashes, read from the files it was set up with.
///
/// A ZIP package was unpacked into its own directory when the preset was added
/// (the user picked where), so its `flashfile.xml` is read there; a package that
/// is already unpacked next to its images is read where it lies.
fn preset_package(preset: &MultiFlashPreset) -> Result<flashfile::FlashPackage, String> {
    let flashfile = match &preset.extract_to {
        Some(directory) => flashfile::find_flashfile(directory).ok_or_else(|| {
            format!(
                "no flashfile.xml is left in {} — unpack the package again",
                directory.display()
            )
        })?,
        // A ZIP without a directory of its own would be unpacked into the
        // work directory the single-device dialog uses — minutes of writing,
        // and two devices of the run would write into the same one.
        None if flashfile::is_zip(&preset.path) => {
            return Err(format!(
                "{} has no directory to unpack into — edit the preset and pick one",
                preset.path.display()
            ));
        }
        None => preset.path.clone(),
    };

    let mut package = flashfile::load(&flashfile, &mut |_, _| {})
        .map_err(|error| format!("{}: {error}", flashfile.display()))?;

    // Only the procedures the preset kept run. The package offers everything it
    // holds — a preset that leaves out the step erasing user data must not have
    // it run anyway.
    let kept: Vec<flashfile::FlashOp> = package
        .steps
        .iter()
        .enumerate()
        .filter(|(index, _)| preset.enabled.get(*index).copied().unwrap_or(true))
        .map(|(_, step)| step.clone())
        .collect();

    if kept.is_empty() {
        return Err("the preset has no procedure left to run".to_owned());
    }

    package.steps = kept;

    Ok(package)
}

/// Whether a preset may be flashed to a device, or why it may not (see
/// [`FlashConditions`]).
///
/// The project code comes first and belongs to no condition: a package is made
/// for one phone, and writing it to another one is the brick the single-device
/// dialog asks the user to confirm. A preset's conditions say which phones *of
/// that model* are accepted, never which model. The rest is the default rule
/// (the one the single-device dialog applies) or the user's own list of CIDs
/// and unlock states, which means exactly what it says.
///
/// The reason is a log line, so a device nothing matches explains itself
/// afterwards.
fn accepts(preset: &MultiFlashPreset, device: &DeviceFacts) -> Result<(), String> {
    // Nothing to compare on either side is not a mismatch: a package without a
    // `vbmeta` image, or a bootloader that reports no `product`, leaves the
    // question open.
    if flashfile::project_code_mismatch(
        preset.identity.codename.as_deref(),
        device.product.as_deref(),
    ) {
        return Err(format!(
            "the firmware is for {}, the device is {}",
            preset.identity.codename.as_deref().unwrap_or("another phone"),
            device.product.as_deref().unwrap_or("an unknown phone"),
        ));
    }

    let state = device.state();

    match &preset.conditions {
        FlashConditions::Default => {
            // A bootloader that was unlocked once and locked again is left
            // alone: it is the state the single-device dialog refuses too.
            if state == Some(SecureState::Relocked) {
                return Err("the bootloader is relocked".to_owned());
            }

            // An engineering bootloader reports a placeholder CID and flashes
            // whatever it is given, so the CID is not what decides — unless the
            // preset was set up not to take one.
            if state == Some(SecureState::Engineering) {
                return if preset.no_engineering {
                    Err("the preset does not take engineering units".to_owned())
                } else {
                    Ok(())
                };
            }

            match (preset.identity.cid.as_deref(), device.cid.as_deref()) {
                (Some(firmware), Some(device_cid))
                    if flashfile::cid_mismatch(firmware, device_cid) =>
                {
                    Err(format!(
                        "the firmware is for CID {firmware}, the device reports {device_cid}"
                    ))
                }
                // Nothing to compare: an open question is not a mismatch.
                _ => Ok(()),
            }
        }
        FlashConditions::Custom { cids, states } => {
            let cid = device.cid.as_deref().unwrap_or_default();

            if !cids
                .iter()
                .any(|wanted| flashfile::normalize_cid(wanted).eq_ignore_ascii_case(cid))
            {
                return Err(format!("its list of CIDs does not hold {cid}"));
            }

            match state {
                Some(state) if states.contains(&state) => Ok(()),
                Some(_) => Err("its list of unlock states does not hold the device's state".to_owned()),
                None => Err("the device reports no unlock state to compare".to_owned()),
            }
        }
    }
}

/// What a preset is called in a log line: its codename, or the file it was set
/// up from when the package carries none.
fn preset_name(preset: &MultiFlashPreset) -> String {
    preset
        .identity
        .codename
        .clone()
        .unwrap_or_else(|| preset.path.display().to_string())
}

/// The pairs of presets of a list that would flash the same phone.
///
/// A run serves a device with the FIRST preset that accepts it, so a list where
/// two of them accept one device flashes a firmware the user did not choose —
/// two firmwares of the same model for the same CID, say. Nothing can decide
/// which of them was meant, so the preset list is not worth closing until the
/// user left one out or gave them apart.
///
/// The question is asked of [`accepts`], the same function a run asks, over the
/// phones the presets themselves talk about: every project code and every CID
/// they name (plus one that none of them names, since a preset may take any),
/// in the unlock states a bootloader is normally in — an engineering one
/// included, which takes any CID and is therefore what two presets of one
/// project usually have in common (that is what "do not match engineering unit"
/// is for). One device is deliberately left out: one that reports no project
/// code at all is taken by every preset, so it would make every pair of presets
/// look ambiguous and the list could never be closed again. A device that
/// reports a super CID is left out the same way — unless both presets name that
/// code, see [`cids`].
pub(crate) fn conflicting_presets(presets: &[MultiFlashPreset]) -> Vec<(usize, usize)> {
    let mut conflicts: Vec<(usize, usize)> = Vec::new();

    for (first, one) in presets.iter().enumerate() {
        for (second, other) in presets.iter().enumerate().skip(first + 1) {
            if ambiguous(one, other) {
                conflicts.push((first, second));
            }
        }
    }

    conflicts
}

/// Whether two presets would flash the same phone.
fn ambiguous(one: &MultiFlashPreset, other: &MultiFlashPreset) -> bool {
    for product in products(one, other) {
        for cid in cids(one, other) {
            for state in WITNESS_STATES {
                let device = DeviceFacts {
                    xt_code: None,
                    cid: Some(cid.clone()),
                    securestate: Some(state.raw().to_owned()),
                    product: Some(product.clone()),
                };

                if accepts(one, &device).is_ok() && accepts(other, &device).is_ok() {
                    return true;
                }
            }
        }
    }

    false
}

/// The unlock states two presets are asked about (see [`conflicting_presets`]).
///
/// A relocked bootloader is not among them: no condition of the default rule
/// takes one, so it never decides an overlap.
const WITNESS_STATES: [SecureState; 3] = [
    SecureState::Locked,
    SecureState::Unlocked,
    // An engineering bootloader takes whatever it is given, so two presets of
    // one project take it together however their CIDs differ.
    SecureState::Engineering,
];

/// The project codes a pair of presets has to be asked about: the ones they
/// name, or one they both leave open when neither names any.
fn products(one: &MultiFlashPreset, other: &MultiFlashPreset) -> Vec<String> {
    let mut names: Vec<String> = [&one.identity.codename, &other.identity.codename]
        .into_iter()
        .flatten()
        .cloned()
        .collect();

    if names.is_empty() {
        // A project code no package carries, so a preset without one is still
        // asked (it takes any project).
        names.push("unnamed".to_owned());
    }

    names.sort();
    names.dedup();

    names
}

/// The device CIDs a pair of presets has to be asked about: every CID they name
/// plus one none of them does, since a preset may take any CID at all.
fn cids(one: &MultiFlashPreset, other: &MultiFlashPreset) -> Vec<String> {
    let (first, second) = (named_cids(one), named_cids(other));
    let mut cids: Vec<String> = first.iter().chain(&second).cloned().collect();

    // A super CID is asked about only when BOTH presets name it. A phone that
    // reports one takes firmware of any region, so every preset matches it —
    // a firmware for factory units and one for retail phones alike — and its
    // code therefore says nothing about which of them the user meant. Two
    // presets that do name it are two answers for those very phones, which is
    // worth saying.
    cids.retain(|cid| {
        !flashfile::cid_allows_any_region(cid)
            || (names(&first, cid) && names(&second, cid))
    });

    // A code none of them names is added, so a preset that takes any CID is
    // asked about one it has never seen.
    let mut unknown = 0x7100;
    let mut cid = format!("0x{unknown:04X}");

    while cids.iter().any(|listed| listed.eq_ignore_ascii_case(&cid)) {
        unknown += 1;
        cid = format!("0x{unknown:04X}");
    }

    cids.push(cid);
    cids.sort();
    cids.dedup();

    cids
}

/// The CIDs a preset names itself: what its package is for (the default rule),
/// or the list a condition written by hand accepts. Both are normalized, the
/// way a run compares them.
fn named_cids(preset: &MultiFlashPreset) -> Vec<String> {
    match &preset.conditions {
        FlashConditions::Default => preset
            .identity
            .cid
            .iter()
            .map(|cid| flashfile::normalize_cid(cid))
            .collect(),
        FlashConditions::Custom { cids, .. } => {
            cids.iter().map(|cid| flashfile::normalize_cid(cid)).collect()
        }
    }
}

/// Whether one of these CIDs is `cid`, however the two are spelled.
fn names(cids: &[String], cid: &str) -> bool {
    cids.iter().any(|listed| listed.eq_ignore_ascii_case(cid))
}

/// The tool a preset's run uses.
///
/// A preset names its tool so its firmware is flashed with the build it was
/// packaged for. A build this machine does not have is not worth failing a
/// device over: the run falls back to the built-in fastboot and says so in the
/// log.
fn preset_engine(
    tool: &PresetTool,
    tools: &[Mfastboot],
    platform_tools: Option<&Mfastboot>,
) -> (Engine, Option<String>) {
    match tool {
        PresetTool::Builtin => (Engine::Builtin, None),
        PresetTool::Shipped(version) => {
            match tools.iter().find(|tool| tool.version == *version) {
                Some(tool) => (Engine::Mfastboot(tool.clone()), None),
                None => (
                    Engine::Builtin,
                    Some(format!(
                        "mfastboot {version} is not on this machine; using the built-in fastboot"
                    )),
                ),
            }
        }
        PresetTool::PlatformTools => match platform_tools {
            Some(tool) => (Engine::Mfastboot(tool.clone()), None),
            None => (
                Engine::Builtin,
                Some(
                    "Google's platform-tools are not installed; using the built-in fastboot"
                        .to_owned(),
                ),
            ),
        },
    }
}

/// A device as one log line: what its bootloader reported about it.
fn describe_device(serial: &str, facts: &DeviceFacts) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(xt_code) = &facts.xt_code {
        parts.push(format!("XT code {xt_code}"));
    }

    if let Some(cid) = &facts.cid {
        parts.push(format!("CID {cid}"));
    }

    if let Some(product) = &facts.product {
        parts.push(format!("product {product}"));
    }

    if let Some(state) = &facts.securestate {
        parts.push(format!("bootloader {state}"));
    }

    if parts.is_empty() {
        format!("device {serial}: the bootloader reported no identity")
    } else {
        format!("device {serial}: {}", parts.join(", "))
    }
}

/// A preset as one log line: which firmware it is, and how much of it runs.
fn describe_preset(preset: &MultiFlashPreset) -> String {
    let identity = &preset.identity;
    let name = preset_name(preset);
    let version = identity
        .software_version
        .as_deref()
        .map(|version| format!(", software version {version}"))
        .unwrap_or_default();
    let enabled = preset
        .enabled
        .iter()
        .filter(|enabled| **enabled)
        .count();

    format!(
        "preset {name}{version}: {enabled} of {} procedures of {}",
        identity.steps.len(),
        preset.path.display()
    )
}

/// One device's log file: `log_[XT code]_[serial].log` in the run's directory.
fn open_device_log(directory: &Path, xt_code: Option<&str>, serial: &str) -> FlashLog {
    FlashLog::create(directory.join(format!(
        "log_{}_{}.log",
        flash_log::file_name_part(xt_code.unwrap_or("unknown")),
        flash_log::file_name_part(serial)
    )))
}

/// Writes one line to a device's log.
fn log_line(log: &Mutex<FlashLog>, line: &str) {
    // A poisoned lock is a panic on the thread that writes the log; the flash
    // itself is unaffected, so the line is simply dropped.
    if let Ok(mut log) = log.lock() {
        log.line(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::flashfile::FlashOp;

    /// A package identity with one `flash` step per partition name.
    fn identity(cid: Option<&str>, partitions: &[&str]) -> flashfile::PackageIdentity {
        flashfile::PackageIdentity {
            codename: Some("arcfox".to_owned()),
            cid: cid.map(str::to_owned),
            steps: partitions
                .iter()
                .map(|partition| FlashOp::Flash {
                    partition: (*partition).to_owned(),
                    file: format!("{partition}.img"),
                    md5: None,
                })
                .collect(),
            ..flashfile::PackageIdentity::default()
        }
    }

    /// A preset with that identity and the given conditions.
    fn preset(conditions: FlashConditions, cid: Option<&str>) -> MultiFlashPreset {
        MultiFlashPreset {
            path: PathBuf::from("firmware.xml"),
            identity: identity(cid, &["boot", "system"]),
            enabled: vec![true, true],
            reboot: false,
            verify_checksums: true,
            tool: PresetTool::Builtin,
            extract_to: None,
            conditions,
            no_engineering: false,
        }
    }

    /// Whether that preset may be flashed to that device.
    fn matches(preset: &MultiFlashPreset, device: &DeviceFacts) -> bool {
        accepts(preset, device).is_ok()
    }

    /// What a device reports.
    fn device(cid: &str, state: Option<SecureState>) -> DeviceFacts {
        DeviceFacts {
            xt_code: Some("XT2451-2".to_owned()),
            cid: Some(flashfile::normalize_cid(cid)),
            securestate: state.map(|state| state.raw().to_owned()),
            product: Some("arcfox".to_owned()),
        }
    }

    #[test]
    fn the_default_rule_needs_the_cid_to_match() {
        let conditions = FlashConditions::Default;

        assert!(matches(
            &preset(conditions.clone(), Some("0x0032")),
            &device("0x0032", Some(SecureState::Unlocked))
        ));
        assert!(!matches(
            &preset(conditions.clone(), Some("0x0032")),
            &device("0x0042", Some(SecureState::Unlocked))
        ));
        // A super CID takes firmware of any region.
        assert!(matches(
            &preset(conditions, Some("0x0032")),
            &device("0x0000", Some(SecureState::Unlocked))
        ));
    }

    #[test]
    fn the_default_rule_leaves_a_relocked_bootloader_alone() {
        let conditions = FlashConditions::Default;

        assert!(!matches(
            &preset(conditions.clone(), Some("0x0032")),
            &device("0x0032", Some(SecureState::Relocked))
        ));
        // An engineering bootloader flashes what it is given, whatever its CID.
        assert!(matches(
            &preset(conditions, Some("0x0032")),
            &device("0xDEAD", Some(SecureState::Engineering))
        ));
    }

    /// An engineering bootloader reports a placeholder CID, so the default rule
    /// takes it whatever the firmware is for — which a preset meant for
    /// production units can refuse.
    #[test]
    fn the_default_rule_can_be_told_not_to_take_an_engineering_unit() {
        let strict = MultiFlashPreset {
            no_engineering: true,
            ..preset(FlashConditions::Default, Some("0x0032"))
        };

        let reason = accepts(&strict, &device("0x0032", Some(SecureState::Engineering)))
            .expect_err("an engineering unit is not taken");
        assert!(reason.contains("engineering"), "{reason}");

        // Every other state it takes is untouched, and a device whose CID
        // matches is still flashed.
        assert!(matches(
            &strict,
            &device("0x0032", Some(SecureState::Unlocked))
        ));
        assert!(matches(&strict, &device("0x0032", Some(SecureState::Locked))));
        assert!(!matches(
            &strict,
            &device("0x0042", Some(SecureState::Unlocked))
        ));
    }

    #[test]
    fn the_default_rule_flashes_when_there_is_nothing_to_compare() {
        let conditions = FlashConditions::Default;

        // A package without a CID (no `vbmeta` image and no `<cid_value>`).
        assert!(matches(
            &preset(conditions.clone(), None),
            &device("0x0032", Some(SecureState::Locked))
        ));
        // A bootloader state this app does not know.
        assert!(matches(
            &preset(conditions, Some("0x0032")),
            &device("0x0032", None)
        ));
    }

    #[test]
    fn a_custom_condition_is_an_explicit_list() {
        let conditions = FlashConditions::Custom {
            cids: vec!["0x0032".to_owned(), "50".to_owned()],
            states: vec![SecureState::Unlocked],
        };

        assert!(matches(
            &preset(conditions.clone(), Some("0x0000")),
            &device("0x0032", Some(SecureState::Unlocked))
        ));
        // The CID is listed, the unlock state is not.
        assert!(!matches(
            &preset(conditions.clone(), Some("0x0000")),
            &device("0x0032", Some(SecureState::Locked))
        ));
        // Listed in the decimal form `50`, which is the same code as `0x0032`.
        assert!(matches(
            &preset(conditions.clone(), Some("0x0000")),
            &device("0x0032", Some(SecureState::Unlocked))
        ));
        // A super CID is not what the list says.
        assert!(!matches(
            &preset(conditions.clone(), Some("0x0000")),
            &device("0x0000", Some(SecureState::Unlocked))
        ));
        // A state the device does not report cannot be in the list.
        assert!(!matches(
            &preset(conditions, Some("0x0000")),
            &device("0x0032", None)
        ));
    }

    /// A firmware made for another phone is left alone whatever its conditions
    /// say — writing it anyway is the brick the single-device dialog asks the
    /// user to confirm, and a run has nobody to ask. This is what a device with
    /// an engineering bootloader hit: the placeholder CID `0xDEAD` makes the
    /// default rule accept the CID, so the project code is what has to decide.
    #[test]
    fn a_firmware_of_another_project_is_refused() {
        let xpeng = MultiFlashPreset {
            identity: flashfile::PackageIdentity {
                codename: Some("xpeng".to_owned()),
                ..identity(Some("0xDEAD"), &["boot"])
            },
            ..preset(FlashConditions::Default, Some("0xDEAD"))
        };
        // The device the user plugged in: an arcfox with an engineering
        // bootloader, which reports `0xDEAD` as its CID.
        let arcfox = device("0xDEAD", Some(SecureState::Engineering));

        let reason = accepts(&xpeng, &arcfox).expect_err("another project is not flashed");
        assert!(reason.contains("xpeng"), "{reason}");
        assert!(reason.contains("arcfox"), "{reason}");

        // No condition makes it acceptable, not even an explicit list that
        // names this device's CID and unlock state.
        let custom = MultiFlashPreset {
            conditions: FlashConditions::Custom {
                cids: vec!["0xDEAD".to_owned()],
                states: vec![SecureState::Engineering],
            },
            ..xpeng
        };
        assert!(!matches(&custom, &arcfox));
    }

    #[test]
    fn a_project_code_the_bootloader_omits_is_not_a_mismatch() {
        // A package without a `vbmeta` image, and a device whose bootloader
        // reports no `product`: there is nothing to compare.
        let nameless = MultiFlashPreset {
            identity: flashfile::PackageIdentity {
                codename: None,
                ..identity(Some("0x0032"), &["boot"])
            },
            ..preset(FlashConditions::Default, Some("0x0032"))
        };
        let unknown = DeviceFacts {
            product: None,
            ..device("0x0032", Some(SecureState::Unlocked))
        };

        assert!(matches(&nameless, &unknown));
        assert!(matches(&nameless, &device("0x0032", Some(SecureState::Unlocked))));
    }

    /// Two firmwares of the same model for the same CID are the case the
    /// conflict warning is for: a run would flash whichever of them comes
    /// first, which is not a choice the user made.
    #[test]
    fn two_presets_for_the_same_phone_conflict() {
        let one = preset(FlashConditions::Default, Some("0x0032"));
        let other = MultiFlashPreset {
            path: PathBuf::from("other.xml"),
            ..preset(FlashConditions::Default, Some("0x0032"))
        };

        assert_eq!(conflicting_presets(&[one.clone(), other]), [(0, 1)]);

        // A preset that takes any CID conflicts with all of them.
        let any_cid = preset(FlashConditions::Default, None);
        assert_eq!(conflicting_presets(&[one.clone(), any_cid]), [(0, 1)]);

        // A custom list that names the CID the other preset is for conflicts as
        // well — and one that names another CID does not.
        let listed = MultiFlashPreset {
            conditions: FlashConditions::Custom {
                cids: vec!["0x0032".to_owned()],
                states: vec![SecureState::Unlocked],
            },
            ..preset(FlashConditions::Default, Some("0x0000"))
        };
        assert_eq!(conflicting_presets(&[one.clone(), listed]), [(0, 1)]);

        let other_cid = MultiFlashPreset {
            conditions: FlashConditions::Custom {
                cids: vec!["0x0042".to_owned()],
                states: vec![SecureState::Unlocked],
            },
            ..preset(FlashConditions::Default, Some("0x0000"))
        };
        assert!(conflicting_presets(&[one.clone(), other_cid]).is_empty());
    }

    #[test]
    fn presets_of_different_projects_do_not_conflict() {
        let arcfox = preset(FlashConditions::Default, Some("0x0032"));
        let xpeng = MultiFlashPreset {
            identity: flashfile::PackageIdentity {
                codename: Some("xpeng".to_owned()),
                ..identity(Some("0x0032"), &["boot"])
            },
            ..preset(FlashConditions::Default, Some("0x0032"))
        };

        // The same CID, but each phone is one project: a device is only ever
        // one of them.
        assert!(conflicting_presets(&[arcfox.clone(), xpeng.clone()]).is_empty());

        // A preset whose package names no project takes any phone, so it does
        // conflict with both of the others (not with itself: it is not in the
        // list twice).
        let nameless = MultiFlashPreset {
            identity: flashfile::PackageIdentity {
                codename: None,
                ..identity(Some("0x0032"), &["boot"])
            },
            ..preset(FlashConditions::Default, Some("0x0032"))
        };
        assert_eq!(
            conflicting_presets(&[arcfox, nameless, xpeng]),
            [(0, 1), (1, 2)]
        );
    }

    /// An engineering bootloader takes any CID and a device that reports no
    /// project code is taken by every preset, so neither can be what decides
    /// whether two presets are meant for the same phones — every pair would
    /// look ambiguous, and the list could never be closed again.
    #[test]
    fn the_cases_every_preset_takes_do_not_make_a_conflict() {
        let one = preset(FlashConditions::Default, Some("0x0032"));
        let other = MultiFlashPreset {
            path: PathBuf::from("other.xml"),
            ..preset(FlashConditions::Default, Some("0x0032"))
        };
        let different_project = MultiFlashPreset {
            identity: flashfile::PackageIdentity {
                codename: Some("xpeng".to_owned()),
                ..identity(Some("0x0032"), &["boot"])
            },
            ..preset(FlashConditions::Default, Some("0x0032"))
        };

        // The project code is what tells these two apart, even though both
        // would take an engineering device with the same CID.
        assert!(conflicting_presets(&[one.clone(), different_project]).is_empty());
        // Unlock states that do not overlap: one list takes only unlocked
        // devices, the other only locked ones.
        let locked = MultiFlashPreset {
            conditions: FlashConditions::Custom {
                cids: vec!["0x0032".to_owned()],
                states: vec![SecureState::Locked],
            },
            ..preset(FlashConditions::Default, Some("0x0000"))
        };
        let unlocked = MultiFlashPreset {
            conditions: FlashConditions::Custom {
                cids: vec!["0x0032".to_owned()],
                states: vec![SecureState::Unlocked],
            },
            ..preset(FlashConditions::Default, Some("0x0000"))
        };
        assert!(conflicting_presets(&[locked, unlocked]).is_empty());

        // The list is otherwise fine: two presets that are told apart and one
        // entry is not a conflict either.
        assert!(conflicting_presets(&[one.clone()]).is_empty());
        assert!(conflicting_presets(&[other]).is_empty());
    }

    /// A phone that reports a super CID takes firmware of any region, so a
    /// firmware for factory units (`0x00FF`, `0xDEAD`) and one for retail
    /// phones (`0x0032`) are not two answers for it: the code says nothing
    /// about which of them the user meant. Only the CIDs that do tell the two
    /// apart — and an engineering unit, which the "do not match engineering
    /// unit" option settles — may make a conflict.
    #[test]
    fn a_super_cid_firmware_does_not_conflict_with_a_retail_one() {
        let factory = MultiFlashPreset {
            conditions: FlashConditions::Custom {
                cids: vec!["0x00FF".to_owned(), "0xDEAD".to_owned()],
                states: vec![
                    SecureState::Engineering,
                    SecureState::Unlocked,
                    SecureState::Locked,
                ],
            },
            ..preset(FlashConditions::Default, Some("0x00FF"))
        };
        let retail = MultiFlashPreset {
            no_engineering: true,
            path: PathBuf::from("other.xml"),
            ..preset(FlashConditions::Default, Some("0x0032"))
        };

        assert!(conflicting_presets(&[factory.clone(), retail.clone()]).is_empty());

        // The other way around as well: a retail firmware does not become a
        // conflict by being listed second.
        assert!(conflicting_presets(&[retail, factory.clone()]).is_empty());

        // Two presets that BOTH name the factory CID are two answers for those
        // phones, which is what the warning is for.
        let also_factory = MultiFlashPreset {
            conditions: FlashConditions::Custom {
                cids: vec!["0x00FF".to_owned()],
                states: vec![SecureState::Locked],
            },
            path: PathBuf::from("factory.xml"),
            ..preset(FlashConditions::Default, Some("0x0000"))
        };
        assert_eq!(conflicting_presets(&[factory, also_factory]), [(0, 1)]);
    }

    /// An engineering bootloader takes any CID, so two presets of one project
    /// are two answers for the same phone however their CIDs differ — that is
    /// what "do not match engineering unit" settles.
    #[test]
    fn two_presets_of_one_project_conflict_through_the_engineering_unit() {
        let one = preset(FlashConditions::Default, Some("0x0032"));
        let other = MultiFlashPreset {
            path: PathBuf::from("other.xml"),
            ..preset(FlashConditions::Default, Some("0x0042"))
        };

        // Different CIDs, but the same prototype phone matches both.
        assert_eq!(conflicting_presets(&[one.clone(), other.clone()]), [(0, 1)]);

        // Taking the engineering units out of one of them tells them apart.
        let strict = MultiFlashPreset {
            no_engineering: true,
            ..other.clone()
        };
        assert!(conflicting_presets(&[one.clone(), strict.clone()]).is_empty());

        // Both of them: an engineering unit matches nothing at all, which is
        // the user's decision to make.
        let also_strict = MultiFlashPreset {
            no_engineering: true,
            ..one
        };
        assert!(conflicting_presets(&[also_strict, strict]).is_empty());
    }

    #[test]
    fn only_the_procedures_the_preset_kept_are_flashed() {
        let preset = MultiFlashPreset {
            enabled: vec![true, false],
            ..preset(FlashConditions::Default, Some("0x0032"))
        };
        let package = flashfile::FlashPackage {
            directory: PathBuf::from("."),
            flashfile: PathBuf::from("firmware.xml"),
            model: None,
            software_version: None,
            cid: None,
            project_code: None,
            ignored_partitions: Vec::new(),
            steps: identity(None, &["boot", "system"]).steps,
        };

        // The same filtering `preset_package` does, without reading a file.
        let kept: Vec<&FlashOp> = package
            .steps
            .iter()
            .enumerate()
            .filter(|(index, _)| preset.enabled.get(*index).copied().unwrap_or(true))
            .map(|(_, step)| step)
            .collect();

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].describe(), "flash boot (boot.img)");
    }

    #[test]
    fn a_preset_tool_falls_back_to_the_built_in_fastboot() {
        let shipped = Mfastboot {
            version: "34.0.4".to_owned(),
            path: PathBuf::from("mfastboot"),
            emulator: flash_engine::Emulator::Native,
            origin: flash_engine::Origin::Shipped,
        };
        let tools = vec![shipped.clone()];

        let (engine, note) = preset_engine(&PresetTool::Shipped("34.0.4".to_owned()), &tools, None);
        assert_eq!(engine, Engine::Mfastboot(shipped));
        assert!(note.is_none());

        let (engine, note) = preset_engine(&PresetTool::Shipped("29.0.6".to_owned()), &tools, None);
        assert_eq!(engine, Engine::Builtin);
        assert!(note.is_some_and(|note| note.contains("29.0.6")));

        let (engine, note) = preset_engine(&PresetTool::PlatformTools, &tools, None);
        assert_eq!(engine, Engine::Builtin);
        assert!(note.is_some());

        let (engine, note) = preset_engine(&PresetTool::Builtin, &tools, None);
        assert_eq!(engine, Engine::Builtin);
        assert!(note.is_none());
    }

    #[test]
    fn the_log_directory_is_named_after_the_moment_the_run_started() {
        let directory = log_directory();

        assert_eq!(
            directory.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("logs"))
        );
        assert_eq!(
            directory
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name),
            Some(std::ffi::OsStr::new("lmnflash"))
        );

        let name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let stamp = name
            .strip_prefix("multiflash_")
            .expect("the directory is named after the run");

        // `yyyyMMdd_HHmmss`.
        assert_eq!(stamp.len(), 15);
        assert_eq!(stamp.as_bytes()[8], b'_');
        assert_eq!(
            stamp
                .chars()
                .filter(char::is_ascii_digit)
                .count(),
            14
        );
    }

    #[test]
    fn a_serial_becomes_a_file_name() {
        // The sanitizing itself is tested with the log files it is for
        // (`crate::flash_log`); what matters here is that a serial reaches the
        // file name of a device log unharmed.
        assert_eq!(flash_log::file_name_part("ZY12345678"), "ZY12345678");
        assert_eq!(flash_log::file_name_part("AB:C/D*E?F"), "AB_C_D_E_F");
        assert_eq!(flash_log::file_name_part(""), "unknown");
    }

    #[test]
    fn the_device_line_says_what_the_bootloader_reported() {
        let line = describe_device("ZY12345678", &device("0x0032", Some(SecureState::Unlocked)));
        assert!(line.contains("ZY12345678"));
        assert!(line.contains("XT2451-2"));
        assert!(line.contains("CID 0x0032"));
        assert!(line.contains("product arcfox"));
        assert!(line.contains("bootloader"));

        let line = describe_device("ZY12345678", &DeviceFacts::default());
        assert!(line.contains("no identity"));
    }
}
