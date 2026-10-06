//! Google's `fastboot` from the Android platform-tools.
//!
//! The dialog offers it as a third engine next to the built-in fastboot and the
//! shipped `mfastboot` builds. Google publishes one archive per system instead
//! of versioned downloads, so it is fetched the first time the user picks it and
//! kept in the configuration directory (`<config>/platform-tools/`): the release
//! artifacts stay small, and the build is always the current one.
//!
//! Only the files the tools need at run time are unpacked — the archive also
//! holds the SDK's other files, which would need tens of megabytes here.
//! `adb` is unpacked as well: the "Remove System Bloatware" feature talks to
//! the phone over it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::config;
use crate::flash_engine::{Emulator, Mfastboot, Origin, intel_emulator};

/// The files that are taken out of the archive, compared without case (the
/// Windows archive spells the DLLs `AdbWinApi.dll`).
const NEEDED: &[&str] = &[
    "fastboot",
    "fastboot.exe",
    "adb",
    "adb.exe",
    "AdbWinApi.dll",
    "AdbWinUsbApi.dll",
];

/// How long the download may take. The archive is around 8 MiB; the limit is
/// only there so that a stalled connection cannot hold the dialog forever.
const TIMEOUT: Duration = Duration::from_secs(300);

/// The archive Google publishes for this system, or `None` where there is none.
fn url() -> Option<&'static str> {
    const WINDOWS: &str =
        "https://dl.google.com/android/repository/platform-tools-latest-windows.zip";
    const LINUX: &str =
        "https://dl.google.com/android/repository/platform-tools-latest-linux.zip";
    const DARWIN: &str =
        "https://dl.google.com/android/repository/platform-tools-latest-darwin.zip";

    if cfg!(target_os = "windows") {
        Some(WINDOWS)
    } else if cfg!(target_os = "macos") {
        Some(DARWIN)
    } else if cfg!(target_os = "linux") {
        Some(LINUX)
    } else {
        None
    }
}

/// The executable with the given base name inside `directory`, including
/// this platform's file extension.
fn executable_in(directory: &Path, base: &str) -> PathBuf {
    directory.join(if cfg!(target_os = "windows") {
        format!("{base}.exe")
    } else {
        base.to_owned()
    })
}

/// The `fastboot` executable of the unpacked archive.
fn binary() -> PathBuf {
    executable_in(&install_dir(), "fastboot")
}

/// The `adb` executable of the unpacked archive.
///
/// The "Remove System Bloatware" feature runs the phone commands through it,
/// so `adb` is unpacked next to `fastboot` (see [`NEEDED`]).
pub fn adb() -> PathBuf {
    executable_in(&install_dir(), "adb")
}

/// Whether that `adb` is already on disk.
///
/// A platform-tools download from before `adb` was unpacked has only
/// `fastboot`, so the caller has to download them again (see [`install`]).
pub fn adb_present() -> bool {
    adb().is_file()
}

/// A command that starts `adb`, through an emulator where one is needed (the
/// Linux platform-tools are x86_64, exactly like the shipped `mfastboot`).
pub fn adb_command() -> Command {
    emulator().command(&adb())
}

/// The directory the archive is unpacked into.
fn install_dir() -> PathBuf {
    config::config_dir().join("platform-tools")
}

/// What runs the downloaded build: Google's macOS archive is universal and its
/// Windows one runs on every edition, but the Linux one is x86_64 — which an
/// ARM Linux machine needs box64 for, exactly like the shipped `mfastboot`.
fn emulator() -> Emulator {
    if cfg!(target_os = "macos") {
        Emulator::Native
    } else {
        intel_emulator().unwrap_or(Emulator::Native)
    }
}

/// The engine entry for the picker, or `None` where Google publishes no
/// `fastboot` for this system.
///
/// It is built once when the dialog opens: the entry is what the picker
/// compares the selection with, and asking the system whether it can run the
/// build (Rosetta 2, box64) should not happen on every frame.
pub fn engine() -> Option<Mfastboot> {
    url()?;

    Some(Mfastboot {
        version: "latest".to_owned(),
        path: binary(),
        emulator: emulator(),
        origin: Origin::PlatformTools,
    })
}

