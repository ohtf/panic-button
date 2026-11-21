// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The session this app runs in: active, locked, or a remote session
//! (`spec/05-desktop-agent.md`, Session and lifecycle). In a remote session
//! the core stops taking part unless `allow_remote_session` is set; a locked
//! screen keeps the alarm working and is reported in heartbeats.
//!
//! Windows asks the system (`platform::windows_session`). Linux asks logind
//! for the graphical session (`loginctl show-session auto`), and treats an
//! xrdp session as remote. macOS reads the console users from `ioreg`: this
//! user's session is inactive when another user has the screen (fast user
//! switching), or locked. macOS Screen Sharing shares the session on the
//! screen, so there is no separate remote session there. Elsewhere, or if
//! nothing can be read, the session counts as active.

use panic_button::proto::SessionState;

#[cfg(windows)]
pub async fn read() -> SessionState {
    tokio::task::spawn_blocking(crate::platform::windows_session::read)
        .await
        .unwrap_or(SessionState::Active)
}

#[cfg(target_os = "linux")]
pub async fn read() -> SessionState {
    if std::env::var_os("XRDP_SESSION").is_some() {
        return SessionState::RemoteSessionDisabled;
    }
    let out = tokio::process::Command::new("loginctl")
        .args(["show-session", "auto", "-p", "LockedHint", "-p", "Remote"])
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(3), out).await {
        Ok(Ok(o)) if o.status.success() => parse_loginctl(&String::from_utf8_lossy(&o.stdout)),
        _ => SessionState::Active,
    }
}

#[cfg(target_os = "macos")]
pub async fn read() -> SessionState {
    let out = tokio::process::Command::new("ioreg")
        .args(["-n", "Root", "-d1"])
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let user = std::env::var("USER").unwrap_or_default();
    match tokio::time::timeout(std::time::Duration::from_secs(3), out).await {
        Ok(Ok(o)) if o.status.success() => parse_ioreg(&String::from_utf8_lossy(&o.stdout), &user),
        _ => SessionState::Active,
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub async fn read() -> SessionState {
    SessionState::Active
}

/// The `"IOConsoleUsers" = ({...},{...})` line of `ioreg -n Root -d1`: one
/// record per signed-in user, `"key"=value` pairs. This user's record says
/// whether their session is on the console and whether its screen is
/// locked. No record for this user counts as active.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_ioreg(s: &str, user: &str) -> SessionState {
    let Some(line) = s.lines().find(|l| l.contains("\"IOConsoleUsers\"")) else {
        return SessionState::Active;
    };
    // Split into records and pairs on braces and commas outside quotes.
    let mut records: Vec<Vec<(String, String)>> = Vec::new();
    let (mut depth, mut quoted, mut field) = (0, false, String::new());
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                field.push(c);
            }
            '{' if !quoted => {
                depth += 1;
                records.push(Vec::new());
                field.clear();
            }
            '}' | ',' if !quoted && depth > 0 => {
                if let Some((k, v)) = field.split_once('=') {
                    let unq = |x: &str| x.trim().trim_matches('"').to_owned();
                    if let Some(r) = records.last_mut() {
                        r.push((unq(k), unq(v)));
                    }
                }
                field.clear();
                if c == '}' {
                    depth -= 1;
                }
            }
            _ if depth > 0 => field.push(c),
            _ => {}
        }
    }
    let get = |r: &[(String, String)], k: &str| {
        r.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    };
    let Some(me) = records
        .iter()
        .find(|r| get(r, "kCGSSessionUserNameKey").as_deref() == Some(user))
    else {
        return SessionState::Active;
    };
    if get(me, "kCGSSessionOnConsoleKey").as_deref() == Some("No") {
        SessionState::Inactive
    } else if get(me, "CGSSessionScreenIsLocked").as_deref() == Some("Yes") {
        SessionState::Locked
    } else {
        SessionState::Active
    }
}

/// `LockedHint=yes` / `Remote=yes` lines.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_loginctl(s: &str) -> SessionState {
    let yes = |k: &str| s.lines().any(|l| l.trim() == format!("{k}=yes"));
    if yes("Remote") {
        SessionState::RemoteSessionDisabled
    } else if yes("LockedHint") {
        SessionState::Locked
    } else {
        SessionState::Active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioreg() {
        let one = |on: &str, locked: &str| {
            format!(
                "+-o Root  <class IORegistryEntry, id 0x100000100, retain 49>\n    {{\n      \"IOConsoleUsers\" = ({{\"kCGSSessionOnConsoleKey\"={on},\"kSCSecuritySessionID\"=100015,\"kCGSessionLongUserNameKey\"=\"Baw, Marcus\",\"kCGSSessionUserNameKey\"=\"marcus\",\"kCGSSessionUserIDKey\"=501{locked}}},{{\"kCGSSessionOnConsoleKey\"=Yes,\"kCGSSessionUserNameKey\"=\"reception\"}})\n    }}\n"
            )
        };
        assert_eq!(parse_ioreg(&one("Yes", ""), "marcus"), SessionState::Active);
        assert_eq!(
            parse_ioreg(&one("Yes", ",\"CGSSessionScreenIsLocked\"=Yes"), "marcus"),
            SessionState::Locked
        );
        assert_eq!(
            parse_ioreg(&one("No", ""), "marcus"),
            SessionState::Inactive
        );
        assert_eq!(
            parse_ioreg(&one("No", ""), "reception"),
            SessionState::Active
        );
        assert_eq!(parse_ioreg(&one("No", ""), "nobody"), SessionState::Active);
        assert_eq!(
            parse_ioreg("no console users", "marcus"),
            SessionState::Active
        );
    }

    #[test]
    fn loginctl() {
        assert_eq!(
            parse_loginctl("Remote=no\nLockedHint=no\n"),
            SessionState::Active
        );
        assert_eq!(
            parse_loginctl("Remote=no\nLockedHint=yes\n"),
            SessionState::Locked
        );
        assert_eq!(
            parse_loginctl("Remote=yes\nLockedHint=yes\n"),
            SessionState::RemoteSessionDisabled
        );
        assert_eq!(parse_loginctl(""), SessionState::Active);
    }
}
