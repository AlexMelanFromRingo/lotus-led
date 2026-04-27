//! # lotus_led
//!
//! BLE controller library for BLEDOM / ELK-BLEDOM / Lotus Lantern LED strips.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use lotus_led::{BLEDOMDevice, Packet};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Discover devices (pass empty string for auto-scan)
//!     let devices = BLEDOMDevice::scan(Duration::from_secs(6)).await?;
//!     println!("Found: {:?}", devices);
//!
//!     // Connect by MAC or name (empty = auto-discover)
//!     let device = BLEDOMDevice::connect("", Duration::from_secs(6)).await?;
//!     device.power_on().await?;
//!     device.set_color(255, 128, 0).await?;     // warm orange
//!     device.set_brightness(80).await?;
//!
//!     // Run a software animation (runs until ctrl-c)
//!     use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
//!     use lotus_led::modes::{ModeConfig, run_mode};
//!     let running = Arc::new(AtomicBool::new(true));
//!     let r2 = running.clone();
//!     ctrlc::set_handler(move || r2.store(false, Ordering::Relaxed)).ok();
//!     run_mode(ModeConfig::rainbow_default(), Arc::new(device), running).await?;
//!     Ok(())
//! }
//! ```

pub mod protocol;
pub mod config;
pub mod modes;

#[cfg(feature = "ble")]
pub mod device;

// ── Re-exports ────────────────────────────────────────────────────────────────
pub use protocol::{
    Packet, HWMode, DeviceStatus,
    hsv_to_rgb, lerp_color, cct_to_rgb,
    SERVICE_UUID_STR, WRITE_UUID_STR, NOTIFY_UUID_STR,
    DEVICE_NAME_PATTERNS, KNOWN_MAC_PREFIX_HINT,
};
pub use config::Config;
pub use modes::ModeConfig;

#[cfg(feature = "ble")]
pub use device::{BLEDOMDevice, FoundDevice, DeviceGroup};
pub use modes::{SequenceStep, AppWatchRule};
