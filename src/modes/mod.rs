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
        #[allow(unreachable_patterns)]
        _ => {
            eprintln!("This mode requires a feature that was not compiled in. \
                       Rebuild with --features audio/ambient/system.");
            Ok(())
        }
    }
}
