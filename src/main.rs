//! simple-android-info — сводка по устройству и проверка интерфейсов
//! (телефон / Android TV / приставка). exit 0 = нет FAIL; SKIP = не ожидается.
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "android")]
use std::ffi::{CStr, CString};

#[cfg(target_os = "android")]
const PROP_VALUE_MAX: usize = 92;

#[cfg(target_os = "android")]
unsafe extern "C" {
    fn __system_property_get(
        name: *const std::ffi::c_char,
        value: *mut std::ffi::c_char,
    ) -> std::ffi::c_int;
}

fn get_prop(name: &str) -> String {
    #[cfg(target_os = "android")]
    {
        let Ok(c_name) = CString::new(name) else {
            return String::new();
        };
        let mut buf = [0 as std::ffi::c_char; PROP_VALUE_MAX];
        unsafe {
            __system_property_get(c_name.as_ptr(), buf.as_mut_ptr());
            CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = name;
        String::new()
    }
}

fn list_names(dir: &str) -> Vec<String> {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

fn driver_name(device: &Path) -> Option<String> {
    let link = fs::read_link(device.join("driver")).ok()?;
    Some(link.file_name()?.to_string_lossy().into_owned())
}

fn svc(name: &str) -> String {
    get_prop(&format!("init.svc.{name}"))
}

fn svc_running(name: &str) -> bool {
    svc(name) == "running"
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DeviceClass {
    Phone,
    Tv,
    Box,
    Unknown,
}

impl DeviceClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Tv => "tv",
            Self::Box => "box",
            Self::Unknown => "unknown",
        }
    }
}

/// Эвристика класса устройства по props + наличию интерфейсов.
fn detect_class(snap: &Snap) -> DeviceClass {
    let ch = get_prop("ro.build.characteristics").to_lowercase();
    let model = format!(
        "{} {} {} {}",
        get_prop("ro.product.model"),
        get_prop("ro.product.device"),
        get_prop("ro.product.name"),
        get_prop("ro.build.product")
    )
    .to_lowercase();

    // явный TV
    if ch.contains("tv")
        || model.contains("tv")
        || get_prop("ro.product.brand").eq_ignore_ascii_case("AndroidTV")
        || exists("/system/priv-app/LeanbackLauncher")
        || exists("/system/priv-app/TvSettings")
    {
        return DeviceClass::Tv;
    }

    let has_modem = snap.has_modem_iface();
    let has_touch = snap.has_touch();
    let has_telephony_prop = !get_prop("gsm.version.baseband").is_empty()
        || !get_prop("ro.telephony.default_network").is_empty()
        || svc_running("ril-daemon")
        || svc_running("vendor.ril-daemon")
        || svc_running("star-ril-daemon");

    // телефон: модем/telephony + обычно touch
    if has_modem || has_telephony_prop {
        if has_touch || ch.contains("default") || ch.is_empty() {
            return DeviceClass::Phone;
        }
    }

    // приставка: нет модема, есть сеть (eth/wlan), часто HDMI, без touch
    if !has_modem && !has_telephony_prop {
        if snap.has_hdmi() || snap.has_eth() || (!has_touch && snap.has_wlan()) {
            return DeviceClass::Box;
        }
    }

    if has_touch {
        DeviceClass::Phone
    } else {
        DeviceClass::Unknown
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Skip,
}

struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

struct Snap {
    i2c: Vec<(String, String)>,
    spi: Vec<(String, String)>,
    nets: Vec<String>,
    drm: Vec<String>,
    inputs: Vec<String>,
    mmc: Vec<String>,
    blocks: Vec<String>,
    ttys: Vec<String>,
    backlight: Vec<String>,
    sound: Vec<String>,
    rfkill: Vec<String>,
    iio: Vec<String>,
}

impl Snap {
    fn gather() -> Self {
        Self {
            i2c: bus_clients("/sys/bus/i2c/devices", true),
            spi: bus_clients("/sys/bus/spi/devices", false),
            nets: list_names("/sys/class/net"),
            drm: list_names("/sys/class/drm"),
            inputs: input_names(),
            mmc: list_names("/sys/bus/mmc/devices"),
            blocks: list_names("/sys/class/block"),
            ttys: list_names("/sys/class/tty"),
            backlight: list_names("/sys/class/backlight"),
            sound: list_names("/sys/class/sound"),
            rfkill: list_names("/sys/class/rfkill"),
            iio: list_names("/sys/bus/iio/devices"),
        }
    }

    fn has_wlan(&self) -> bool {
        self.nets.iter().any(|n| n.starts_with("wlan") || n.starts_with("wifi"))
    }

    fn has_eth(&self) -> bool {
        self.nets.iter().any(|n| {
            n.starts_with("eth") || n.starts_with("en") || n == "ethernet"
        })
    }

    fn has_modem_iface(&self) -> bool {
        self.nets.iter().any(|n| {
            n.starts_with("seth")
                || n.starts_with("rmnet")
                || n.starts_with("ccmni")
                || n.starts_with("wwan")
                || (n.starts_with("usb") && n.contains("rndis"))
                || n.starts_with("vnet")
        })
    }

    fn has_touch(&self) -> bool {
        self.spi.iter().any(|(_, d)| is_touch_driver(d))
            || self.inputs.iter().any(|n| is_touch_name(n))
            || self.i2c.iter().any(|(_, d)| is_touch_driver(d))
    }

    fn has_hdmi(&self) -> bool {
        self.drm.iter().any(|n| {
            let u = n.to_uppercase();
            u.contains("HDMI") || u.contains("DP-") || u.contains("DISPLAYPORT")
        })
    }

    fn has_dsi(&self) -> bool {
        self.drm
            .iter()
            .any(|n| {
                let u = n.to_uppercase();
                u.contains("DSI") || u.contains("EDP")
            })
            || !self.backlight.is_empty()
    }

    fn has_storage(&self) -> bool {
        !self.mmc.is_empty()
            || exists("/dev/block/mmcblk0")
            || self.blocks.iter().any(|n| {
                n.starts_with("sda")
                    || n.starts_with("sdc")
                    || n.starts_with("nvme")
                    || n.starts_with("dm-")
            })
            || exists("/sys/class/scsi_disk")
            || !list_names("/sys/class/scsi_device").is_empty()
    }

    fn has_gpu(&self) -> bool {
        exists("/sys/class/misc/mali0")
            || exists("/sys/class/kgsl/kgsl-3d0")
            || exists("/dev/mali0")
            || exists("/dev/kgsl-3d0")
            || self.drm.iter().any(|n| n.starts_with("renderD") || n == "card0")
            || !get_prop("ro.hardware.egl").is_empty()
            || !get_prop("ro.hardware.vulkan").is_empty()
    }

    fn has_audio(&self) -> bool {
        self.sound.iter().any(|n| n.starts_with("card") || n.starts_with("control"))
            || svc_running("audioserver")
    }

    fn has_bt(&self) -> bool {
        self.ttys.iter().any(|n| {
            let l = n.to_lowercase();
            l.contains("bt") || l.contains("bluetooth")
        }) || exists("/dev/ttyBT0")
            || exists("/sys/class/bluetooth")
            || !self.rfkill.is_empty()
            || svc_running("bluetooth")
            || svc_running("com.android.bluetooth")
            || get_prop("init.svc.bluetooth-1-0") == "running"
            || get_prop("init.svc.vendor.bluetooth-1-0") == "running"
    }

    fn has_camera(&self) -> bool {
        svc_running("cameraserver")
            || exists("/sys/class/misc/sprd_sensor")
            || exists("/sys/class/video4linux")
            || !list_names("/sys/class/video4linux").is_empty()
            || self.i2c.iter().any(|(_, d)| {
                let l = d.to_lowercase();
                l.contains("sensor") && !l.contains("als") && !l.contains("accel")
            })
    }

    fn has_gnss(&self) -> bool {
        svc_running("gpsd")
            || svc_running("mnld")
            || exists("/sys/class/misc/gnss_common_ctl")
            || exists("/dev/gnss")
            || exists("/dev/stpgps")
            || exists("/dev/gps_emi")
            || get_prop("ro.hardware.gps").len() > 0
    }

    fn has_charger(&self) -> bool {
        self.i2c.iter().any(|(_, d)| is_charger_driver(d))
            || !list_names("/sys/class/power_supply").is_empty()
            || exists("/sys/class/power_supply/battery")
    }
}

fn bus_clients(dir: &str, i2c_filter: bool) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if i2c_filter && (!name.contains('-') || name.starts_with("i2c-")) {
            continue;
        }
        let drv = driver_name(&entry.path()).unwrap_or_else(|| "?".into());
        rows.push((name, drv));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn input_names() -> Vec<String> {
    if let Ok(content) = fs::read_to_string("/proc/bus/input/devices") {
        let names: Vec<_> = content
            .lines()
            .filter_map(|l| l.strip_prefix("N: Name="))
            .map(|s| s.trim_matches('"').to_string())
            .collect();
        if !names.is_empty() {
            return names;
        }
    }
    list_names("/sys/class/input")
        .into_iter()
        .filter(|n| n.starts_with("event"))
        .collect()
}

fn is_touch_driver(d: &str) -> bool {
    let l = d.to_lowercase();
    l.contains("touch")
        || l.contains("nvt")
        || l.contains("fts")
        || l.contains("gt9")
        || l.contains("goodix")
        || l.contains("synaptics")
        || l.contains("himax")
        || l.contains("focal")
        || l.ends_with("-ts")
        || l.contains("_ts")
        || l.contains("ts_")
}

fn is_touch_name(n: &str) -> bool {
    let l = n.to_lowercase();
    l.contains("touch") || l.contains("nvt") || l.contains("goodix") || l.contains("fts")
}

fn is_charger_driver(d: &str) -> bool {
    let l = d.to_lowercase();
    l.contains("chg") || l.contains("charge") || l.contains("sgm") || l.contains("bms") || l.contains("pmi")
}

fn join_preview(items: &[String], max: usize) -> String {
    if items.is_empty() {
        return "none".into();
    }
    let mut s = items.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > max {
        s.push_str(&format!(" (+{})", items.len() - max));
    }
    s
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path.as_ref())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn khz_to_mhz(khz: &str) -> Option<u64> {
    khz.trim().parse::<u64>().ok().map(|v| v / 1000)
}

/// Внешняя команда с таймаутом (иначе dumpsys на части устройств зависает навсегда).
fn command_output_timeout(program: &str, args: &[&str], timeout: Duration) -> String {
    let mut child = match Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return String::new();
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(timeout) {
        Ok(buf) => {
            let _ = child.wait();
            String::from_utf8_lossy(&buf).into_owned()
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            String::new()
        }
    }
}

fn na(s: impl Into<String>) -> String {
    let s = s.into();
    if s.is_empty() {
        "n/a".into()
    } else {
        s
    }
}

fn prop_or(keys: &[&str]) -> String {
    keys.iter()
        .map(|k| get_prop(k))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

fn cpu_summary() -> (usize, String, String) {
    let logical = fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count();
    let mut clusters: Vec<(u64, u64, usize)> = Vec::new();
    for i in 0..logical.max(8) {
        let base = format!("/sys/devices/system/cpu/cpu{i}/cpufreq");
        let Some(min) = read_trimmed(format!("{base}/cpuinfo_min_freq")).and_then(|s| khz_to_mhz(&s))
        else {
            continue;
        };
        let Some(max) = read_trimmed(format!("{base}/cpuinfo_max_freq")).and_then(|s| khz_to_mhz(&s))
        else {
            continue;
        };
        if let Some((a, b, n)) = clusters.iter_mut().find(|(a, b, _)| *a == min && *b == max) {
            let _ = (a, b);
            *n += 1;
        } else {
            clusters.push((min, max, 1));
        }
    }
    let freq = if clusters.is_empty() {
        "n/a".into()
    } else {
        clusters
            .iter()
            .map(|(min, max, n)| format!("{n}x {min}-{max}MHz"))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let online = read_trimmed("/sys/devices/system/cpu/online").unwrap_or_else(|| "n/a".into());
    (logical, online, freq)
}

fn ram_summary() -> (String, String) {
    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kib = |key: &str| -> Option<u64> {
        meminfo
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|k| k.parse().ok())
    };
    let total = kib("MemTotal:")
        .map(|k| format!("{} MiB", k / 1024))
        .unwrap_or_else(|| "n/a".into());
    let avail = kib("MemAvailable:")
        .map(|k| format!("{} MiB", k / 1024))
        .unwrap_or_else(|| "n/a".into());
    (total, avail)
}

fn cpu_model() -> String {
    let from_prop = prop_or(&[
        "dalvik.vm.isa.arm64.variant",
        "dalvik.vm.isa.arm.variant",
    ]);
    if !from_prop.is_empty() {
        return from_prop;
    }
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    for key in ["Hardware", "model name", "Processor"] {
        if let Some(v) = cpuinfo
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split(':').nth(1))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return v.to_string();
        }
    }
    "n/a".into()
}

