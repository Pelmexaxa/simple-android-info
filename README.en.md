# simple-android-info

[Русский](README.md)

A program for Android phones, televisions, and set-top boxes. It runs over a USB cable, collects device information, and checks which main system parts are actually working after power-on or flashing. Superuser (root) access is not required.

Ready-made executables are in the [`bin/`](bin/) folder. For normal use you do not need to build from source.

Before you run the program, check the processor architecture of the device:

```powershell
adb shell getprop ro.product.cpu.abi
```

| File | Architecture | Typical devices |
|------|--------------|-----------------|
| [`bin/simple-android-info-aarch64`](bin/simple-android-info-aarch64) | 64-bit ARM (`arm64-v8a`) | most modern phones and televisions |
| [`bin/simple-android-info-armeabi-v7a`](bin/simple-android-info-armeabi-v7a) | 32-bit ARM (`armeabi-v7a`) | older devices and some set-top boxes |

If the file architecture does not match the device, the system may say the file was not found even though it is present on the device.

---

## How to run

You need the Android Debug Bridge (`adb`) and USB debugging enabled on the device.

Copy the file into `/data/local/tmp/`. On shared storage (`/sdcard`, `/storage`) the program usually cannot start, because execution of files is disabled there.

```powershell
# for 64-bit ARM
adb push bin\simple-android-info-aarch64 /data/local/tmp/simple-android-info

# for 32-bit ARM
# adb push bin\simple-android-info-armeabi-v7a /data/local/tmp/simple-android-info

adb shell chmod 755 /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info
```

If the system reports that execution is not allowed after the copy, run the `chmod` command again as shown above.

### Command-line options

| Option | Meaning |
|--------|---------|
| (no options) | full report: facts for each check item |
| `--lang=ru` or `--lang=en` | the same report, plus explanations in Russian or English (what is checked, where the data comes from, why it matters). Same meaning: `--explain=ru` or `--explain=en` |
| `--summary` or `-s` | short summary table only, without the check list |
| `--smt` | same as running with no options |
| `--class=phone`, `--class=tv`, or `--class=box` | force the device class to phone, television, or set-top box |
| `--class=summary` | same as `--summary` |
| `--json` or `-j` | output in JSON format |
| `-v` or `--verbose` | also print a list of devices on the buses (I2C, SPI, network, and others) |

Examples:

```powershell
adb shell /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info --lang=ru
adb shell /data/local/tmp/simple-android-info --lang=en
adb shell /data/local/tmp/simple-android-info --summary
adb shell /data/local/tmp/simple-android-info --json --lang=en
adb shell /data/local/tmp/simple-android-info --class=tv
```

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | no required failures (`--summary` always exits with `0`) |
| `1` | at least one required check failed |
| `2` | invalid command-line argument |

---

## How to read the output

In the normal mode the program first prints a short header: manufacturer, model, serial number, Android version. Then it prints separate check blocks.

Each block looks like this:

```text
[PASS] check_name
  lines with facts

  ———
  what:   …
  where:  …
  why:    …
```

The three explanation lines appear only with `--lang=en` (or `--lang=ru`, where the labels are in Russian).

At the end the program prints `RESULT: PASS` or `RESULT: FAIL`.

### Check statuses

| Status | Meaning |
|--------|---------|
| `PASS` | the part was found and looks operational |
| `FAIL` | for this device type the part was expected but is missing or not ready; the program exits with code `1` |
| `SKIP` | this check is not required for this device type |

### List of checks

