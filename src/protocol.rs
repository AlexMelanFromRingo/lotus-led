/// BLEDOM 9-byte BLE packet protocol constants, enums, and packet builder.
///
/// All commands follow the envelope: `7E [cmd] [sub] [data × 5] EF`
use serde::{Deserialize, Serialize};

// ── UUIDs ─────────────────────────────────────────────────────────────────────
// Standard BLE service/characteristic UUIDs for the BLEDOM firmware (FFF0 family)
pub const SERVICE_UUID_STR: &str = "0000fff0-0000-1000-8000-00805f9b34fb";
pub const WRITE_UUID_STR:   &str = "0000fff3-0000-1000-8000-00805f9b34fb";
pub const NOTIFY_UUID_STR:  &str = "0000fff4-0000-1000-8000-00805f9b34fb";

// ── Device discovery hints (informational only, not required) ─────────────────
// Substrings present in advertisement names of BLEDOM controllers
pub const DEVICE_NAME_PATTERNS: &[&str] = &["BLEDOM", "ELK-BLEDOM", "ELK-BLEDOB", "LEDBLE", "ELK_BLEDOM", "BLEDOB"];
// Known OUI prefix for this controller family (used as a scan hint, not a filter)
pub const KNOWN_MAC_PREFIX_HINT: &str = "BE:60:65";

// ── Hardware animation mode IDs ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(u8)]
pub enum HWMode {
    Jump7Color      = 0x81,
    FadeRed         = 0x82,
    FadeGreen       = 0x83,
    FadeBlue        = 0x84,
    FadeYellow      = 0x85,
    FadeCyan        = 0x86,
    FadePurple      = 0x87,
    FadeWhite       = 0x88,
    CrossRedGreen   = 0x89,
    CrossRedBlue    = 0x8A,
    CrossGreenBlue  = 0x8B,
    Strobe7Color    = 0x8C,
    StrobeRed       = 0x8D,
    StrobeGreen     = 0x8E,
    StrobeBlue      = 0x8F,
    StrobeYellow    = 0x90,
    StrobeCyan      = 0x91,
    StrobePurple    = 0x92,
    StrobeWhite     = 0x93,
    Fade7Color      = 0x94,
}

impl HWMode {
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_uppercase().replace(['-', ' '], "_").as_str() {
            "JUMP7COLOR"     | "JUMP_7_COLOR"    => Some(Self::Jump7Color),
            "FADERED"        | "FADE_RED"        => Some(Self::FadeRed),
            "FADEGREEN"      | "FADE_GREEN"      => Some(Self::FadeGreen),
            "FADEBLUE"       | "FADE_BLUE"       => Some(Self::FadeBlue),
            "FADEYELLOW"     | "FADE_YELLOW"     => Some(Self::FadeYellow),
            "FADECYAN"       | "FADE_CYAN"       => Some(Self::FadeCyan),
            "FADEPURPLE"     | "FADE_PURPLE"     => Some(Self::FadePurple),
            "FADEWHITE"      | "FADE_WHITE"      => Some(Self::FadeWhite),
            "CROSSREDGREEN"  | "CROSS_RED_GREEN" => Some(Self::CrossRedGreen),
            "CROSSREDBLUE"   | "CROSS_RED_BLUE"  => Some(Self::CrossRedBlue),
            "CROSSGREENBLUE" | "CROSS_GREEN_BLUE"=> Some(Self::CrossGreenBlue),
            "STROBE7COLOR"   | "STROBE_7_COLOR"  => Some(Self::Strobe7Color),
            "STROBERED"      | "STROBE_RED"      => Some(Self::StrobeRed),
            "STROBEGREEN"    | "STROBE_GREEN"    => Some(Self::StrobeGreen),
            "STROBEBLUE"     | "STROBE_BLUE"     => Some(Self::StrobeBlue),
            "STROBEYELLOW"   | "STROBE_YELLOW"   => Some(Self::StrobeYellow),
            "STROBECYAN"     | "STROBE_CYAN"     => Some(Self::StrobeCyan),
            "STROBEPURPLE"   | "STROBE_PURPLE"   => Some(Self::StrobePurple),
            "STROBEWHITE"    | "STROBE_WHITE"    => Some(Self::StrobeWhite),
            "FADE7COLOR"     | "FADE_7_COLOR"    => Some(Self::Fade7Color),
            _ => None,
        }
    }

    pub fn all_names() -> &'static [&'static str] {
        &[
            "jump_7_color", "fade_red", "fade_green", "fade_blue", "fade_yellow",
            "fade_cyan", "fade_purple", "fade_white", "cross_red_green",
            "cross_red_blue", "cross_green_blue", "strobe_7_color", "strobe_red",
            "strobe_green", "strobe_blue", "strobe_yellow", "strobe_cyan",
            "strobe_purple", "strobe_white", "fade_7_color",
        ]
    }
}

