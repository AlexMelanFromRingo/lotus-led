/// Async BLE driver for BLEDOM / Lotus Lantern controllers.
///
/// Provides scan, connect, send, and notification subscription.
/// Rate-limited to MAX_PACKETS_PER_SEC to avoid overwhelming the controller.
#[cfg(feature = "ble")]
use {
    anyhow::{anyhow, Result},
    btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType},
    btleplug::platform::{Manager, Peripheral},
    futures::StreamExt,
    std::time::{Duration, Instant},
    tokio::sync::broadcast,
    uuid::Uuid,
};

#[cfg(not(feature = "ble"))]
use anyhow::Result;

use std::sync::Arc;
use crate::protocol::{
    DeviceStatus, Packet, DEVICE_NAME_PATTERNS, KNOWN_MAC_PREFIX_HINT,
    NOTIFY_UUID_STR, WRITE_UUID_STR,
};

const MAX_PACKETS_PER_SEC: u64 = 20;
const MIN_INTERVAL_MS: u64 = 1000 / MAX_PACKETS_PER_SEC;

// ── Discovered device descriptor ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FoundDevice {
    pub name:    String,
    pub address: String,
}

impl std::fmt::Display for FoundDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:20}  [{}]", self.name, self.address)
    }
}

// ── BLE Device ────────────────────────────────────────────────────────────────

#[cfg(feature = "ble")]
pub struct BLEDOMDevice {
    peripheral:     Peripheral,
    write_char:     btleplug::api::Characteristic,
    status_tx:      broadcast::Sender<DeviceStatus>,
    last_send:      std::sync::Mutex<Instant>,
}

#[cfg(feature = "ble")]
impl BLEDOMDevice {
    // ── Scanning ─────────────────────────────────────────────────────────────

    /// Scan for BLEDOM-like devices. Returns up to `max` devices found within `timeout`.
    pub async fn scan(timeout: Duration) -> Result<Vec<FoundDevice>> {
        let manager  = Manager::new().await?;
        let adapters = manager.adapters().await?;
        let adapter  = adapters.into_iter().next()
            .ok_or_else(|| anyhow!("No Bluetooth adapter found"))?;

        adapter.start_scan(ScanFilter::default()).await?;
        tokio::time::sleep(timeout).await;
        adapter.stop_scan().await?;

        let peripherals = adapter.peripherals().await?;
        let mut found = Vec::new();

        for p in peripherals {
            if let Ok(Some(props)) = p.properties().await {
                let name = props.local_name.unwrap_or_default();
                let addr = props.address.to_string();

                let by_name   = DEVICE_NAME_PATTERNS.iter().any(|pat| name.to_uppercase().contains(pat));
                let by_prefix = addr.to_uppercase().starts_with(KNOWN_MAC_PREFIX_HINT);

                if by_name || by_prefix {
                    found.push(FoundDevice { name, address: addr });
                }
            }
        }
        Ok(found)
    }

    // ── Connection ────────────────────────────────────────────────────────────

    /// Connect to a device.
    ///
    /// `identifier` can be:
    /// - A MAC address string (`"BE:60:65:XX:XX:XX"`)
    /// - A device name fragment (`"BLEDOM"`)
    /// - An empty string → auto-discover first BLEDOM-like device
    pub async fn connect(identifier: &str, scan_timeout: Duration) -> Result<Self> {
        let manager  = Manager::new().await?;
        let adapters = manager.adapters().await?;
        let adapter  = adapters.into_iter().next()
            .ok_or_else(|| anyhow!("No Bluetooth adapter found"))?;

        adapter.start_scan(ScanFilter::default()).await?;
        tokio::time::sleep(scan_timeout).await;
        adapter.stop_scan().await?;

        let peripherals = adapter.peripherals().await?;
        let mut target: Option<Peripheral> = None;

        for p in peripherals {
            if let Ok(Some(props)) = p.properties().await {
                let name = props.local_name.clone().unwrap_or_default();
                let addr = props.address.to_string();

                let matched = if identifier.is_empty() {
                    DEVICE_NAME_PATTERNS.iter().any(|pat| name.to_uppercase().contains(pat))
                        || addr.to_uppercase().starts_with(KNOWN_MAC_PREFIX_HINT)
                } else {
                    addr.to_uppercase() == identifier.to_uppercase()
                        || name.to_uppercase().contains(&identifier.to_uppercase())
                };

                if matched {
                    target = Some(p);
                    break;
                }
            }
        }

        let peripheral = target.ok_or_else(|| {
            if identifier.is_empty() {
                anyhow!("No BLEDOM device found. Try 'led scan' first.")
            } else {
                anyhow!("Device '{}' not found after scan.", identifier)
            }
        })?;

        peripheral.connect().await?;
        peripheral.discover_services().await?;

        let chars      = peripheral.characteristics();
        let write_uuid = Uuid::parse_str(WRITE_UUID_STR)?;
        let write_char = chars.iter()
            .find(|c| c.uuid == write_uuid)
            .cloned()
            .ok_or_else(|| anyhow!("Write characteristic FFF3 not found — wrong device?"))?;

        let (status_tx, _) = broadcast::channel(16);
        let dev = Self {
            peripheral,
            write_char,
            status_tx,
            last_send: std::sync::Mutex::new(Instant::now()),
        };

        dev.subscribe_notifications().await;
        Ok(dev)
    }

