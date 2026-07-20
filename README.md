# simple-android-info

[English](README.en.md)

Утилита для Android (телефон / TV / приставка): **подробный отчёт bring-up** после пайки или прошивки — что поднялось, что нет. Root не нужен.

Готовые бинарники в [`bin/`](bin/) — для обычного использования **собирать не обязательно**.

| Файл | ABI | Устройства |
|------|-----|------------|
| [`bin/simple-android-info-aarch64`](bin/simple-android-info-aarch64) | `arm64-v8a` | большинство телефонов / TV (64-bit) |
| [`bin/simple-android-info-armeabi-v7a`](bin/simple-android-info-armeabi-v7a) | `armeabi-v7a` | старые / 32-bit приставки |

```powershell
adb shell getprop ro.product.cpu.abi
```

---

## Использование

Нужны `adb` и USB-отладка. Запуск только из `/data/local/tmp/` (`/sdcard` и `/storage` часто `noexec`).

```powershell
# 64-bit
adb push bin\simple-android-info-aarch64 /data/local/tmp/simple-android-info
# 32-bit
# adb push bin\simple-android-info-armeabi-v7a /data/local/tmp/simple-android-info

adb shell chmod 755 /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info
```

Если после `push` — `Permission denied`, сделайте `chmod`.  
Если файл есть в `ls`, но shell пишет `No such file or directory` — **неверный ABI** (нет `linker64` на 32-bit).

### Режимы

| Команда | Результат |
|---------|-----------|
| `simple-android-info` | подробный отчёт по каждому пункту |
| `--lang=ru` / `--lang=en` | то же + пояснения (что / где / зачем); синоним `--explain=` |
| `--summary` / `-s` | только сводная таблица |
| `--smt` | алиас основного режима |
| `--class=phone` / `tv` / `box` | принудительный класс |
| `--class=summary` | то же, что `--summary` |
| `--json` / `-j` | JSON |
| `-v` / `--verbose` | плюс инвентарь шин (I2C, SPI, net…) |

```powershell
adb shell /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info --lang=ru
adb shell /data/local/tmp/simple-android-info --lang=en
adb shell /data/local/tmp/simple-android-info --summary
adb shell /data/local/tmp/simple-android-info --json --lang=ru
adb shell /data/local/tmp/simple-android-info --class=tv
```

### Код выхода

| Код | Когда |
|-----|--------|
| `0` | нет обязательных FAIL (`--summary` всегда `0`) |
| `1` | есть FAIL |
| `2` | неверный аргумент |

---

## Как читать вывод

По умолчанию печатается шапка (модель, serial, Android) и блоки:

```text
[PASS|FAIL|SKIP] <имя>
  …факты…
  ———                    # только с --lang=
  что / где / зачем      # или what / where / why
```

В конце: `RESULT: PASS|FAIL`.

### Пункты DEVICE CHECKS

| Пункт | Смысл |
|-------|--------|
| `boot_completed` | Android userspace загрузился |
| `verified_boot` | AVB / vbmeta (green/yellow/…) |
| `nvram` | заводская NVRAM / калибровки Ready (MTK и аналоги) |
| `storage` | eMMC/UFS виден ядру |
| `data_mounted` | `/data` смонтирован, есть размер |
| `display` | DRM + разрешение + SurfaceFlinger |
| `gpu` | EGL/GPU |
| `audio` | ALSA card / audioserver |
| `wlan` | интерфейс + wificond |
| `ethernet` | eth (TV/box, если нет Wi‑Fi) |
| `bluetooth` | BT контроллер |
| `touch` | тач (phone) |
| `battery_charger` | power_supply / зарядка (phone) |
| `camera` | число камер HAL |
| `sensors` | SensorService / IIO |
| `modem_baseband` | baseband + modem iface |
| `gnss` | GPS/mnld /dev |
| `hdmi` | HDMI/DRM (TV/box) |
| `usb` | gadget ADB/MTP + UDC |
| `surfaceflinger` | композитор |
| `identity` | serial, SoC, CPU, RAM, uptime… |

**SKIP** — пункт не обязателен для этого класса устройства.  
Заводской Wi‑Fi MAC обычно недоступен без root — поле `wlan_mac` часто `n/a`.

### Сводная таблица (`--summary`)

| Поле | Смысл |
|------|--------|
| `class` | phone / tv / box / unknown |
| `product` / `device` / `serial` / `hardware` | идентификация |
| `android` / `build` / `boot` | ОС, сборка, slot/vbmeta |
| `soc` / `platform` / `cpu*` / `ram` | железо |
| `gpu` / `display` / `refresh` | графика и экран |
| `storage` / `data` | накопитель и `/data` |
| `network` / `wlan_mac` | сети (MAC часто закрыт) |
| `timezone` / `uptime` | пояс и аптайм |
| `buses` / `features` | шины и возможности |

### JSON

```powershell
adb shell /data/local/tmp/simple-android-info --json --lang=ru > report.json
adb shell /data/local/tmp/simple-android-info --summary --json > summary.json
```

Полный прогон: `mode` (`report`), `summary`, `checks`, `result` (`PASS`/`FAIL`); при `-v` — `inventory`.  
В `checks.items[]`: `name`, `status`, `detail`; при `--lang=` ещё `explain.{what,where,why}`.

### Класс устройства (auto)

1. Похоже на TV → `tv`
2. Есть модем / telephony → `phone`
3. Нет модема, есть сеть / HDMI → `box`
4. Иначе → `unknown` (чеклист всё равно дожимает phone-профиль по железу)

---

## Сборка из исходников

Нужны: Rust, Android NDK, `adb`.

```powershell
rustup target add aarch64-linux-android armv7-linux-androideabi
```

В [`.cargo/config.toml`](.cargo/config.toml) укажите linker из своего NDK:

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

`target/` в git не хранится — только исходники и файлы в `bin/`.

---

## Ограничения

- Нужен загруженный Android и `adb`.
- Без root закрыты часть sysfs (часто Wi‑Fi MAC, частота GPU).
- Имена узлов зависят от вендора (MediaTek / Amlogic / типичные паттерны покрыты).

```text
simple-android-info --help
```
