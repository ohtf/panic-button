// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The Panic Button desktop app: a thin shell around the embedded agent
//! (`panic-button-agent`). Rust owns behaviour; the webviews render the button,
//! sender panel, alert window and main window (`spec/05-desktop-agent.md`).
//!
//! Environment overrides, for testing several devices on one computer:
//! `PANIC_BUTTON_DATA_DIR`, `PANIC_BUTTON_CONFIG` and `PANIC_BUTTON_CONTROL_SOCKET`. With
//! `PANIC_BUTTON_DATA_DIR` set, the single-instance check is skipped.

pub mod audio;
mod placement;
mod platform;
mod session;
mod shortcut;
mod volume;
mod windows;

use std::path::{Path, PathBuf};

use panic_button::agent::config::{ConfigFile, UserDisplaySource};
use panic_button::agent::{AgentHandle, AgentOptions, paths};
use panic_button::core::io::{Input, UiEvent};
use panic_button::proto::{AudioStatus, SoundPattern};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

use crate::audio::{Audio, Cmd};

/// State shared by commands.
struct Shared {
    data_dir: PathBuf,
    control_socket: PathBuf,
    config: Mutex<ConfigFile>,
    allow_quit: bool,
    agent: Mutex<Option<AgentHandle>>,
    audio: Audio,
    audio_ok: bool,
    volume: volume::Volume,
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(|v| paths::expand(Path::new(&v)))
}

fn load_optional(p: &Path) -> anyhow::Result<ConfigFile> {
    if p.exists() {
        Ok(ConfigFile::load(p)?)
    } else {
        Ok(ConfigFile::default())
    }
}

/// Machine config with policy over it. The person's own room and label are
/// kept by the agent (ADR 0014).
fn load_config() -> anyhow::Result<ConfigFile> {
    let machine =
        load_optional(&env_path("PANIC_BUTTON_CONFIG").unwrap_or_else(paths::default_config_file))?;
    let policy = load_optional(&paths::default_policy_file())?;
    Ok(machine.with_policy(&policy))
}

/// The person's display name, as `device.user_display_source` asks
/// (`spec/04-security.md`, Privacy): the OS full name where there is one,
/// the user name, or none at all (colleagues then see this computer's name
/// or room instead).
fn user_display(source: UserDisplaySource) -> String {
    if source == UserDisplaySource::RoomOnly {
        return String::new();
    }
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    if source == UserDisplaySource::OsUsername {
        return user;
    }
    #[cfg(unix)]
    if let Ok(pw) = std::fs::read_to_string("/etc/passwd") {
        for line in pw.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() > 4 && f[0] == user {
                let full = f[4].split(',').next().unwrap_or("").trim();
                if !full.is_empty() {
                    return full.to_owned();
                }
            }
        }
    }
    user
}

