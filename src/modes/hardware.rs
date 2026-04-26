/// Hardware (firmware-native) mode pass-through.
use anyhow::Result;
use crate::device::BLEDOMDevice;
use crate::protocol::{HWMode, Packet};

pub async fn run_hw(
    device: &BLEDOMDevice,
    mode: HWMode,
    speed: u8,
    brightness: Option<u8>,
) -> Result<()> {
    if let Some(b) = brightness {
        device.send(Packet::brightness(b)).await?;
    }
    device.send(Packet::hw_mode(mode, speed)).await?;
    Ok(())
}

pub async fn run_mic(device: &BLEDOMDevice, sensitivity: u8) -> Result<()> {
    device.send(Packet::mic_sensitivity(sensitivity)).await
}
