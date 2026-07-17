# simple-android-info

[Русский](README.md)

A small Android utility: shows **what the device is**, and checks **which interfaces are actually up** (display, network, touch, camera, and so on).

Works on phones, Android TV, and set-top boxes. No root required.

A prebuilt binary is in [`bin/`](bin/). For normal use you **do not need to build from source**.

---

## Usage (prebuilt binary)

You only need `adb` and a device with USB debugging.

```powershell
adb push bin\simple-android-info /data/local/tmp/
adb shell chmod 755 /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info
```

If you get `Permission denied` after `push`, run `chmod`.

### Modes

| Command | Result |
|---------|--------|
| `simple-android-info` | summary + interface checks (class auto) |
| `--summary` / `-s` | summary table only |
| `--class=phone` / `tv` / `box` | full run with forced class |
| `--class=summary` | same as `--summary` |
| `--json` / `-j` | JSON output |
| `-v` / `--verbose` | also list buses (I2C, SPI, net…) |

Flags can be combined.

```powershell
adb shell /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info --summary
adb shell /data/local/tmp/simple-android-info --summary --json
adb shell /data/local/tmp/simple-android-info --class=tv
adb shell /data/local/tmp/simple-android-info --json -v
```

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | OK (`--summary` always exits `0`) |
| `1` | at least one required FAIL |
| `2` | bad argument |

---

## Reading the output

### Summary table (SUMMARY)

| Field | Meaning |
|-------|---------|
| `class` | phone / tv / box |
| `product` | manufacturer and model |
| `device` | codename |
| `android` / `build` | OS version and build id |
| `soc` / `platform` | chipset |
| `cpu` / `cpu_freq` | cores and per-cluster clocks |
| `ram` / `ddr_*` | memory and DDR (when available) |
| `gpu` | graphics |
| `display` / `refresh` | resolution and refresh rates |
| `storage` | eMMC / UFS |
| `network` | wlan, eth, modem, bt |
| `buses` | I2C / SPI / input counts |
| `features` | detected capabilities |

`n/a` means the value was not exposed (often without root: DDR type, GPU clock).

### Interface checks (INTERFACE CHECKS)

| Status | Meaning |
|--------|---------|
| **PASS** | found and looks alive |
| **FAIL** | expected for this class but missing → exit `1` |
| **SKIP** | not required for this class |

| Check | Phone | TV / box |
|-------|:-----:|:--------:|
| Storage, display, GPU, audio, network, BT, SurfaceFlinger | yes | yes |
| DSI panel, touch, modem, camera, charger, GNSS | yes | no |
| HDMI / external display | no | yes |
| Ethernet | no | yes if no Wi‑Fi |

`info_*` lines never cause FAIL.  
End line: `summary: N pass, M fail, K skip`.  
With `-v`, a bus inventory is printed.

### JSON

```powershell
adb shell /data/local/tmp/simple-android-info --summary --json > info.json
```

A full run returns `mode`, `summary`, `checks` (and `inventory` with `-v`).  
Check `status`: `pass` | `fail` | `skip`.

### Auto class detection

1. Looks like TV → `tv`
2. Modem present → `phone`
3. No modem, has network / HDMI → `box`
4. Otherwise → `unknown`

---

## Build from source

You need: Rust, Android NDK, and `adb` (to run on a device).

```powershell
rustup target add aarch64-linux-android
```

Point [`.cargo/config.toml`](.cargo/config.toml) at your NDK linker:

```toml
[target.aarch64-linux-android]
linker = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\aarch64-linux-android24-clang.cmd"
ar     = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\llvm-ar.exe"
```

Build and refresh `bin/`:

```powershell
cargo build --release --target aarch64-linux-android
New-Item -ItemType Directory -Force bin | Out-Null
Copy-Item -Force target\aarch64-linux-android\release\simple-android-info bin\simple-android-info
```

Then use the binary from `bin/` as in **Usage**.

The `target/` directory is not kept in git — only sources and the file in `bin/`.

---

## Limitations

- Needs a booted Android system and `adb`.
- Without root, some data is unavailable (often GPU frequency, DDR type).
- Node names are vendor-specific (common Unisoc / Qualcomm / MediaTek patterns are covered).

```text
simple-android-info --help
```
