/// Lotus LED — web GUI server.
///
/// Starts an HTTP server on localhost:7483, opens the browser, and
/// exposes a JSON API + WebSocket for controlling BLEDOM strips.
///
/// Build: cargo build --release --target x86_64-pc-windows-gnu --features gui
use std::{
    net::SocketAddr,
    sync::Arc,
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
use tokio::sync::{broadcast, Mutex};
use tower_http::cors::{Any, CorsLayer};

use lotus_led::{
    BLEDOMDevice, Config, Packet, HWMode,
    modes::{ModeConfig, AppWatchRule, SequenceStep, run_mode},
};

const PORT: u16 = 7483;
const HTML: &str = include_str!("../gui/index.html");

// ── App state ─────────────────────────────────────────────────────────────────

struct AppState {
    device: Option<Arc<BLEDOMDevice>>,
    config: Config,
    /// cancel handle for the currently running software mode
    mode_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// broadcast channel for WebSocket events
    tx: broadcast::Sender<WsEvent>,
}

type Shared = Arc<Mutex<AppState>>;

// ── WebSocket event ───────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
enum WsEvent {
    Connected { mac: String },
    Disconnected,
    Status(StatusPayload),
    Error { message: String },
}

#[derive(Clone, Serialize)]
struct StatusPayload {
    power: bool,
    mode:  u8,
    speed: u8,
    r: u8, g: u8, b: u8,
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)] struct ConnectReq  { mac: Option<String> }
#[derive(Deserialize)] struct ColorReq    { r: u8, g: u8, b: u8 }
#[derive(Deserialize)] struct LevelReq    { level: u8 }
#[derive(Deserialize)] struct SceneReq    { name: String }

#[derive(Deserialize)]
struct ModeReq {
    name:  String,
    /// Override fields forwarded as-is to mode config
    #[serde(flatten)]
    extra: serde_json::Value,
}