/// Running as a Wayland client, where the compositor places windows and
/// global shortcuts need a portal. `GDK_BACKEND=x11` runs the app under
/// XWayland instead.
fn wayland() -> bool {
    cfg!(target_os = "linux")
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && !std::env::var("GDK_BACKEND").is_ok_and(|b| b.starts_with("x11"))
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

async fn start_agent(app: &AppHandle) -> Result<(), String> {
    let st = app.state::<Shared>();
    let mut slot = st.agent.lock().await;
    if slot.is_some() {
        return Ok(());
    }
    let config = st.config.lock().await.clone();
    let name_source = config.device.user_display_source.unwrap_or_default();
    let (h, _task) = panic_button::agent::start(AgentOptions {
        data_dir: st.data_dir.clone(),
        control_socket: st.control_socket.clone(),
        config,
        allow_raise: false,
        label: None,
        headless: false,
    })
    .await
    .map_err(err)?;
    h.input(Input::UserDisplay(user_display(name_source))).await;
    tauri::async_runtime::spawn(pump_events(app.clone(), h.clone()));
    tauri::async_runtime::spawn(pump_audio(h.clone(), st.audio.clone()));
    tauri::async_runtime::spawn(watch_volume(h.clone(), st.audio_ok, st.volume.clone()));
    tauri::async_runtime::spawn(pump_volume_override(h.clone(), st.volume.clone()));
    tauri::async_runtime::spawn(watch_session(h.clone()));
    *slot = Some(h);
    drop(slot);
    let _ = app.emit("panic-button-started", ());
    refresh_tray(app).await;
    Ok(())
}

async fn pump_events(app: AppHandle, h: AgentHandle) {
    use tokio::sync::broadcast::error::RecvError;
    let mut rx = h.events();
    loop {
        match rx.recv().await {
            Ok(e) => {
                match &e {
                    UiEvent::AlarmShown { alarm, .. } if !alarm.own => windows::show_alert(&app),
                    UiEvent::SenderPanel { .. } | UiEvent::DrillToast { .. } => {
                        windows::show_panel(&app)
                    }
                    UiEvent::JoinRequested { .. } => windows::show_main(&app),
                    _ => {}
                }
                if matches!(e, UiEvent::StatusChanged(_)) {
                    refresh_tray(&app).await;
                }
                let _ = app.emit("panic-button", panic_button::agent::views::ui_event(&e));
            }
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
    }
}

/// Tells the agent whether sound is muted or at zero, now and whenever it
/// changes (checked every 10 s). An unreadable level counts as not muted.
async fn watch_volume(h: AgentHandle, output_present: bool, volume: volume::Volume) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut last = None;
    let mut readable = None;
    loop {
        tick.tick().await;
        let level = volume.read().await;
        if readable != Some(level.is_some()) {
            readable = Some(level.is_some());
            if level.is_none() {
                tracing::info!("cannot read the sound level on this computer");
            }
        }
        let status = AudioStatus {
            output_present,
            muted: level.is_some_and(|l| l.muted),
            volume_pct: level.map_or(100, |l| l.volume_pct),
        };
        if last != Some(status) {
            tracing::info!(
                muted = status.muted,
                volume_pct = status.volume_pct,
                "sound level"
            );
            h.input(Input::AudioStatus {
                status,
                override_available: volume.override_available(),
            })
            .await;
            last = Some(status);
        }
    }
}

/// Tells the agent the session state (active, locked, remote), now and
/// whenever it changes (checked every 5 s).
async fn watch_session(h: AgentHandle) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    let mut last = None;
    loop {
        tick.tick().await;
        let s = session::read().await;
        if last != Some(s) {
            tracing::info!(session = ?s, "session");
            h.input(Input::SessionState(s)).await;
            last = Some(s);
        }
    }
}

