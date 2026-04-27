/// Lotus LED Controller CLI — compiles to `led.exe` on Windows.
///
/// Build for Windows:  cargo build --release --target x86_64-pc-windows-gnu
/// Copy to Windows:    cp target/x86_64-pc-windows-gnu/release/led.exe
///                        /mnt/c/Users/Alex_Melan/Desktop/Tests/
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use anyhow::Result;
use clap::{Parser, Subcommand, Args as ClapArgs};

use lotus_led::{BLEDOMDevice, ModeConfig, Config, Packet, HWMode};
use lotus_led::modes::run_mode;

// ══════════════════════════════════════════════════════════════════════════════
// CLI DEFINITIONS
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Parser)]
#[command(
    name    = "led",
    version = "2.0.0",
    about   = "Lotus LED Controller — BLEDOM / ELK-BLEDOM / Lotus Lantern",
    long_about = None,
)]
struct Cli {
    /// Device MAC address (overrides config.json)
    #[arg(long, global = true)]
    mac: Option<String>,

    /// Path to config.json
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    /// Scan timeout in seconds
    #[arg(long, global = true, default_value = "6")]
    scan_timeout: f32,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan for BLEDOM devices and print MAC addresses
    Scan {
        /// Scan duration in seconds
        #[arg(default_value = "6")]
        timeout: f32,
    },

    /// Power on
    On,

    /// Power off
    Off,

    /// Set a static RGB color
    ///
    /// Examples:
    ///   led color 255 0 128
    ///   led color ff0080     (hex, no #)
    Color {
        #[arg(required = true, num_args = 1..=3)]
        value: Vec<String>,
    },

    /// Set brightness 0–100
    Brightness {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        level: u8,
    },

    /// Set speed 0–100 (hardware animation modes)
    Speed {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        level: u8,
    },

    /// Start an animation mode (Ctrl-C to stop, or use --run N)
    ///
    /// Software:  static pulse rainbow wave fire meteor comet
    ///            sunrise sunset cct sleep_timer alarm
    /// Hardware:  hw jump_7_color fade_red fade_green fade_blue fade_yellow
    ///            fade_cyan fade_purple fade_white cross_red_green
    ///            cross_red_blue cross_green_blue strobe_7_color
    ///            strobe_red strobe_green strobe_blue strobe_yellow
    ///            strobe_cyan strobe_purple strobe_white fade_7_color
    ///            mic_hw
    /// Reactive:  audio music ambient system notify
    Mode(ModeArgs),

    /// Apply a scene preset (movie party romance relax focus gaming chill)
    Scene {
        name: String,
    },

    /// Read device firmware version and last known status
    Status,

    /// List all available modes
    Modes,

    /// Print current config path and contents
    Config,

    /// Write default config.json next to the binary
    InitConfig,

    /// Set RGB pin sequence (1–6, use if R and G are swapped)
    ColorOrder {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=6))]
        order: u8,
    },
}

#[derive(ClapArgs)]
struct ModeArgs {
    /// Mode name
    name: String,

    /// Speed for hardware modes (0–100)
    #[arg(long)]
    speed: Option<u8>,

    /// Target FPS for software modes (1–60)
    #[arg(long)]
    fps: Option<u8>,

    /// Period in seconds (pulse, wave)
    #[arg(long)]
    period: Option<f32>,

    /// Color temperature in Kelvin (cct mode, 1800–10000)
    #[arg(long)]
    temp: Option<u32>,

    /// Duration in seconds (sunrise, sunset, sleep_timer)
    #[arg(long)]
    duration: Option<u32>,

    /// Audio sensitivity multiplier (audio, music)
    #[arg(long)]
    sensitivity: Option<f32>,

    /// Color as R G B (0–255 each)
    #[arg(long, num_args = 3)]
    color: Option<Vec<u8>>,

    /// Run for N seconds then exit (omit to run until Ctrl-C)
    #[arg(long)]
    run: Option<f32>,
}

// ══════════════════════════════════════════════════════════════════════════════
// HELPERS
// ══════════════════════════════════════════════════════════════════════════════

fn parse_color_arg(args: &[String]) -> Result<(u8, u8, u8)> {
    if args.len() == 3 {
        return Ok((args[0].parse()?, args[1].parse()?, args[2].parse()?));
    }
    // Single hex string (with or without #)
    let hex = args[0].trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16)?;
        let g = u8::from_str_radix(&hex[2..4], 16)?;
        let b = u8::from_str_radix(&hex[4..6], 16)?;
        return Ok((r, g, b));
    }
    anyhow::bail!("Color must be three integers (R G B) or a 6-digit hex string")
}