/// Downloads the archive and unpacks the tools out of it (`fastboot`, and the
/// `adb` the bloatware feature uses), returning `fastboot`'s path.
pub fn install() -> Result<PathBuf, String> {
    let url = url().ok_or_else(|| "no platform-tools build for this system".to_owned())?;

    let response = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .build()
        .get(url)
        .call()
        .map_err(|error| format!("download failed: {error}"))?;

    let mut archive = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut archive)
        .map_err(|error| format!("download failed: {error}"))?;

    unpack(&archive, &install_dir())
}

/// Writes the files `fastboot` needs out of `archive` into `directory`, and
/// returns the executable's path.
///
/// Only the file names are looked at: everything in the archive lives under
/// `platform-tools/`, and nothing outside [`NEEDED`] is written.
fn unpack(archive: &[u8], directory: &Path) -> Result<PathBuf, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|error| format!("unreadable archive: {error}"))?;

    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unreadable archive: {error}"))?;

        let name = match entry.name().rsplit(['/', '\\']).next() {
            Some(name) => name.to_owned(),
            None => continue,
        };

        if !NEEDED.iter().any(|needed| needed.eq_ignore_ascii_case(&name)) {
            continue;
        }

        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|error| format!("unreadable archive entry {name}: {error}"))?;

        let target = directory.join(&name);
        std::fs::write(&target, &contents)
            .map_err(|error| format!("could not write {}: {error}", target.display()))?;

        // The executable bit is ours to set: the archive does not have it
        // everywhere, and a program that cannot be started is of no use.
        #[cfg(unix)]
        if name.eq_ignore_ascii_case("fastboot") || name.eq_ignore_ascii_case("adb") {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                .map_err(|error| {
                    format!("could not make {} executable: {error}", target.display())
                })?;
        }
    }

    let binary = executable_in(directory, "fastboot");

    if !binary.is_file() {
        return Err("the archive did not contain fastboot".to_owned());
    }

    Ok(binary)
}

/// The directory the `fastboot` alias lives in, next to the tools it stands in
/// for.
fn alias_dir() -> PathBuf {
    install_dir().join("lmnflash-alias")
}

/// The file name of the alias: a batch file on Windows, a script elsewhere.
/// Either way it is found through the `PATH` the terminal is started with.
const ALIAS_FILE: &str = if cfg!(windows) {
    "fastboot.cmd"
} else {
    "fastboot"
};

/// Whether any string value in the registry tree rooted at `key` contains
/// `needle_lower` (which must already be lower-case). Mirrors a recursive
/// `reg query <key> /s /f <needle> /d`, including its case-insensitive,
/// substring match on value data.
#[cfg(target_os = "windows")]
fn key_tree_contains(key: &winreg::RegKey, needle_lower: &str) -> bool {
    use winreg::types::FromRegValue;

    let value_matches = key.enum_values().flatten().any(|(_, value)| {
        String::from_reg_value(&value)
            .map(|text| text.to_lowercase().contains(needle_lower))
            .unwrap_or(false)
    });

    value_matches
        || key
            .enum_keys()
            .flatten()
            .any(|child| match key.open_subkey(&child) {
                Ok(child) => key_tree_contains(&child, needle_lower),
                Err(_) => false,
            })
}

