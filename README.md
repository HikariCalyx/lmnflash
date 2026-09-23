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
instead. Every build found is offered in the tool picker, newest version first.

The release workflow bundles the contents of `prebuilt_binary` this way:

| Artifact | `mfastboot` |
| --- | --- |
| `lmnflash-win64.zip` | `mfastboot/windows_amd64/{34.0.4,28.0.2,26.0.0}` |
| `lmnflash-linux-x86_64.tar.gz` | `mfastboot/linux_amd64/31.0.2` |
| `lmnflash-macos_unsigned.dmg` | `Contents/Resources/mfastboot/darwin_amd64/29.0.6` |

The single-file artifacts (`lmnflash-win64.exe`, `lmnflash-linux-x86_64`, …) do
not carry `mfastboot` and therefore use the built-in fastboot.

