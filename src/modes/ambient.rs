/// Screen ambient (Ambilight) mode using the `screenshots` crate.
#[cfg(feature = "ambient")]
use {
    screenshots::Screen,
    std::sync::atomic::{AtomicBool, Ordering},
    anyhow::Result,
};

use crate::device::BLEDOMDevice;
use crate::protocol::Packet;

#[cfg(feature = "ambient")]
pub async fn run_ambient(
    device: &BLEDOMDevice,
    fps: u8,
    saturation_boost: f32,
    smoothing: f32,
    flag: &AtomicBool,
) -> Result<()> {
    let dt      = std::time::Duration::from_millis(1000 / fps.max(1) as u64);
    let screens = Screen::all()?;
    let screen  = screens.into_iter()
        .find(|s| s.display_info.is_primary)
        .ok_or_else(|| anyhow::anyhow!("No primary screen found"))?;

    let (mut prev_r, mut prev_g, mut prev_b) = (0u8, 0u8, 0u8);

    while flag.load(Ordering::Relaxed) {
        if let Ok(image) = screen.capture() {
            let (r_raw, g_raw, b_raw) = sample_edges(image.width(), image.height(), image.as_raw());
            let (r_n, g_n, b_n) = boost_saturation(r_raw, g_raw, b_raw, saturation_boost);

            let blend = |old: u8, new: u8| -> u8 {
                (old as f32 * smoothing + new as f32 * (1.0 - smoothing)) as u8
            };
            let r = blend(prev_r, r_n);
            let g = blend(prev_g, g_n);
            let b = blend(prev_b, b_n);

            device.send(Packet::color(r, g, b)).await?;
            (prev_r, prev_g, prev_b) = (r, g, b);
        }
        tokio::time::sleep(dt).await;
    }
    Ok(())
}

#[cfg(feature = "ambient")]
fn sample_edges(width: u32, height: u32, rgba: &[u8]) -> (u8, u8, u8) {
    let edge_w = (height / 8).max(40) as usize;
    let edge_h = (width  / 8).max(40) as usize;
    let stride = width as usize * 4; // RGBA

    let mut sum_r = 0u64;
    let mut sum_g = 0u64;
    let mut sum_b = 0u64;
    let mut count = 0u64;

    let add = |row: usize, col: usize, sr: &mut u64, sg: &mut u64, sb: &mut u64, cnt: &mut u64| {
        let base = row * stride + col * 4;
        if base + 3 < rgba.len() {
            *sr  += rgba[base]     as u64;
            *sg  += rgba[base + 1] as u64;
            *sb  += rgba[base + 2] as u64;
            *cnt += 1;
        }
    };

    for row in 0..edge_w {
        for col in 0..width as usize {
            add(row, col, &mut sum_r, &mut sum_g, &mut sum_b, &mut count);
            add(height as usize - 1 - row, col, &mut sum_r, &mut sum_g, &mut sum_b, &mut count);
        }
    }
    for row in 0..height as usize {
        for col in 0..edge_h {
            add(row, col, &mut sum_r, &mut sum_g, &mut sum_b, &mut count);
            add(row, width as usize - 1 - col, &mut sum_r, &mut sum_g, &mut sum_b, &mut count);
        }
    }
    if count == 0 { return (0, 0, 0); }
    ((sum_r / count) as u8, (sum_g / count) as u8, (sum_b / count) as u8)
}

#[cfg(feature = "ambient")]
fn boost_saturation(r: u8, g: u8, b: u8, boost: f32) -> (u8, u8, u8) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let v   = max;
    if v < 1e-6 { return (r, g, b); }
    let s = (max - min) / max;
    let s_new = (s * boost).clamp(0.0, 1.0);
    let h = {
        if max == min { 0.0 }
        else if max == rf { (gf - bf) / (max - min) }
        else if max == gf { 2.0 + (bf - rf) / (max - min) }
        else               { 4.0 + (rf - gf) / (max - min) }
    } / 6.0;
    let h = h.rem_euclid(1.0);
    let (rn, gn, bn) = crate::protocol::hsv_to_rgb(h, s_new, v);
    (rn, gn, bn)
}