/// Whether "Minimal ADB and Fastboot" is installed (Windows).
///
/// That tool puts its own `adb.exe` and `fastboot.exe` on the `PATH` when it is
/// installed, which would be used instead of the ones unpacked here, so the
/// terminal has to ask the user to uninstall it first.
///
/// The uninstall entries are searched by value data (its `DisplayName` is the
/// only place the name is recorded), across the 64-bit and 32-bit HKLM views
/// and the per-user hive. The registry is read directly rather than shelling
/// out to `reg.exe`.
pub fn minimal_adb_fastboot_installed() -> bool {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        };
        use winreg::RegKey;

        const NAME: &str = "Minimal ADB and Fastboot";
        const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

        // Every view an install can live in: 64-bit HKLM, 32-bit
        // (WOW6432Node) HKLM, and the per-user hive.
        let roots = [
            (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_64KEY),
            (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_32KEY),
            (HKEY_CURRENT_USER, KEY_READ),
        ];
        let needle = NAME.to_lowercase();

        let registered = roots.iter().any(|(hive, flags)| {
            RegKey::predef(*hive)
                .open_subkey_with_flags(UNINSTALL, *flags)
                .is_ok_and(|root| key_tree_contains(&root, &needle))
        });

        if registered {
            return true;
        }

        // A copy that was unpacked rather than installed is not registered,
        // but the installer's own directory is where it would be.
        let program_files = std::env::var_os("ProgramFiles(x86)")
            .or_else(|| std::env::var_os("ProgramFiles"));

        program_files.is_some_and(|root| PathBuf::from(root).join(NAME).is_dir())
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Opens a terminal in the platform-tools directory, with `fastboot` aliased
/// to `tool` when a build is selected, and returns that directory.
///
/// The alias matters because the tools are used from the command line: typing
/// `fastboot` has to run the build the dialog is set to, not Google's copy.
pub fn open_terminal(tool: Option<&Mfastboot>) -> Result<PathBuf, String> {
    let directory = install_dir();

    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    let alias = write_alias(tool)?;
    let banner = match tool {
        Some(tool) => format!("LMN Flash: fastboot now runs {}", tool.label()),
        None => "LMN Flash: platform-tools terminal".to_owned(),
    };

    #[cfg(target_os = "windows")]
    spawn_windows_terminal(&directory, alias.as_deref(), &banner)?;

    // A Unix terminal has no way to be given an environment, so the work is
    // done by a script that the terminal runs.
    #[cfg(not(target_os = "windows"))]
    {
        let script = directory.join(LAUNCHER_FILE);

        std::fs::write(&script, launcher_script(&directory, alias.as_deref(), &banner))
            .map_err(|error| format!("could not write {}: {error}", script.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .map_err(|error| {
                    format!("could not make {} executable: {error}", script.display())
                })?;
        }

        spawn_unix_terminal(&script)?;
    }

    Ok(directory)
}

/// Writes the `fastboot` alias for `tool`, returning the directory holding it,
/// or removes an alias left over from an earlier build when there is nothing
/// to alias.
fn write_alias(tool: Option<&Mfastboot>) -> Result<Option<PathBuf>, String> {
    let directory = alias_dir();
    let alias = directory.join(ALIAS_FILE);

    let Some(tool) = tool else {
        // The dialog is set to the built-in fastboot: an alias written before
        // must not keep shadowing it.
        let _ = std::fs::remove_file(&alias);
        return Ok(None);
    };

    // The alias has to start what the engine starts: on an ARM machine that is
    // the emulator in front of an Intel build, not the build itself.
    let command = tool.emulator.command(&tool.path);
    let program = command.get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    let script = if cfg!(windows) {
        windows_alias(&program, &args)
    } else {
        unix_alias(&program, &args)
    };

    std::fs::write(&alias, script)
        .map_err(|error| format!("could not write {}: {error}", alias.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&alias, std::fs::Permissions::from_mode(0o755)).map_err(
            |error| format!("could not make {} executable: {error}", alias.display()),
        )?;
    }

    Ok(Some(directory))
}

/// The batch shim: `%*` passes the arguments on as they were typed.
fn windows_alias(program: &str, args: &[String]) -> String {
    let mut script = String::from(
        "@echo off\r\nREM LMN Flash: fastboot runs the build selected in the dialog.\r\n",
    );

    script.push_str(&windows_word(program));

    for arg in args {
        script.push(' ');
        script.push_str(&windows_word(arg));
    }

    script.push_str(" %*\r\n");
    script
}

/// The shell shim: `"$@"` passes the arguments on as they were typed.
fn unix_alias(program: &str, args: &[String]) -> String {
    let mut script = String::from(
        "#!/bin/sh\n# LMN Flash: fastboot runs the build selected in the dialog.\nexec",
    );

    for word in std::iter::once(program).chain(args.iter().map(String::as_str)) {
        script.push(' ');
        script.push_str(&shell_word(word));
    }

    script.push_str(" \"$@\"\n");
    script
}

/// `word` as a cmd argument (the quotes are what a path with spaces needs).
fn windows_word(word: &str) -> String {
    format!("\"{word}\"")
}

/// `word` as a single-quoted shell word, so nothing in it is expanded.
fn shell_word(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The script a Unix terminal runs: it puts the alias (and the unpacked tools)
/// first on the `PATH`, enters the directory, and hands over to the shell.
///
/// Built on every platform (it is only *used* off Windows) so that the Unix
/// path is still compiled — and tested — where the application is developed.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn launcher_script(directory: &Path, alias: Option<&Path>, banner: &str) -> String {
    let mut paths = Vec::new();

    if let Some(alias) = alias {
        paths.push(alias.to_string_lossy().into_owned());
    }

    paths.push(directory.to_string_lossy().into_owned());

    let path = paths
        .iter()
        .map(|path| shell_word(path))
        .collect::<Vec<_>>()
        .join(":");

    format!(
        "#!/bin/sh\n\
         # LMN Flash: the terminal opened by the Firmware Flash dialog.\n\
         cd {} || exit 1\n\
         export PATH={path}:\"$PATH\"\n\
         echo {}\n\
         exec \"${{SHELL:-/bin/sh}}\" -i\n",
        shell_word(&directory.to_string_lossy()),
        shell_word(banner),
    )
}

/// The name of that script; Terminal.app only runs a file it recognises as a
/// script, which is what the `.command` extension is for.
#[cfg_attr(target_os = "windows", allow(dead_code))]
const LAUNCHER_FILE: &str = if cfg!(target_os = "macos") {
    "lmnflash-terminal.command"
} else {
    "lmnflash-terminal.sh"
};

/// Starts a console window of its own: the application has none, and the user
/// has to be able to type into it.
#[cfg(target_os = "windows")]
fn spawn_windows_terminal(
    directory: &Path,
    alias: Option<&Path>,
    banner: &str,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    let mut command = Command::new("cmd.exe");
    // `/K` prints the banner and keeps the prompt open.
    command.arg("/K").arg(format!("echo {banner}"));
    command.current_dir(directory);
    command.env("PATH", terminal_path(alias, directory)?);
    // Windows looks in the current directory before it looks at the `PATH`,
    // and the terminal is opened *in* the platform-tools directory: without
    // this, `fastboot` would always find Google's `fastboot.exe` there and
    // never the alias. With it, the `PATH` decides — which is what the alias
    // needs. (The variable is the documented way to turn that lookup off.)
    command.env("NoDefaultCurrentDirectoryInExePath", "1");
    command.creation_flags(CREATE_NEW_CONSOLE);

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start cmd.exe: {error}"))
}

/// The `PATH` the terminal gets: the alias, then the unpacked tools, then what
/// the user already had.
#[cfg(target_os = "windows")]
fn terminal_path(alias: Option<&Path>, directory: &Path) -> Result<std::ffi::OsString, String> {
    let mut paths: Vec<PathBuf> = Vec::new();

    if let Some(alias) = alias {
        paths.push(alias.to_path_buf());
    }

    paths.push(directory.to_path_buf());
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));

    std::env::join_paths(paths).map_err(|error| format!("could not build PATH: {error}"))
}

