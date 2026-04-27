/// Lotus LED — web GUI server.
///
/// Starts an HTTP server on localhost:7483, opens the browser, and
/// exposes a JSON API + WebSocket for controlling BLEDOM strips.
///
/// Build: cargo build --release --target x86_64-pc-windows-gnu --features gui
use std::{
    net::SocketAddr,
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::Duration,
};
use axum::{
    Router,
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    http::{Method, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex, mpsc};
use tower_http::cors::{Any, CorsLayer};

use lotus_led::{
    BLEDOMDevice, Config,
    modes::{ModeConfig, AppWatchRule, SequenceStep, run_mode},
};

const PORT: u16 = 7483;
const HTML: &str = include_str!("../gui/index.html");

// ── Mode runner thread command ────────────────────────────────────────────────

struct ModeCmd {
    config: ModeConfig,
    device: Arc<BLEDOMDevice>,
    flag:   Arc<AtomicBool>,
}

// ── App state ─────────────────────────────────────────────────────────────────

struct AppState {
    device:          Option<Arc<BLEDOMDevice>>,
    config:          Config,
    mode_stop:       Option<Arc<AtomicBool>>,
    mode_tx:         mpsc::Sender<ModeCmd>,
    tx:              broadcast::Sender<WsEvent>,
    gui_state:       serde_json::Value,
    gui_state_path:  std::path::PathBuf,
}

type Shared = Arc<Mutex<AppState>>;

// ── WebSocket event ───────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
enum WsEvent {
    Connected   { mac: String },
    Disconnected,
    Status(StatusPayload),
    Error       { message: String },
}

#[derive(Clone, Serialize)]
struct StatusPayload {
    power: bool, mode: u8, speed: u8,
    r: u8, g: u8, b: u8,
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)] struct ConnectReq { mac: Option<String> }
#[derive(Deserialize)] struct ColorReq   { r: u8, g: u8, b: u8 }
#[derive(Deserialize)] struct LevelReq   { level: u8 }
#[derive(Deserialize)] struct SceneReq   { name: String }
#[derive(Deserialize)] struct ModeReq    { name: String, #[serde(flatten)] extra: serde_json::Value }

#[derive(Serialize)] struct OkResp  { ok: bool, #[serde(skip_serializing_if="Option::is_none")] mac: Option<String> }
#[derive(Serialize)] struct ErrResp { ok: bool, error: String }
#[derive(Serialize)] struct ScanResp { devices: Vec<FoundDev> }
#[derive(Serialize)] struct StatusResp { status: Option<StatusPayload> }
#[derive(Serialize)] struct FoundDev { name: String, address: String }

fn ok(mac: Option<String>) -> Json<OkResp> { Json(OkResp { ok: true, mac }) }
fn err_resp(msg: impl Into<String>) -> (StatusCode, Json<ErrResp>) {
    (StatusCode::BAD_REQUEST, Json(ErrResp { ok: false, error: msg.into() }))
}

// ── Mode runner thread ────────────────────────────────────────────────────────
//
// cpal::Stream is !Send, so we can't use tokio::spawn.
// Solution: a dedicated OS thread with its own current_thread runtime + LocalSet.
// Axum handlers send ModeCmd over an mpsc channel into this thread.

fn start_mode_thread(event_tx: broadcast::Sender<WsEvent>) -> mpsc::Sender<ModeCmd> {
    let (tx, mut rx) = mpsc::channel::<ModeCmd>(4);

    std::thread::Builder::new()
        .name("led-mode".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("mode thread runtime");

            let local = tokio::task::LocalSet::new();
            rt.block_on(local.run_until(async move {
                while let Some(cmd) = rx.recv().await {
                    let (mc, dev, flag) = (cmd.config, cmd.device, cmd.flag);
                    let etx = event_tx.clone();
                    tokio::task::spawn_local(async move {
                        if let Err(e) = run_mode(mc, dev, flag).await {
                            let _ = etx.send(WsEvent::Error { message: e.to_string() });
                        }
                    });
                }
            }));
        })
        .expect("spawn mode thread");

    tx
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cfg_path       = Config::default_path();
    let config         = Config::load(&cfg_path).unwrap_or_default();
    let gui_state_path = cfg_path.with_file_name("gui_state.json");
    let gui_state      = std::fs::read_to_string(&gui_state_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let (tx, _)  = broadcast::channel::<WsEvent>(64);
    let mode_tx  = start_mode_thread(tx.clone());

    let state: Shared = Arc::new(Mutex::new(AppState {
        device: None, config, mode_stop: None, mode_tx, tx,
        gui_state, gui_state_path,
    }));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/",               get(serve_html))
        .route("/ws",             get(ws_handler))
        .route("/api/scan",       get(api_scan))
        .route("/api/connect",    post(api_connect))
        .route("/api/disconnect", post(api_disconnect))
        .route("/api/on",         post(api_on))
        .route("/api/off",        post(api_off))
        .route("/api/color",      post(api_color))
        .route("/api/brightness", post(api_brightness))
        .route("/api/speed",      post(api_speed))
        .route("/api/mode",       post(api_mode))
        .route("/api/stop",       post(api_stop))
        .route("/api/scene",      post(api_scene))
        .route("/api/status",     get(api_status))
        .route("/api/state",      get(api_get_state).post(api_save_state))
        .with_state(state)
        .layer(cors);

    let addr     = SocketAddr::from(([127, 0, 0, 1], PORT));
    let listener = tokio::net::TcpListener::bind(addr).await
        .expect("Cannot bind port 7483 — is it already in use?");

    println!("Lotus LED GUI  →  http://localhost:{PORT}");
    println!("Press Ctrl-C to stop.");
    open_browser(PORT);

    axum::serve(listener, app).await.unwrap();
}