    /// Read firmware version string from the write characteristic.
    pub async fn read_firmware(&self) -> String {
        self.peripheral
            .read(&self.write_char)
            .await
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| "unknown".into())
    }

    /// Returns the device MAC address string.
    pub fn address(&self) -> String {
        self.peripheral.address().to_string()
    }

    /// Returns `true` if the underlying BLE peripheral is connected.
    pub fn is_connected(&self) -> bool {
        futures::executor::block_on(self.peripheral.is_connected()).unwrap_or(false)
    }

    /// Disconnect from the device.
    pub async fn disconnect(&self) -> Result<()> {
        self.peripheral.disconnect().await?;
        Ok(())
    }

    // ── Notifications ─────────────────────────────────────────────────────────

    async fn subscribe_notifications(&self) {
        let notify_uuid = match Uuid::parse_str(NOTIFY_UUID_STR) {
            Ok(u) => u,
            Err(_) => return,
        };
        let chars      = self.peripheral.characteristics();
        let notify_char = chars.iter().find(|c| c.uuid == notify_uuid).cloned();

        if let Some(nc) = notify_char {
            if self.peripheral.subscribe(&nc).await.is_ok() {
                if let Ok(mut notifs) = self.peripheral.notifications().await {
                    let tx = self.status_tx.clone();
                    tokio::spawn(async move {
                        while let Some(notif) = notifs.next().await {
                            if notif.uuid == notify_uuid {
                                if let Some(status) = Packet::parse_status(&notif.value) {
                                    let _ = tx.send(status);
                                }
                            }
                        }
                    });
                }
            }
        }
    }

    /// Subscribe to device status notifications (FFF4).
    pub fn status_receiver(&self) -> broadcast::Receiver<DeviceStatus> {
        self.status_tx.subscribe()
    }

    // ── Packet sending ────────────────────────────────────────────────────────

    /// Send a single packet with automatic rate limiting (max 20 pkt/s).
    pub async fn send(&self, packet: Packet) -> Result<()> {
        let wait = {
            let last = self.last_send.lock().unwrap();
            let elapsed = last.elapsed();
            let min = Duration::from_millis(MIN_INTERVAL_MS);
            if elapsed < min { min - elapsed } else { Duration::ZERO }
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        self.peripheral
            .write(&self.write_char, packet.bytes(), WriteType::WithoutResponse)
            .await?;
        *self.last_send.lock().unwrap() = Instant::now();
        Ok(())
    }

    // ── Convenience wrappers ──────────────────────────────────────────────────

    pub async fn power_on(&self)  -> Result<()> { self.send(Packet::power_on()).await }
    pub async fn power_off(&self) -> Result<()> { self.send(Packet::power_off()).await }

    pub async fn set_color(&self, r: u8, g: u8, b: u8) -> Result<()> {
        self.send(Packet::color(r, g, b)).await
    }
    pub async fn set_brightness(&self, level: u8) -> Result<()> {
        self.send(Packet::brightness(level)).await
    }
    pub async fn set_speed(&self, level: u8) -> Result<()> {
        self.send(Packet::speed(level)).await
    }
    pub async fn set_hw_mode(&self, mode: crate::protocol::HWMode, speed: u8) -> Result<()> {
        self.send(Packet::speed(speed)).await?;
        self.send(Packet::hw_mode(mode)).await
    }
    pub async fn set_mic(&self, sensitivity: u8) -> Result<()> {
        self.send(Packet::mic_sensitivity(sensitivity)).await
    }
}

// ── Multi-device group ────────────────────────────────────────────────────────

/// Control multiple LED strips simultaneously.
///
/// ```no_run
/// let group = DeviceGroup::new(vec![strip_a, strip_b]);
/// group.broadcast(Packet::color(255, 0, 128)).await?;
/// group.get(0).set_color(255, 0, 0).await?;
/// ```
pub struct DeviceGroup {
    pub devices: Vec<Arc<BLEDOMDevice>>,
}

impl DeviceGroup {
    pub fn new(devices: Vec<Arc<BLEDOMDevice>>) -> Self {
        Self { devices }
    }

    pub fn get(&self, idx: usize) -> Option<&Arc<BLEDOMDevice>> {
        self.devices.get(idx)
    }

    pub fn len(&self) -> usize { self.devices.len() }
    pub fn is_empty(&self) -> bool { self.devices.is_empty() }

    /// Send the same packet to every device in parallel.
    pub async fn broadcast(&self, packet: Packet) -> Result<()> {
        let futs: Vec<_> = self.devices.iter().map(|d: &Arc<BLEDOMDevice>| d.send(packet)).collect();
        futures::future::try_join_all(futs).await?;
        Ok(())
    }

    pub async fn power_on(&self)  -> Result<()> { self.broadcast(Packet::power_on()).await }
    pub async fn power_off(&self) -> Result<()> { self.broadcast(Packet::power_off()).await }

    pub async fn set_color(&self, r: u8, g: u8, b: u8) -> Result<()> {
        self.broadcast(Packet::color(r, g, b)).await
    }
    pub async fn set_brightness(&self, level: u8) -> Result<()> {
        self.broadcast(Packet::brightness(level)).await
    }
    pub async fn set_speed(&self, level: u8) -> Result<()> {
        self.broadcast(Packet::speed(level)).await
    }

    /// Disconnect all devices.
    pub async fn disconnect_all(&self) {
        for d in &self.devices {
            let _ = d.disconnect().await;  // ignore individual errors
        }
    }
}