/// The core asks for the mute override while an alarm sounds
/// (`alarm.override_mute`).
async fn pump_volume_override(h: AgentHandle, volume: volume::Volume) {
    use tokio::sync::broadcast::error::RecvError;
    let mut rx = h.volume_override();
    loop {
        match rx.recv().await {
            Ok(on) => volume.set_override(on),
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
    }
}

async fn pump_audio(h: AgentHandle, audio: Audio) {
    use tokio::sync::broadcast::error::RecvError;
    let mut rx = h.audio();
    loop {
        match rx.recv().await {
            Ok(c) => audio.send(Cmd::Agent(c)),
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
    }
}

/// The running agent, if this computer is set up.
async fn agent_handle(app: &AppHandle) -> Option<AgentHandle> {
    app.state::<Shared>().agent.lock().await.clone()
}

async fn agent(st: &State<'_, Shared>) -> Result<AgentHandle, String> {
    st.agent
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Panic Button is not set up on this computer yet".to_owned())
}

#[derive(Debug, Serialize)]
struct AppInfo {
    enrolled: bool,
    running: bool,
    version: &'static str,
    data_dir: String,
    allow_quit: bool,
    audio_ok: bool,
    wayland: bool,
    token_configured: bool,
    home_dir: String,
    sounds_dir: String,
    shortcut: Option<shortcut::Info>,
}

#[tauri::command]
async fn app_info(app: AppHandle, st: State<'_, Shared>) -> Result<AppInfo, String> {
    let running = st.agent.lock().await.is_some();
    let cfg = st.config.lock().await;
    Ok(AppInfo {
        enrolled: panic_button::agent::setup::is_enrolled(&st.data_dir).map_err(err)?,
        running,
        version: env!("CARGO_PKG_VERSION"),
        data_dir: st.data_dir.display().to_string(),
        allow_quit: st.allow_quit,
        audio_ok: st.audio_ok,
        wayland: wayland(),
        token_configured: cfg.enrolment.token.is_some() || cfg.enrolment.token_file.is_some(),
        home_dir: paths::expand(Path::new("~")).display().to_string(),
        sounds_dir: st.data_dir.join("sounds").display().to_string(),
        shortcut: app.try_state::<shortcut::State>().map(|s| s.info()),
    })
}

/// A control API call on the embedded agent (`status`, `alarms`,
/// `respond`, `hold.start`, ...).
#[tauri::command]
async fn agent_call(st: State<'_, Shared>, method: String, params: Value) -> Result<Value, String> {
    // Raising directly is for the CLI and tests; the app raises by holding.
    if method == "raise" {
        return Err("hold the button to raise an alarm".into());
    }
    agent(&st)
        .await?
        .call(&method, params)
        .await
        .map_err(|e| e.message)
}

#[tauri::command]
async fn setup_practice(
    app: AppHandle,
    st: State<'_, Shared>,
    name: String,
    label: String,
    passphrase: String,
    kit_path: String,
) -> Result<Value, String> {
    let dir = st.data_dir.clone();
    let kit = paths::expand(Path::new(kit_path.trim()));
    let p = tauri::async_runtime::spawn_blocking(move || {
        panic_button::agent::setup::init_practice(&dir, &name, label.trim(), &passphrase, &kit)
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    // Running already (the person tried "Ask to join" first): rebuild its
    // core from the new practice.
    if let Some(h) = agent_handle(&app).await {
        h.call("enrol", json!({})).await.map_err(|e| e.message)?;
    } else {
        start_agent(&app).await?;
    }
    Ok(json!({
        "practice": p.name,
        "practice_id": p.practice_id,
        "root_pub": p.root_pub,
        "recovery_kit": p.recovery_kit.display().to_string(),
    }))
}

/// "Create a new recovery kit": the same practice root under a new
/// passphrase (`setup::new_recovery_kit`).
#[tauri::command]
async fn new_recovery_kit(
    st: State<'_, Shared>,
    old_path: String,
    old_pass: String,
    new_path: String,
    new_pass: String,
) -> Result<Value, String> {
    let dir = st.data_dir.clone();
    let old = paths::expand(Path::new(old_path.trim()));
    let new = paths::expand(Path::new(new_path.trim()));
    let k = tauri::async_runtime::spawn_blocking(move || {
        panic_button::agent::setup::new_recovery_kit(Some(&dir), &old, &old_pass, &new, &new_pass)
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    Ok(json!({"practice": k.practice, "recovery_kit": k.path.display().to_string()}))
}

#[tauri::command]
async fn join_with_token(
    app: AppHandle,
    st: State<'_, Shared>,
    token: String,
) -> Result<(), String> {
    let t = token.trim().to_owned();
    panic_button::proto::Token::parse(&t)
        .map_err(|e| format!("That is not a valid enrolment code: {e}"))?;
    if let Some(h) = agent_handle(&app).await {
        h.call("enrol", json!({ "token": t }))
            .await
            .map_err(|e| e.message)?;
        return Ok(());
    }
    {
        let mut c = st.config.lock().await;
        c.enrolment.token = Some(t);
        c.enrolment.token_file = None;
    }
    let r = start_agent(&app).await;
    // The token is used once; do not keep it in memory.
    st.config.lock().await.enrolment.token = None;
    r
}

/// Join by approval (flow C): start the agent with no practice and ask
/// admins on the network to approve this computer. Returns the join view
/// (`join.status`), including the six-digit code to show.
#[tauri::command]
async fn start_join(app: AppHandle) -> Result<Value, String> {
    start_agent(&app).await?;
    let h = agent_handle(&app)
        .await
        .ok_or_else(|| "Panic Button did not start".to_owned())?;
    h.call("join.start", json!({})).await.map_err(|e| e.message)
}

#[tauri::command]
async fn create_token(st: State<'_, Shared>, days: u64, label: String) -> Result<Value, String> {
    let dir = st.data_dir.clone();
    let relays = st.config.lock().await.network.relay_urls.clone();
    let t = tauri::async_runtime::spawn_blocking(move || {
        panic_button::agent::setup::create_enrolment_token(
            &dir,
            &panic_button::agent::setup::TokenOptions {
                days,
                label,
                relay_urls: relays,
                seed_peers: Vec::new(),
            },
        )
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    Ok(json!({"token_id": t.token_id, "expires_at": t.expires_at, "text": t.text}))
}

#[tauri::command]
async fn set_room(st: State<'_, Shared>, room: String) -> Result<(), String> {
    agent(&st)
        .await?
        .call("device.set_room", json!({ "room": room.trim() }))
        .await
        .map_err(|e| e.message)?;
    Ok(())
}

#[tauri::command]
fn play_test_sound(st: State<'_, Shared>, pattern: Option<String>) {
    let p = match pattern.as_deref() {
        Some("B") => SoundPattern::B,
        Some("C") => SoundPattern::C,
        _ => SoundPattern::A,
    };
    st.audio.send(Cmd::Test(p));
}

#[tauri::command]
fn show_window(app: AppHandle, label: String) {
    match label.as_str() {
        "main" => windows::show_main(&app),
        "panel" => windows::show_panel(&app),
        "alert" => windows::show_alert(&app),
        "button" => windows::show_button(&app),
        _ => {}
    }
}

#[tauri::command]
fn hide_window(app: AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.hide();
    }
}

/// In the data folder while the person has quit Panic Button: the
/// watchdog leaves it stopped until it is next started normally (at
/// sign-in, or from the applications menu).
const QUIT_MARKER: &str = "quit-by-user";

/// Whether this start should end at once: the watchdog's start
/// (`--ensure-running`) after the person quit. Any other start clears the
/// marker, so the watchdog looks after Panic Button again.
fn stay_quit(watchdog_start: bool, data_dir: &std::path::Path) -> bool {
    let marker = data_dir.join(QUIT_MARKER);
    if watchdog_start {
        return marker.exists();
    }
    let _ = std::fs::remove_file(marker);
    false
}

#[tauri::command]
fn quit(app: AppHandle, st: State<'_, Shared>) -> Result<(), String> {
    if !st.allow_quit {
        return Err("Quitting is turned off by this computer's IT configuration".into());
    }
    if let Err(e) = std::fs::write(st.data_dir.join(QUIT_MARKER), b"") {
        tracing::warn!("cannot pause the watchdog, so it will start Panic Button again: {e}");
    }
    tracing::info!("quit from Settings");
    app.exit(0);
    Ok(())
}

/// Tray: the status line, and admin-only items enabled to match.
async fn refresh_tray(app: &AppHandle) {
    let st = app.state::<Shared>();
    let Some(h) = st.agent.lock().await.clone() else {
        return;
    };
    let Ok(s) = h.call("status", json!({})).await else {
        return;
    };
    let summary = s["summary"].as_str().unwrap_or("Starting").to_owned();
    let admin = s["admin"].as_bool().unwrap_or(false);
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.status.set_text(&summary);
        let _ = items.drill.set_enabled(admin);
    }
    if let Some(t) = app.tray_by_id("panic-button") {
        let _ = t.set_tooltip(Some(format!("Panic Button - {summary}")));
    }
}

struct TrayItems {
    status: MenuItem<tauri::Wry>,
    drill: MenuItem<tauri::Wry>,
}

const TRAY_ICON: tauri::image::Image<'static> = tauri::include_image!("icons/tray.png");

fn build_tray(app: &AppHandle, quit_in_tray: bool) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Starting...", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Panic Button", true, None::<&str>)?;
    let mv = MenuItem::with_id(app, "move", "Move button", true, None::<&str>)?;
    let drill = MenuItem::with_id(app, "drill", "Run a test alarm", false, None::<&str>)?;
    let about = MenuItem::with_id(app, "about", "About and source", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &sep, &open, &mv, &drill, &about])?;
    if quit_in_tray {
        let q = MenuItem::with_id(app, "quit", "Quit Panic Button", true, None::<&str>)?;
        menu.append(&q)?;
    }
    TrayIconBuilder::with_id("panic-button")
        .icon(TRAY_ICON)
        .menu(&menu)
        .tooltip("Panic Button")
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => windows::show_main(app),
            "about" => {
                windows::show_main(app);
                let _ = app.emit("panic-button-nav", "about");
            }
            "move" => {
                let _ = app.emit("panic-button-move", ());
            }
            "drill" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let st = app.state::<Shared>();
                    if let Some(h) = st.agent.lock().await.clone() {
                        let _ = h.call("drill.run", json!({})).await;
                    }
                });
            }
            "quit" => {
                windows::show_main(app);
                let _ = app.emit("panic-button-nav", "quit");
            }
            _ => {}
        })
        .build(app)?;
    app.manage(TrayItems { status, drill });
    Ok(())
}

