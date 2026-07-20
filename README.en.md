# simple-android-info

[Русский](README.md)

Android utility (phone / TV / set-top box): a **detailed bring-up report** after SMT or flashing — what came up and what did not. No root required.

Prebuilt binaries are in [`bin/`](bin/). For normal use you **do not need to build from source**.

| File | ABI | Devices |
|------|-----|---------|
| [`bin/simple-android-info-aarch64`](bin/simple-android-info-aarch64) | `arm64-v8a` | most phones / TVs (64-bit) |
| [`bin/simple-android-info-armeabi-v7a`](bin/simple-android-info-armeabi-v7a) | `armeabi-v7a` | older / 32-bit boxes |

```powershell
adb shell getprop ro.product.cpu.abi
```

---

## Usage

You need `adb` and USB debugging. Run only from `/data/local/tmp/` (`/sdcard` and `/storage` are often `noexec`).

```powershell
# 64-bit
adb push bin\simple-android-info-aarch64 /data/local/tmp/simple-android-info
# 32-bit
# adb push bin\simple-android-info-armeabi-v7a /data/local/tmp/simple-android-info

adb shell chmod 755 /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info
```

If you get `Permission denied` after `push`, run `chmod`.  
If `ls` shows the file but the shell says `No such file or directory` — wrong **ABI** (no `linker64` on a 32-bit system).

### Modes

| Command | Result |
|---------|--------|
| `simple-android-info` | detailed report per check |
| `--lang=ru` / `--lang=en` | same + explanations (what / where / why); `--explain=` is an alias |
| `--summary` / `-s` | summary table only |
| `--smt` | alias of the default mode |
| `--class=phone` / `tv` / `box` | force device class |
| `--class=summary` | same as `--summary` |
| `--json` / `-j` | JSON |
| `-v` / `--verbose` | also list buses (I2C, SPI, net…) |

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
| `0` | no required FAIL (`--summary` always `0`) |
| `1` | at least one FAIL |
| `2` | bad argument |

---

## Reading the output

Default output is a header (model, serial, Android) plus blocks:

```text
[PASS|FAIL|SKIP] <name>
  …facts…
  ———                    # only with --lang=
  what / where / why     # or что / где / зачем
```

Ends with `RESULT: PASS|FAIL`.

### DEVICE CHECKS

| Check | Meaning |
|-------|---------|
| `boot_completed` | Android userspace finished booting |
| `verified_boot` | AVB / vbmeta (green/yellow/…) |
| `nvram` | factory NVRAM / RF calibration Ready (MTK and similar) |
| `storage` | eMMC/UFS visible to the kernel |
| `data_mounted` | `/data` mounted with a size |
| `display` | DRM + resolution + SurfaceFlinger |
| `gpu` | EGL/GPU |
| `audio` | ALSA card / audioserver |
| `wlan` | interface + wificond |
| `ethernet` | eth (TV/box when no Wi‑Fi) |
| `bluetooth` | BT controller |
| `touch` | touchscreen (phone) |
| `battery_charger` | power_supply / charger (phone) |
| `camera` | Camera HAL device count |
| `sensors` | SensorService / IIO |
| `modem_baseband` | baseband + modem ifaces |
| `gnss` | GPS/mnld /dev nodes |
| `hdmi` | HDMI/DRM (TV/box) |
| `usb` | ADB/MTP gadget + UDC |
| `surfaceflinger` | display compositor |
| `identity` | serial, SoC, CPU, RAM, uptime… |

**SKIP** means the check is not required for this device class.  
Factory Wi‑Fi MAC is usually unavailable without root — `wlan_mac` is often `n/a`.

### Summary table (`--summary`)

| Field | Meaning |
|-------|---------|
| `class` | phone / tv / box / unknown |
| `product` / `device` / `serial` / `hardware` | identity |
| `android` / `build` / `boot` | OS, build, slot/vbmeta |
| `soc` / `platform` / `cpu*` / `ram` | silicon / memory |
| `gpu` / `display` / `refresh` | graphics and panel |
| `storage` / `data` | flash and `/data` usage |
| `network` / `wlan_mac` | networks (MAC often blocked) |
| `timezone` / `uptime` | timezone and uptime |
| `buses` / `features` | bus counts and capabilities |

### JSON

```powershell
adb shell /data/local/tmp/simple-android-info --json --lang=en > report.json
adb shell /data/local/tmp/simple-android-info --summary --json > summary.json
```

Full run: `mode` (`report`), `summary`, `checks`, `result` (`PASS`/`FAIL`); with `-v` also `inventory`.  
Each `checks.items[]` has `name`, `status`, `detail`; with `--lang=` also `explain.{what,where,why}`.

### Auto class detection

1. Looks like TV → `tv`
2. Modem / telephony present → `phone`
3. No modem, has network / HDMI → `box`
4. Otherwise → `unknown` (checklist still applies a phone-like profile from hardware signals)

---

## Build from source

You need: Rust, Android NDK, and `adb`.

```powershell
rustup target add aarch64-linux-android armv7-linux-androideabi
```

Point [`.cargo/config.toml`](.cargo/config.toml) at your NDK linker:

```toml
[target.aarch64-linux-android]
linker = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\aarch64-linux-android24-clang.cmd"
ar     = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\llvm-ar.exe"

[target.armv7-linux-androideabi]
linker = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\armv7a-linux-androideabi24-clang.cmd"
ar     = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\llvm-ar.exe"
```

```powershell
cargo build --release --target aarch64-linux-android
cargo build --release --target armv7-linux-androideabi
New-Item -ItemType Directory -Force bin | Out-Null
Copy-Item -Force target\aarch64-linux-android\release\simple-android-info bin\simple-android-info-aarch64
Copy-Item -Force target\armv7-linux-androideabi\release\simple-android-info bin\simple-android-info-armeabi-v7a
```

`target/` is not kept in git — only sources and the files in `bin/`.

---

## Limitations

- Needs a booted Android system and `adb`.
- Without root, some sysfs is closed (often Wi‑Fi MAC, GPU clock).
- Node names are vendor-specific (MediaTek / Amlogic / common patterns are covered).

```text
simple-android-info --help
```
