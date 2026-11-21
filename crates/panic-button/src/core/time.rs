// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Time types and protocol timers (`spec/03-protocol.md`, Timers and
//! constants). The core never reads a clock: the runtime passes a monotonic
//! [`Mono`] and a wall-clock time in milliseconds into every call.

use std::ops::{Add, Sub};

/// Monotonic time in milliseconds since an arbitrary origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Mono(pub u64);

impl Mono {
    pub const ZERO: Mono = Mono(0);

    pub fn ms(self) -> u64 {
        self.0
    }

    /// Milliseconds elapsed since `earlier` (0 if `earlier` is later).
    pub fn since(self, earlier: Mono) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl Add<u64> for Mono {
    type Output = Mono;
    fn add(self, ms: u64) -> Mono {
        Mono(self.0.saturating_add(ms))
    }
}

impl Sub<u64> for Mono {
    type Output = Mono;
    fn sub(self, ms: u64) -> Mono {
        Mono(self.0.saturating_sub(ms))
    }
}

/// Wall-clock time: milliseconds since the Unix epoch.
pub type WallMs = u64;

pub const SECOND: u64 = 1000;
pub const MINUTE: u64 = 60 * SECOND;
pub const HOUR: u64 = 60 * MINUTE;
pub const DAY: u64 = 24 * HOUR;

pub const HEARTBEAT_INTERVAL: u64 = 15 * SECOND;
pub const HEARTBEAT_JITTER: u64 = 2 * SECOND;
pub const PEER_STALE: u64 = 45 * SECOND;
/// After two missed heartbeats, unicast to every advertised address of a
/// peer, not just the one it was last heard from (the path may have broken).
pub const ADDRESS_FALLBACK_AFTER: u64 = 2 * (HEARTBEAT_INTERVAL + HEARTBEAT_JITTER);
pub const PEER_FORGET: u64 = 7 * DAY;
pub const PRESENCE_INTERVAL: u64 = 60 * SECOND;
pub const ALARM_MAX_LIFE: u64 = 60 * MINUTE;
pub const REFLOOD_JITTER: u64 = 150;
/// Gaps between repeats of an alarm raise re-flooded across a bridge
/// (ADR 0013): +0.5 s, +1.5 s, +3 s after the first re-flood.
pub const REFLOOD_REPEAT: [u64; 3] = [500, 1000, 1500];
/// Expected number of LAN re-floods per message on a flat segment (ADR 0013).
pub const REFLOOD_LAN_TARGET: u64 = 3;
pub const SYNC_RATE_LIMIT: u64 = 30 * SECOND;
/// Peers off the LAN group (unicast-only, relay-only) are few and on the
/// lossy links, so they may sync more often (ADR 0013).
pub const SYNC_RATE_LIMIT_NON_LAN: u64 = 10 * SECOND;
pub const PENDING_CERT_WAIT: u64 = 30 * SECOND;
pub const EPOCH_GRACE: u64 = 7 * DAY;
pub const DRILL_AUTO_CLOSE: u64 = 60 * SECOND;
pub const NEAR_MISS_MIN_MS: u64 = 300;
pub const HOLD_MS_DEFAULT: u32 = 1500;
pub const HOLD_MS_MIN: u32 = 1000;
pub const HOLD_MS_MAX: u32 = 3000;
/// Clock skew beyond which `raised_at` is treated as "now".
pub const FUTURE_SKEW_LIMIT: u64 = 5 * MINUTE;
/// A gap this long between inputs means the device slept.
pub const WAKE_GAP: u64 = 30 * SECOND;
/// Hold progress events are emitted at about 30 Hz.
pub const HOLD_PROGRESS_INTERVAL: u64 = 33;
/// Sender panel thresholds (`spec/05-desktop-agent.md`, Sender panel).
pub const SENDER_NOT_YET_DELIVERED: u64 = 5 * SECOND;
pub const SENDER_COULD_NOT_REACH: u64 = 15 * SECOND;
/// Join requests are re-sent every 5 s for at most 10 minutes.
pub const JOIN_RESEND: u64 = 5 * SECOND;
pub const JOIN_MAX: u64 = 10 * MINUTE;
/// How long an unenrolled device counts a computer it heard (four
/// heartbeats).
pub const NEARBY_WINDOW: u64 = MINUTE;
/// Processed message ids are kept this long in memory.
pub const PROCESSED_ID_TTL: u64 = DAY;
/// Closed alarms and their envelopes are kept for sync this long.
pub const ENVELOPE_KEEP_AFTER_CLOSE: u64 = DAY;

/// UDP retransmission offsets for a raised alarm, relative to T0.
pub const RETRANSMIT_EARLY: [u64; 4] = [250, 500, 1000, 2000];
/// After the early retransmits: every 2 s until 30 s, then every 10 s.
pub const RETRANSMIT_FAST_EVERY: u64 = 2 * SECOND;
pub const RETRANSMIT_FAST_UNTIL: u64 = 30 * SECOND;
pub const RETRANSMIT_SLOW_EVERY: u64 = 10 * SECOND;
/// TCP resends to peers with no receipt.
pub const TCP_RESEND: [u64; 2] = [2 * SECOND, 10 * SECOND];
/// UDP retransmission offsets for alarm events (ADR 0013).
pub const EVENT_RETRANSMIT: [u64; 3] = [250, 1000, 3000];
/// Wait this long for a flooded cert before asking for it.
pub const CERT_REQUEST_DELAY: u64 = 2 * SECOND;
/// Minimum gap between re-sending our receipt for the same alarm.
pub const RECEIPT_RESEND_MIN: u64 = SECOND;

/// Next retransmission time strictly after `elapsed` ms since T0.
pub fn next_retransmit_after(elapsed: u64) -> u64 {
    for t in RETRANSMIT_EARLY {
        if t > elapsed {
            return t;
        }
    }
    if elapsed < RETRANSMIT_FAST_UNTIL {
        let n = elapsed / RETRANSMIT_FAST_EVERY + 1;
        return (n * RETRANSMIT_FAST_EVERY).min(RETRANSMIT_FAST_UNTIL);
    }
    let n = (elapsed - RETRANSMIT_FAST_UNTIL) / RETRANSMIT_SLOW_EVERY + 1;
    RETRANSMIT_FAST_UNTIL + n * RETRANSMIT_SLOW_EVERY
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "Timers and constants" and "Size limits" tables in
    /// `spec/03-protocol.md`, value by value. Changing one here means
    /// changing the protocol.
    #[test]
    fn constants_match_the_spec_tables() {
        use crate::proto::limits::*;
        assert_eq!(DEFAULT_PORT, 47474);
        assert_eq!(DEFAULT_MCAST_GROUP, [239, 255, 71, 71]);
        assert_eq!((HEARTBEAT_INTERVAL, HEARTBEAT_JITTER), (15_000, 2_000));
        assert_eq!(PEER_STALE, 45_000);
        assert_eq!(PEER_FORGET, 7 * 24 * 3_600_000);
        assert_eq!(PRESENCE_INTERVAL, 60_000);
        assert_eq!(ALARM_MAX_LIFE, 60 * 60_000);
        assert_eq!(REFLOOD_JITTER, 150);
        assert_eq!(SYNC_RATE_LIMIT, 30_000);
        assert_eq!(PENDING_CERT_WAIT, 30_000);
        assert_eq!(EPOCH_GRACE, 7 * 24 * 3_600_000);
        assert_eq!(DRILL_AUTO_CLOSE, 60_000);
        assert_eq!(
            (HOLD_MS_DEFAULT, HOLD_MS_MIN, HOLD_MS_MAX),
            (1500, 1000, 3000)
        );
        assert_eq!(NEAR_MISS_MIN_MS, 300);
        assert_eq!(FUTURE_SKEW_LIMIT, 5 * 60_000);
        assert_eq!(TCP_RESEND, [2_000, 10_000]);
        assert_eq!(UDP_MAX, 1200);
        assert_eq!(TCP_FRAME_MAX, 256 * 1024);
        assert_eq!(
            (RELAY_WS_FRAME_MAX, RELAY_LP_BATCH_MAX),
            (64 * 1024, 256 * 1024)
        );
        assert_eq!((STRING_SEND_MAX, STRING_RECV_MAX), (64, 256));
        assert_eq!(PRESENCE_ENTRIES_MAX, 20);
        assert_eq!(HEARTBEAT_ADDRS_MAX, 8);
        assert_eq!(CERT_CHAIN_MAX, 8);
        assert_eq!((JOIN_RESEND, JOIN_MAX), (5_000, 10 * 60_000));
        assert_eq!(MDNS_SERVICE, "_panic-button._udp.local.");
    }

    #[test]
    fn retransmit_schedule_matches_spec() {
        let mut t = 0;
        let mut seen = vec![];
        while t < 60_000 {
            t = next_retransmit_after(t);
            seen.push(t);
        }
        assert_eq!(&seen[..6], &[250, 500, 1000, 2000, 4000, 6000]);
        assert!(seen.contains(&30_000));
        assert!(seen.contains(&40_000));
        assert!(!seen.contains(&32_000));
    }
}