/// Logs to stderr and to daily files in `<data dir>/logs`, keeping 7
/// (FR-LOG-003). Logs never contain secrets or alarm content beyond what
/// the audit log holds.
fn init_logging(data_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_env("PANIC_BUTTON_LOG")
            .unwrap_or_else(|_| "info".into())
    };
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()));
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("panic-button")
        .filename_suffix("log")
        .max_log_files(7)
        .build(data_dir.join("logs"));
    match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::registry()
                .with(filter())
                .with(stderr)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(writer)
                        .with_ansi(false),
                )
                .init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry()
                .with(filter())
                .with(stderr)
                .init();
            tracing::warn!("no log file: {e}");
            None
        }
    }
}

/// `--install-settings --token <t> --token-file <path> --room <r>
/// --relay-url <url> --seed-peers <a,b>`: the MSI's public properties
/// (`spec/08-deployment.md`), written into the machine config by
/// `setup::write_install_settings`. Empty values are ignored. Returns the
/// process exit code; a failure fails the install, with the reason in the
/// MSI log.
pub fn install_settings(args: &[String]) -> i32 {
    let mut s = panic_button::agent::setup::InstallSettings::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next().cloned().unwrap_or_default();
        let slot = match flag.as_str() {
            "--token" => &mut s.token,
            "--token-file" => &mut s.token_file,
            "--room" => &mut s.room,
            "--relay-url" => &mut s.relay_url,
            "--seed-peers" => &mut s.seed_peers,
            _ => {
                eprintln!("unknown setting {flag}");
                return 2;
            }
        };
        *slot = value;
    }
    let config = paths::default_config_file();
    let enrol_token = paths::default_data_dir(false).join("enrol.token");
    match panic_button::agent::setup::write_install_settings(&config, &enrol_token, &s) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Panic Button settings: {e:#}");
            1
        }
    }
}

