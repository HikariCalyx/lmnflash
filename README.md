# lmnflash

Flashing and firmware-lookup utility for Motorola / Lenovo devices.

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

`<platform>` names the architecture the binary was built for (`windows_amd64`,
`darwin_arm64`, …), mirroring the `prebuilt_binary` directory. A platform that
is not the one the application runs as is skipped, so the app never starts an
`mfastboot` the CPU cannot execute — it falls back to its built-in fastboot
instead. Apple silicon is the one exception: the Intel builds run there through
Rosetta 2 (see below). Every build found is offered in the tool picker, newest
version first.

The release workflow bundles the contents of `prebuilt_binary` this way:

| Artifact | `mfastboot` |
| --- | --- |
| `lmnflash-win64.zip` | `mfastboot/windows_amd64/{34.0.4,28.0.2,26.0.0}` |
| `lmnflash-linux-x86_64.tar.gz` | `mfastboot/linux_amd64/31.0.2` |
| `lmnflash-macos_unsigned.dmg` | `Contents/Resources/mfastboot/darwin_amd64/29.0.6` |

The single-file artifacts (`lmnflash-win64.exe`, `lmnflash-linux-x86_64`, …) do
not carry `mfastboot` and therefore use the built-in fastboot.

### Rosetta 2 on Apple silicon

`mfastboot` is published for Intel only, so on an Apple silicon Mac the app runs
it through [Rosetta 2](https://support.apple.com/en-us/102527). The dialog checks
whether the Rosetta 2 runtime is installed — the single file check Homebrew uses
(`/Library/Apple/usr/libexec/oah/libRosettaRuntime`) — and when it is missing it
says how to install it:

```sh
softwareupdate --install-rosetta --agree-to-license
```

`Start Flashing` stays disabled until Rosetta 2 is installed and the dialog is
reopened, or the built-in fastboot engine is picked instead. Apple ships Rosetta 2
for Apple silicon from macOS 11 through macOS 27; on macOS 28 and later the Intel
build is not offered at all and the built-in fastboot is used.


