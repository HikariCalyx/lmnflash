# lmnflash

Flashing and firmware-lookup utility for Lenovo / Motorola / NEC devices
(also why this name).

## Firmware Flash

The **Smartphone Flash** tab can flash a complete factory firmware package
(Motorola `flashfile.xml`, or the firmware ZIP containing it) to a phone in
fastboot mode. Two backends can run the package:

* **`mfastboot`** — Motorola's `fastboot` fork, preferred when it ships with
  the application, because its behaviour matches the firmware packages exactly.
* **Built-in fastboot** — the pure-Rust implementation used by the other
  features, used when no `mfastboot` is available for the platform.

### Shipping `mfastboot`

The application looks for `mfastboot` next to its own executable
(`Contents/Resources` inside a macOS `.app`) using this layout:

```
mfastboot/<version>/mfastboot[.exe]                 # used as-is
mfastboot/<platform>/<version>/mfastboot[.exe]      # only for <platform>
mfastboot[.exe]                                     # used as-is
```

`<platform>` names the operating system and architecture the binary was built
for (`windows_x86`, `darwin_arm64`, `linux_amd64`, …), mirroring the
`prebuilt_binary` directory. A platform that is not the one the application runs
as is skipped, so the app never starts an `mfastboot` the CPU cannot execute —
it falls back to its built-in fastboot instead. Two kinds of machine are the
exception: Windows takes the 32-bit build on every edition (as it is on win32,
through WOW64 on win64 and through its own x86 emulation on ARM64), and an ARM
Mac or an ARM Linux machine runs the Intel builds through an emulator (see
below). Every build found is offered in the tool picker, newest version first.

The release workflow bundles the contents of `prebuilt_binary` this way:

| Artifact | `mfastboot` |
| --- | --- |
| `lmnflash-win64.zip` | `mfastboot/windows_x86/{34.0.4,28.0.2,26.0.0}` |
| `lmnflash-win32.zip` | `mfastboot/windows_x86/{34.0.4,28.0.2,26.0.0}` |
| `lmnflash-win-arm64.zip` | `mfastboot/windows_x86/{34.0.4,28.0.2,26.0.0}` (via Windows' x86 emulation) |
| `lmnflash-linux-x86_64.tar.gz` | `mfastboot/linux_amd64/31.0.2` |
| `lmnflash-linux-arm64.tar.gz` | `mfastboot/linux_amd64/31.0.2` (via box64) |
| `lmnflash-macos_unsigned.dmg` | `Contents/Resources/mfastboot/darwin_amd64/29.0.6` |

The Windows payload is the 32-bit build, which is the only one Motorola
publishes; `windows_x86` says so instead of claiming the architecture of the app
it travels with.

The single-file artifacts (`lmnflash-win64.exe`, `lmnflash-win32.exe`,
`lmnflash-win-arm64.exe`, `lmnflash-linux-x86_64`, …) do not carry `mfastboot`
and therefore use the built-in fastboot.

### Intel `mfastboot` on ARM machines

`mfastboot` is published for Intel only, so an ARM machine needs a translator to
run it: [Rosetta 2](https://support.apple.com/en-us/102527) on Apple silicon,
[box64](https://github.com/ptitSeb/box64) on ARM Linux. Windows on ARM is the
exception — it emulates x86 binaries itself, so the bundled 32-bit `mfastboot`
runs there with nothing to install. The dialog checks whether a translator is
installed, and when it is missing it says how to get it:

* macOS — the Rosetta 2 runtime, detected by the same single file check Homebrew
  uses (`/Library/Apple/usr/libexec/oah/libRosettaRuntime`) and installed with
  `softwareupdate --install-rosetta --agree-to-license`. macOS applies it to the
  binary by itself.
* ARM Linux — `box64`, searched in `PATH` and then in `/usr/local/bin`,
  `/usr/bin` and `/usr/local/sbin`. The binary is started as
  `box64 mfastboot …`.

`Start Flashing` stays disabled until the translator is installed and the dialog
is reopened, or the built-in fastboot engine is picked instead. Apple ships
Rosetta 2 for Apple silicon from macOS 11 through macOS 27. Where the Intel build
could not run at all — 32-bit ARM Linux, and macOS 28, which dropped Rosetta 2 —
it is not offered and the built-in fastboot is used.



## Install Driver

The **Install Driver** tile installs the USB driver a phone in fastboot mode
needs. It only appears where there is something to install: not on macOS (which
ships the drivers it needs) and not on Windows on ARM (Motorola publishes no
installer for it).

* **Windows** — two buttons, because phones and tablets are served differently:
  * **Smartphone** downloads Motorola's Mobile Drivers and starts the MSI. The
    installer is picked by the architecture of the *system*, not of the
    application: `PROCESSOR_ARCHITEW6432` (which WOW64 reports to a 32-bit
    process on a 64-bit system) wins over `PROCESSOR_ARCHITECTURE`, so the
    32-bit build of this app still downloads the 64-bit driver. The download is
    split into five ranged connections into one pre-allocated file, and a server
    that does not honour byte ranges is downloaded in one piece instead.
  * **Tablet** opens Lenovo's [Software Fix](https://support.lenovo.com/us/en/downloads/ds101291)
    page, which is the flashing tool for tablets and brings its own drivers.
* **Linux** — one button. The upstream
  [android-udev-rules](https://github.com/M0Rf30/android-udev-rules) installer
  (`install.sh`) is unpacked from the binary and run as root: it copies
  `51-android.rules` into `/etc/udev/rules.d`, makes sure the `adbusers` group
  exists, adds the invoking user to it and restarts udev. The rules need root,
  so the dialog asks for the password and hands it to `sudo -S` through its
  standard input.

  That project is a submodule at `vendor/android-udev-rules`, and the files
  `install.sh` reads are compiled into the binary, so installing needs neither
  the checkout nor a download — but a clone does have to fetch it:

  ```
  git submodule update --init --recursive
  ```