fn make_cancellation_pair() -> (Arc<AtomicBool>, impl FnOnce()) {
    let running = Arc::new(AtomicBool::new(true));
    let r2 = running.clone();
    let stopper = move || r2.store(false, Ordering::Relaxed);
    (running, stopper)
}

fn print_modes() {
    let software = [
        ("static",      "Solid static color"),
        ("pulse",       "Breathing / pulse (breathe alias)"),
        ("rainbow",     "Full-spectrum HSV cycle"),
        ("wave",        "Hue wave oscillation"),
        ("fire",        "Warm flickering fire"),
        ("meteor",      "Meteor burst"),
        ("comet",       "Sparkling comet"),
        ("sunrise",     "Gradual warm sunrise (--duration secs)"),
        ("sunset",      "Sunset fade to off"),
        ("cct",         "Color temperature in Kelvin (--temp K)"),
        ("sleep_timer", "Dim to off (--duration secs)"),
        ("alarm",       "Flash alarm"),
        ("notify",      "Notification flash"),
    ];
    let reactive = [
        ("audio",   "Mic FFT -> RGB blend (requires --features audio)"),
        ("music",   "Beat detection sync  (requires --features audio)"),
        ("ambient", "Screen Ambilight     (requires --features ambient)"),
        ("system",  "CPU/RAM heatmap      (requires --features system)"),
    ];
    println!("Software modes:");
    for (n, d) in &software { println!("  {n:<16} {d}"); }
    println!("\nReactive modes:");
    for (n, d) in &reactive { println!("  {n:<16} {d}"); }
    println!("\nHardware modes:");
    for n in HWMode::all_names() { println!("  {n}"); }
    println!("  mic_hw            On-board microphone reactive");
    println!("\nScenes: movie party romance relax focus gaming chill");
}


// ══════════════════════════════════════════════════════════════════════════════
// MAIN
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let cfg_path = cli.config.clone().unwrap_or_else(Config::default_path);
    let mut cfg  = Config::load(&cfg_path)?;

    if let Some(mac) = &cli.mac {
        cfg.device.mac = mac.clone();
    }

    // Save default config on first run
    if !cfg_path.exists() {
        cfg.save(&cfg_path)?;
        println!("Created default config: {}", cfg_path.display());
    }

    // ── Scan (no connection needed) ───────────────────────────────────────────
    if let Commands::Scan { timeout } = &cli.command {
        println!("Scanning for BLEDOM devices ({timeout}s)...");
        let found = BLEDOMDevice::scan(Duration::from_secs_f32(*timeout)).await?;
        if found.is_empty() {
            println!("  No devices found.");
        } else {
            for d in &found {
                println!("  {d}");
            }
            println!("\nSet the MAC address in config.json to connect automatically:");
            println!("  \"mac\": \"{}\"", found[0].address);
        }
        return Ok(());
    }

    if let Commands::Modes = &cli.command {
        print_modes();
        return Ok(());
    }

    if let Commands::Config = &cli.command {
        println!("Config path: {}", cfg_path.display());
        if cfg_path.exists() {
            println!("{}", std::fs::read_to_string(&cfg_path)?);
        } else {
            println!("(not yet created — run 'led init-config')");
        }
        return Ok(());
    }

    if let Commands::InitConfig = &cli.command {
        Config::save_default(&cfg_path)?;
        println!("Written: {}", cfg_path.display());
        return Ok(());
    }

    // ── Connect ───────────────────────────────────────────────────────────────
    let identifier    = cfg.device.mac.clone();
    let scan_timeout  = Duration::from_secs_f32(cfg.device.scan_timeout_secs);

    println!("Connecting ({})...",
        if identifier.is_empty() { "auto-discover".to_string() } else { identifier.clone() });

    let device = Arc::new(BLEDOMDevice::connect(&identifier, scan_timeout).await?);
    let fw = device.read_firmware().await;
    println!("Connected!  Firmware: {fw}");

    // ── Ctrl-C handler ────────────────────────────────────────────────────────
    let (running, _stopper) = make_cancellation_pair();
    let r_ctrlc = running.clone();
    ctrlc::set_handler(move || {
        r_ctrlc.store(false, Ordering::Relaxed);
        println!("\nStopped.");
    }).ok();

    // ── Execute command ───────────────────────────────────────────────────────
    device.power_on().await?;

    match cli.command {
        Commands::On  => {}  // already powered on
        Commands::Off => { device.power_off().await?; }

        Commands::Status => {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let fw = device.read_firmware().await;
            println!("Firmware : {fw}");
            let mut rx = device.status_receiver();
            if let Ok(s) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                println!("Status   : {}", s?);
            } else {
                println!("Status   : (no notification received — send a command first)");
            }
        }

        Commands::Color { value } => {
            let (r, g, b) = parse_color_arg(&value)?;
            device.set_color(r, g, b).await?;
        }

        Commands::Brightness { level } => { device.set_brightness(level).await?; }
        Commands::Speed      { level } => { device.set_speed(level).await?; }

        Commands::ColorOrder { order } => {
            device.send(Packet::color_order(order)).await?;
            println!("Color order set to {order}");
        }

        Commands::Mode(m) => {
            // Build ModeConfig from args
            let mut mc = ModeConfig::from_name(&m.name, &cfg)
                .ok_or_else(|| anyhow::anyhow!("Unknown mode '{}'. Run 'led modes' for list.", m.name))?;

            // Apply any CLI overrides to the mode config
            apply_mode_overrides(&mut mc, &m);

            let run_secs = m.run;
            let dev = device.clone();
            let run_flag = running.clone();

            // cpal::Stream is !Send, so use LocalSet + spawn_local
            let local = tokio::task::LocalSet::new();
            let mode_fut = local.run_until(async move {
                tokio::task::spawn_local(async move {
                    if let Err(e) = run_mode(mc, dev, run_flag).await {
                        eprintln!("Mode error: {e}");
                    }
                }).await.ok()
            });

            if let Some(secs) = run_secs {
                tokio::select! {
                    _ = mode_fut => {}
                    _ = tokio::time::sleep(Duration::from_secs_f32(secs)) => {
                        running.store(false, Ordering::Relaxed);
                    }
                }
            } else {
                mode_fut.await;
            }
        }

        Commands::Scene { name } => {
            match cfg.scenes.get(&name).cloned() {
                Some(scene) => {
                    // For scene, temporarily clone device raw (workaround)
                    if let Some(b) = scene.brightness {
                        device.set_brightness(b).await?;
                    }
                    if let Some(c) = scene.color {
                        device.set_color(c[0], c[1], c[2]).await?;
                    }
                    if let Some(mode_name) = &scene.mode {
                        if mode_name == "hw" {
                            if let Some(hw) = scene.hw_mode {
                                device.set_hw_mode(hw, scene.speed.unwrap_or(50)).await?;
                            }
                        } else if let Some(mc) = ModeConfig::from_name(mode_name, &cfg) {
                            run_mode(mc, device.clone(), running.clone()).await?;
                        }
                    }
                }
                None => {
                    eprintln!("Unknown scene '{name}'. Available: {:?}", cfg.scenes.keys().collect::<Vec<_>>());
                }
            }
        }

        _ => {}
    }

    // Power off if the running flag was cleared (Ctrl-C or --run timer expired)
    if !running.load(Ordering::Relaxed) {
        device.power_off().await?;
    }

    device.disconnect().await?;
    Ok(())
}

