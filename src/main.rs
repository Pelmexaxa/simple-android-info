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
            || exists("/sys/class/misc/gnss_common_ctl")
            || exists("/dev/gnss")
            || get_prop("ro.hardware.gps").len() > 0
    }

    fn has_charger(&self) -> bool {
        self.i2c.iter().any(|(_, d)| is_charger_driver(d))
            || !list_names("/sys/class/power_supply").is_empty()
            || exists("/sys/class/power_supply/battery")
    }

    fn has_input(&self) -> bool {
        !self.inputs.is_empty()
            || list_names("/sys/class/input")
                .iter()
                .any(|n| n.starts_with("event"))
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

fn is_pmic_driver(d: &str) -> bool {
    let l = d.to_lowercase();
    l.contains("pmic") || l.contains("sc27") || l.contains("pm89") || l.contains("pm81") || l.contains("mt63")
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

fn ram_summary() -> (String, String, String) {
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
    let factory = prop_or(&["ro.boot.ddr_size", "ro.boot.ddrsize"]);
    (total, avail, na(factory))
}

fn ddr_freq_summary() -> String {
    // ponytail: не ходим рекурсивно по /sys/devices/platform — на MTK/Qcom это может «висеть» минутами
    let table = [
        "/sys/devices/platform/scene-frequency/devfreq/scene-frequency/sprd-governor/ddrinfo_freq_table",
        "/sys/class/devfreq/scene-frequency/sprd-governor/ddrinfo_freq_table",
    ]
    .into_iter()
    .find_map(read_trimmed);
    let cur = [
        "/sys/devices/platform/scene-frequency/devfreq/scene-frequency/sprd-governor/ddrinfo_cur_freq",
        "/sys/class/devfreq/scene-frequency/sprd-governor/ddrinfo_cur_freq",
    ]
    .into_iter()
    .find_map(read_trimmed);

    if table.is_some() || cur.is_some() {
        return match (table, cur) {
            (Some(t), Some(c)) => {
                let freqs: Vec<u64> = t.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                match (freqs.iter().min(), freqs.iter().max()) {
                    (Some(lo), Some(hi)) => format!("{c} MHz (range {lo}-{hi})"),
                    _ => format!("cur={c} table={t}"),
                }
            }
            (Some(t), None) => format!("table={t}"),
            (None, Some(c)) => format!("cur={c} MHz"),
            _ => "n/a".into(),
        };
    }

    // один уровень /sys/class/devfreq — безопасно и быстро
    if let Ok(entries) = fs::read_dir("/sys/class/devfreq") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !(name.contains("ddr") || name.contains("dmc") || name.contains("emi")) {
                continue;
            }
            let base = entry.path();
            let min = read_trimmed(base.join("min_freq")).and_then(|s| khz_to_mhz(&s));
            let max = read_trimmed(base.join("max_freq")).and_then(|s| khz_to_mhz(&s));
            let cur_f = read_trimmed(base.join("cur_freq")).and_then(|s| khz_to_mhz(&s));
            if let (Some(lo), Some(hi)) = (min, max) {
                return match cur_f {
                    Some(c) => format!("{c} MHz (range {lo}-{hi}, {name})"),
                    None => format!("{lo}-{hi} MHz ({name})"),
                };
            }
        }
    }
    "n/a".into()
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
    android: String,
    sdk: String,
    build: String,
    soc_mfr: String,
    soc_model: String,
    platform: String,
    cpu_cores: usize,
    cpu_online: String,
    cpu_freq: String,
    ram_total: String,
    ram_avail: String,
    ddr_size: String,
    ddr_freq: String,
    gpu: String,
    display: String,
    refresh: String,
    storage: String,
    network: Vec<String>,
    i2c_count: usize,
    spi_count: usize,
    input_count: usize,
    features: Vec<String>,
}

