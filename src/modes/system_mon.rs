/// System monitor mode: maps CPU/RAM load to a color heatmap.
#[cfg(feature = "system")]
use {
    sysinfo::System,
    std::sync::atomic::{AtomicBool, Ordering},
    anyhow::Result,
};

use crate::device::BLEDOMDevice;
use crate::modes::SysMetric;
use crate::protocol::{Packet, lerp_color};

#[cfg(feature = "system")]
pub async fn run_system(
    device: &BLEDOMDevice,
    metric: SysMetric,
    fps: u8,
    lo_color: (u8, u8, u8),
    hi_color: (u8, u8, u8),
    flag: &AtomicBool,
) -> Result<()> {
    let dt  = std::time::Duration::from_millis(1000 / fps.max(1) as u64);
    let mut sys = System::new_all();

    while flag.load(Ordering::Relaxed) {
        sys.refresh_all();
        let load = match metric {
            SysMetric::Cpu => {
                let cpus = sys.cpus();
                cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len().max(1) as f32 / 100.0
            }
            SysMetric::Ram => {
                let total = sys.total_memory() as f32;
                let used  = (sys.total_memory() - sys.available_memory()) as f32;
                if total > 0.0 { used / total } else { 0.0 }
            }
        };
        let (r, g, b) = lerp_color(lo_color, hi_color, load);
        device.send(Packet::color(r, g, b)).await?;
        let bright = (50.0 + 50.0 * load) as u8;
        device.send(Packet::brightness(bright)).await?;
        tokio::time::sleep(dt).await;
    }
    Ok(())
}