// ── 9-byte packet wrapper ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct Packet([u8; 9]);

impl Packet {
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn power_on() -> Self {
        Self([0x7E, 0x04, 0x04, 0xF0, 0x00, 0x01, 0xFF, 0x00, 0xEF])
    }

    pub fn power_off() -> Self {
        Self([0x7E, 0x04, 0x04, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xEF])
    }

    pub fn color(r: u8, g: u8, b: u8) -> Self {
        Self([0x7E, 0x07, 0x05, 0x03, r, g, b, 0x10, 0xEF])
    }

    /// Brightness level 0–100
    pub fn brightness(level: u8) -> Self {
        let lvl = level.min(100);
        Self([0x7E, 0x04, 0x01, lvl, 0x01, 0xFF, 0xFF, 0x00, 0xEF])
    }

    /// Speed for hardware animations 0–100
    pub fn speed(level: u8) -> Self {
        let lvl = level.min(100);
        Self([0x7E, 0x04, 0x02, lvl, 0xFF, 0xFF, 0xFF, 0x00, 0xEF])
    }

    /// Activate a hardware built-in animation mode
    pub fn hw_mode(mode: HWMode, speed: u8) -> Self {
        Self([0x7E, 0x05, 0x03, mode as u8, speed.min(100), 0x00, 0x00, 0x00, 0xEF])
    }

    /// Activate on-board microphone (if present), sensitivity 0–255
    pub fn mic_sensitivity(sens: u8) -> Self {
        Self([0x7E, 0x04, 0x05, sens, 0x00, 0x00, 0x00, 0x00, 0xEF])
    }

    /// Set RGB pin order (1–6)
    pub fn color_order(id: u8) -> Self {
        let id = id.max(1).min(6);
        Self([0x7E, 0x08, 0x05, 0x02, id, 0x00, 0x00, 0x00, 0xEF])
    }

    /// Parse a FFF4 status notification.
    /// Format: `7E 08 [Power:01/00] [ModeID] [Speed] [R] [G] [B] 00 EF`
    pub fn parse_status(data: &[u8]) -> Option<DeviceStatus> {
        if data.len() >= 9 && data[0] == 0x7E && data[data.len() - 1] == 0xEF {
            Some(DeviceStatus {
                power:  data[2] != 0,
                mode:   data[3],
                speed:  data[4],
                r:      data[5],
                g:      data[6],
                b:      data[7],
            })
        } else {
            None
        }
    }
}

// ── Status notification ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub power: bool,
    pub mode:  u8,
    pub speed: u8,
    pub r:     u8,
    pub g:     u8,
    pub b:     u8,
}

impl std::fmt::Display for DeviceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Power: {}  Mode: {:#04x}  Speed: {}  RGB: ({}, {}, {})",
            if self.power { "ON" } else { "OFF" },
            self.mode, self.speed, self.r, self.g, self.b
        )
    }
}

// ── Color utilities ────────────────────────────────────────────────────────────

/// HSV → RGB (all values 0.0–1.0, returns 0–255 per channel)
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h as u32;
    let f = h - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Linearly interpolate two RGB colors by factor t (0.0–1.0)
pub fn lerp_color(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

/// Convert color temperature (Kelvin, ~1000–10000) to RGB.
pub fn cct_to_rgb(kelvin: u32) -> (u8, u8, u8) {
    let temp = kelvin as f32 / 100.0;
    let r: f32 = if temp <= 66.0 {
        255.0
    } else {
        (329.698_727 * (temp - 60.0).powf(-0.133_204_759)).clamp(0.0, 255.0)
    };
    let g: f32 = if temp <= 66.0 {
        (99.470_802 * temp.ln() - 161.119_568).clamp(0.0, 255.0)
    } else {
        (288.122_169 * (temp - 60.0).powf(-0.075_514_849)).clamp(0.0, 255.0)
    };
    let b: f32 = if temp >= 66.0 {
        255.0
    } else if temp <= 19.0 {
        0.0
    } else {
        (138.517_731 * (temp - 10.0).ln() - 305.044_793).clamp(0.0, 255.0)
    };
    (r as u8, g as u8, b as u8)
}