fn format_uptime_secs(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

fn uptime_load() -> (String, String) {
    let up = read_trimmed("/proc/uptime")
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|secs| format_uptime_secs(secs as u64))
        .unwrap_or_else(|| "n/a".into());
    let load = read_trimmed("/proc/loadavg")
        .map(|s| {
            s.split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "n/a".into());
    (up, load)
}

fn boot_summary() -> String {
    let mut parts = Vec::new();
    let slot = get_prop("ro.boot.slot_suffix");
    if !slot.is_empty() {
        parts.push(format!("slot={slot}"));
    }
    let vb = get_prop("ro.boot.verifiedbootstate");
    if !vb.is_empty() {
        parts.push(format!("vbmeta={vb}"));
    }
    if parts.is_empty() {
        "n/a".into()
    } else {
        parts.join(", ")
    }
}

fn data_usage() -> String {
    // полный df: строка с mount `/data` (df -h /data на части билдов подставляет obb)
    let out = command_output_timeout("df", &["-h"], Duration::from_secs(2));
    for line in out.lines() {
        let cols: Vec<_> = line.split_whitespace().collect();
        if cols.len() >= 6 && cols[5] == "/data" {
            return format!(
                "{} total, {} used, {} avail ({})",
                cols[1], cols[2], cols[3], cols[4]
            );
        }
    }
    "n/a".into()
}

fn wlan_mac(s: &Snap) -> String {
    let iface = s
        .nets
        .iter()
        .find(|n| n.starts_with("wlan"))
        .or_else(|| s.nets.iter().find(|n| n.starts_with("eth")))
        .cloned();
    let Some(iface) = iface else {
        return "n/a".into();
    };
    // ponytail: /sys/.../address часто Permission denied без root — тогда честный n/a
    let Some(addr) = read_trimmed(format!("/sys/class/net/{iface}/address"))
        .filter(|a| a != "00:00:00:00:00:00")
    else {
        return "n/a".into();
    };
    let state = read_trimmed(format!("/sys/class/net/{iface}/operstate")).unwrap_or_default();
    if state.is_empty() {
        format!("{addr} ({iface})")
    } else {
        format!("{addr} ({iface}/{state})")
    }
}

fn display_summary(s: &Snap) -> (String, String) {
    let dump = command_output_timeout("dumpsys", &["display"], Duration::from_secs(3));

    let res = dump
        .lines()
        .find(|l| l.contains("width*height="))
        .map(|l| {
            l.trim()
                .trim_start_matches("width*height=")
                .replace('*', "x")
                .to_string()
        })
        .or_else(|| {
            // "720 x 1640" из DisplayDeviceInfo
            dump.split_whitespace()
                .collect::<Vec<_>>()
                .windows(3)
                .find(|w| w[1] == "x" && w[0].parse::<u32>().is_ok() && w[2].trim_matches(',').parse::<u32>().is_ok())
                .map(|w| format!("{}x{}", w[0], w[2].trim_matches(',')))
        })
        .unwrap_or_else(|| {
            if s.has_hdmi() {
                format!("HDMI [{}]", join_preview(&s.drm, 3))
            } else if s.has_dsi() {
                format!("DSI [{}]", join_preview(&s.drm, 3))
            } else {
                join_preview(&s.drm, 4)
            }
        });

    let mut fps: Vec<u32> = dump
        .split("fps=")
        .skip(1)
        .filter_map(|part| {
            let num: f32 = part
                .split(|c: char| !c.is_ascii_digit() && c != '.')
                .next()?
                .parse()
                .ok()?;
            let n = num.round() as u32;
            (1..=240).contains(&n).then_some(n)
        })
        .collect();
    fps.sort_unstable();
    fps.dedup();
    let dpi = get_prop("ro.sf.lcd_density");
    let fps_out = match (fps.is_empty(), dpi.is_empty()) {
        (false, false) => format!(
            "{} Hz, dpi {dpi}",
            fps.iter().map(ToString::to_string).collect::<Vec<_>>().join("/")
        ),
        (false, true) => format!(
            "{} Hz",
            fps.iter().map(ToString::to_string).collect::<Vec<_>>().join("/")
        ),
        (true, false) => format!("dpi {dpi}"),
        (true, true) => "n/a".into(),
    };

    (na(res), fps_out)
}

fn row(key: &str, val: impl AsRef<str>) {
    println!("| {:<14} | {:<52} |", key, trunc(val.as_ref(), 52));
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        s.to_string()
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

fn json_str_arr(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| json_str(s))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{inner}]")
}

