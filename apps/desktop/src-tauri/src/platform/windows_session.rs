// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Windows session state (`spec/05-desktop-agent.md`, Session and
//! lifecycle): a Remote Desktop session (`SM_REMOTESESSION`); another
//! user's session at the screen (fast user switching: this process's
//! session is not the active console session); and whether this session is
//! locked (`WTSSessionInfoEx`).

#![allow(unsafe_code)]

use panic_button::proto::SessionState;
use windows::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK, WTSFreeMemory,
    WTSGetActiveConsoleSessionId, WTSINFOEXW, WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};
use windows::core::PWSTR;

pub fn read() -> SessionState {
    // SAFETY: GetSystemMetrics takes a constant and has no other inputs.
    if unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0 {
        return SessionState::RemoteSessionDisabled;
    }
    if !at_console() {
        return SessionState::Inactive;
    }
    if locked() == Some(true) {
        SessionState::Locked
    } else {
        SessionState::Active
    }
}

/// Whether this process's session is the one at the screen. While Windows
/// switches users there may be no console session; that counts as at the
/// console, so a switch never pauses Panic Button by mistake.
fn at_console() -> bool {
    let mut own = 0u32;
    // SAFETY: writes this process's session id into `own`; no other inputs.
    if unsafe { ProcessIdToSessionId(std::process::id(), &mut own) }.is_err() {
        return true;
    }
    // SAFETY: takes no arguments and returns a session id.
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    console == u32::MAX || console == own
}

fn locked() -> Option<bool> {
    let mut buf = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: on success Windows allocates `buf` (`len` bytes), which is
    // freed below with WTSFreeMemory; nothing is kept after that.
    unsafe {
        WTSQuerySessionInformationW(
            None,
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buf,
            &mut len,
        )
        .ok()?;
        let flags = if !buf.is_null() && len as usize >= std::mem::size_of::<WTSINFOEXW>() {
            let info = &*(buf.0 as *const WTSINFOEXW);
            // The union holds level 1 data only when Level says so.
            if info.Level == 1 {
                Some(info.Data.WTSInfoExLevel1.SessionFlags)
            } else {
                None
            }
        } else {
            None
        };
        WTSFreeMemory(buf.0.cast());
        flags.map(|f| f as u32 == WTS_SESSIONSTATE_LOCK)
    }
}
