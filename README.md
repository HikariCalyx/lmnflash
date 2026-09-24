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
for (`windows_amd64`, `darwin_arm64`, `linux_amd64`, …), mirroring the
`prebuilt_binary` directory. A platform that is not the one the application runs
as is skipped, so the app never starts an `mfastboot` the CPU cannot execute —
it falls back to its built-in fastboot instead. ARM machines are the exception:
the Intel builds run there through an emulator (see below). Every build found is
offered in the tool picker, newest version first.

The release workflow bundles the contents of `prebuilt_binary` this way:

| Artifact | `mfastboot` |
| --- | --- |
| `lmnflash-win64.zip` | `mfastboot/windows_amd64/{34.0.4,28.0.2,26.0.0}` |
| `lmnflash-linux-x86_64.tar.gz` | `mfastboot/linux_amd64/31.0.2` |
| `lmnflash-linux-arm64.tar.gz` | `mfastboot/linux_amd64/31.0.2` (via box64) |
| `lmnflash-macos_unsigned.dmg` | `Contents/Resources/mfastboot/darwin_amd64/29.0.6` |

The single-file artifacts (`lmnflash-win64.exe`, `lmnflash-linux-x86_64`, …) do
not carry `mfastboot` and therefore use the built-in fastboot.

### Intel `mfastboot` on ARM machines

`mfastboot` is published for Intel only, so an ARM machine needs a translator to
run it: [Rosetta 2](https://support.apple.com/en-us/102527) on Apple silicon,
[box64](https://github.com/ptitSeb/box64) on ARM Linux. The dialog checks whether
that is installed, and when it is missing it says how to get it:

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
could not run at all — ARM Windows, 32-bit ARM Linux, and macOS 28, which dropped
Rosetta 2 — it is not offered and the built-in fastboot is used.


