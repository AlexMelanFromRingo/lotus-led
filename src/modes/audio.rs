/// Audio reactive modes using CPAL (Windows WASAPI / cross-platform).
#[cfg(feature = "audio")]
use {
    cpal::traits::{DeviceTrait, HostTrait, StreamTrait},
    std::sync::{Arc, Mutex, atomic::AtomicBool},
    anyhow::{anyhow, Result},
};

use crate::device::BLEDOMDevice;
use crate::modes::AudioSource;
use crate::protocol::Packet;

#[cfg(feature = "audio")]
pub async fn run_audio(
    device: &BLEDOMDevice,
    source: AudioSource,
    sensitivity: f32,
    fps: u8,
    lo_color: (u8, u8, u8),
    mid_color: (u8, u8, u8),
    hi_color: (u8, u8, u8),
    flag: &AtomicBool,
) -> Result<()> {
    use std::sync::atomic::Ordering;

    let host        = cpal::default_host();
    let cpal_device = pick_device(&host, source)?;
    let config      = cpal_device.default_input_config()?;
    let sr          = config.sample_rate().0 as f32;
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let buf_clone = buf.clone();

    let stream = cpal_device.build_input_stream(
        &config.into(),
        move |data: &[f32], _| {
            let mut b = buf_clone.lock().unwrap();
            b.clear();
            b.extend_from_slice(data);
        },
        |e| eprintln!("[audio] stream error: {e}"),
        None,
    )?;
    stream.play()?;

    let dt = std::time::Duration::from_millis(1000 / fps.max(1) as u64);
    while flag.load(Ordering::Relaxed) {
        let samples = { buf.lock().unwrap().clone() };
        if samples.len() >= 512 {
            let (r, g, b) = analyze_spectrum(&samples, sr, sensitivity, lo_color, mid_color, hi_color);
            device.send(Packet::color(r, g, b)).await?;
        }
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

#[cfg(feature = "audio")]
pub async fn run_music(
    device: &BLEDOMDevice,
    source: AudioSource,
    sensitivity: f32,
    fps: u8,
    beat_color: (u8, u8, u8),
    idle_color: (u8, u8, u8),
    flag: &AtomicBool,
) -> Result<()> {
    use std::sync::atomic::Ordering;

    let host        = cpal::default_host();
    let cpal_device = pick_device(&host, source)?;
    let config      = cpal_device.default_input_config()?;
    let sr          = config.sample_rate().0 as f32;
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let buf_clone = buf.clone();

    let stream = cpal_device.build_input_stream(
        &config.into(),
        move |data: &[f32], _| {
            let mut b = buf_clone.lock().unwrap();
            b.clear();
            b.extend_from_slice(data);
        },
        |e| eprintln!("[audio] stream error: {e}"),
        None,
    )?;
    stream.play()?;

    let dt       = std::time::Duration::from_millis(1000 / fps.max(1) as u64);
    let mut hist = Vec::<f32>::new();

    while flag.load(Ordering::Relaxed) {
        let samples = { buf.lock().unwrap().clone() };
        if samples.len() >= 512 {
            let (is_beat, energy) = detect_beat(&samples, sr, sensitivity, &mut hist);
            let (r, g, b) = if is_beat {
                beat_color
            } else {
                let t = (energy * 0.5).clamp(0.0, 1.0);
                lerp_color_f32(idle_color, beat_color, t)
            };
            device.send(Packet::color(r, g, b)).await?;
        }
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

#[cfg(feature = "audio")]
fn pick_device(host: &cpal::Host, source: AudioSource) -> Result<cpal::Device> {
    match source {
        AudioSource::Microphone => host.default_input_device()
            .ok_or_else(|| anyhow!("No microphone found")),
        AudioSource::Loopback => {
            // Try loopback / stereo mix first, fall back to default input
            for dev in host.input_devices()? {
                let name = dev.name().unwrap_or_default().to_lowercase();
                if name.contains("loopback") || name.contains("stereo mix") || name.contains("what u hear") {
                    return Ok(dev);
                }
            }
            host.default_input_device()
                .ok_or_else(|| anyhow!("No audio input device found"))
        }
    }
}

#[cfg(feature = "audio")]
fn band_energy(samples: &[f32], sample_rate: f32, lo_hz: f32, hi_hz: f32) -> f32 {
    let n   = samples.len();
    let bin = sample_rate / n as f32;
    let lo  = (lo_hz / bin) as usize;
    let hi  = (hi_hz / bin).min(n as f32 / 2.0) as usize;
    if lo >= hi { return 0.0; }
    // Simple DFT over the band (no external FFT crate needed)
    let mut energy = 0.0f32;
    for k in lo..hi {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        let factor = 2.0 * std::f32::consts::PI * k as f32 / n as f32;
        for (i, &s) in samples.iter().enumerate() {
            re += s * (factor * i as f32).cos();
            im += s * (factor * i as f32).sin();
        }
        energy += re * re + im * im;
    }
    (energy / (hi - lo) as f32).sqrt()
}

#[cfg(feature = "audio")]
fn analyze_spectrum(
    samples: &[f32],
    sr: f32,
    sensitivity: f32,
    lo_col: (u8, u8, u8),
    mid_col: (u8, u8, u8),
    hi_col: (u8, u8, u8),
) -> (u8, u8, u8) {
    let bass   = (band_energy(samples, sr, 20.0,   250.0) * sensitivity * 0.003).clamp(0.0, 1.0);
    let mid    = (band_energy(samples, sr, 250.0,  4000.0) * sensitivity * 0.002).clamp(0.0, 1.0);
    let treble = (band_energy(samples, sr, 4000.0, 20000.0) * sensitivity * 0.002).clamp(0.0, 1.0);
    let total  = bass + mid + treble + 1e-9;
    let r = ((lo_col.0 as f32 * bass + mid_col.0 as f32 * mid + hi_col.0 as f32 * treble) / total * (bass + mid + treble) / 3.0).clamp(0.0, 255.0) as u8;
    let g = ((lo_col.1 as f32 * bass + mid_col.1 as f32 * mid + hi_col.1 as f32 * treble) / total * (bass + mid + treble) / 3.0).clamp(0.0, 255.0) as u8;
    let b = ((lo_col.2 as f32 * bass + mid_col.2 as f32 * mid + hi_col.2 as f32 * treble) / total * (bass + mid + treble) / 3.0).clamp(0.0, 255.0) as u8;
    (r, g, b)
}

#[cfg(feature = "audio")]
fn detect_beat(samples: &[f32], sr: f32, sensitivity: f32, hist: &mut Vec<f32>) -> (bool, f32) {
    let energy = band_energy(samples, sr, 40.0, 200.0) * sensitivity;
    hist.push(energy);
    if hist.len() > 43 { hist.remove(0); }
    if hist.len() < 10 { return (false, energy); }
    let avg: f32 = hist[..hist.len() - 1].iter().sum::<f32>() / (hist.len() - 1) as f32;
    let is_beat  = energy > avg * 1.5 && energy > 0.01;
    (is_beat, (energy / (avg + 1e-9) / 2.0).clamp(0.0, 1.0))
}

#[cfg(feature = "audio")]
fn lerp_color_f32(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}