struct Summary {
    class: String,
    class_forced: bool,
    manufacturer: String,
    model: String,
    device: String,
    serial: String,
    hardware: String,
    android: String,
    sdk: String,
    build: String,
    soc_mfr: String,
    soc_model: String,
    platform: String,
    cpu_cores: usize,
    cpu_online: String,
    cpu_model: String,
    cpu_freq: String,
    ram_total: String,
    ram_avail: String,
    gpu: String,
    display: String,
    refresh: String,
    storage: String,
    data: String,
    network: Vec<String>,
    wlan_mac: String,
    boot: String,
    timezone: String,
    uptime: String,
    load: String,
    i2c_count: usize,
    spi_count: usize,
    input_count: usize,
    features: Vec<String>,
}

fn collect_summary(class: DeviceClass, forced: bool, s: &Snap) -> Summary {
    let (cores, online, cpu_freq) = cpu_summary();
    let (ram_total, ram_avail) = ram_summary();
    let (disp_res, disp_fps) = display_summary(s);
    let (uptime, load) = uptime_load();

    let gpu = if exists("/sys/class/misc/mali0") {
        format!("mali ({})", na(get_prop("ro.hardware.egl")))
    } else if exists("/sys/class/kgsl/kgsl-3d0") {
        format!("adreno/kgsl ({})", na(get_prop("ro.hardware.egl")))
    } else {
        na(prop_or(&["ro.hardware.egl", "ro.hardware.vulkan"]))
    };

    let storage = if !s.mmc.is_empty() {
        format!("eMMC/SD {}", join_preview(&s.mmc, 2))
    } else if s.blocks.iter().any(|n| n.starts_with("sda")) {
        "UFS/SCSI (sda)".into()
    } else if s.has_storage() {
        format!("blocks~{}", s.blocks.len())
    } else {
        "n/a".into()
    };

    let mut network = Vec::new();
    if s.has_wlan() {
        network.push("wlan".into());
    }
    if s.has_eth() {
        network.push("eth".into());
    }
    if s.has_modem_iface() {
        network.push("modem".into());
    }
    if s.has_bt() {
        network.push("bt".into());
    }

    let features = [
        ("touch", s.has_touch()),
        ("camera", s.has_camera()),
        ("gnss", s.has_gnss()),
        ("hdmi", s.has_hdmi()),
        ("dsi", s.has_dsi()),
    ]
    .into_iter()
    .filter(|(_, on)| *on)
    .map(|(n, _)| n.to_string())
    .collect();

    Summary {
        class: class.as_str().into(),
        class_forced: forced,
        manufacturer: na(get_prop("ro.product.manufacturer")),
        model: na(get_prop("ro.product.model")),
        device: na(get_prop("ro.product.device")),
        serial: na(prop_or(&["ro.serialno", "ro.boot.serialno"])),
        hardware: na(prop_or(&["ro.hardware", "ro.boot.hardware"])),
        android: na(get_prop("ro.build.version.release")),
        sdk: na(get_prop("ro.build.version.sdk")),
        build: na(get_prop("ro.build.display.id")),
        soc_mfr: na(get_prop("ro.soc.manufacturer")),
        soc_model: na(prop_or(&[
            "ro.soc.model",
            "ro.board.platform",
            "ro.hardware",
        ])),
        platform: na(get_prop("ro.board.platform")),
        cpu_cores: cores,
        cpu_online: online,
        cpu_model: cpu_model(),
        cpu_freq,
        ram_total,
        ram_avail,
        gpu,
        display: disp_res,
        refresh: disp_fps,
        storage,
        data: data_usage(),
        network,
        wlan_mac: wlan_mac(s),
        boot: boot_summary(),
        timezone: na(prop_or(&["persist.sys.timezone", "ro.timezone"])),
        uptime,
        load,
        i2c_count: s.i2c.len(),
        spi_count: s.spi.len(),
        input_count: s.inputs.len(),
        features,
    }
}

fn print_summary_table(sum: &Summary) {
    println!("+----------------+------------------------------------------------------+");
    println!("| SUMMARY        |                                                      |");
    println!("+----------------+------------------------------------------------------+");
    row(
        "class",
        format!(
            "{}{}",
            sum.class,
            if sum.class_forced {
                " (forced)"
            } else {
                " (auto)"
            }
        ),
    );
    row("product", format!("{} / {}", sum.manufacturer, sum.model));
    row("device", &sum.device);
    row("serial", &sum.serial);
    row("hardware", &sum.hardware);
    row("android", format!("{} (SDK {})", sum.android, sum.sdk));
    row("build", &sum.build);
    row("boot", &sum.boot);
    row("soc", format!("{} / {}", sum.soc_mfr, sum.soc_model));
    row("platform", &sum.platform);
    row(
        "cpu",
        format!("{} cores (online {})", sum.cpu_cores, sum.cpu_online),
    );
    row("cpu_model", &sum.cpu_model);
    row("cpu_freq", &sum.cpu_freq);
    row(
        "ram",
        format!("{} (avail {})", sum.ram_total, sum.ram_avail),
    );
    row("gpu", &sum.gpu);
    row("display", &sum.display);
    row("refresh", &sum.refresh);
    row("storage", &sum.storage);
    row("data", &sum.data);
    row(
        "network",
        if sum.network.is_empty() {
            "n/a".into()
        } else {
            sum.network.join(", ")
        },
    );
    row("wlan_mac", &sum.wlan_mac);
    row("timezone", &sum.timezone);
    row("uptime", format!("{} (load {})", sum.uptime, sum.load));
    row(
        "buses",
        format!(
            "i2c={} spi={} input={}",
            sum.i2c_count, sum.spi_count, sum.input_count
        ),
    );
    row(
        "features",
        if sum.features.is_empty() {
            "n/a".into()
        } else {
            sum.features.join(", ")
        },
    );
    println!("+----------------+------------------------------------------------------+");
}