// ── Apply CLI flag overrides to a ModeConfig ──────────────────────────────────

fn apply_mode_overrides(mc: &mut ModeConfig, m: &ModeArgs) {
    match mc {
        ModeConfig::Pulse { r, g, b, period_secs, fps, .. } => {
            if let Some(rgb) = &m.color { *r = rgb[0]; *g = rgb[1]; *b = rgb[2]; }
            if let Some(p) = m.period { *period_secs = p; }
            if let Some(f) = m.fps    { *fps = f; }
        }
        ModeConfig::Rainbow { cycle_secs, fps, .. } => {
            if let Some(p) = m.period { *cycle_secs = p; }
            if let Some(f) = m.fps    { *fps = f; }
        }
        ModeConfig::Wave { cycle_secs, fps, .. } => {
            if let Some(p) = m.period { *cycle_secs = p; }
            if let Some(f) = m.fps    { *fps = f; }
        }
        ModeConfig::Static { r, g, b } => {
            if let Some(rgb) = &m.color { *r = rgb[0]; *g = rgb[1]; *b = rgb[2]; }
        }
        ModeConfig::Cct { kelvin, .. } => {
            if let Some(t) = m.temp { *kelvin = t; }
        }
        ModeConfig::Sunrise { duration_secs, fps }
        | ModeConfig::Sunset  { duration_secs, fps }
        | ModeConfig::SleepTimer { duration_secs, fps } => {
            if let Some(d) = m.duration { *duration_secs = d; }
            if let Some(f) = m.fps      { *fps = f; }
        }
        ModeConfig::Hardware { speed, .. } => {
            if let Some(s) = m.speed { *speed = s; }
        }
        ModeConfig::Audio { sensitivity, fps, .. }
        | ModeConfig::Music { sensitivity, fps, .. } => {
            if let Some(s) = m.sensitivity { *sensitivity = s; }
            if let Some(f) = m.fps         { *fps = f; }
        }
        _ => {}
    }
}
