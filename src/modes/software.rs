/// Pure software animation modes — no OS-specific dependencies.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use anyhow::Result;

use crate::device::BLEDOMDevice;
use crate::protocol::{Packet, hsv_to_rgb, lerp_color, cct_to_rgb};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn frame_duration(fps: u8) -> Duration {
    Duration::from_millis(1000 / fps.max(1) as u64)
}

fn running(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Relaxed)
}

// ── Static ────────────────────────────────────────────────────────────────────

pub async fn run_static(device: &BLEDOMDevice, r: u8, g: u8, b: u8) -> Result<()> {
    device.send(Packet::color(r, g, b)).await
}

// ── Pulse / Breathe ───────────────────────────────────────────────────────────

pub async fn run_pulse(
    device: &BLEDOMDevice,
    r: u8, g: u8, b: u8,
    period_secs: f32,
    min_bright: u8,
    max_bright: u8,
    fps: u8,
    flag: &AtomicBool,
) -> Result<()> {
    device.send(Packet::color(r, g, b)).await?;
    let dt = frame_duration(fps);
    let mut t = 0.0f32;

    while running(flag) {
        let phase  = (std::f32::consts::PI * t / period_secs).sin().powi(2);
        let bright = (min_bright as f32 + (max_bright - min_bright) as f32 * phase) as u8;
        device.send(Packet::brightness(bright)).await?;
        tokio::time::sleep(dt).await;
        t = (t + dt.as_secs_f32()) % (period_secs * 2.0);
    }
    Ok(())
}

// ── Rainbow ───────────────────────────────────────────────────────────────────