#[derive(Serialize)]
struct OkResp   { ok: bool, #[serde(skip_serializing_if="Option::is_none")] mac: Option<String> }
#[derive(Serialize)]
struct ErrResp  { ok: bool, error: String }
#[derive(Serialize)]
struct ScanResp { devices: Vec<FoundDev> }
#[derive(Serialize)]
struct StatusResp { status: Option<StatusPayload> }

#[derive(Serialize)]
struct FoundDev { name: String, address: String }

fn ok(mac: Option<String>) -> Json<OkResp> {
    Json(OkResp { ok: true, mac })
}
fn err(msg: impl Into<String>) -> (StatusCode, Json<ErrResp>) {
    (StatusCode::BAD_REQUEST, Json(ErrResp { ok: false, error: msg.into() }))
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let local = tokio::task::LocalSet::new();
    local.run_until(run_server()).await;
}

async fn run_server() {
    let cfg_path = Config::default_path();
    let config   = Config::load(&cfg_path).unwrap_or_default();

    let (tx, _) = broadcast::channel::<WsEvent>(64);

    let state: Shared = Arc::new(Mutex::new(AppState {
        device: None,
        config,
        mode_stop: None,
        tx: tx.clone(),
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
        .with_state(state)
        .layer(cors);

    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
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
    ws: WebSocketUpgrade,
    State(state): State<Shared>,
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
                // ping/pong keepalive
                if let Message::Close(_) = msg { break; }
            }
            else => break,
        }
    }
}

async fn api_scan(State(state): State<Shared>) -> impl IntoResponse {
    let timeout = {
        let s = state.lock().await;
        Duration::from_secs_f32(s.config.device.scan_timeout_secs)
    };
    match BLEDOMDevice::scan(timeout).await {
        Ok(found) => Json(ScanResp {
            devices: found.iter().map(|d| FoundDev {
                name:    d.name.clone(),
                address: d.address.clone(),
            }).collect()
        }).into_response(),
        Err(e) => err(e.to_string()).into_response(),
    }
}

async fn api_connect(
    State(state): State<Shared>,
    body: Option<Json<ConnectReq>>,
) -> impl IntoResponse {
    let mac = body.and_then(|b| b.mac.clone()).unwrap_or_default();
    let timeout = {
        let s = state.lock().await;
        Duration::from_secs_f32(s.config.device.scan_timeout_secs)
    };
    match BLEDOMDevice::connect(&mac, timeout).await {
        Ok(dev) => {
            let address = dev.address().to_string();
            let dev = Arc::new(dev);
            let mut s = state.lock().await;
            s.device = Some(dev);
            let _ = s.tx.send(WsEvent::Connected { mac: address.clone() });
            ok(Some(address)).into_response()
        }
        Err(e) => err(e.to_string()).into_response(),
    }
}

async fn api_disconnect(State(state): State<Shared>) -> impl IntoResponse {
    let mut s = state.lock().await;
    stop_mode_inner(&mut s).await;
    if let Some(dev) = s.device.take() {
        let _ = dev.disconnect().await;
    }
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
    Json(req): Json<ColorReq>,
) -> impl IntoResponse {
    let (r, g, b) = (req.r, req.g, req.b);
    with_device(state, move |dev| async move {
        dev.set_color(r, g, b).await
    }).await
}

async fn api_brightness(
    State(state): State<Shared>,
    Json(req): Json<LevelReq>,
) -> impl IntoResponse {
    let level = req.level;
    with_device(state, move |dev| async move { dev.set_brightness(level).await }).await
}

async fn api_speed(
    State(state): State<Shared>,
    Json(req): Json<LevelReq>,
) -> impl IntoResponse {
    let level = req.level;
    with_device(state, move |dev| async move { dev.set_speed(level).await }).await
}

async fn api_stop(State(state): State<Shared>) -> impl IntoResponse {
    let mut s = state.lock().await;
    stop_mode_inner(&mut s).await;
    ok(None)
}

async fn api_scene(
    State(state): State<Shared>,
    Json(req): Json<SceneReq>,
) -> impl IntoResponse {
    let (dev, cfg) = {
        let s = state.lock().await;
        (s.device.clone(), s.config.clone())
    };
    let Some(dev) = dev else { return err("Not connected").into_response(); };

    let Some(scene) = cfg.scenes.get(&req.name).cloned() else {
        return err(format!("Unknown scene '{}'", req.name)).into_response();
    };

    if let Some(b) = scene.brightness {
        let _ = dev.set_brightness(b).await;
    }
    if let Some(c) = scene.color {
        let _ = dev.set_color(c[0], c[1], c[2]).await;
    }
    if let Some(mode_name) = &scene.mode {
        if let Some(mc) = ModeConfig::from_name(mode_name, &cfg) {
            launch_mode(state.clone(), dev, mc).await;
        }
    }
    ok(None).into_response()
}

async fn api_mode(
    State(state): State<Shared>,
    Json(req): Json<ModeReq>,
) -> impl IntoResponse {
    let (dev, cfg) = {
        let s = state.lock().await;
        (s.device.clone(), s.config.clone())
    };
    let Some(dev) = dev else { return err("Not connected").into_response(); };

    let mc = build_mode_config(&req.name, &req.extra, &cfg);
    let Some(mc) = mc else {
        return err(format!("Unknown mode '{}'", req.name)).into_response();
    };

    launch_mode(state, dev, mc).await;
    ok(None).into_response()
}

async fn api_status(State(state): State<Shared>) -> impl IntoResponse {
    let s = state.lock().await;
    let Some(dev) = s.device.clone() else {
        return Json(StatusResp { status: None });
    };
    drop(s);
    let mut rx = dev.status_receiver();
    let status = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .ok()
        .and_then(|r| r.ok())
        .map(|ds| StatusPayload {
            power: ds.power, mode: ds.mode, speed: ds.speed,
            r: ds.r, g: ds.g, b: ds.b,
        });
    Json(StatusResp { status })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn stop_mode_inner(s: &mut AppState) {
    if let Some(flag) = s.mode_stop.take() {
        flag.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

async fn launch_mode(state: Shared, dev: Arc<BLEDOMDevice>, mc: ModeConfig) {
    let mut s = state.lock().await;
    stop_mode_inner(&mut s).await;

    let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
    s.mode_stop = Some(flag.clone());
    drop(s);

    tokio::task::spawn_local(async move {
        let _ = run_mode(mc, dev, flag).await;
    });
}

async fn with_device<F, Fut>(state: Shared, f: F) -> impl IntoResponse
where
    F: FnOnce(Arc<BLEDOMDevice>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let dev = state.lock().await.device.clone();
    match dev {
        None      => err("Not connected").into_response(),
        Some(dev) => match f(dev).await {
            Ok(_)  => ok(None).into_response(),
            Err(e) => err(e.to_string()).into_response(),
        }
    }
}

/// Build a ModeConfig from the mode name + extra JSON params from the request.
fn build_mode_config(name: &str, extra: &serde_json::Value, cfg: &Config) -> Option<ModeConfig> {
    // Handle sequence mode with inline steps from the GUI
    if name == "sequence" {
        let steps = extra.get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| {
                let dur = s.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                let off = s.get("off").and_then(|v| v.as_bool()).unwrap_or(false);
                let color = s.get("color").and_then(|v| v.as_array()).map(|a| {
                    let nums: Vec<u8> = a.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect();
                    if nums.len() == 3 { Some([nums[0], nums[1], nums[2]]) } else { None }
                }).flatten();
                Some(SequenceStep {
                    duration_secs: dur, off,
                    color, brightness: None, hw_mode: None, hw_speed: None, raw: None,
                })
            }).collect())
            .unwrap_or_default();
        let loop_forever = extra.get("loop").and_then(|v| v.as_bool()).unwrap_or(true);
        return Some(ModeConfig::Sequence { steps, loop_forever });
    }

    // Handle appwatch with inline rules
    if name == "appwatch" || name == "app_watch" {
        let rules = extra.get("rules")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|r| {
                let process = r.get("process").and_then(|v| v.as_str())?.to_string();
                let red   = r.get("r").and_then(|v| v.as_u64()).unwrap_or(128) as u8;
                let green = r.get("g").and_then(|v| v.as_u64()).unwrap_or(128) as u8;
                let blue  = r.get("b").and_then(|v| v.as_u64()).unwrap_or(128) as u8;
                let brightness = r.get("brightness").and_then(|v| v.as_u64()).map(|n| n as u8);
                Some(AppWatchRule { process, r: red, g: green, b: blue, brightness })
            }).collect())
            .unwrap_or_default();
        return Some(ModeConfig::AppWatch {
            rules, default_r: 80, default_g: 80, default_b: 80, check_ms: 1000,
        });
    }

    ModeConfig::from_name(name, cfg)
}
