// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Status computation (`spec/03-protocol.md`, Status computation; FR-STS-003).
//!
//! Green is shown only when this device has a current path to at least one
//! other enrolled device and its audio works. A device that cannot reach
//! anyone is red, even if a relay is connected, unless it is the only
//! device in the practice (ADR 0013).

use crate::proto::{AudioStatus, StatusLevel};

use crate::core::io::{StatusReason, StatusView};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusInputs {
    pub enrolled: bool,
    pub own_cert_valid: bool,
    pub own_revoked: bool,
    pub have_group_key: bool,
    pub remote_session_disabled: bool,
    /// Another user's session has the screen (fast user switching).
    pub session_inactive: bool,
    /// macOS Local Network permission refused.
    pub local_network_blocked: bool,
    /// Certs known for subjects other than this device (other members,
    /// admins or enrolment tokens). Zero means a single-device practice.
    pub other_members: usize,
    /// Peers reachable by a LAN path in the last PEER_STALE.
    pub direct: usize,
    /// Peers reachable only via a relay.
    pub via_relay: usize,
    pub audio: Option<AudioStatus>,
    pub override_available: bool,
    pub override_mute: bool,
    pub relays_configured: usize,
    pub relays_connected: usize,
    pub stale_epoch: bool,
}

pub fn compute(i: &StatusInputs) -> StatusView {
    let mut red = Vec::new();
    if !i.enrolled {
        red.push(StatusReason::NotEnrolled);
    } else {
        if i.own_revoked || !i.own_cert_valid {
            red.push(StatusReason::CertRevoked);
        }
        if !i.have_group_key {
            red.push(StatusReason::NoGroupKey);
        }
        if i.direct + i.via_relay == 0 && i.other_members > 0 {
            red.push(StatusReason::NoDevicesReachable);
        }
    }
    if i.remote_session_disabled {
        red.push(StatusReason::RemoteSessionDisabled);
    }
    if i.session_inactive {
        red.push(StatusReason::SessionInactive);
    }
    if i.local_network_blocked {
        red.push(StatusReason::LocalNetworkBlocked);
    }
    if !red.is_empty() {
        return StatusView {
            level: StatusLevel::Red,
            reasons: red,
            direct: i.direct,
            via_relay: i.via_relay,
        };
    }
    let mut amber = Vec::new();
    let override_helps = i.override_mute && i.override_available;
    match i.audio {
        None => amber.push(StatusReason::AudioMissing),
        Some(a) if !a.output_present => amber.push(StatusReason::AudioMissing),
        Some(a) if (a.muted || a.volume_pct == 0) && !override_helps => {
            amber.push(StatusReason::AudioMuted)
        }
        Some(_) => {}
    }
    if i.relays_configured > 0 && i.relays_connected == 0 {
        amber.push(StatusReason::RelayUnreachable);
    }
    if i.direct == 0 && i.via_relay > 0 {
        amber.push(StatusReason::OnlyViaRelay);
    }
    if i.stale_epoch {
        amber.push(StatusReason::StaleEpoch);
    }
    if i.other_members == 0 && i.direct + i.via_relay == 0 {
        amber.push(StatusReason::NoOtherDevices);
    }
    StatusView {
        level: if amber.is_empty() {
            StatusLevel::Green
        } else {
            StatusLevel::Amber
        },
        reasons: amber,
        direct: i.direct,
        via_relay: i.via_relay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> StatusInputs {
        StatusInputs {
            enrolled: true,
            own_cert_valid: true,
            have_group_key: true,
            other_members: 3,
            direct: 3,
            audio: Some(AudioStatus {
                output_present: true,
                muted: false,
                volume_pct: 50,
            }),
            ..Default::default()
        }
    }

    fn level(i: StatusInputs) -> (StatusLevel, Vec<StatusReason>) {
        let v = compute(&i);
        (v.level, v.reasons)
    }

    #[test]
    fn green_when_all_good() {
        assert_eq!(level(good()), (StatusLevel::Green, vec![]));
    }

    #[test]
    fn fr_sts_003_red_reasons() {
        let cases: Vec<(StatusInputs, StatusReason)> = vec![
            (
                StatusInputs {
                    enrolled: false,
                    ..good()
                },
                StatusReason::NotEnrolled,
            ),
            (
                StatusInputs {
                    own_revoked: true,
                    ..good()
                },
                StatusReason::CertRevoked,
            ),
            (
                StatusInputs {
                    own_cert_valid: false,
                    ..good()
                },
                StatusReason::CertRevoked,
            ),
            (
                StatusInputs {
                    have_group_key: false,
                    ..good()
                },
                StatusReason::NoGroupKey,
            ),
            (
                StatusInputs {
                    direct: 0,
                    ..good()
                },
                StatusReason::NoDevicesReachable,
            ),
            (
                StatusInputs {
                    direct: 0,
                    relays_configured: 1,
                    relays_connected: 1,
                    ..good()
                },
                StatusReason::NoDevicesReachable,
            ),
            (
                StatusInputs {
                    remote_session_disabled: true,
                    ..good()
                },
                StatusReason::RemoteSessionDisabled,
            ),
            (
                StatusInputs {
                    session_inactive: true,
                    ..good()
                },
                StatusReason::SessionInactive,
            ),
            (
                StatusInputs {
                    local_network_blocked: true,
                    ..good()
                },
                StatusReason::LocalNetworkBlocked,
            ),
        ];
        for (i, r) in cases {
            let (l, rs) = level(i);
            assert_eq!(l, StatusLevel::Red, "{r:?}");
            assert!(rs.contains(&r), "{r:?} in {rs:?}");
        }
    }

    #[test]
    fn amber_reasons() {
        let muted = AudioStatus {
            output_present: true,
            muted: true,
            volume_pct: 50,
        };
        let cases: Vec<(StatusInputs, StatusReason)> = vec![
            (
                StatusInputs {
                    audio: Some(muted),
                    ..good()
                },
                StatusReason::AudioMuted,
            ),
            (
                StatusInputs {
                    audio: None,
                    ..good()
                },
                StatusReason::AudioMissing,
            ),
            (
                StatusInputs {
                    audio: Some(AudioStatus {
                        output_present: false,
                        ..muted
                    }),
                    ..good()
                },
                StatusReason::AudioMissing,
            ),
            (
                StatusInputs {
                    relays_configured: 1,
                    relays_connected: 0,
                    ..good()
                },
                StatusReason::RelayUnreachable,
            ),
            (
                StatusInputs {
                    direct: 0,
                    via_relay: 2,
                    ..good()
                },
                StatusReason::OnlyViaRelay,
            ),
            (
                StatusInputs {
                    stale_epoch: true,
                    ..good()
                },
                StatusReason::StaleEpoch,
            ),
            (
                StatusInputs {
                    other_members: 0,
                    direct: 0,
                    ..good()
                },
                StatusReason::NoOtherDevices,
            ),
        ];
        for (i, r) in cases {
            let (l, rs) = level(i);
            assert_eq!(l, StatusLevel::Amber, "{r:?}");
            assert!(rs.contains(&r), "{r:?} in {rs:?}");
        }
    }

    #[test]
    fn mute_override_clears_amber() {
        let muted = AudioStatus {
            output_present: true,
            muted: true,
            volume_pct: 50,
        };
        let i = StatusInputs {
            audio: Some(muted),
            override_available: true,
            override_mute: true,
            ..good()
        };
        assert_eq!(level(i).0, StatusLevel::Green);
    }
}