| Name | What is checked |
|------|-----------------|
| `boot_completed` | Android has finished starting and is ready for applications |
| `verified_boot` | firmware integrity state at boot (green, yellow, and other states) |
| `nvram` | factory settings and radio calibration area is initialized (typical for MediaTek and similar platforms) |
| `storage` | flash storage is visible to the operating system |
| `data_mounted` | the user data partition `/data` is mounted and its size is known |
| `display` | image output works: a screen resolution and the display service are present |
| `gpu` | a graphics accelerator was found |
| `audio` | a sound card and the audio service are present |
| `wlan` | a Wi‑Fi network interface exists and the Wi‑Fi service is running |
| `ethernet` | a wired network interface exists (usually for a television or set-top box when there is no Wi‑Fi) |
| `bluetooth` | the system sees a Bluetooth controller |
| `touch` | a touchscreen is registered (for phones and tablets) |
| `battery_charger` | battery, charger, or USB power supply entries are present (for phones) |
| `camera` | the camera service reports at least one camera |
| `sensors` | the sensor service or sensor bus lists devices (accelerometer, gyroscope, and others) |
| `modem_baseband` | modem firmware is loaded and modem network interfaces exist |
| `gnss` | the satellite navigation subsystem (GPS and similar) is available |
| `hdmi` | an external HDMI video output or a live graphics path is present (for television and set-top box) |
| `usb` | the device is visible to a computer over USB (debugging, file transfer) |
| `surfaceflinger` | the service that composes the screen image is running |
| `identity` | a short device identity summary: serial number, platform, processor, memory, uptime |

On recent Android versions the factory Wi‑Fi address is often hidden without superuser access. In that case the network address field in the summary is `n/a` (“not available”). That is expected.

### Summary table (`--summary`)

| Field | Contents |
|-------|----------|
| `class` | device type: phone, television, set-top box, or unknown |
| `product`, `device`, `serial`, `hardware` | manufacturer, model, internal name, serial number, board name |
| `android`, `build`, `boot` | system version, build identifier, boot slot and firmware verification state |
| `soc`, `platform`, `cpu`, `cpu_model`, `cpu_freq`, `ram` | chipset, platform, core count, processor model, frequencies, memory |
| `gpu`, `display`, `refresh` | graphics, screen resolution, refresh rate |
| `storage`, `data` | storage type and usage of the `/data` partition |
| `network`, `wlan_mac` | which networks exist; Wi‑Fi address if the system exposes it |
| `timezone`, `uptime` | time zone, uptime, and load averages |
| `buses`, `features` | number of devices on buses and detected features (touch, camera, and others) |

### JSON format

```powershell
adb shell /data/local/tmp/simple-android-info --json --lang=en > report.json
adb shell /data/local/tmp/simple-android-info --summary --json > summary.json
```

A full report includes the run mode, the summary, the list of checks, and the overall result (`PASS` or `FAIL`). With `-v` a bus inventory is added. Each check has a name, a status, and a detail text; with `--lang=` an explanation block is added as well.

### How the device type is chosen

1. Television-like signals → type “television”.
2. A modem or telephony signals → type “phone”.
3. No modem, but network or HDMI → type “set-top box”.
4. Otherwise → “unknown”. Even then the check list still adapts to the hardware that is actually present (for example modem or battery).

---

## Building from source

You need: the Rust compiler, the Android Native Development Kit (NDK), and `adb`.

Install the build targets:

```powershell
rustup target add aarch64-linux-android armv7-linux-androideabi
```

In [`.cargo/config.toml`](.cargo/config.toml) point the linker entries at your NDK install (replace the path with yours):

```toml
[target.aarch64-linux-android]
linker = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\aarch64-linux-android24-clang.cmd"
ar     = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\llvm-ar.exe"

[target.armv7-linux-androideabi]
linker = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\armv7a-linux-androideabi24-clang.cmd"
ar     = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\llvm-ar.exe"
```

Build and copy into `bin/`:

```powershell
cargo build --release --target aarch64-linux-android
cargo build --release --target armv7-linux-androideabi
New-Item -ItemType Directory -Force bin | Out-Null
Copy-Item -Force target\aarch64-linux-android\release\simple-android-info bin\simple-android-info-aarch64
Copy-Item -Force target\armv7-linux-androideabi\release\simple-android-info bin\simple-android-info-armeabi-v7a
```

The intermediate build folder `target/` is not stored in the repository. Git keeps the sources and the ready files in `bin/`.

---

## Limitations

- The device must already be running Android, with a debugging cable connected.
- Without superuser access some information is unavailable (often the Wi‑Fi address and the graphics processor clock).
- System node names depend on the board vendor. Common MediaTek, Amlogic, and similar layouts are covered.

Help text on the device:

```text
simple-android-info --help
```