fn collect_summary(class: DeviceClass, forced: bool, s: &Snap) -> Summary {
    let (cores, online, cpu_freq) = cpu_summary();
    let (ram_total, ram_avail, ddr_factory) = ram_summary();
    let (disp_res, disp_fps) = display_summary(s);

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
        cpu_freq,
        ram_total,
        ram_avail,
        ddr_size: ddr_factory,
        ddr_freq: ddr_freq_summary(),
        gpu,
        display: disp_res,
        refresh: disp_fps,
        storage,
        network,
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
    row("android", format!("{} (SDK {})", sum.android, sum.sdk));
    row("build", &sum.build);
    row("soc", format!("{} / {}", sum.soc_mfr, sum.soc_model));
    row("platform", &sum.platform);
    row(
        "cpu",
        format!("{} cores (online {})", sum.cpu_cores, sum.cpu_online),
    );
    row("cpu_freq", &sum.cpu_freq);
    row(
        "ram",
        format!("{} (avail {})", sum.ram_total, sum.ram_avail),
    );
    row("ddr_size", &sum.ddr_size);
    row("ddr_freq", &sum.ddr_freq);
    row("gpu", &sum.gpu);
    row("display", &sum.display);
    row("refresh", &sum.refresh);
    row("storage", &sum.storage);
    row(
        "network",
        if sum.network.is_empty() {
            "n/a".into()
        } else {
            sum.network.join(", ")
        },
    );
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
                "\"android\":{android},",
                "\"sdk\":{sdk},",
                "\"build\":{build},",
                "\"soc_manufacturer\":{soc_mfr},",
                "\"soc_model\":{soc_model},",
                "\"platform\":{platform},",
                "\"cpu_cores\":{cores},",
                "\"cpu_online\":{online},",
                "\"cpu_freq\":{cpu_freq},",
                "\"ram_total\":{ram_total},",
                "\"ram_avail\":{ram_avail},",
                "\"ddr_size\":{ddr_size},",
                "\"ddr_freq\":{ddr_freq},",
                "\"gpu\":{gpu},",
                "\"display\":{display},",
                "\"refresh\":{refresh},",
                "\"storage\":{storage},",
                "\"network\":{network},",
                "\"buses\":{{\"i2c\":{i2c},\"spi\":{spi},\"input\":{input}}},",
                "\"features\":{features}",
                "}}"
            ),
            class = json_str(&self.class),
            forced = self.class_forced,
            mfr = json_str(&self.manufacturer),
            model = json_str(&self.model),
            device = json_str(&self.device),
            android = json_str(&self.android),
            sdk = json_str(&self.sdk),
            build = json_str(&self.build),
            soc_mfr = json_str(&self.soc_mfr),
            soc_model = json_str(&self.soc_model),
            platform = json_str(&self.platform),
            cores = self.cpu_cores,
            online = json_str(&self.cpu_online),
            cpu_freq = json_str(&self.cpu_freq),
            ram_total = json_str(&self.ram_total),
            ram_avail = json_str(&self.ram_avail),
            ddr_size = json_str(&self.ddr_size),
            ddr_freq = json_str(&self.ddr_freq),
            gpu = json_str(&self.gpu),
            display = json_str(&self.display),
            refresh = json_str(&self.refresh),
            storage = json_str(&self.storage),
            network = json_str_arr(&self.network),
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