pub async fn run_rainbow(
    device: &BLEDOMDevice,
    cycle_secs: f32,
    saturation: f32,
    value: f32,
    fps: u8,
    flag: &AtomicBool,
) -> Result<()> {
    let dt   = frame_duration(fps);
    let step = dt.as_secs_f32() / cycle_secs;
    let mut hue = 0.0f32;

    while running(flag) {
        let (r, g, b) = hsv_to_rgb(hue, saturation, value);
        device.send(Packet::color(r, g, b)).await?;
        hue = (hue + step) % 1.0;
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

// ── Wave ──────────────────────────────────────────────────────────────────────

pub async fn run_wave(
    device: &BLEDOMDevice,
    cycle_secs: f32,
    saturation: f32,
    value: f32,
    fps: u8,
    flag: &AtomicBool,
) -> Result<()> {
    let dt = frame_duration(fps);
    let mut t = 0.0f32;

    while running(flag) {
        let hue   = 0.5 + 0.5 * (2.0 * std::f32::consts::PI * t / cycle_secs).sin();
        let (r, g, b) = hsv_to_rgb(hue, saturation, value);
        device.send(Packet::color(r, g, b)).await?;
        t = (t + dt.as_secs_f32()) % cycle_secs;
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

// ── Fire ──────────────────────────────────────────────────────────────────────

pub async fn run_fire(
    device: &BLEDOMDevice,
    fps: u8,
    intensity: f32,
    flag: &AtomicBool,
) -> Result<()> {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let dt    = frame_duration(fps);
    let mut t = 0u64;

    while running(flag) {
        // Deterministic pseudo-random flicker using the frame counter
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        let rnd = (h.finish() & 0xFFFF) as f32 / 65535.0;

        let hue = 0.01 + rnd * 0.07;   // orange-red
        let sat = 0.8  + rnd * 0.2;
        let val = intensity * (0.5 + rnd * 0.5);
        let (r, g, b) = hsv_to_rgb(hue, sat, val);
        device.send(Packet::color(r, g, b)).await?;
        t = t.wrapping_add(1);
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

// ── Meteor ────────────────────────────────────────────────────────────────────

pub async fn run_meteor(
    device: &BLEDOMDevice,
    r: u8, g: u8, b: u8,
    fps: u8,
    flag: &AtomicBool,
) -> Result<()> {
    let dt = frame_duration(fps);
    let mut phase = 0.0f32;
    // burst_period varies; we use a fixed 2-second burst cycle
    let burst_period = 2.0f32;

    while running(flag) {
        let bright_val = if phase < 0.15 {
            phase / 0.15
        } else {
            1.0 - (phase - 0.15) / 0.85
        };
        let dr = (r as f32 * bright_val) as u8;
        let dg = (g as f32 * bright_val) as u8;
        let db = (b as f32 * bright_val) as u8;
        device.send(Packet::color(dr, dg, db)).await?;
        phase = (phase + dt.as_secs_f32() / burst_period) % 1.0;
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

// ── Comet ─────────────────────────────────────────────────────────────────────

pub async fn run_comet(device: &BLEDOMDevice, fps: u8, flag: &AtomicBool) -> Result<()> {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let dt    = frame_duration(fps);
    let mut hue = 0.0f32;
    let mut t   = 0u64;

    while running(flag) {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        let sparkle = 0.85 + (h.finish() & 0xFFFF) as f32 / 65535.0 * 0.15;
        let (r, g, b) = hsv_to_rgb(hue, 0.9, sparkle);
        device.send(Packet::color(r, g, b)).await?;
        hue = (hue + dt.as_secs_f32() * 0.05) % 1.0;
        t = t.wrapping_add(1);
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

// ── Sunrise ───────────────────────────────────────────────────────────────────

pub async fn run_sunrise(
    device: &BLEDOMDevice,
    duration_secs: u32,
    fps: u8,
    flag: &AtomicBool,
) -> Result<()> {
    let dt    = frame_duration(fps);
    let steps = (duration_secs as u64 * fps as u64).max(1);

    for i in 0..steps {
        if !running(flag) { break; }
        let t      = i as f32 / steps as f32;
        let kelvin = (1800.0 + (5500.0 - 1800.0) * t) as u32;
        let (r, g, b) = cct_to_rgb(kelvin);
        let start     = (80_f32, 5_f32, 0_f32);
        let r = (start.0 + (r as f32 - start.0) * t) as u8;
        let g = (start.1 + (g as f32 - start.1) * t) as u8;
        let b = (start.2 + (b as f32 - start.2) * t) as u8;
        device.send(Packet::color(r, g, b)).await?;
        let bright = (5.0 + 95.0 * t) as u8;
        device.send(Packet::brightness(bright)).await?;
        tokio::time::sleep(dt).await;
    }
    // Hold at full brightness
    while running(flag) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(())
}

// ── Sunset ────────────────────────────────────────────────────────────────────

pub async fn run_sunset(
    device: &BLEDOMDevice,
    duration_secs: u32,
    fps: u8,
    flag: &AtomicBool,
) -> Result<()> {
    let dt    = frame_duration(fps);
    let steps = (duration_secs as u64 * fps as u64).max(1);

    for i in 0..steps {
        if !running(flag) { break; }
        let t      = i as f32 / steps as f32;
        let kelvin = (5500.0 - (5500.0 - 1800.0) * t) as u32;
        let (r, g, b) = cct_to_rgb(kelvin);
        let bright = (100.0 - 98.0 * t) as u8;
        device.send(Packet::color(r, g, b)).await?;
        device.send(Packet::brightness(bright)).await?;
        tokio::time::sleep(dt).await;
    }
    device.send(Packet::power_off()).await?;
    Ok(())
}

// ── Color temperature ─────────────────────────────────────────────────────────

pub async fn run_cct(device: &BLEDOMDevice, kelvin: u32, brightness: u8) -> Result<()> {
    let (r, g, b) = cct_to_rgb(kelvin);
    device.send(Packet::color(r, g, b)).await?;
    device.send(Packet::brightness(brightness)).await?;
    Ok(())
}

// ── Sleep timer ───────────────────────────────────────────────────────────────

pub async fn run_sleep_timer(
    device: &BLEDOMDevice,
    duration_secs: u32,
    fps: u8,
    flag: &AtomicBool,
) -> Result<()> {
    let dt    = frame_duration(fps);
    let steps = (duration_secs as u64 * fps as u64).max(1);

    for i in 0..steps {
        if !running(flag) { break; }
        let bright = (100.0 * (1.0 - i as f32 / steps as f32)) as u8;
        device.send(Packet::brightness(bright)).await?;
        tokio::time::sleep(dt).await;
    }
    device.send(Packet::power_off()).await?;
    Ok(())
}

// ── Alarm flash ───────────────────────────────────────────────────────────────

pub async fn run_alarm(
    device: &BLEDOMDevice,
    r: u8, g: u8, b: u8,
    flash_count: u32,
    flash_ms: u64,
) -> Result<()> {
    for _ in 0..flash_count {
        device.send(Packet::color(r, g, b)).await?;
        device.send(Packet::brightness(100)).await?;
        tokio::time::sleep(Duration::from_millis(flash_ms)).await;
        device.send(Packet::brightness(0)).await?;
        tokio::time::sleep(Duration::from_millis(flash_ms)).await;
    }
    Ok(())
}

// ── Notification flash ────────────────────────────────────────────────────────

pub async fn run_notification(
    device: &BLEDOMDevice,
    r: u8, g: u8, b: u8,
    count: u32,
    duration_ms: u64,
) -> Result<()> {
    run_alarm(device, r, g, b, count, duration_ms).await
}