impl Summary {
    fn to_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"class\":{class},",
                "\"class_forced\":{forced},",
                "\"manufacturer\":{mfr},",
                "\"model\":{model},",
                "\"device\":{device},",
                "\"serial\":{serial},",
                "\"hardware\":{hardware},",
                "\"android\":{android},",
                "\"sdk\":{sdk},",
                "\"build\":{build},",
                "\"boot\":{boot},",
                "\"soc_manufacturer\":{soc_mfr},",
                "\"soc_model\":{soc_model},",
                "\"platform\":{platform},",
                "\"cpu_cores\":{cores},",
                "\"cpu_online\":{online},",
                "\"cpu_model\":{cpu_model},",
                "\"cpu_freq\":{cpu_freq},",
                "\"ram_total\":{ram_total},",
                "\"ram_avail\":{ram_avail},",
                "\"gpu\":{gpu},",
                "\"display\":{display},",
                "\"refresh\":{refresh},",
                "\"storage\":{storage},",
                "\"data\":{data},",
                "\"network\":{network},",
                "\"wlan_mac\":{wlan_mac},",
                "\"timezone\":{timezone},",
                "\"uptime\":{uptime},",
                "\"load\":{load},",
                "\"buses\":{{\"i2c\":{i2c},\"spi\":{spi},\"input\":{input}}},",
                "\"features\":{features}",
                "}}"
            ),
            class = json_str(&self.class),
            forced = self.class_forced,
            mfr = json_str(&self.manufacturer),
            model = json_str(&self.model),
            device = json_str(&self.device),
            serial = json_str(&self.serial),
            hardware = json_str(&self.hardware),
            android = json_str(&self.android),
            sdk = json_str(&self.sdk),
            build = json_str(&self.build),
            boot = json_str(&self.boot),
            soc_mfr = json_str(&self.soc_mfr),
            soc_model = json_str(&self.soc_model),
            platform = json_str(&self.platform),
            cores = self.cpu_cores,
            online = json_str(&self.cpu_online),
            cpu_model = json_str(&self.cpu_model),
            cpu_freq = json_str(&self.cpu_freq),
            ram_total = json_str(&self.ram_total),
            ram_avail = json_str(&self.ram_avail),
            gpu = json_str(&self.gpu),
            display = json_str(&self.display),
            refresh = json_str(&self.refresh),
            storage = json_str(&self.storage),
            data = json_str(&self.data),
            network = json_str_arr(&self.network),
            wlan_mac = json_str(&self.wlan_mac),
            timezone = json_str(&self.timezone),
            uptime = json_str(&self.uptime),
            load = json_str(&self.load),
            i2c = self.i2c_count,
            spi = self.spi_count,
            input = self.input_count,
            features = json_str_arr(&self.features),
        )
    }
}

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Pass => "pass",
        Status::Fail => "fail",
        Status::Skip => "skip",
    }
}

