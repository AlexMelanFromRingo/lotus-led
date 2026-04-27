/// Mode system: enum of all supported lighting modes + async runner.
pub mod software;
pub mod hardware;

#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "ambient")]
pub mod ambient;
#[cfg(feature = "system")]
pub mod system_mon;

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::protocol::HWMode;
#[cfg(feature = "ble")]
use crate::device::BLEDOMDevice;

// ── Mode configuration enum ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModeConfig {
    // ── Static / simple ───────────────────────────────────────────────────────
    Static   { r: u8, g: u8, b: u8 },
    Pulse    { r: u8, g: u8, b: u8, period_secs: f32, min_bright: u8, max_bright: u8, fps: u8 },
    Rainbow  { cycle_secs: f32, saturation: f32, value: f32, fps: u8 },
    Wave     { cycle_secs: f32, saturation: f32, value: f32, fps: u8 },
    Fire     { fps: u8, intensity: f32 },
    Meteor   { r: u8, g: u8, b: u8, fps: u8 },
    Comet    { fps: u8 },

    // ── Timed sequences ───────────────────────────────────────────────────────
    Sunrise    { duration_secs: u32, fps: u8 },
    Sunset     { duration_secs: u32, fps: u8 },
    Cct        { kelvin: u32, brightness: u8 },
    SleepTimer { duration_secs: u32, fps: u8 },
    Alarm      { r: u8, g: u8, b: u8, flash_count: u32, flash_ms: u64 },
    WakeUp     { duration_secs: u32, fps: u8 },

    // ── Hardware (device-native) ───────────────────────────────────────────────
    Hardware     { mode: HWMode, speed: u8, brightness: Option<u8> },
    MicHardware  { sensitivity: u8 },

    // ── Reactive (optional features) ─────────────────────────────────────────
    Audio   { source: AudioSource, sensitivity: f32, fps: u8,
              lo_r: u8, lo_g: u8, lo_b: u8,
              mid_r: u8, mid_g: u8, mid_b: u8,
              hi_r: u8, hi_g: u8, hi_b: u8 },
    Music   { source: AudioSource, sensitivity: f32, fps: u8,
              beat_r: u8, beat_g: u8, beat_b: u8,
              idle_r: u8, idle_g: u8, idle_b: u8 },
    Ambient { fps: u8, saturation_boost: f32, smoothing: f32 },
    SysMonitor { metric: SysMetric, fps: u8,
                 lo_r: u8, lo_g: u8, lo_b: u8,
                 hi_r: u8, hi_g: u8, hi_b: u8 },

    Notification { r: u8, g: u8, b: u8, count: u32, duration_ms: u64 },

    /// User-defined sequence of packets with per-step timing.
    Sequence { steps: Vec<SequenceStep>, loop_forever: bool },

    /// Per-process color rules (matches against running process names).
    AppWatch {
        /// Map of process name substring → (r, g, b)
        rules: Vec<AppWatchRule>,
        default_r: u8, default_g: u8, default_b: u8,
        check_ms: u64,
    },

    /// Auto-detect game processes → rainbow mode.
    #[cfg(feature = "system")]
    Game {
        keywords:       Vec<String>,
        check_secs:     f32,
        rainbow_fps:    u8,
        cycle_secs:     f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceStep {
    /// Duration to hold this step (seconds)
    pub duration_secs: f32,
    /// Optional RGB color
    pub color: Option<[u8; 3]>,
    /// Optional brightness 0–100
    pub brightness: Option<u8>,
    /// Optional hardware mode + speed
    pub hw_mode: Option<HWMode>,
    pub hw_speed: Option<u8>,
    /// Raw 9-byte packet as array
    pub raw: Option<[u8; 9]>,
    /// If true, send power_off packet
    pub off: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppWatchRule {
    pub process: String,       // substring match against process name
    pub r: u8, pub g: u8, pub b: u8,
    pub brightness: Option<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource { Microphone, Loopback }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SysMetric { Cpu, Ram }

// ── Default constructors ──────────────────────────────────────────────────────

impl ModeConfig {
    pub fn pulse_default() -> Self {
        Self::Pulse { r: 255, g: 100, b: 30, period_secs: 3.0, min_bright: 5, max_bright: 100, fps: 20 }
    }
    pub fn rainbow_default() -> Self {
        Self::Rainbow { cycle_secs: 10.0, saturation: 1.0, value: 1.0, fps: 20 }
    }
    pub fn fire_default() -> Self {
        Self::Fire { fps: 15, intensity: 0.85 }
    }
    pub fn from_name(name: &str, cfg: &crate::config::Config) -> Option<Self> {
        // Try hardware mode by name first
        if let Some(hw) = HWMode::from_name(name) {
            return Some(Self::Hardware { mode: hw, speed: cfg.defaults.speed, brightness: None });
        }
        match name.to_lowercase().as_str() {
            "static"       => Some(Self::Static { r: cfg.defaults.color[0], g: cfg.defaults.color[1], b: cfg.defaults.color[2] }),
            "pulse"|"breathe" => Some(Self::pulse_default()),
            "rainbow"      => Some(Self::rainbow_default()),
            "wave"         => Some(Self::Wave { cycle_secs: 5.0, saturation: 1.0, value: 1.0, fps: 20 }),
            "fire"         => Some(Self::fire_default()),
            "meteor"       => Some(Self::Meteor { r: 200, g: 150, b: 255, fps: 20 }),
            "comet"        => Some(Self::Comet { fps: 20 }),
            "sunrise"|"wake" => Some(Self::Sunrise { duration_secs: 1200, fps: 2 }),
            "sunset"       => Some(Self::Sunset { duration_secs: 600, fps: 2 }),
            "cct"          => Some(Self::Cct { kelvin: 4000, brightness: 80 }),
            "sleep"|"sleep_timer" => Some(Self::SleepTimer { duration_secs: 1800, fps: 1 }),
            "alarm"        => Some(Self::Alarm { r: 255, g: 200, b: 50, flash_count: 15, flash_ms: 300 }),
            "hw"|"hardware"=> Some(Self::Hardware { mode: HWMode::Fade7Color, speed: cfg.defaults.speed, brightness: None }),
            "mic_hw"       => Some(Self::MicHardware { sensitivity: 200 }),
            "audio"        => Some(Self::Audio { source: AudioSource::Microphone, sensitivity: 1.0, fps: 25,
                                                  lo_r: 255, lo_g: 0, lo_b: 0, mid_r: 0, mid_g: 255, mid_b: 0,
                                                  hi_r: 0, hi_g: 100, hi_b: 255 }),
            "music"        => Some(Self::Music { source: AudioSource::Loopback, sensitivity: 1.2, fps: 30,
                                                  beat_r: 255, beat_g: 220, beat_b: 0,
                                                  idle_r: 50, idle_g: 0, idle_b: 120 }),
            "ambient"|"ambilight" => Some(Self::Ambient { fps: 10, saturation_boost: 1.4, smoothing: 0.65 }),
            "system"|"cpu" => Some(Self::SysMonitor { metric: SysMetric::Cpu, fps: 2,
                                                        lo_r: 0, lo_g: 200, lo_b: 0, hi_r: 255, hi_g: 0, hi_b: 0 }),
            "notify"|"notification" => Some(Self::Notification { r: 255, g: 230, b: 0, count: 4, duration_ms: 200 }),
            "sequence"     => Some(Self::Sequence { steps: vec![], loop_forever: true }),
            "appwatch"|"app_watch" => Some(Self::AppWatch {
                rules: vec![], default_r: 80, default_g: 80, default_b: 80, check_ms: 1000,
            }),
            #[cfg(feature = "system")]
            "game" => Some(Self::Game {
                keywords:    vec!["steam".into(), "csgo".into(), "valorant".into(),
                                  "minecraft".into(), "overwatch".into()],
                check_secs:  5.0, rainbow_fps: 30, cycle_secs: 3.0,
            }),
            _ => None,
        }
    }
}

// ── Mode runner ───────────────────────────────────────────────────────────────

/// Run `mode` on `device` until `running` is set to `false`.
#[cfg(feature = "ble")]
pub async fn run_mode(
    mode: ModeConfig,
    device: Arc<BLEDOMDevice>,
    running: Arc<AtomicBool>,
) -> Result<()> {
    use ModeConfig::*;
    match mode {
        Static { r, g, b }                      => software::run_static(&device, r, g, b).await,
        Pulse { r, g, b, period_secs, min_bright, max_bright, fps }
                                                 => software::run_pulse(&device, r, g, b, period_secs, min_bright, max_bright, fps, &running).await,
        Rainbow { cycle_secs, saturation, value, fps }
                                                 => software::run_rainbow(&device, cycle_secs, saturation, value, fps, &running).await,
        Wave { cycle_secs, saturation, value, fps }
                                                 => software::run_wave(&device, cycle_secs, saturation, value, fps, &running).await,
        Fire { fps, intensity }                  => software::run_fire(&device, fps, intensity, &running).await,
        Meteor { r, g, b, fps }                  => software::run_meteor(&device, r, g, b, fps, &running).await,
        Comet { fps }                            => software::run_comet(&device, fps, &running).await,
        Sunrise { duration_secs, fps }           => software::run_sunrise(&device, duration_secs, fps, &running).await,
        Sunset  { duration_secs, fps }           => software::run_sunset(&device, duration_secs, fps, &running).await,
        Cct     { kelvin, brightness }           => software::run_cct(&device, kelvin, brightness).await,
        SleepTimer { duration_secs, fps }        => software::run_sleep_timer(&device, duration_secs, fps, &running).await,
        Alarm { r, g, b, flash_count, flash_ms } => software::run_alarm(&device, r, g, b, flash_count, flash_ms).await,
        WakeUp { duration_secs, fps }            => software::run_sunrise(&device, duration_secs, fps, &running).await,
        Hardware { mode, speed, brightness }     => hardware::run_hw(&device, mode, speed, brightness).await,
        MicHardware { sensitivity }              => hardware::run_mic(&device, sensitivity).await,
        Notification { r, g, b, count, duration_ms }
                                                 => software::run_notification(&device, r, g, b, count, duration_ms).await,
        #[cfg(feature = "audio")]
        Audio { source, sensitivity, fps, lo_r, lo_g, lo_b, mid_r, mid_g, mid_b, hi_r, hi_g, hi_b }
                                                 => audio::run_audio(&device, source, sensitivity, fps,
                                                                     (lo_r,lo_g,lo_b), (mid_r,mid_g,mid_b), (hi_r,hi_g,hi_b), &running).await,
        #[cfg(feature = "audio")]
        Music { source, sensitivity, fps, beat_r, beat_g, beat_b, idle_r, idle_g, idle_b }
                                                 => audio::run_music(&device, source, sensitivity, fps,
                                                                     (beat_r,beat_g,beat_b), (idle_r,idle_g,idle_b), &running).await,
        #[cfg(feature = "ambient")]
        Ambient { fps, saturation_boost, smoothing }
                                                 => ambient::run_ambient(&device, fps, saturation_boost, smoothing, &running).await,
        #[cfg(feature = "system")]
        SysMonitor { metric, fps, lo_r, lo_g, lo_b, hi_r, hi_g, hi_b }
                                                 => system_mon::run_system(&device, metric, fps,
                                                                            (lo_r,lo_g,lo_b), (hi_r,hi_g,hi_b), &running).await,
        Sequence { steps, loop_forever } =>
            run_sequence(&device, steps, loop_forever, &running).await,

        AppWatch { rules, default_r, default_g, default_b, check_ms } =>
            run_appwatch(&device, rules, (default_r, default_g, default_b), check_ms, &running).await,

        #[cfg(feature = "system")]
        Game { keywords, check_secs, rainbow_fps, cycle_secs } =>
            run_game(&device, keywords, check_secs, rainbow_fps, cycle_secs, &running).await,

        #[allow(unreachable_patterns)]
        _ => {
            eprintln!("This mode requires a feature that was not compiled in. \
                       Rebuild with --features audio/ambient/system.");
            Ok(())
        }
    }
}

// ── Sequence runner ───────────────────────────────────────────────────────────

#[cfg(feature = "ble")]
async fn run_sequence(
    device: &BLEDOMDevice,
    steps: Vec<SequenceStep>,
    loop_forever: bool,
    running: &AtomicBool,
) -> Result<()> {
    use crate::protocol::Packet;
    use tokio::time::{sleep, Duration};

    if steps.is_empty() {
        return Ok(());
    }

    loop {
        for step in &steps {
            if !running.load(Ordering::Relaxed) { return Ok(()); }

            if step.off {
                device.send(Packet::power_off()).await?;
            }
            if let Some([r, g, b]) = step.color {
                device.send(Packet::color(r, g, b)).await?;
            }
            if let Some(br) = step.brightness {
                device.send(Packet::brightness(br)).await?;
            }
            if let Some(hw) = step.hw_mode {
                device.send(Packet::hw_mode(hw, step.hw_speed.unwrap_or(50))).await?;
            }
            if let Some(raw) = step.raw {
                device.send(Packet::raw(raw)).await?;
            }

            let end = std::time::Instant::now()
                + Duration::from_secs_f32(step.duration_secs.max(0.0));
            while running.load(Ordering::Relaxed) && std::time::Instant::now() < end {
                sleep(Duration::from_millis(50)).await;
            }
        }
        if !loop_forever || !running.load(Ordering::Relaxed) { break; }
    }
    Ok(())
}

// ── AppWatch runner ───────────────────────────────────────────────────────────

#[cfg(feature = "ble")]
async fn run_appwatch(
    device: &BLEDOMDevice,
    rules: Vec<AppWatchRule>,
    default_rgb: (u8, u8, u8),
    check_ms: u64,
    running: &AtomicBool,
) -> Result<()> {
    use crate::protocol::Packet;
    use tokio::time::{sleep, Duration};

    let mut last_proc = String::new();
    while running.load(Ordering::Relaxed) {
        let proc_name = current_foreground_process();
        if proc_name != last_proc {
            last_proc = proc_name.clone();
            if let Some(rule) = rules.iter().find(|r| proc_name.to_lowercase().contains(&r.process.to_lowercase())) {
                if let Some(br) = rule.brightness {
                    device.send(Packet::brightness(br)).await?;
                }
                device.send(Packet::color(rule.r, rule.g, rule.b)).await?;
            } else {
                device.send(Packet::color(default_rgb.0, default_rgb.1, default_rgb.2)).await?;
            }
        }
        sleep(Duration::from_millis(check_ms.max(100))).await;
    }
    Ok(())
}

/// Returns the name of a process currently in focus / recently active.
/// On Windows uses GetForegroundWindow via raw WinAPI if sysinfo is available,
/// otherwise returns empty string (AppWatch still works — it just won't react
/// until the system feature is enabled).
fn current_foreground_process() -> String {
    #[cfg(all(target_os = "windows", feature = "system"))]
    {
        use sysinfo::{System, ProcessesToUpdate};

        unsafe extern "system" {
            fn GetForegroundWindow() -> *mut std::ffi::c_void;
            fn GetWindowThreadProcessId(hwnd: *mut std::ffi::c_void, lpdwProcessId: *mut u32) -> u32;
        }
        let pid = unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() { return String::new(); }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            pid
        };
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, false);
        sys.process(sysinfo::Pid::from(pid as usize))
           .map(|p| p.name().to_string_lossy().into_owned())
           .unwrap_or_default()
    }
    #[cfg(not(all(target_os = "windows", feature = "system")))]
    { String::new() }
}

// ── Game mode runner ──────────────────────────────────────────────────────────

#[cfg(all(feature = "ble", feature = "system"))]
async fn run_game(
    device: &BLEDOMDevice,
    keywords: Vec<String>,
    check_secs: f32,
    rainbow_fps: u8,
    cycle_secs: f32,
    running: &AtomicBool,
) -> Result<()> {
    use crate::protocol::{Packet, hsv_to_rgb};
    use tokio::time::{sleep, Duration};
    use sysinfo::{System, ProcessesToUpdate};

    let dt     = Duration::from_secs_f32(1.0 / rainbow_fps.max(1) as f32);
    let check  = Duration::from_secs_f32(check_secs.max(1.0));
    let mut hue = 0.0_f32;
    let mut last_check = std::time::Instant::now();
    let mut game_active = false;

    while running.load(Ordering::Relaxed) {
        if last_check.elapsed() >= check {
            last_check = std::time::Instant::now();
            let mut sys = System::new();
            sys.refresh_processes(ProcessesToUpdate::All, false);
            game_active = sys.processes().values().any(|p| {
                let name = p.name().to_ascii_lowercase().to_string_lossy().to_string();
                keywords.iter().any(|kw| name.contains(kw.as_str()))
            });
        }

        if game_active {
            let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
            device.send(Packet::color(r, g, b)).await?;
            hue = (hue + dt.as_secs_f32() / cycle_secs).rem_euclid(1.0);
        }
        sleep(dt).await;
    }
    Ok(())
}
