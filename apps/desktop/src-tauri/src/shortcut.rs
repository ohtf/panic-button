// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The global keyboard shortcut (FR-ACT-006). Holding it is the same as
//! holding the button: Pressed sends `hold.start` with source `hotkey` and
//! Released sends `hold.end`, so the core decides when the hold counts.
//! Repeated Pressed events (key auto-repeat) are ignored.
//!
//! Works on Windows, macOS and X11 directly. On Wayland it goes through the
//! XDG GlobalShortcuts portal (`portal` below), which reports press and
//! release too; the desktop may ask the person to confirm the keys, or
//! offer others. Where there is no working portal, the shortcut is reported
//! as unavailable (Settings).

use std::str::FromStr;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// `alarm.hotkey` default (`spec/08-deployment.md`).
pub const DEFAULT: &str = "Ctrl+Shift+F12";

/// What Settings shows about the shortcut.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Info {
    /// As configured, for example "Ctrl+Shift+F12"; empty when turned off.
    pub keys: String,
    pub active: bool,
    /// Why it is not active, in words for staff.
    pub problem: Option<String>,
}

#[derive(Debug)]
pub struct State {
    info: Mutex<Info>,
    held: Mutex<bool>,
}

impl State {
    pub fn info(&self) -> Info {
        lock(&self.info).clone()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The configured shortcut: `None` when turned off (`""` or `"none"`).
pub fn parse(configured: Option<&str>) -> Result<Option<Shortcut>, String> {
    let s = configured.unwrap_or(DEFAULT).trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    Shortcut::from_str(s)
        .map(Some)
        .map_err(|e| format!("\"{s}\" is not a keyboard shortcut Panic Button understands ({e})"))
}

/// A session where global shortcuts cannot work: Wayland, even when the app
/// runs under XWayland (X11 key grabs only see keys sent to X11 windows).
fn wayland_session() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Registers the plugin and the shortcut. Never fails the app: a shortcut
/// that cannot be used is logged and shown in Settings.
pub fn setup(app: &AppHandle, configured: Option<&str>) {
    let keys = configured.unwrap_or(DEFAULT).trim().to_owned();
    let info = |active, problem: Option<String>| Info {
        keys: keys.clone(),
        active,
        problem,
    };
    let result = match parse(configured) {
        Ok(None) => info(
            false,
            Some("Turned off by this computer's IT configuration.".into()),
        ),
        Ok(Some(_)) if wayland_session() => {
            #[cfg(target_os = "linux")]
            tauri::async_runtime::spawn(portal::run(app.clone(), keys.clone()));
            info(false, Some(NO_WAYLAND.into()))
        }
        Ok(Some(sc)) => match register(app, sc) {
            Ok(()) => info(true, None),
            Err(e) => {
                tracing::warn!("keyboard shortcut {keys}: {e}");
                info(
                    false,
                    Some(format!(
                        "{keys} could not be set up, perhaps because another program uses it. Ask IT to choose another."
                    )),
                )
            }
        },
        Err(e) => {
            tracing::warn!("keyboard shortcut: {e}");
            info(false, Some(format!("{e}. Ask IT to correct it.")))
        }
    };
    tracing::info!(keys = %result.keys, active = result.active, "keyboard shortcut");
    app.manage(State {
        info: Mutex::new(result),
        held: Mutex::new(false),
    });
}

fn register(app: &AppHandle, sc: Shortcut) -> Result<(), String> {
    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(on_event)
            .build(),
    )
    .map_err(|e| e.to_string())?;
    app.global_shortcut()
        .register(sc)
        .map_err(|e| e.to_string())
}

const NO_WAYLAND: &str = "Keyboard shortcuts for other apps do not work on this desktop (Wayland). Use the on-screen button.";

fn on_event(app: &AppHandle, _sc: &Shortcut, e: ShortcutEvent) {
    held(app, e.state() == ShortcutState::Pressed);
}

/// The shortcut went down (`pressed`) or up: start or end a hold, ignoring
/// repeats.
fn held(app: &AppHandle, pressed: bool) {
    let Some(st) = app.try_state::<State>() else {
        return;
    };
    let method = {
        let mut held = lock(&st.held);
        match (pressed, *held) {
            (true, false) => {
                *held = true;
                "hold.start"
            }
            (false, true) => {
                *held = false;
                "hold.end"
            }
            _ => return,
        }
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(h) = crate::agent_handle(&app).await
            && let Err(e) = h.call(method, json!({ "source": "hotkey" })).await
        {
            tracing::debug!("shortcut {method}: {}", e.message);
        }
    });
}