fn open_browser(port: u16) {
    let url = format!("http://localhost:{port}");
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("explorer").arg(&url).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(&url).spawn(); }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(&url).spawn(); }
}

// ── Route handlers ────────────────────────────────────────────────────────────

async fn serve_html() -> Html<&'static str> { Html(HTML) }

async fn ws_handler(
    ws:             WebSocketUpgrade,
    State(state):   State<Shared>,
) -> impl IntoResponse {
    let rx = state.lock().await.tx.subscribe();
    ws.on_upgrade(|socket| ws_task(socket, rx))
}

async fn ws_task(mut socket: WebSocket, mut rx: broadcast::Receiver<WsEvent>) {
    loop {
        tokio::select! {
            Ok(event) = rx.recv() => {
                let json = serde_json::to_string(&event).unwrap_or_default();
                if socket.send(Message::Text(json.into())).await.is_err() { break; }
            }
            Some(Ok(msg)) = socket.recv() => {
                if let Message::Close(_) = msg { break; }
            }
            else => break,
        }
    }
}

async fn api_scan(State(state): State<Shared>) -> impl IntoResponse {
    let timeout = Duration::from_secs_f32(
        state.lock().await.config.device.scan_timeout_secs
    );
    match BLEDOMDevice::scan(timeout).await {
        Ok(found) => Json(ScanResp {
            devices: found.iter().map(|d| FoundDev {
                name:    d.name.clone(),
                address: d.address.clone(),
            }).collect()
        }).into_response(),
        Err(e) => err_resp(e.to_string()).into_response(),
    }
}

async fn api_connect(
    State(state): State<Shared>,
    body:         Option<Json<ConnectReq>>,
) -> impl IntoResponse {
    let mac     = body.and_then(|b| b.mac.clone()).unwrap_or_default();
    let timeout = Duration::from_secs_f32(
        state.lock().await.config.device.scan_timeout_secs
    );
    match BLEDOMDevice::connect(&mac, timeout).await {
        Ok(dev) => {
            let address = dev.address();
            let dev     = Arc::new(dev);
            let mut s   = state.lock().await;
            s.device    = Some(dev);
            let _ = s.tx.send(WsEvent::Connected { mac: address.clone() });
            ok(Some(address)).into_response()
        }
        Err(e) => err_resp(e.to_string()).into_response(),
    }
}

async fn api_disconnect(State(state): State<Shared>) -> impl IntoResponse {
    let mut s = state.lock().await;
    stop_mode_inner(&mut s);
    if let Some(dev) = s.device.take() {
        drop(s); // release lock before awaiting
        let _ = dev.disconnect().await;
    }
    let mut s = state.lock().await;
    let _ = s.tx.send(WsEvent::Disconnected);
    ok(None)
}

async fn api_on(State(state): State<Shared>) -> impl IntoResponse {
    with_device(state, |dev| async move { dev.power_on().await }).await
}
async fn api_off(State(state): State<Shared>) -> impl IntoResponse {
    with_device(state, |dev| async move { dev.power_off().await }).await
}
async fn api_color(
    State(state): State<Shared>,
    Json(req):    Json<ColorReq>,
) -> impl IntoResponse {
    let (r, g, b) = (req.r, req.g, req.b);
    with_device(state, move |dev| async move {
        dev.power_on().await?;
        dev.set_color(r, g, b).await
    }).await
}
async fn api_brightness(
    State(state): State<Shared>,
    Json(req):    Json<LevelReq>,
) -> impl IntoResponse {
    let level = req.level;
    with_device(state, move |dev| async move { dev.set_brightness(level).await }).await
}
async fn api_speed(
    State(state): State<Shared>,
    Json(req):    Json<LevelReq>,
) -> impl IntoResponse {
    let level = req.level;
    with_device(state, move |dev| async move { dev.set_speed(level).await }).await
}

