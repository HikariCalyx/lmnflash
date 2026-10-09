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

### Google's platform-tools

The tool picker also offers Google's own `fastboot`, from the Android
platform-tools: the archive for the system is downloaded the first time it is
picked and unpacked into the configuration directory
(`%AppData%\lmnflash\platform-tools`, `~/.config/lmnflash/platform-tools`,
`~/Library/Application Support/com.hikaricalyx.lmnflash/platform-tools`). Only
`fastboot` and the two ADB DLLs it loads on Windows are kept, not the rest of the
archive.

| System | Archive |
| --- | --- |
| Windows | `platform-tools-latest-windows.zip` |
| Linux | `platform-tools-latest-linux.zip` |
| macOS | `platform-tools-latest-darwin.zip` |

Later runs use that copy as it is; the **Update** button next to the version
downloads the current build again (deleting the folder has the same effect).

Whichever engine is picked, the dialog also shows the version the tool reports
for `--version` (`Reported version: 37.0.1-15733141`) — that is the build which
will actually run, which for a downloaded `fastboot` is the only place its
version is visible.

Google publishes no ARM build for Linux, so there an ARM machine runs the
downloaded `fastboot` through box64, exactly like the shipped `mfastboot`; the
macOS archive is universal and the Windows one runs on every edition (see
below).

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
  * **Smartphone** opens the Install Driver dialog; its **Install** button
    downloads Motorola's Mobile Drivers and starts the MSI. The
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
  standard input when its **Install** button is pressed.

  That project is a submodule at `vendor/android-udev-rules`, and the files
  `install.sh` reads are compiled into the binary, so installing needs neither
  the checkout nor a download — but a clone does have to fetch it:

  ```
  git submodule update --init --recursive
  ```

## Login and the `softwarefix://` protocol

Firmware lookup needs a Lenovo ID login. The browser flow ends by redirecting
to a `softwarefix://callback?Authorization=…` link, which the official
Software Fix program registers itself to handle.

LMN Flash registers the same scheme so that link can be handed straight to it:

* If nothing handles the scheme, LMN Flash registers itself automatically on
  startup; an existing handler is never taken over silently. The **Manual
  Login** page (right-click → *Log in manually*) shows a button that switches
  the handler between the registered program and LMN Flash, and back again.
* **Windows** — a per-user registry class
  (`HKCU\Software\Classes\softwarefix`). No administrator rights are needed,
  and a per-user handler that is replaced is remembered so switching back
  restores it; a machine-wide handler needs no backup, because deleting the
  per-user key reveals it again.
* **Linux** — a desktop entry (`~/.local/share/applications/lmnflash.desktop`)
  plus `xdg-mime default`, which needs `xdg-mime` on `PATH`. The entry runs the
  executable with the URL as an argument, so the callback arrives directly.
* **macOS** — not registered: LaunchServices would start the application, but
  the URL arrives as an Apple Event that iced exposes no API for, so the
  callback would be silently dropped. Paste the link into the **Manual Login**
  page instead.

Both registrations name the executable by its full path, so moving the
application is detected on the next launch and the entry is rewritten in place;
a handler that was previously replaced stays remembered for switching back.

### Browser login

Once LMN Flash owns the scheme, **Log in** uses the browser installed on the
system instead of the built-in WebView: the login page opens in the default
browser, and when it redirects to `softwarefix://callback…` the operating
system starts a second copy of the application. That copy hands the URL to the
running instance over a loopback socket (advertised in `<config>/instance`) and
exits, so the login completes in the window the user is already looking at
instead of in a second one. Without the scheme in hand the callback would go to
the other handler, so the built-in WebView is still used then.

## Remove System Bloatware

The **Remove System Bloatware** tile removes apps that Lenovo and Motorola
preinstall on their phones and tablets. It works on any Android device with
**USB debugging** enabled — fastboot is not involved.

The known apps are the Lenovo/ZUI and Motorola system apps (Browser, Tianxi
Agent, the push service, the Moto AI apps, SmartFeed, the app store, QuickApp,
…), the Chinese apps that are bundled with them (Toutiao, Douyin, Kuaishou,
Baidu Search and Maps, Weibo, Xiaohongshu, Hongguo Short Drama, and the other
vendors' QuickApp engines), the apps that come with operator builds — the ~200
entries marked as safe to delete in the *Universal Android Debloater*
[carriers list](https://github.com/MuntashirAkon/android-debloat-list) (AT&T,
Verizon, Sprint, T-Mobile, Rogers, Bell, Orange, SFR, Bouygues, Telekom, EE,
NTT Docomo, …) — and the store apps and games that get bundled with them (Temu,
Genshin Impact, PUBG Mobile, Honor of Kings, Arena of Valor, Onmyoji, Delta
Force, League of Legends: Wild Rift, …). A carrier app's name is a product name
and is shown the way that list spells it; a game is named per region and
language (Genshin Impact / 原神), so those names are translated.

Pressing the tile's button opens a dialog that:

1. Prepares ADB. The `adb` of Google's platform-tools is downloaded into the
   configuration directory on first use (next to the `fastboot` the Firmware
   Flash dialog uses) and is always the build that runs, so the feature does not
   depend on whatever happens to be on the `PATH`.
2. Lists the connected devices (`adb devices -l`) and, once a device is picked,
   reads its installed packages (`pm list packages`).
3. Shows every known bloatware app that is installed as a checklist in a
   scrollable list, all checked by default. A search box above it filters the
   list by app name or package id, and **Select all** / **Deselect all** act on
   the rows the search leaves on screen. The checked apps are removed with
   `pm uninstall --user 0 <package>`.

The uninstall is per user: the system partition is not touched, and a removed
app comes back with `pm install-existing --user 0 <package>` over ADB or a
factory reset — the dialog says so afterwards. A device that has not accepted
this computer's debugging key yet is reported as unauthorized until the "Allow
USB debugging?" prompt is accepted on it; **Refresh** re-scans the USB bus.

## Unpack Image

The **Unpack Image** tile extracts the files stored in one of Motorola's
container images — the ones whose 256-byte header carries the magic
`SINGLE_N_LONELY`, such as the `radio.img` / `bootloader.img` files of a
firmware package. The picker accepts `.img` and `.bin` files. It is a host-side
job; nothing has to be connected.

Pressing the tile's button opens a dialog that:

1. Asks for the image. Its directory table is read right away — only the
   256-byte header and the entries are read, the payloads are skipped over — so
   the dialog shows what the image holds (each file with its size) before
   anything is written, and reports a file that is not this format.
2. Asks for the output folder.
3. Writes every file into that folder, next to nothing else, reporting the
   progress and the file it is working on.

The directory table holds at most 64 entries and ends with an entry named
`LONELY_N_SINGLE`; payloads are padded so each entry starts on a 4096-byte
boundary. An entry whose name would leave the chosen folder (a path separator,
`.` or `..`) is refused instead of followed.

