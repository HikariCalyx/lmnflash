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
| `lmnflash_<version>_amd64.deb` | — (none) |
| `lmnflash-<version>-1.x86_64.rpm` | — (none) |
| `lmnflash-macos_unsigned.dmg` | `Contents/Resources/mfastboot/darwin_amd64/29.0.6` |

The Windows payload is the 32-bit build, which is the only one Motorola
publishes; `windows_x86` says so instead of claiming the architecture of the app
it travels with.

The single-file artifacts (`lmnflash-win64.exe`, `lmnflash-win32.exe`,
`lmnflash-win-arm64.exe`, `lmnflash-linux-x86_64`, …) do not carry `mfastboot`
and therefore use the built-in fastboot. Neither do the two distribution
packages below — `mfastboot` is not redistributable with them — so they start
with the built-in fastboot as well, until one is put into the folder of the
executable (see below) or Google's platform-tools are downloaded.

### Debian / Ubuntu package

`lmnflash_<version>_amd64.deb` is the same x86_64 build as the tarball, laid out
the way a distribution expects, so `apt install ./lmnflash_<version>_amd64.deb`
is all it takes:

* `/usr/lib/lmnflash/lmnflash` — the executable, in a folder of its own so an
  `mfastboot` build can be put beside it
  (`mfastboot/linux_amd64/<version>/mfastboot`), plus a `/usr/bin/lmnflash`
  symlink so the command is on `PATH`.
* `/usr/share/applications/lmnflash.desktop` — the launcher entry, with the icon
  in `/usr/share/icons/hicolor/512x512/apps/lmnflash.png`.