pub fn run() {
    let custom_dir = env_path("PANIC_BUTTON_DATA_DIR");
    let data_dir = custom_dir
        .clone()
        .unwrap_or_else(|| paths::default_data_dir(false));
    let _log_guard = init_logging(&data_dir);
    let control_socket = env_path("PANIC_BUTTON_CONTROL_SOCKET")
        .unwrap_or_else(|| paths::default_control_socket(&data_dir));
    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("configuration: {e:#}");
            ConfigFile::default()
        }
    };
    let watchdog_start = std::env::args().any(|a| a == "--ensure-running");
    if stay_quit(watchdog_start, &data_dir) {
        tracing::info!("not starting: Panic Button was quit, and starts again at the next sign-in");
        return;
    }
    // Development builds and test data folders can always quit.
    let dev = cfg!(debug_assertions) || custom_dir.is_some();
    let allow_quit = config.allow_quit() || dev;
    let quit_in_tray = config.quit_in_tray() || dev;
    let (audio, audio_ok) = Audio::start(data_dir.join("sounds"));
    let mut builder = tauri::Builder::default();
    if custom_dir.is_none() {
        // Opening Panic Button again shows its window; the Windows
        // watchdog's every-5-minutes start (--ensure-running) shows nothing.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !args.iter().any(|a| a == "--ensure-running") {
                windows::show_main(app);
            }
        }));
        #[cfg(windows)]
        std::thread::spawn(platform::windows_watchdog::register);
    }
    let hotkey = config.alarm.hotkey.clone();
    let result = builder
        .manage(Shared {
            data_dir: data_dir.clone(),
            control_socket,
            config: Mutex::new(config),
            allow_quit,
            agent: Mutex::new(None),
            audio,
            audio_ok,
            volume: volume::Volume::new(),
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            agent_call,
            setup_practice,
            new_recovery_kit,
            join_with_token,
            start_join,
            create_token,
            set_room,
            play_test_sound,
            show_window,
            hide_window,
            quit,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            build_tray(&handle, quit_in_tray)?;
            shortcut::setup(&handle, hotkey.as_deref());
            let enrolled = panic_button::agent::setup::is_enrolled(&data_dir).unwrap_or(false);
            app.manage(windows::Placer::new(&data_dir, wayland()));
            windows::create_all(&handle, !enrolled)?;
            tauri::async_runtime::spawn(windows::watch_button(handle.clone()));
            // Always start the agent: with no practice yet, it listens so
            // first run can say whether Panic Button is already running on
            // this network, and the first-run choices enrol it in place.
            let h = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = start_agent(&h).await {
                    tracing::error!("agent: {e}");
                    let _ = h.emit("panic-button-error", e);
                    windows::show_main(&h);
                }
            });
            Ok(())
        })
        .on_window_event(|w, e| match e {
            // Closing a window hides it: Panic Button keeps running so alarms
            // still arrive.
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = w.hide();
            }
            tauri::WindowEvent::Moved(pos) if w.label() == "button" => {
                windows::button_moved(w.app_handle(), *pos);
            }
            _ => {}
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        tracing::error!("Panic Button could not start: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::TRAY_ICON;

    #[test]
    fn tray_uses_a_separate_icon_from_the_app() {
        let app_icon = tauri::include_image!("icons/32x32.png");
        assert_eq!((TRAY_ICON.width(), TRAY_ICON.height()), (32, 32));
        assert_ne!(TRAY_ICON.rgba(), app_icon.rgba());
    }

    /// After a quit from Settings the watchdog leaves Panic Button stopped,
    /// until it is next started normally.
    #[test]
    fn the_watchdog_respects_a_quit_until_a_normal_start() {
        let d = tempfile::tempdir().expect("tempdir");
        assert!(
            !super::stay_quit(true, d.path()),
            "no quit: the watchdog starts it"
        );
        std::fs::write(d.path().join(super::QUIT_MARKER), b"").expect("mark");
        assert!(
            super::stay_quit(true, d.path()),
            "quit: the watchdog leaves it"
        );
        assert!(super::stay_quit(true, d.path()));
        assert!(!super::stay_quit(false, d.path()), "a normal start runs");
        assert!(
            !super::stay_quit(true, d.path()),
            "and the watchdog is back"
        );
    }

    /// `room_only` sends no name at all (`spec/04-security.md`, Privacy).
    #[test]
    fn room_only_sends_no_name() {
        use super::{UserDisplaySource, user_display};
        assert_eq!(user_display(UserDisplaySource::RoomOnly), "");
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        assert_eq!(user_display(UserDisplaySource::OsUsername), user);
    }
}