async fn api_stop(State(state): State<Shared>) -> impl IntoResponse {
    stop_mode_inner(&mut *state.lock().await);
    ok(None)
}

async fn api_scene(
    State(state): State<Shared>,
    Json(req):    Json<SceneReq>,
) -> impl IntoResponse {
    let (dev, cfg) = {
        let s = state.lock().await;
        (s.device.clone(), s.config.clone())
    };
    let Some(dev) = dev else { return err_resp("Not connected").into_response(); };
    let Some(scene) = cfg.scenes.get(&req.name).cloned() else {
        return err_resp(format!("Unknown scene '{}'", req.name)).into_response();
    };
    if let Some(b) = scene.brightness { let _ = dev.set_brightness(b).await; }
    if let Some(c) = scene.color      { let _ = dev.set_color(c[0], c[1], c[2]).await; }
    if let Some(mode_name) = &scene.mode {
        if let Some(mc) = ModeConfig::from_name(mode_name, &cfg) {
            launch_mode(&state, dev, mc).await;
        }
    }
    ok(None).into_response()
}

async fn api_mode(
    State(state): State<Shared>,
    Json(req):    Json<ModeReq>,
) -> impl IntoResponse {
    let (dev, cfg) = {
        let s = state.lock().await;
        (s.device.clone(), s.config.clone())
    };
    let Some(dev) = dev else { return err_resp("Not connected").into_response(); };
    let Some(mc) = build_mode(&req.name, &req.extra, &cfg) else {
        return err_resp(format!("Unknown mode '{}'", req.name)).into_response();
    };
    launch_mode(&state, dev, mc).await;
    ok(None).into_response()
}

async fn api_status(State(state): State<Shared>) -> impl IntoResponse {
    let dev = state.lock().await.device.clone();
    let Some(dev) = dev else { return Json(StatusResp { status: None }); };
    let mut rx = dev.status_receiver();
    let status = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await.ok().and_then(|r| r.ok())
        .map(|ds| StatusPayload { power: ds.power, mode: ds.mode, speed: ds.speed,
                                   r: ds.r, g: ds.g, b: ds.b });
    Json(StatusResp { status })
}

async fn api_get_state(State(state): State<Shared>) -> impl IntoResponse {
    Json(state.lock().await.gui_state.clone())
}