* `Depends` — the runtime libraries the executable links or opens (`libgtk-3-0`,
  `libwebkit2gtk-4.1-0`, `libsoup-3.0-0`, the X11/Wayland libraries …), so
  `apt` pulls them in. Two libraries are missing on purpose: `libusb`, which is
  compiled into the executable (rusb's `vendored` feature) rather than loaded
  from the system, and `libc6`, which is an *essential* package that every
  Debian system already has.

It is built on Ubuntu 24.04 and therefore needs glibc 2.39 or newer (Ubuntu
24.04+, Debian 13+) — the tarball is the very same build. The package does not
enforce that: since `libc6` is essential it is not a dependency, so the version
floor only lives in this line.

### RPM package

`lmnflash-<version>-1.x86_64.rpm` is the same build for the RPM distributions
(Fedora, RHEL, openSUSE) and installs the same layout — except that the folder
of the executable is `/usr/lib64/lmnflash`, where `rpm` keeps an application's
library data, which is also where an `mfastboot` build would go — with the
launcher entry and the icon included:

```
sudo dnf install ./lmnflash-<version>-1.x86_64.rpm
```

The dependency list is not written by hand: `rpmbuild` reads the libraries (and
with them the glibc symbol version the binary was built against) out of the
executable, which is the only naming every RPM distribution agrees on. Only the
icon theme and `xdg-utils` are named manually, because they cannot be seen in
the executable.

The same glibc 2.39 floor applies (Fedora 40+, RHEL 10, openSUSE Tumbleweed).

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
* **macOS** — a LaunchServices registration
  (`LSSetDefaultHandlerForURLScheme`) for the scheme the bundle declares in its
  `Info.plist` (`CFBundleURLTypes`), so the application has to be run from
  `LMNFlash.app`. The URL arrives as an Apple Event sent to the running
  application, which handles it itself. LaunchServices can only point a scheme
  at another handler, never back at nothing, so switching back needs a handler
  to have been registered before.

The Windows and Linux registrations name the executable by its full path, so
moving the application is detected on the next launch and the entry is rewritten
in place; macOS registers the bundle identifier instead, which survives a move.
A handler that was previously replaced stays remembered for switching back.

### Browser login

Once LMN Flash owns the scheme, **Log in** uses the browser installed on the
system instead of the built-in WebView: the login page opens in the default
browser, and when it redirects to `softwarefix://callback…` the operating
system hands the link to LMN Flash. On Windows and Linux that starts a second
copy of the application, which hands the URL to the running instance over a
loopback socket (advertised in `<config>/instance`) and exits; on macOS the
running application is sent the URL as an Apple Event instead. Either way the
login completes in the window the user is already looking at instead of in a
second one. Without the scheme in hand the callback would go to the other
handler, so the built-in WebView is still used then.

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
USB debugging?" prompt is accepted on it; such a device carries a **Retry**
button (`adb -s <serial> reconnect`), which makes it show that prompt again if
it was dismissed by mistake, and **Refresh** re-scans the USB bus.

## Connect ADB Wireless Debugging

The **Connect ADB Wireless Debugging** tile pairs this computer with a phone
that has Android's **wireless debugging** on, so the phone can be reached over
Wi-Fi instead of the cable. The phone is the one that serves the pairing; the
dialog only has to hand it the pairing code, and there are two ways to do that:

* **QR code** — the dialog shows a QR code carrying a random service name and
  password, in the format Android's *"Pair device with QR code"* screen reads.
  Scanning it makes the phone announce its pairing service under that name,
  which the dialog finds over mDNS and pairs with — nothing has to be typed.
* **Pairing code** — the phone's *"Pair device with pairing code"* screen shows
  a six-digit code. The dialog lists the phones that are offering a pairing
  (`<ip>:<port>`), and pairs with the chosen one once the code is entered.

Pairing is a one-time step: afterwards `adb` connects on its own whenever the
phone is on the network, and the dialog reports the device it finds. The
**Disconnect** menu drops the connection again (`adb disconnect`): it lists
every wireless device `adb` reports — so a phone connected in an earlier session
can be dropped as well — and its last entry drops all of them at once. Both
devices have to be on the same network, and that network has to pass mDNS
between them. The dialog uses the same `adb` of Google's platform-tools as the
bloatware feature, downloading it on first use.

## Fix Captive Portal Server

The **Fix Captive Portal Server** tile (修复网络连接受限) helps a phone that
reports *"Connected, no Internet"* because the server it checks the connection
against cannot be reached. It points the phone's captive-portal check and its
NTP server at servers that answer — the setting that spells that out depends on
the Android version:

```sh
# Android 7.1 and newer
settings put global captive_portal_http_url  http://connect.rom.miui.com/generate_204
settings put global captive_portal_https_url https://connect.rom.miui.com/generate_204

# Android 6.0 – 7.0: one server, no URLs
settings put global captive_portal_server connect.rom.miui.com

# Android 5.0 – 5.1: detection is turned off instead
settings put global captive_portal_mode 0

# every supported version
settings put global ntp_server time.asia.apple.com
```

The dialog lists the devices connected with USB debugging enabled; the one that
is picked runs the fix. A device that has not accepted this computer's debugging
key is listed as unauthorized and offers a **Retry** button instead, which runs
`adb -s <serial> reconnect` — that makes the device show its *"Allow USB
debugging?"* prompt again, so a prompt dismissed by mistake can be answered.
Android 5.0 is the oldest version with any of these
settings, so anything older is reported as unsupported instead of being
changed. On Android 10 and newer the connection is also cycled afterwards
(`cmd connectivity airplane-mode enable`, a five-second wait, `disable`),
because the connectivity service only re-reads its settings when the network
comes up; on Android 9 and older the dialog asks the user to switch Airplane
mode on and off again by hand, which has the same effect.

The sequence is started **detached** on the phone (`nohup … & disown`), so the
airplane-mode cycle cannot kill it by taking away the very connection the
command came in on. When the device was reached over wireless debugging, the
dialog drops that connection itself as soon as the job is started rather than
waiting for it to time out; a USB connection is not affected and the phone
reconnects to Wi-Fi once the cycle is over.

## Tablet Firmware Decrypt

The **Firmware Decrypt** tab offers the host-side firmware-file tools as a grid
of cards, like the flashing tab. **Tablet Firmware Decrypt** decrypts the `*.x`
and `*.t` files an L-branded tablet firmware dump is made of: a `*.x` file
becomes a `*.xml` and a `*.t` file a `*.txt`, written next to the original. A
custom password can be given; without one the standard password is used. The
card reports how many files were decrypted and lists the ones that failed.

## Unpack Image

The **Unpack Image** card in the same tab extracts the files stored in one of
Motorola's container images — the ones whose 256-byte header carries the magic
`SINGLE_N_LONELY`, such as the `radio.img` / `bootloader.img` files of a
firmware package. The picker accepts `.img` and `.bin` files. It is a host-side
job; nothing has to be connected.

Pressing the card's button opens a dialog that:

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