/// The XDG GlobalShortcuts portal, for Wayland.
#[cfg(target_os = "linux")]
mod portal {
    use ashpd::desktop::CreateSessionOptions;
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use tauri::{AppHandle, Manager};

    const ID: &str = "call-for-help";

    /// Binds the shortcut and turns its activations into holds, for as long
    /// as the app runs. Updates what Settings shows.
    pub async fn run(app: AppHandle, keys: String) {
        if let Err(e) = bind_and_listen(&app, &keys).await {
            tracing::warn!("keyboard shortcut through the portal: {e}");
        }
    }

    async fn bind_and_listen(app: &AppHandle, keys: &str) -> Result<(), ashpd::Error> {
        // Apps outside Flatpak tell the portal who they are first; it
        // refuses GlobalShortcuts without an app id. Our identifier, as in
        // tauri.conf.json.
        ashpd::register_host_app(ashpd::AppID::try_from("org.ohtf.panic-button")?).await?;
        let portal = GlobalShortcuts::new().await?;
        let session = portal
            .create_session(CreateSessionOptions::default())
            .await?;
        let trigger = super::xdg_trigger(keys);
        let shortcut = NewShortcut::new(ID, "Call for help (hold for the hold time)")
            .preferred_trigger(trigger.as_str());
        let bound = portal
            .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
            .await?
            .response()?;
        let Some(granted) = bound.shortcuts().iter().find(|s| s.id() == ID) else {
            tracing::warn!("the desktop did not grant the keyboard shortcut");
            return Ok(());
        };
        let shown = if granted.trigger_description().is_empty() {
            keys.to_owned()
        } else {
            granted.trigger_description().to_owned()
        };
        tracing::info!(keys = %shown, "keyboard shortcut granted by the portal");
        if let Some(st) = app.try_state::<super::State>() {
            *super::lock(&st.info) = super::Info {
                keys: shown,
                active: true,
                problem: None,
            };
        }
        let mut down = portal.receive_activated().await?;
        let mut up = portal.receive_deactivated().await?;
        loop {
            tokio::select! {
                Some(a) = down.next() => if a.shortcut_id() == ID { super::held(app, true) },
                Some(d) = up.next() => if d.shortcut_id() == ID { super::held(app, false) },
                else => break,
            }
        }
        drop(session);
        Ok(())
    }
}

/// `Ctrl+Shift+F12` in the XDG shortcuts format the portal takes
/// (`CTRL+SHIFT+F12`; Super is `LOGO`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn xdg_trigger(keys: &str) -> String {
    keys.split('+')
        .map(|k| match k.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => "CTRL".to_owned(),
            "shift" => "SHIFT".to_owned(),
            "alt" | "option" => "ALT".to_owned(),
            "super" | "meta" | "cmd" | "command" | "logo" => "LOGO".to_owned(),
            _ => k.trim().to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_triggers() {
        assert_eq!(xdg_trigger("Ctrl+Shift+F12"), "CTRL+SHIFT+F12");
        assert_eq!(xdg_trigger("Super+Alt+p"), "LOGO+ALT+p");
    }

    #[test]
    fn default_and_off() {
        assert!(parse(None).expect("default").is_some());
        assert_eq!(
            parse(None).expect("default"),
            parse(Some("ctrl+shift+f12")).expect("ctrl+shift+f12")
        );
        assert_eq!(parse(Some("")).expect(""), None);
        assert_eq!(parse(Some(" None ")).expect(" None "), None);
    }

    #[test]
    fn other_shortcuts() {
        assert!(parse(Some("Alt+Shift+P")).expect("Alt+Shift+P").is_some());
        assert!(
            parse(Some("CmdOrCtrl+Shift+F12"))
                .expect("CmdOrCtrl+Shift+F12")
                .is_some()
        );
        assert!(parse(Some("Ctrl+Banana")).is_err());
        assert!(parse(Some("Ctrl+")).is_err());
    }
}