/// Opens the script in Terminal.app, which runs it in a new window.
#[cfg(target_os = "macos")]
fn spawn_unix_terminal(script: &Path) -> Result<(), String> {
    Command::new("/usr/bin/open")
        .arg("-a")
        .arg("Terminal")
        .arg(script)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start Terminal: {error}"))
}

/// Opens the script in whichever terminal the desktop ships with; the first
/// one that starts wins.
#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_unix_terminal(script: &Path) -> Result<(), String> {
    const EMULATORS: &[(&str, &[&str])] = &[
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-e"]),
        ("xterm", &["-e"]),
    ];

    let mut failures = Vec::new();

    for (program, args) in EMULATORS {
        match Command::new(program).args(*args).arg(script).spawn() {
            Ok(_) => return Ok(()),
            Err(error) => failures.push(format!("{program}: {error}")),
        }
    }

    Err(format!(
        "no terminal emulator found ({})",
        failures.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// An archive shaped like Google's: everything under `platform-tools/`.
    fn archive(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

        for (name, contents) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }

        writer.finish().unwrap().into_inner()
    }

    /// A directory of its own for a test, removed first so leftovers from an
    /// interrupted run cannot pass for a result.
    fn test_dir(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("lmnflash-platform-tools-{name}"));
        let _ = std::fs::remove_dir_all(&directory);

        directory
    }

    #[test]
    fn unpacks_the_tools_it_needs() {
        let directory = test_dir("needed");

        let archive = archive(&[
            ("platform-tools/fastboot", "binary"),
            ("platform-tools/fastboot.exe", "binary"),
            ("platform-tools/adb", "binary"),
            ("platform-tools/adb.exe", "binary"),
            ("platform-tools/AdbWinApi.dll", "helper"),
            ("platform-tools/AdbWinUsbApi.dll", "helper"),
            ("platform-tools/NOTICE.txt", "not needed"),
        ]);

        let binary = unpack(&archive, &directory).unwrap();

        assert_eq!(binary, directory.join(binary.file_name().unwrap()));
        assert!(binary.is_file());
        // The two DLLs `fastboot.exe` (and `adb.exe`) load next to themselves
        // on Windows.
        assert!(directory.join("AdbWinApi.dll").is_file());
        assert!(directory.join("AdbWinUsbApi.dll").is_file());
        // `adb` is unpacked as well: the bloatware feature runs it.
        assert!(directory.join("adb").is_file() || directory.join("adb.exe").is_file());
        // The rest of the platform-tools stays in the archive.
        assert!(!directory.join("NOTICE.txt").exists());

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The executable bit is what makes the unpacked file usable on Unix, and
    /// the archive does not always carry it.
    #[cfg(unix)]
    #[test]
    fn makes_the_executable_runnable() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_dir("executable");
        let archive = archive(&[("platform-tools/fastboot", "binary")]);

        let binary = unpack(&archive, &directory).unwrap();
        let mode = std::fs::metadata(&binary).unwrap().permissions().mode();

        assert_eq!(mode & 0o111, 0o111, "mode {mode:o}");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn rejects_an_archive_without_fastboot() {
        let directory = test_dir("empty");
        let archive = archive(&[("platform-tools/NOTICE.txt", "not a tool")]);

        assert!(unpack(&archive, &directory).is_err());

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The alias has to start the build the dialog selected — through the
    /// emulator when one is needed — and pass the arguments on untouched.
    #[test]
    fn the_alias_starts_the_selected_build() {
        let script = windows_alias(r"C:\tools\mfastboot.exe", &[]);
        assert!(
            script.contains("\"C:\\tools\\mfastboot.exe\" %*"),
            "{script}"
        );

        // An Intel build on an ARM Linux machine is started through box64,
        // which is what the alias has to spell out.
        let script = unix_alias(
            "box64",
            &["/opt/mfastboot/darwin_amd64/29.0.6".to_owned()],
        );
        assert!(
            script.contains("exec 'box64' '/opt/mfastboot/darwin_amd64/29.0.6' \"$@\""),
            "{script}"
        );
    }

    /// A path with a quote in it must not end the shell word it sits in.
    #[test]
    fn shell_words_survive_quotes() {
        assert_eq!(shell_word("/a b"), "'/a b'");
        assert_eq!(shell_word("/it's"), r"'/it'\''s'");
    }

    /// The Unix launcher enters the directory, puts the alias first on the
    /// `PATH` and keeps the user's own entries behind it.
    #[test]
    fn the_launcher_script_sets_up_the_terminal() {
        let script = launcher_script(
            Path::new("/home/me/platform-tools"),
            Some(Path::new("/home/me/platform-tools/lmnflash-alias")),
            "LMN Flash: fastboot now runs mfastboot 34.0.4",
        );

        assert!(script.starts_with("#!/bin/sh\n"), "{script}");
        assert!(
            script.contains("cd '/home/me/platform-tools' || exit 1"),
            "{script}"
        );
        assert!(
            script.contains(
                "export PATH='/home/me/platform-tools/lmnflash-alias':\
                 '/home/me/platform-tools':\"$PATH\""
            ),
            "{script}"
        );
        assert!(script.contains("exec \"${SHELL:-/bin/sh}\" -i"), "{script}");
    }
}