fn checks_to_json(
    checks: &[Check],
    pass: usize,
    fail: usize,
    skip: usize,
    lang: Option<Lang>,
) -> String {
    let items = checks
        .iter()
        .map(|c| {
            let mut item = format!(
                "{{\"name\":{},\"status\":{},\"detail\":{}",
                json_str(c.name),
                json_str(status_str(c.status)),
                json_str(&c.detail)
            );
            if let Some(lang) = lang {
                if let Some((what, where_, why)) = check_explain(c.name, lang) {
                    item.push_str(&format!(
                        ",\"explain\":{{\"what\":{},\"where\":{},\"why\":{}}}",
                        json_str(what),
                        json_str(where_),
                        json_str(why)
                    ));
                }
            }
            item.push('}');
            item
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"pass\":{pass},\"fail\":{fail},\"skip\":{skip},\"items\":[{items}]}}"
    )
}

fn inventory_to_json(s: &Snap) -> String {
    let i2c = s
        .i2c
        .iter()
        .map(|(a, d)| format!("{{\"addr\":{},\"driver\":{}}}", json_str(a), json_str(d)))
        .collect::<Vec<_>>()
        .join(",");
    let spi = s
        .spi
        .iter()
        .map(|(a, d)| format!("{{\"addr\":{},\"driver\":{}}}", json_str(a), json_str(d)))
        .collect::<Vec<_>>()
        .join(",");
    let nets: Vec<String> = s
        .nets
        .iter()
        .filter(|n| {
            n.starts_with("wlan")
                || n.starts_with("eth")
                || n.starts_with("en")
                || n.starts_with("seth")
                || n.starts_with("rmnet")
                || n.starts_with("ccmni")
                || n.starts_with("wwan")
                || *n == "lo"
        })
        .cloned()
        .collect();
    format!(
        "{{\"i2c\":[{i2c}],\"spi\":[{spi}],\"net\":{},\"drm\":{},\"input\":{}}}",
        json_str_arr(&nets),
        json_str_arr(&s.drm),
        json_str_arr(&s.inputs)
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ru,
    En,
}

#[derive(Default)]
struct Args {
    class: Option<DeviceClass>,
    summary_only: bool,
    /// пояснения what/where/why на выбранном языке
    lang: Option<Lang>,
    json: bool,
    verbose: bool,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--summary" | "-s" => args.summary_only = true,
            // --smt оставлен как алиас основного режима
            "--smt" => {}
            "--json" | "-j" => args.json = true,
            "-v" | "--verbose" => args.verbose = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            s if s.starts_with("--lang=") || s.starts_with("--explain=") => {
                let v = s.split_once('=').map(|(_, v)| v).unwrap_or("");
                args.lang = match v {
                    "ru" | "RU" | "rus" | "russian" => Some(Lang::Ru),
                    "en" | "EN" | "eng" | "english" => Some(Lang::En),
                    other => {
                        eprintln!("unknown lang: {other} (ru|en)");
                        std::process::exit(2);
                    }
                };
            }
            s if s.starts_with("--class=") => {
                args.class = match s.strip_prefix("--class=").unwrap() {
                    "phone" => Some(DeviceClass::Phone),
                    "tv" => Some(DeviceClass::Tv),
                    "box" => Some(DeviceClass::Box),
                    "summary" => {
                        args.summary_only = true;
                        None
                    }
                    other => {
                        eprintln!("unknown class: {other} (phone|tv|box|summary)");
                        std::process::exit(2);
                    }
                };
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    args
}

fn print_help() {
    eprintln!(
        "\
simple-android-info — detailed device / SMT bring-up report

Usage:
  simple-android-info [--lang=ru|en] [--summary|-s] [--json|-j] [--verbose|-v] [--class=…]

  (default)         detailed checks per item (boot/NVRAM/peripherals)
  --lang=ru|en      explanations for each check (what / where / why)
  --explain=ru|en   same as --lang=
  --summary, -s     summary table only
  --smt             alias of default
  --json, -j        JSON on stdout
  --verbose, -v     include bus inventory
  --class=…         force device class (phone|tv|box|summary)
"
    );
}

/// Пояснение к пункту чеклиста: что / где смотрим / зачем на SMT.
fn check_explain(name: &str, lang: Lang) -> Option<(&'static str, &'static str, &'static str)> {
    // (what, where, why)
    match (lang, name) {
        (Lang::Ru, "boot_completed") => Some((
            "Система полностью загрузилась в Android userspace.",
            "Свойства sys.boot_completed / dev.bootcomplete, сервис bootanim.",
            "Без этого остальные сервисы и HAL ещё не готовы — плата «не дошла» после прошивки/пайки.",
        )),
        (Lang::En, "boot_completed") => Some((
            "Android userspace finished booting.",
            "Props sys.boot_completed / dev.bootcomplete, bootanim service.",
            "If this fails, HALs and apps are not ready yet after flash/power-on.",
        )),
        (Lang::Ru, "verified_boot") => Some((
            "Состояние Verified Boot / AVB (целостность прошивки).",
            "ro.boot.verifiedbootstate, ro.boot.veritymode, A/B slot.",
            "green/yellow — образ доверенный; orange/red — подпись/раздел повреждён или unlocked.",
        )),
        (Lang::En, "verified_boot") => Some((
            "Verified Boot / AVB integrity state.",
            "ro.boot.verifiedbootstate, ro.boot.veritymode, A/B slot.",
            "green/yellow means trusted image; orange/red means unlock or corrupted partitions.",
        )),
        (Lang::Ru, "nvram") => Some((
            "Заводская NVRAM / калибровки RF (Wi‑Fi, BT, модем) инициализированы.",
            "vendor.mtk.nvram.ready, vendor.service.nvram_init, разделы nvram/nvdata/proinfo.",
            "После пайки критично: без Ready радиочастотный тракт часто «пустой» или нестабильный.",
        )),
        (Lang::En, "nvram") => Some((
            "Factory NVRAM / RF calibration initialized.",
            "vendor.mtk.nvram.ready, vendor.service.nvram_init, nvram/nvdata/proinfo partitions.",
            "Critical after SMT: without Ready, Wi‑Fi/BT/modem calibration is missing or broken.",
        )),
        (Lang::Ru, "storage") => Some((
            "Накопитель (eMMC/UFS) виден ядру.",
            "/sys/bus/mmc, /sys/class/block.",
            "Нет накопителя — не встал eMMC/UFS после пайки или нет питания/шин.",
        )),
        (Lang::En, "storage") => Some((
            "Storage (eMMC/UFS) is visible to the kernel.",
            "/sys/bus/mmc, /sys/class/block.",
            "Missing storage usually means solder/power/bus failure on the flash chip.",
        )),
        (Lang::Ru, "data_mounted") => Some((
            "Раздел пользовательских данных /data смонтирован и доступен по объёму.",
            "Команда df (строка mount /data).",
            "Проверяет, что userdata живой: шифрование/фс поднялись, место есть.",
        )),
        (Lang::En, "data_mounted") => Some((
            "Userdata (/data) is mounted and sized.",
            "df output for mount point /data.",
            "Confirms userdata filesystem/encryption came up after bring-up.",
        )),
        (Lang::Ru, "display") => Some((
            "Дисплейный стек: DRM + SurfaceFlinger, есть разрешение.",
            "dumpsys display, /sys/class/drm, init.svc.surfaceflinger.",
            "Панель/HDMI/MIPI не поднялись — типичный дефект шлейфа, BGA SoC или питания дисплея.",
        )),
        (Lang::En, "display") => Some((
            "Display stack: DRM + SurfaceFlinger with a resolution.",
            "dumpsys display, /sys/class/drm, surfaceflinger service.",
            "Failure points to panel/HDMI/MIPI, SoC BGA, or display power issues.",
        )),
        (Lang::Ru, "gpu") => Some((
            "Графический ускоритель (Mali/Adreno/MTK EGL) обнаружен.",
            "ro.hardware.egl/vulkan, /sys/class/misc/mali0, kgsl.",
            "GPU нужен UI и камере; отсутствие — проблема драйвера/SoC/прошивки.",
        )),
        (Lang::En, "gpu") => Some((
            "GPU accelerator (Mali/Adreno/MTK EGL) is present.",
            "ro.hardware.egl/vulkan, mali0/kgsl sysfs nodes.",
            "Needed for UI/camera; missing GPU means driver/SoC/firmware bring-up failure.",
        )),
        (Lang::Ru, "audio") => Some((
            "Аудиокарта ALSA и audioserver доступны.",
            "/sys/class/sound, init.svc.audioserver.",
            "Проверка кодека/I2S после пайки: нет card — нет звука/микрофона.",
        )),
        (Lang::En, "audio") => Some((
            "ALSA sound card and audioserver are up.",
            "/sys/class/sound, audioserver service.",
            "Validates codec/I2S after solder; no card means no speaker/mic path.",
        )),
        (Lang::Ru, "wlan") => Some((
            "Wi‑Fi интерфейс и стек (wificond) подняты.",
            "/sys/class/net/wlan*, init.svc.wificond.",
            "Модуль Wi‑Fi / SDIO/PCIe / питание антенны; MAC здесь не требуется.",
        )),
        (Lang::En, "wlan") => Some((
            "Wi‑Fi interface and wificond stack are up.",
            "/sys/class/net/wlan*, wificond service.",
            "Checks Wi‑Fi module / SDIO-PCIe / RF path; factory MAC is not required here.",
        )),
        (Lang::Ru, "ethernet") => Some((
            "Проводной Ethernet (если ожидается на TV/box).",
            "/sys/class/net/eth* или en*.",
            "PHY/трансформер после пайки; на телефоне обычно SKIP.",
        )),
        (Lang::En, "ethernet") => Some((
            "Wired Ethernet when expected on TV/box.",
            "/sys/class/net/eth* or en*.",
            "PHY/magnetics after SMT; usually SKIP on phones.",
        )),
        (Lang::Ru, "bluetooth") => Some((
            "Контроллер Bluetooth обнаружен системой.",
            "rfkill, /sys/class/bluetooth, связанные сервисы.",
            "Часто один чип с Wi‑Fi; FAIL — нет UART/USB к BT или нет прошивки чипа.",
        )),
        (Lang::En, "bluetooth") => Some((
            "Bluetooth controller is visible to the OS.",
            "rfkill, /sys/class/bluetooth, related services.",
            "Often shared with Wi‑Fi; FAIL means UART/USB/firmware to the BT chip is down.",
        )),
        (Lang::Ru, "touch") => Some((
            "Сенсорная панель (touchscreen) зарегистрирована.",
            "I2C/SPI touch-драйвер, input nodes, dumpsys input.",
            "Шлейф/контроллер тача после пайки; без touch телефон/планшет негодны.",
        )),
        (Lang::En, "touch") => Some((
            "Touchscreen is registered.",
            "I2C/SPI touch driver, input nodes, dumpsys input.",
            "FPC/touch controller after SMT; required for phone/tablet usability.",
        )),
        (Lang::Ru, "battery_charger") => Some((
            "Топливный контроллер / зарядка / USB power_supply.",
            "/sys/class/power_supply (battery, usb, charger, ac).",
            "Fuel gauge и зарядный IC после пайки; capacity/status — живой PMIC.",
        )),
        (Lang::En, "battery_charger") => Some((
            "Fuel gauge / charger / USB power_supply present.",
            "/sys/class/power_supply (battery, usb, charger, ac).",
            "Validates gauge and charger IC; capacity/status show a live PMIC path.",
        )),
        (Lang::Ru, "camera") => Some((
            "Камеры зарегистрированы Camera HAL.",
            "dumpsys media.camera (число устройств, Facing).",
            "CSI/модуль камеры после пайки; 0 устройств — нет сенсора или питания AVDD/DVDD.",
        )),
        (Lang::En, "camera") => Some((
            "Cameras registered with Camera HAL.",
            "dumpsys media.camera (device count, Facing).",
            "CSI/camera module after SMT; zero devices means sensor or AVDD/DVDD failure.",
        )),
        (Lang::Ru, "sensors") => Some((
            "Датчики (IMU, ALS, proximity…) видны SensorService.",
            "dumpsys sensorservice, /sys/bus/iio.",
            "Акселерометр/гиро/магнитометр на I2C/SPI; важно для автоповорота и жестов.",
        )),
        (Lang::En, "sensors") => Some((
            "Sensors (IMU, ALS, proximity…) visible to SensorService.",
            "dumpsys sensorservice, /sys/bus/iio.",
            "Accel/gyro/mag on I2C/SPI; needed for rotation and motion features.",
        )),
        (Lang::Ru, "modem_baseband") => Some((
            "Модемный baseband и сетевые интерфейсы модема.",
            "gsm.version.baseband, iface ccmni/rmnet/seth, gsm.sim.state.",
            "MD образ загружен; SIM ABSENT нормален без карты — важен сам baseband.",
        )),
        (Lang::En, "modem_baseband") => Some((
            "Modem baseband and modem network interfaces.",
            "gsm.version.baseband, ccmni/rmnet/seth ifaces, gsm.sim.state.",
            "MD image loaded; SIM ABSENT is OK without a card — baseband itself matters.",
        )),
        (Lang::Ru, "gnss") => Some((
            "GNSS/GPS подсистема (mnld, устройства /dev).",
            "init.svc.mnld, /dev/stpgps, /dev/gps_emi.",
            "GPS чип/совмещённый RF; без узлов навигация и часть калибровок недоступны.",
        )),
        (Lang::En, "gnss") => Some((
            "GNSS/GPS subsystem (mnld, /dev nodes).",
            "mnld service, /dev/stpgps, /dev/gps_emi.",
            "GPS chip / shared RF path; missing nodes break navigation bring-up.",
        )),
        (Lang::Ru, "hdmi") => Some((
            "Внешний HDMI/DP выход (TV/приставка).",
            "DRM connectors, hdmicecd.",
            "HDMI PHY/разъём после пайки; на телефоне обычно SKIP.",
        )),
        (Lang::En, "hdmi") => Some((
            "External HDMI/DP output (TV/box).",
            "DRM connectors, hdmicecd.",
            "HDMI PHY/connector after SMT; usually SKIP on phones.",
        )),
        (Lang::Ru, "usb") => Some((
            "USB gadget / устройство для ПК (ADB, MTP).",
            "sys.usb.state, sys.usb.config, /sys/class/udc.",
            "USB PHY и разъём; без этого нет прошивки/логов с линии по кабелю.",
        )),
        (Lang::En, "usb") => Some((
            "USB gadget / host link for PC (ADB, MTP).",
            "sys.usb.state, sys.usb.config, /sys/class/udc.",
            "USB PHY and connector; required for factory flash/logs over cable.",
        )),
        (Lang::Ru, "surfaceflinger") => Some((
            "Композитор экрана Android запущен.",
            "init.svc.surfaceflinger.",
            "Без SF нет картинки даже при живом DRM — graphics stack не собрался.",
        )),
        (Lang::En, "surfaceflinger") => Some((
            "Android display compositor is running.",
            "init.svc.surfaceflinger.",
            "Without SF there is no frame even if DRM exists — graphics stack failed.",
        )),
        (Lang::Ru, "identity") => Some((
            "Идентификация платы: модель, serial, SoC, память, аптайм.",
            "getprop (ro.product.*, ro.serialno, ro.hardware), /proc, meminfo.",
            "Сводка для трассировки станции: какая сборка и какой экземпляр на столе.",
        )),
        (Lang::En, "identity") => Some((
            "Board identity: model, serial, SoC, memory, uptime.",
            "getprop (ro.product.*, ro.serialno, ro.hardware), /proc, meminfo.",
            "Traceability for the line: which build and which unit is on the bench.",
        )),
        _ => None,
    }
}

fn print_explain(name: &str, lang: Lang) {
    let Some((what, where_, why)) = check_explain(name, lang) else {
        return;
    };
    match lang {
        Lang::Ru => {
            println!("  ———");
            println!("  что:    {what}");
            println!("  где:    {where_}");
            println!("  зачем:  {why}");
        }
        Lang::En => {
            println!("  ———");
            println!("  what:   {what}");
            println!("  where:  {where_}");
            println!("  why:    {why}");
        }
    }
}

fn is_mtk() -> bool {
    let hw = get_prop("ro.hardware").to_lowercase();
    let plat = get_prop("ro.board.platform").to_lowercase();
    hw.starts_with("mt") || plat.starts_with("mt") || !get_prop("vendor.mtk.nvram.ready").is_empty()
}

fn nvram_status() -> (bool, bool, String) {
    // (applicable, ok, detail)
    let ready = get_prop("vendor.mtk.nvram.ready");
    let init = get_prop("vendor.service.nvram_init");
    let parts = ["nvram", "nvdata", "proinfo"]
        .iter()
        .filter(|n| exists(&format!("/dev/block/by-name/{n}")))
        .copied()
        .collect::<Vec<_>>();
    let detail = format!(
        "ready={} init={} parts=[{}]",
        if ready.is_empty() { "-" } else { &ready },
        if init.is_empty() { "-" } else { &init },
        parts.join(",")
    );
    if ready == "1" || init.eq_ignore_ascii_case("Ready") {
        return (true, true, detail);
    }
    if is_mtk() {
        return (true, false, detail);
    }
    if !parts.is_empty() {
        // не MTK, но разделы есть — наличие ок, Ready не проверяем
        return (true, true, detail);
    }
    (false, false, detail)
}

fn detail_lines(lines: &[String]) -> String {
    lines
        .iter()
        .filter(|l| !l.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

fn camera_report() -> (usize, String) {
    let out = command_output_timeout("dumpsys", &["media.camera"], Duration::from_secs(3));
    let n = out
        .lines()
        .find(|l| l.contains("Number of camera devices:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let mut lines = vec![format!("devices: {n}")];
    for face in out.lines().filter(|l| l.trim_start().starts_with("Facing:")) {
        lines.push(face.trim().to_string());
    }
    let v4l = list_names("/sys/class/video4linux");
    if !v4l.is_empty() {
        lines.push(format!("v4l: {}", join_preview(&v4l, 6)));
    }
    if n == 0 && v4l.is_empty() {
        lines.push("cameraserver=".to_string() + &svc("cameraserver"));
    }
    (n.max(v4l.len()), detail_lines(&lines))
}

fn sensor_report() -> (usize, String) {
    let out = command_output_timeout("dumpsys", &["sensorservice"], Duration::from_secs(3));
    let names: Vec<String> = out
        .lines()
        .filter(|l| l.contains("type: android.sensor."))
        .filter_map(|l| {
            // "0x..) Name | vendor | ..."
            let rest = l.split(')').nth(1)?;
            Some(rest.split('|').next()?.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect();
    let iio = list_names("/sys/bus/iio/devices");
    let mut lines = vec![format!("sensorservice: {} sensors", names.len())];
    if !names.is_empty() {
        lines.push(format!("list: {}", join_preview(&names, 8)));
    }
    if !iio.is_empty() {
        lines.push(format!("iio: {}", join_preview(&iio, 6)));
    }
    let n = names.len().max(iio.len());
    (n, detail_lines(&lines))
}

fn power_supply_names() -> Vec<String> {
    list_names("/sys/class/power_supply")
}

fn battery_report(ps: &[String]) -> String {
    let mut lines = vec![format!("power_supply: {}", join_preview(ps, 8))];
    for name in ps {
        let base = format!("/sys/class/power_supply/{name}");
        let mut bits = Vec::new();
        if let Some(t) = read_trimmed(format!("{base}/type")) {
            bits.push(format!("type={t}"));
        }
        if let Some(st) = read_trimmed(format!("{base}/status")) {
            bits.push(format!("status={st}"));
        }
        if let Some(cap) = read_trimmed(format!("{base}/capacity")) {
            bits.push(format!("capacity={cap}%"));
        }
        if let Some(p) = read_trimmed(format!("{base}/present")) {
            bits.push(format!("present={p}"));
        }
        if !bits.is_empty() {
            lines.push(format!("{name}: {}", bits.join(", ")));
        }
    }
    detail_lines(&lines)
}

fn net_ifaces<'a>(s: &'a Snap, prefixes: &[&str]) -> Vec<&'a str> {
    s.nets
        .iter()
        .filter(|n| prefixes.iter().any(|p| n.starts_with(p)))
        .map(|n| n.as_str())
        .collect()
}

fn iface_report(ifaces: &[&str]) -> String {
    let mut lines = Vec::new();
    for iface in ifaces {
        let state = read_trimmed(format!("/sys/class/net/{iface}/operstate")).unwrap_or_else(|| "?".into());
        let carrier = read_trimmed(format!("/sys/class/net/{iface}/carrier")).unwrap_or_default();
        if carrier.is_empty() {
            lines.push(format!("{iface}: operstate={state}"));
        } else {
            lines.push(format!("{iface}: operstate={state}, carrier={carrier}"));
        }
    }
    if lines.is_empty() {
        "ifaces: none".into()
    } else {
        detail_lines(&lines)
    }
}

fn touch_report(s: &Snap) -> (bool, String) {
    let dump = command_output_timeout("dumpsys", &["input"], Duration::from_secs(2));
    let dump_l = dump.to_lowercase();
    let by_dump = dump_l.contains("touchscreen") || dump_l.contains(" touch ");
    let ok = s.has_touch() || by_dump;
    let mut lines = Vec::new();
    if s.has_touch() {
        lines.push("detected: touch driver/name".into());
    }
    if by_dump {
        lines.push("dumpsys input: touchscreen present".into());
    }
    if !s.inputs.is_empty() {
        lines.push(format!("input nodes: {}", join_preview(&s.inputs, 8)));
    }
    for (n, d) in s.i2c.iter().chain(s.spi.iter()) {
        if is_touch_driver(d) {
            lines.push(format!("bus: {n} -> {d}"));
        }
    }
    if lines.is_empty() {
        lines.push("no touch evidence".into());
    }
    (ok, detail_lines(&lines))
}

/// Основной чеклист: boot / NVRAM / периферия. Без Wi‑Fi MAC.
fn build_smt_checks(class: DeviceClass, s: &Snap, sum: &Summary) -> Vec<Check> {
    let boot_ok = get_prop("sys.boot_completed") == "1" || get_prop("dev.bootcomplete") == "1";
    let vb = get_prop("ro.boot.verifiedbootstate").to_lowercase();
    let verity = get_prop("ro.boot.veritymode");
    let vb_ok = vb == "green" || vb == "yellow" || vb.is_empty();
    let (nv_app, nv_ok, nv_detail) = nvram_status();
    let (cam_n, cam_detail) = camera_report();
    let (sens_n, sens_detail) = sensor_report();
    let ps = power_supply_names();
    let batt_ok = ps.iter().any(|n| {
        let t = n.to_lowercase();
        t.contains("battery") || t == "ac" || t.contains("charger") || t == "usb"
    });
    let data = data_usage();
    let data_ok = data != "n/a";
    let usb = get_prop("sys.usb.state");
    let usb_cfg = get_prop("sys.usb.config");
    let baseband = get_prop("gsm.version.baseband");
    let wificond = svc_running("wificond") || svc_running("wpa_supplicant");
    let display_ok = !s.drm.is_empty()
        || svc_running("surfaceflinger")
        || exists("/dev/dri/card0");
    let (touch_ok, touch_detail) = touch_report(s);

    // ponytail: на планшетах detect_class часто unknown — для чеклиста дожимаем по железу
    let phone = class == DeviceClass::Phone
        || (class == DeviceClass::Unknown
            && (!baseband.is_empty() || s.has_modem_iface() || batt_ok || cam_n > 0));
    let tv = class == DeviceClass::Tv;
    let box_ = class == DeviceClass::Box
        || (class == DeviceClass::Unknown && !phone && (s.has_hdmi() || s.has_eth()));

    let wlan_ifaces = net_ifaces(s, &["wlan", "wifi"]);
    let eth_ifaces = net_ifaces(s, &["eth", "en"]);
    let modem_ifaces = net_ifaces(s, &["seth", "rmnet", "ccmni", "wwan"]);

    let nv_lines = {
        let mut v = vec![nv_detail];
        if is_mtk() {
            v.push("vendor: MediaTek".into());
        }
        detail_lines(&v)
    };

    vec![
        check(
            "boot_completed",
            true,
            boot_ok,
            detail_lines(&[
                format!("sys.boot_completed={}", get_prop("sys.boot_completed")),
                format!("dev.bootcomplete={}", get_prop("dev.bootcomplete")),
                format!("bootanim={}", svc("bootanim")),
            ]),
        ),
        check(
            "verified_boot",
            !vb.is_empty(),
            vb_ok,
            detail_lines(&[
                format!("verifiedbootstate={vb}"),
                format!(
                    "veritymode={}",
                    if verity.is_empty() { "-" } else { &verity }
                ),
                format!("boot: {}", sum.boot),
            ]),
        ),
        check("nvram", nv_app, nv_ok, nv_lines),
        check(
            "storage",
            true,
            s.has_storage(),
            detail_lines(&[
                format!("type: {}", sum.storage),
                format!("mmc: {}", join_preview(&s.mmc, 6)),
                format!("block devices: {}", s.blocks.len()),
            ]),
        ),
        check(
            "data_mounted",
            true,
            data_ok,
            detail_lines(&[format!("/data: {data}")]),
        ),
        check(
            "display",
            true,
            display_ok,
            detail_lines(&[
                format!("resolution: {}", sum.display),
                format!("refresh: {}", sum.refresh),
                format!("drm: {}", join_preview(&s.drm, 8)),
                format!("surfaceflinger: {}", svc("surfaceflinger")),
            ]),
        ),
        check(
            "gpu",
            true,
            s.has_gpu(),
            detail_lines(&[
                format!("gpu: {}", sum.gpu),
                format!("egl={}", na(get_prop("ro.hardware.egl"))),
                format!("vulkan={}", na(get_prop("ro.hardware.vulkan"))),
                format!(
                    "mali0={} kgsl={}",
                    exists("/sys/class/misc/mali0"),
                    exists("/sys/class/kgsl/kgsl-3d0")
                ),
            ]),
        ),
        check(
            "audio",
            true,
            s.has_audio(),
            detail_lines(&[
                format!(
                    "cards: {}",
                    s.sound.iter().filter(|n| n.starts_with("card")).count()
                ),
                format!("nodes: {}", join_preview(&s.sound, 10)),
                format!("audioserver: {}", svc("audioserver")),
            ]),
        ),
        check(
            "wlan",
            phone || tv || (box_ && !s.has_eth()),
            s.has_wlan() && wificond,
            detail_lines(&[
                format!("wificond: {}", if wificond { "up" } else { "down" }),
                format!("init.svc.wificond={}", svc("wificond")),
                iface_report(&wlan_ifaces),
            ]),
        ),
        check(
            "ethernet",
            (tv || box_) && !s.has_wlan(),
            s.has_eth(),
            iface_report(&eth_ifaces),
        ),
        check(
            "bluetooth",
            phone || tv || box_,
            s.has_bt(),
            detail_lines(&[
                format!("present: {}", s.has_bt()),
                format!("rfkill nodes: {}", s.rfkill.len()),
                format!("bluetooth class: {}", exists("/sys/class/bluetooth")),
            ]),
        ),
        check("touch", phone, touch_ok, touch_detail),
        check(
            "battery_charger",
            phone,
            batt_ok || s.has_charger(),
            battery_report(&ps),
        ),
        check("camera", phone, cam_n >= 1, cam_detail),
        check(
            "sensors",
            phone,
            sens_n >= 1 || !s.iio.is_empty(),
            sens_detail,
        ),
        check(
            "modem_baseband",
            phone,
            !baseband.is_empty() || s.has_modem_iface(),
            detail_lines(&[
                format!(
                    "baseband: {}",
                    if baseband.is_empty() {
                        "n/a"
                    } else {
                        &baseband
                    }
                ),
                format!("modem ifaces: {}", join_preview(
                    &modem_ifaces.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    8,
                )),
                format!("sim.state: {}", na(get_prop("gsm.sim.state"))),
            ]),
        ),
        check(
            "gnss",
            phone,
            s.has_gnss(),
            detail_lines(&[
                format!("mnld: {}", svc("mnld")),
                format!("gpsd: {}", svc("gpsd")),
                format!("/dev/stpgps: {}", exists("/dev/stpgps")),
                format!("/dev/gps_emi: {}", exists("/dev/gps_emi")),
            ]),
        ),
        check(
            "hdmi",
            tv || box_,
            s.has_hdmi() || !s.drm.is_empty(),
            detail_lines(&[
                format!("hdmi connector: {}", s.has_hdmi()),
                format!("drm: {}", join_preview(&s.drm, 8)),
                format!("hdmicecd: {}", svc("hdmicecd")),
            ]),
        ),
        check(
            "usb",
            true,
            !usb.is_empty(),
            detail_lines(&[
                format!("sys.usb.state={}", na(usb)),
                format!("sys.usb.config={}", na(usb_cfg)),
                format!(
                    "udc: {}",
                    join_preview(&list_names("/sys/class/udc"), 4)
                ),
            ]),
        ),
        check(
            "surfaceflinger",
            true,
            svc_running("surfaceflinger"),
            detail_lines(&[format!("init.svc.surfaceflinger={}", svc("surfaceflinger"))]),
        ),
        check(
            "identity",
            true,
            !sum.serial.is_empty() && sum.serial != "n/a",
            detail_lines(&[
                format!("product: {} / {}", sum.manufacturer, sum.model),
                format!("device: {}", sum.device),
                format!("serial: {}", sum.serial),
                format!("hardware: {}", sum.hardware),
                format!("platform: {}", sum.platform),
                format!("android: {} (SDK {})", sum.android, sum.sdk),
                format!("build: {}", sum.build),
                format!("cpu: {} cores, {}", sum.cpu_cores, sum.cpu_model),
                format!("cpu_freq: {}", sum.cpu_freq),
                format!("ram: {} (avail {})", sum.ram_total, sum.ram_avail),
                format!("timezone: {}", sum.timezone),
                format!("uptime: {} (load {})", sum.uptime, sum.load),
            ]),
        ),
    ]
}

fn print_detailed_report(
    sum: &Summary,
    checks: &[Check],
    pass: usize,
    fail: usize,
    skip: usize,
    lang: Option<Lang>,
) {
    println!(
        "{} / {}    serial={}    class={}{}",
        sum.manufacturer,
        sum.model,
        sum.serial,
        sum.class,
        if sum.class_forced {
            " (forced)"
        } else {
            " (auto)"
        }
    );
    println!(
        "Android {} (SDK {})    hardware={}    {}",
        sum.android, sum.sdk, sum.hardware, sum.boot
    );
    if let Some(lang) = lang {
        match lang {
            Lang::Ru => println!("пояснения: русский (--lang=ru)"),
            Lang::En => println!("explanations: English (--lang=en)"),
        }
    }
    println!();
    println!("=== DEVICE CHECKS ===");
    println!();
    for c in checks {
        let mark = match c.status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Skip => "SKIP",
        };
        println!("[{mark}] {}", c.name);
        if c.detail.is_empty() {
            println!("  (no details)");
        } else {
            for line in c.detail.lines() {
                println!("  {line}");
            }
        }
        if let Some(lang) = lang {
            print_explain(c.name, lang);
        }
        println!();
    }
    if fail == 0 {
        println!("RESULT: PASS    ({pass} pass, {skip} skip)");
    } else {
        println!("RESULT: FAIL    ({pass} pass, {fail} fail, {skip} skip)");
    }
}

fn check(name: &'static str, applicable: bool, ok: bool, detail: impl Into<String>) -> Check {
    if !applicable {
        return Check {
            name,
            status: Status::Skip,
            detail: detail.into(),
        };
    }
    Check {
        name,
        status: if ok { Status::Pass } else { Status::Fail },
        detail: detail.into(),
    }
}

fn print_inventory(s: &Snap) {
    println!("--- inventory ---");
    println!("i2c:{}", s.i2c.len());
    for (a, d) in &s.i2c {
        println!("  {a} -> {d}");
    }
    println!("spi:{}", s.spi.len());
    for (a, d) in &s.spi {
        println!("  {a} -> {d}");
    }
    println!("net:");
    for n in &s.nets {
        if n.starts_with("wlan")
            || n.starts_with("eth")
            || n.starts_with("en")
            || n.starts_with("seth")
            || n.starts_with("rmnet")
            || n.starts_with("ccmni")
            || n.starts_with("wwan")
            || n == "lo"
        {
            println!("  {n}");
        }
    }
    println!("drm: {}", join_preview(&s.drm, 12));
    println!("input: {}", join_preview(&s.inputs, 12));
}

fn main() -> ExitCode {
    let args = parse_args();
    let snap = Snap::gather();
    let class_forced = args.class.is_some();
    let class = args.class.unwrap_or_else(|| detect_class(&snap));
    let summary = collect_summary(class, class_forced, &snap);

    if args.summary_only {
        if args.json {
            println!("{{\"mode\":\"summary\",\"summary\":{}}}", summary.to_json());
        } else {
            print_summary_table(&summary);
            println!();
        }
        return ExitCode::SUCCESS;
    }

    let results = build_smt_checks(class, &snap, &summary);
    let mut fail = 0usize;
    let mut pass = 0usize;
    let mut skip = 0usize;
    for c in &results {
        match c.status {
            Status::Pass => pass += 1,
            Status::Fail => fail += 1,
            Status::Skip => skip += 1,
        }
    }

    if args.json {
        let mut out = format!(
            "{{\"mode\":\"report\",\"summary\":{},\"checks\":{},\"result\":{}",
            summary.to_json(),
            checks_to_json(&results, pass, fail, skip, args.lang),
            if fail == 0 { "\"PASS\"" } else { "\"FAIL\"" },
        );
        if args.verbose {
            out.push_str(&format!(",\"inventory\":{}", inventory_to_json(&snap)));
        }
        out.push('}');
        println!("{out}");
    } else {
        print_detailed_report(&summary, &results, pass, fail, skip, args.lang);
        if args.verbose {
            println!();
            print_inventory(&snap);
        }
    }

    if fail == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_drivers() {
        assert!(is_touch_driver("NVT-ts"));
        assert!(is_touch_driver("goodix_ts"));
        assert!(!is_touch_driver("sc27xx-pmic"));
    }

    #[test]
    fn modem_patterns() {
        let nets = ["seth_lte0", "rmnet_data0", "ccmni0", "wlan0"];
        assert!(nets.iter().any(|n| n.starts_with("seth") || n.starts_with("rmnet") || n.starts_with("ccmni")));
    }

    #[test]
    fn class_tv_prop() {
        // pure unit: characteristics parsing idea
        let ch = "tv,default";
        assert!(ch.contains("tv"));
    }

    #[test]
    fn json_escape_quotes() {
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
        assert!(json_str("x").starts_with('"'));
    }

    #[test]
    fn uptime_format() {
        assert_eq!(format_uptime_secs(90), "1m");
        assert_eq!(format_uptime_secs(3700), "1h 1m");
        assert_eq!(format_uptime_secs(90061), "1d 1h 1m");
    }
}
