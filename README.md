# lotus-led

Rust library and CLI for controlling **BLEDOM / ELK-BLEDOM / Lotus Lantern** LED strips over Bluetooth LE.

Cross-compiles to a Windows `.exe` from WSL2 / Linux using MinGW.

## Features

| Category | Modes |
|---|---|
| **Hardware** | jump_7_color, fade_7_color, strobe_*, cross_*, mic_hw |
| **Software** | static, pulse, rainbow, wave, fire, meteor, comet |
| **Transitions** | sunrise, sunset, sleep_timer, alarm, notify |
| **Reactive** | audio (FFT → RGB), music (beat detection), ambient (Ambilight), system (CPU/RAM heatmap) |
| **Scenes** | movie, party, romance, relax, focus, gaming, chill |

## Quick start

```bash
# Scan for devices
led scan

# One-shot commands
led on
led color ff8020
led brightness 70
led speed 50

# Modes (Ctrl-C to stop, or --run N seconds)
led mode rainbow
led mode pulse --color 255 0 128 --period 4
led mode music --run 60
led mode ambient

# Scenes
led scene party

# List all modes
led modes
```

## Building

**Linux / WSL2 → Windows EXE:**

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64

cargo build --release --target x86_64-pc-windows-gnu
# Binary: target/x86_64-pc-windows-gnu/release/led.exe
```

**Linux binary:**

```bash
cargo build --release --features full
```

### Feature flags

| Flag | Adds |
|---|---|
| `ble` *(default)* | BLE via btleplug |
| `audio` | Mic / loopback FFT reactive modes (cpal) |
| `ambient` | Ambilight screen capture (screenshots) |
| `system` | CPU / RAM heatmap (sysinfo) |
| `full` | All of the above |

## Configuration

On first run, `led init-config` writes a `config.json` next to the binary:

```jsonc
{
  "device": {
    "mac": "",          // leave empty for auto-discovery
    "scan_timeout_secs": 6
  }
}
```

`config.json` is gitignored — never commit your MAC address.

## Protocol

BLEDOM 9-byte BLE packets over service `FFF0`, write characteristic `FFF3`, notify `FFF4`.

```
7E [cmd] [sub] [d0] [d1] [d2] [d3] [d4] EF
```

## License

MIT
