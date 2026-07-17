# simple-android-info

[English](README.en.md)

Небольшая утилита для Android: показывает, **что за устройство**, и проверяет, **какие интерфейсы реально работают** (дисплей, сеть, тач, камера и т.д.).

Работает на телефонах, Android TV и приставках. Root не нужен.

Готовый бинарник лежит в [`bin/`](bin/) — для обычного использования **собирать проект не обязательно**.

---

## Использование (готовая сборка)

Нужны только `adb` и устройство с USB-отладкой.

```powershell
adb push bin\simple-android-info /data/local/tmp/
adb shell chmod 755 /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info
```

Если после `push` пишет `Permission denied` — выполните `chmod`.

### Режимы

| Команда | Результат |
|---------|-----------|
| `simple-android-info` | сводка + проверка интерфейсов (класс auto) |
| `--summary` / `-s` | только сводная таблица |
| `--class=phone` / `tv` / `box` | полный режим с принудительным классом |
| `--class=summary` | то же, что `--summary` |
| `--json` / `-j` | вывод в JSON |
| `-v` / `--verbose` | плюс список шин (I2C, SPI, сеть…) |

Флаги можно сочетать.

```powershell
adb shell /data/local/tmp/simple-android-info
adb shell /data/local/tmp/simple-android-info --summary
adb shell /data/local/tmp/simple-android-info --summary --json
adb shell /data/local/tmp/simple-android-info --class=tv
adb shell /data/local/tmp/simple-android-info --json -v
```

### Код выхода

| Код | Когда |
|-----|--------|
| `0` | всё ок (в `--summary` всегда `0`) |
| `1` | есть обязательный FAIL в проверках |
| `2` | неверный аргумент |

---

## Как читать вывод

### Сводная таблица (SUMMARY)

| Поле | Простыми словами |
|------|------------------|
| `class` | тип: телефон / TV / приставка |
| `product` | производитель и модель |
| `device` | внутреннее имя |
| `android` / `build` | версия системы и сборка |
| `soc` / `platform` | чипсет |
| `cpu` / `cpu_freq` | ядра и частоты по кластерам |
| `ram` / `ddr_*` | оперативка и DDR (если доступно) |
| `gpu` | графика |
| `display` / `refresh` | разрешение и герцовки |
| `storage` | накопитель (eMMC / UFS) |
| `network` | Wi‑Fi, Ethernet, модем, BT |
| `buses` | сколько устройств на I2C / SPI / input |
| `features` | найденные возможности (touch, camera…) |

`n/a` — система не отдала значение (часто без root: тип памяти, частота GPU).

### Проверки интерфейсов (INTERFACE CHECKS)

| Статус | Значение |
|--------|----------|
| **PASS** | нашлось, выглядит живым |
| **FAIL** | для этого типа ожидалось, но нет → код выхода `1` |
| **SKIP** | для этого типа не обязательно |

| Проверка | Телефон | TV / приставка |
|----------|:-------:|:--------------:|
| Накопитель, дисплей, GPU, звук, сеть, BT, SurfaceFlinger | да | да |
| Панель DSI, тач, модем, камера, зарядка, GNSS | да | нет |
| HDMI / внешний дисплей | нет | да |
| Ethernet | нет | да, если нет Wi‑Fi |

Строки `info_*` на FAIL не влияют.  
В конце: `summary: N pass, M fail, K skip`.  
С `-v` печатается инвентарь шин.

### JSON

```powershell
adb shell /data/local/tmp/simple-android-info --summary --json > info.json
```

Полный прогон отдаёт объект с полями `mode`, `summary`, `checks` (и `inventory` при `-v`).  
`status` в checks: `pass` | `fail` | `skip`.

### Класс устройства (auto)

1. Похоже на TV → `tv`
2. Есть модем → `phone`
3. Нет модема, есть сеть / HDMI → `box`
4. Иначе → `unknown`

---

## Сборка из исходников

Нужны: Rust, Android NDK, `adb` (для проверки на устройстве).

```powershell
rustup target add aarch64-linux-android
```

В [`.cargo/config.toml`](.cargo/config.toml) укажите linker из своего NDK:

```toml
[target.aarch64-linux-android]
linker = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\aarch64-linux-android24-clang.cmd"
ar     = "C:\\path\\to\\ndk\\toolchains\\llvm\\prebuilt\\windows-x86_64\\bin\\llvm-ar.exe"
```

Сборка и обновление `bin/`:

```powershell
cargo build --release --target aarch64-linux-android
New-Item -ItemType Directory -Force bin | Out-Null
Copy-Item -Force target\aarch64-linux-android\release\simple-android-info bin\simple-android-info
```

После этого используйте бинарник из `bin/`, как в разделе «Использование».

Каталог `target/` в git не хранится — только исходники и готовый файл в `bin/`.

---

## Ограничения

- Нужен загруженный Android и `adb`.
- Без root часть данных закрыта (часто частота GPU, тип DDR).
- Имена узлов зависят от вендора (типичные Unisoc / Qualcomm / MediaTek покрыты).

```text
simple-android-info --help
```