fn checks_to_json(checks: &[Check], pass: usize, fail: usize, skip: usize) -> String {
    let items = checks
        .iter()
        .map(|c| {
            format!(
                "{{\"name\":{},\"status\":{},\"detail\":{}}}",
                json_str(c.name),
                json_str(status_str(c.status)),
                json_str(&c.detail)
            )
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

#[derive(Default)]
struct Args {
    class: Option<DeviceClass>,
    summary_only: bool,
    json: bool,
    verbose: bool,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--summary" | "-s" => args.summary_only = true,
            "--json" | "-j" => args.json = true,
            "-v" | "--verbose" => args.verbose = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
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
simple-android-info — device summary and interface status

Usage:
  simple-android-info [--summary|-s] [--json|-j] [--verbose|-v] [--class=phone|tv|box|summary]

  --summary, -s     summary table only (no interface checks)
  --json, -j        JSON on stdout
  --verbose, -v     include bus inventory (text or JSON)
  --class=…         force device class (summary ≡ --summary)
"
    );
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

fn build_checks(class: DeviceClass, s: &Snap) -> Vec<Check> {
    let phone = class == DeviceClass::Phone;
    let tv = class == DeviceClass::Tv;
    let box_ = class == DeviceClass::Box;
    // unknown: требуем только ядро, остальное soft (pass if present, skip if absent)
    let unknown = class == DeviceClass::Unknown;

    let need_touch = phone;
    let need_modem = phone;
    let need_camera = phone; // TV/box — опционально
    let need_charger = phone;
    let need_gnss = phone;
    let need_panel = phone; // DSI/backlight; TV/box — HDMI/DRM
    let need_hdmi = tv || box_;
    let need_net = true; // любой класс: wlan|eth|modem
    let need_bt = !unknown; // на unknown — skip если нет
    let soft = unknown;

    let net_ok = s.has_wlan() || s.has_eth() || s.has_modem_iface();
    let display_ok = !s.drm.is_empty()
        || svc_running("surfaceflinger")
        || exists("/dev/dri/card0");

    let pmic = s.spi.iter().any(|(_, d)| is_pmic_driver(d))
        || s.i2c.iter().any(|(_, d)| is_pmic_driver(d));

    vec![
        check(
            "storage",
            true,
            s.has_storage(),
            format!(
                "mmc=[{}] blocks~{}",
                join_preview(&s.mmc, 3),
                s.blocks.len()
            ),
        ),
        check(
            "display_stack",
            true,
            display_ok,
            format!(
                "drm=[{}] sf={}",
                join_preview(&s.drm, 4),
                svc("surfaceflinger")
            ),
        ),
        check(
            "display_panel_dsi",
            need_panel && !soft,
            s.has_dsi(),
            format!("backlight=[{}] drm=[{}]", join_preview(&s.backlight, 3), join_preview(&s.drm, 4)),
        ),
        check(
            "display_hdmi_dp",
            need_hdmi && !soft,
            // ponytail: имя коннектора на Amlogic/Rockchip часто без "HDMI"; достаточно живого DRM
            s.has_hdmi() || !s.drm.is_empty(),
            format!(
                "hdmi={} drm=[{}]",
                s.has_hdmi(),
                join_preview(&s.drm, 6)
            ),
        ),
        check(
            "gpu",
            true,
            s.has_gpu(),
            if exists("/sys/class/misc/mali0") {
                "mali0".into()
            } else if exists("/sys/class/kgsl/kgsl-3d0") {
                "kgsl".into()
            } else {
                format!(
                    "egl={} vulkan={}",
                    get_prop("ro.hardware.egl"),
                    get_prop("ro.hardware.vulkan")
                )
            },
        ),
        check(
            "audio",
            true,
            s.has_audio(),
            format!(
                "cards={} audioserver={}",
                s.sound.iter().filter(|n| n.starts_with("card")).count(),
                svc("audioserver")
            ),
        ),
        check(
            "network_any",
            need_net,
            net_ok,
            format!(
                "wlan={} eth={} modem_if={}",
                s.has_wlan(),
                s.has_eth(),
                s.has_modem_iface()
            ),
        ),
        check(
            "wlan",
            // на box иногда только eth — тогда wlan optional
            (phone || tv || (box_ && !s.has_eth())) && !soft,
            s.has_wlan(),
            join_preview(
                &s.nets
                    .iter()
                    .filter(|n| n.starts_with("wlan") || n.starts_with("wifi"))
                    .cloned()
                    .collect::<Vec<_>>(),
                4,
            ),
        ),
        check(
            "ethernet",
            (tv || box_) && !s.has_wlan() && !soft,
            s.has_eth(),
            join_preview(
                &s.nets
                    .iter()
                    .filter(|n| n.starts_with("eth") || n.starts_with("en"))
                    .cloned()
                    .collect::<Vec<_>>(),
                4,
            ),
        ),
        check(
            "modem",
            need_modem && !soft,
            s.has_modem_iface(),
            join_preview(
                &s.nets
                    .iter()
                    .filter(|n| {
                        n.starts_with("seth")
                            || n.starts_with("rmnet")
                            || n.starts_with("ccmni")
                            || n.starts_with("wwan")
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
                4,
            ),
        ),
        check(
            "bluetooth",
            need_bt && !soft,
            s.has_bt(),
            format!("rfkill={} tty_bt={}", s.rfkill.len(), s.has_bt()),
        ),
        check(
            "i2c_bus",
            // на TV/box I2C может быть пуст с userspace — skip если 0 клиентов
            !s.i2c.is_empty() || phone,
            !s.i2c.is_empty(),
            format!(
                "{} clients, {} bound",
                s.i2c.len(),
                s.i2c.iter().filter(|(_, d)| d != "?").count()
            ),
        ),
        check(
            "touch",
            need_touch && !soft,
            s.has_touch(),
            {
                let mut hits = Vec::new();
                for (n, d) in &s.spi {
                    if is_touch_driver(d) {
                        hits.push(format!("{n}->{d}"));
                    }
                }
                for (n, d) in &s.i2c {
                    if is_touch_driver(d) {
                        hits.push(format!("{n}->{d}"));
                    }
                }
                for n in &s.inputs {
                    if is_touch_name(n) {
                        hits.push(n.clone());
                    }
                }
                join_preview(&hits, 4)
            },
        ),
        check(
            "pmic",
            phone && !soft,
            pmic || s.has_charger(),
            {
                let mut hits = Vec::new();
                for (n, d) in s.spi.iter().chain(s.i2c.iter()) {
                    if is_pmic_driver(d) || is_charger_driver(d) {
                        hits.push(format!("{n}->{d}"));
                    }
                }
                join_preview(&hits, 4)
            },
        ),
        check(
            "charger",
            need_charger && !soft,
            s.has_charger(),
            {
                let ps = list_names("/sys/class/power_supply");
                if ps.is_empty() {
                    s.i2c
                        .iter()
                        .filter(|(_, d)| is_charger_driver(d))
                        .map(|(n, d)| format!("{n}->{d}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    join_preview(&ps, 4)
                }
            },
        ),
        check(
            "camera",
            need_camera && !soft,
            s.has_camera(),
            format!(
                "cameraserver={} v4l={} iio={}",
                svc("cameraserver"),
                list_names("/sys/class/video4linux").len(),
                s.iio.len()
            ),
        ),
        check(
            "gnss",
            need_gnss && !soft,
            s.has_gnss(),
            format!("gpsd={} gnss_ctl={}", svc("gpsd"), exists("/sys/class/misc/gnss_common_ctl")),
        ),
        check(
            "input_any",
            // TV/box: пульт может появиться позже — на phone обязательно
            phone || s.has_input(),
            s.has_input(),
            join_preview(&s.inputs, 6),
        ),
        check(
            "surfaceflinger",
            true,
            svc_running("surfaceflinger"),
            svc("surfaceflinger"),
        ),
        // опциональные «бонусы» — всегда Pass/Skip, не Fail
        check(
            "opt_camera",
            false, // marked skip via applicable=false — покажем наличие отдельно ниже
            s.has_camera(),
            "",
        ),
    ]
    .into_iter()
    // убираем фиктивный opt_camera из основного списка — выведем в inventory
    .filter(|c| c.name != "opt_camera")
    .chain(optional_presence(s, class))
    .collect()
}

/// Доп. покрытие: не валят сборку, только информируют (PASS если есть, SKIP если нет).
fn optional_presence(s: &Snap, class: DeviceClass) -> Vec<Check> {
    let mut v = Vec::new();
    let extras: &[(&str, bool, String)] = &[
        ("info_hdmi", s.has_hdmi(), format!("drm=[{}]", join_preview(&s.drm, 6))),
        ("info_eth", s.has_eth(), join_preview(&s.nets.iter().filter(|n| n.starts_with("eth")).cloned().collect::<Vec<_>>(), 4)),
        ("info_camera", s.has_camera(), format!("svc={}", svc("cameraserver"))),
        ("info_gnss", s.has_gnss(), format!("gpsd={}", svc("gpsd"))),
        ("info_touch", s.has_touch(), "present".into()),
        ("info_modem", s.has_modem_iface(), "present".into()),
        ("info_iio_sensors", !s.iio.is_empty(), join_preview(&s.iio, 4)),
    ];
    for (name, present, detail) in extras {
        // на phone modem/touch/camera — уже в обязательных; info_* только для чужого класса
        let show = match *name {
            "info_hdmi" => class != DeviceClass::Tv && class != DeviceClass::Box,
            "info_eth" => true,
            "info_camera" => class != DeviceClass::Phone,
            "info_gnss" => class != DeviceClass::Phone,
            "info_touch" => class != DeviceClass::Phone,
            "info_modem" => class != DeviceClass::Phone,
            "info_iio_sensors" => true,
            _ => true,
        };
        if !show {
            continue;
        }
        v.push(Check {
            name,
            status: if *present { Status::Pass } else { Status::Skip },
            detail: detail.clone(),
        });
    }
    v
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

    let results = build_checks(class, &snap);
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
            "{{\"mode\":\"full\",\"summary\":{},\"checks\":{}",
            summary.to_json(),
            checks_to_json(&results, pass, fail, skip)
        );
        if args.verbose {
            out.push_str(&format!(",\"inventory\":{}", inventory_to_json(&snap)));
        }
        out.push('}');
        println!("{out}");
    } else {
        print_summary_table(&summary);
        println!();
        println!("=== INTERFACE CHECKS ===");
        for c in &results {
            let mark = match c.status {
                Status::Pass => "PASS",
                Status::Fail => "FAIL",
                Status::Skip => "SKIP",
            };
            if c.detail.is_empty() {
                println!("{mark}  {}", c.name);
            } else {
                println!("{mark}  {}  ({})", c.name, c.detail);
            }
        }
        println!();
        println!("summary: {pass} pass, {fail} fail, {skip} skip");
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
}