async fn api_save_state(
    State(state): State<Shared>,
    Json(body):   Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut s = state.lock().await;
    s.gui_state = body.clone();
    let path   = s.gui_state_path.clone();
    drop(s);
    if let Ok(json) = serde_json::to_string_pretty(&body) {
        let _ = std::fs::write(&path, json);
    }
    ok(None)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn stop_mode_inner(s: &mut AppState) {
    if let Some(flag) = s.mode_stop.take() {
        flag.store(false, Ordering::Relaxed);
    }
}

async fn launch_mode(state: &Shared, dev: Arc<BLEDOMDevice>, mc: ModeConfig) {
    let mut s    = state.lock().await;
    stop_mode_inner(&mut s);
    let flag     = Arc::new(AtomicBool::new(true));
    s.mode_stop  = Some(flag.clone());
    let tx       = s.mode_tx.clone();
    drop(s);
    let _ = dev.power_on().await;  // ensure device is on (Sunset/SleepTimer may have turned it off)
    let _ = tx.send(ModeCmd { config: mc, device: dev, flag }).await;
}

async fn with_device<F, Fut>(state: Shared, f: F) -> impl IntoResponse
where
    F: FnOnce(Arc<BLEDOMDevice>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let dev = state.lock().await.device.clone();
    match dev {
        None      => err_resp("Not connected").into_response(),
        Some(dev) => match f(dev).await {
            Ok(_)  => ok(None).into_response(),
            Err(e) => err_resp(e.to_string()).into_response(),
        }
    }
}

fn build_mode(name: &str, extra: &serde_json::Value, cfg: &Config) -> Option<ModeConfig> {
    if name == "sequence" {
        let steps = extra.get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| {
                let dur   = s.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                let off   = s.get("off").and_then(|v| v.as_bool()).unwrap_or(false);
                let color = s.get("color").and_then(|v| v.as_array()).and_then(|a| {
                    let n: Vec<u8> = a.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect();
                    if n.len() == 3 { Some([n[0], n[1], n[2]]) } else { None }
                });
                Some(SequenceStep { duration_secs: dur, off, color,
                                    brightness: None, hw_mode: None, hw_speed: None, raw: None })
            }).collect())
            .unwrap_or_default();
        let loop_forever = extra.get("loop").and_then(|v| v.as_bool()).unwrap_or(true);
        return Some(ModeConfig::Sequence { steps, loop_forever });
    }
    if name == "appwatch" || name == "app_watch" {
        let rules = extra.get("rules")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|r| {
                let process    = r.get("process").and_then(|v| v.as_str())?.to_string();
                let red        = r.get("r").and_then(|v| v.as_u64()).unwrap_or(128) as u8;
                let green      = r.get("g").and_then(|v| v.as_u64()).unwrap_or(128) as u8;
                let blue       = r.get("b").and_then(|v| v.as_u64()).unwrap_or(128) as u8;
                let brightness = r.get("brightness").and_then(|v| v.as_u64()).map(|n| n as u8);
                Some(AppWatchRule { process, r: red, g: green, b: blue, brightness })
            }).collect())
            .unwrap_or_default();
        return Some(ModeConfig::AppWatch {
            rules, default_r: 80, default_g: 80, default_b: 80, check_ms: 1000,
        });
    }

    let mut mc = ModeConfig::from_name(name, cfg)?;

    // Apply extra JSON params sent from the GUI
    let f32p = |k: &str| extra.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
    let u8p  = |k: &str| extra.get(k).and_then(|v| v.as_u64()).map(|v| v as u8);
    let u32p = |k: &str| extra.get(k).and_then(|v| v.as_u64()).map(|v| v as u32);
    let rgbp = |k: &str| -> Option<[u8; 3]> {
        extra.get(k).and_then(|v| v.as_array()).and_then(|a| {
            if a.len() == 3 {
                Some([a[0].as_u64().unwrap_or(0) as u8,
                      a[1].as_u64().unwrap_or(0) as u8,
                      a[2].as_u64().unwrap_or(0) as u8])
            } else { None }
        })
    };

    match &mut mc {
        ModeConfig::Static { r, g, b } => {
            if let Some([cr,cg,cb]) = rgbp("color") { *r=cr; *g=cg; *b=cb; }
        }
        ModeConfig::Pulse { r, g, b, period_secs, fps, .. } => {
            if let Some(v) = f32p("period") { *period_secs = v; }
            if let Some(v) = u8p("fps")    { *fps = v; }
            if let Some([cr,cg,cb]) = rgbp("color") { *r=cr; *g=cg; *b=cb; }
        }
        ModeConfig::Rainbow { cycle_secs, fps, .. } | ModeConfig::Wave { cycle_secs, fps, .. } => {
            if let Some(v) = f32p("period") { *cycle_secs = v; }
            if let Some(v) = u8p("fps")    { *fps = v; }
        }
        ModeConfig::Fire { fps, intensity } => {
            if let Some(v) = u8p("fps")        { *fps = v; }
            if let Some(v) = f32p("intensity") { *intensity = v; }
        }
        ModeConfig::Meteor { r, g, b, fps } => {
            if let Some(v) = u8p("fps") { *fps = v; }
            if let Some([cr,cg,cb]) = rgbp("color") { *r=cr; *g=cg; *b=cb; }
        }
        ModeConfig::Comet { fps } => {
            if let Some(v) = u8p("fps") { *fps = v; }
        }
        ModeConfig::Sunrise    { duration_secs, fps }
        | ModeConfig::Sunset   { duration_secs, fps }
        | ModeConfig::SleepTimer { duration_secs, fps }
        | ModeConfig::WakeUp   { duration_secs, fps } => {
            if let Some(v) = u32p("duration") { *duration_secs = v; }
            if let Some(v) = u8p("fps")       { *fps = v; }
        }
        ModeConfig::Cct { kelvin, brightness } => {
            if let Some(v) = u32p("temp")       { *kelvin = v; }
            if let Some(v) = u8p("brightness")  { *brightness = v; }
        }
        ModeConfig::Alarm { r, g, b, flash_count, .. } => {
            if let Some([cr,cg,cb]) = rgbp("color") { *r=cr; *g=cg; *b=cb; }
            if let Some(v) = u32p("count") { *flash_count = v; }
        }
        ModeConfig::Hardware { speed, .. } => {
            if let Some(v) = u8p("speed") { *speed = v; }
        }
        ModeConfig::MicHardware { sensitivity } => {
            if let Some(v) = u8p("sensitivity") { *sensitivity = v; }
        }
        ModeConfig::Audio { sensitivity, fps, .. }
        | ModeConfig::Music { sensitivity, fps, .. } => {
            if let Some(v) = f32p("sensitivity") { *sensitivity = v; }
            if let Some(v) = u8p("fps")          { *fps = v; }
        }
        ModeConfig::Ambient { fps, .. } => {
            if let Some(v) = u8p("fps") { *fps = v; }
        }
        ModeConfig::SysMonitor { fps, .. } => {
            if let Some(v) = u8p("fps") { *fps = v; }
        }
        _ => {}
    }

    Some(mc)
}
