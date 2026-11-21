// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Sending: raising alarms, events, heartbeats, sync and re-flooding
//! (`spec/03-protocol.md`, Delivery algorithm).

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};

use crate::crypto::{make_keyshare, open_envelope, seal, sign_body};
use crate::proto::{
    AlarmEvent, AlarmKind, AlarmRaised, Body, Cbor, CertAnnounce, DeviceId, EventDetail, EventKind,
    Heartbeat, MsgId, PathKind, Payload, Platform, PresenceEntry, PresenceSummary, ReceiptDetail,
    Source, SyncRequest, SyncResponse, caps, limits, want,
};

use super::recv::Arrival;
use super::{Core, DrillRun, EventRetx, OwnAlarm};
use crate::core::alarm::{EventRec, RaisedRec};
use crate::core::io::{AuditEvent, LocalAck, Output, SenderPhase, UdpDest, UiEvent};
use crate::core::peers::PeerClass;
use crate::core::persist::PersistOp;
use crate::core::time::*;

/// Keep sync responses under the relay frame limit (with sealing overhead).
const SYNC_CHUNK: usize = 48 * 1024;
/// Unicast heartbeats go to peers seen within this long.
const UNICAST_HEARTBEAT_WINDOW: u64 = HOUR;

impl Core {
    pub(crate) fn body(&mut self, wall: WallMs, payload: Payload) -> Body {
        Body {
            msg_id: MsgId(self.rng.bytes()),
            created_at: wall,
            sender: self.me,
            payload,
        }
    }

    /// Seals a sealed-type body in the current epoch, plus copies in older
    /// epochs still advertised by valid peers, within EPOCH_GRACE (ADR 0013).
    pub(crate) fn seal_all(&mut self, now: Mono, wall: WallMs, body: &Body) -> Vec<Vec<u8>> {
        let Some((epoch, key)) = self.current.clone() else {
            return Vec::new();
        };
        let signed = sign_body(&self.keys, body);
        let mut out = vec![seal(&key, epoch, self.rng.bytes(), &signed).to_cbor()];
        let since = self.epoch_since.get(&epoch).copied().unwrap_or(wall);
        if wall.saturating_sub(since) <= EPOCH_GRACE {
            let mut older: BTreeSet<u64> = BTreeSet::new();
            for p in self.peers.iter() {
                if !p.reachable(now) {
                    continue;
                }
                if let Some(h) = &p.heartbeat
                    && h.epoch < epoch
                {
                    older.insert(h.epoch);
                }
            }
            for e in older {
                let keys: Vec<_> = self
                    .keys_by_tag
                    .values()
                    .filter(|(ke, _)| *ke == e)
                    .map(|(_, k)| k.clone())
                    .collect();
                for k in keys {
                    out.push(seal(&k, e, self.rng.bytes(), &signed).to_cbor());
                }
            }
        }
        out
    }

    pub(crate) fn open_env(&mut self, body: &Body) -> Vec<u8> {
        let pid = self
            .practice
            .as_ref()
            .map(|p| p.practice_id)
            .unwrap_or(crate::proto::PracticeId::ANY);
        open_envelope(pid, &sign_body(&self.keys, body)).to_cbor()
    }

    fn udp(&mut self, dest: UdpDest, envs: &[Vec<u8>]) {
        for e in envs {
            if e.len() <= limits::UDP_MAX {
                self.emit(Output::SendDatagram {
                    dest,
                    bytes: e.clone(),
                });
            }
        }
    }

    fn tcp(&mut self, to: SocketAddrV4, envs: &[Vec<u8>]) {
        for e in envs {
            if e.len() <= limits::TCP_FRAME_MAX {
                self.emit(Output::SendStream {
                    to,
                    bytes: e.clone(),
                });
            }
        }
    }

    fn relay_all(&mut self, envs: &[Vec<u8>]) {
        if !self.relays.values().any(|r| r.connected) {
            return;
        }
        for e in envs {
            if e.len() <= limits::RELAY_WS_FRAME_MAX {
                self.emit(Output::SendRelay {
                    relay: None,
                    bytes: e.clone(),
                });
            }
        }
    }

    pub(crate) fn send_lan_group(&mut self, envs: &[Vec<u8>]) {
        self.udp(UdpDest::Multicast, envs);
        self.udp(UdpDest::Broadcast, envs);
    }

    /// Seed peers and mDNS-discovered addresses.
    fn extra_targets(&self) -> Vec<SocketAddrV4> {
        let mut v: Vec<SocketAddrV4> = self.cfg.seed_addrs.clone();
        for a in self.discovered.keys() {
            if !v.contains(a) {
                v.push(*a);
            }
        }
        // Skip addresses already covered by a known peer's unicast address.
        v.retain(|a| self.peers.by_udp_addr(a).is_none());
        v
    }

    fn is_member(&self, d: &DeviceId, wall: WallMs) -> bool {
        self.trust.as_ref().is_some_and(|t| t.is_member(d, wall))
    }

    /// Sends to one device by every direct means we have.
    pub(crate) fn send_direct(&mut self, now: Mono, d: &DeviceId, envs: &[Vec<u8>], tcp: bool) {
        let Some(p) = self.peers.get(d) else { return };
        let class = p.class(now);
        let ucast = p.unicast_targets(self.cfg.port);
        let tcp_t = p.tcp_targets();
        let tcp_to = if tcp { tcp_t.first().copied() } else { None };
        // UDP unicast too, unless TCP is going and the peer hears the LAN group.
        if tcp_to.is_none() || class != PeerClass::LanGroup {
            for a in ucast {
                self.udp(UdpDest::Unicast(a), envs);
            }
        }
        if let Some(a) = tcp_to {
            self.tcp(a, envs);
        }
        if class == PeerClass::RelayOnly {
            self.relay_all(envs);
        }
    }

    /// Fan-out for alarm events and other flooded messages: LAN group,
    /// unicast to peers not on the LAN group, TCP to unicast-only peers,
    /// relays.
    fn send_flood(&mut self, now: Mono, wall: WallMs, envs: &[Vec<u8>]) {
        self.send_lan_group(envs);
        let mut uni: Vec<SocketAddrV4> = Vec::new();
        let mut tcps: Vec<SocketAddrV4> = Vec::new();
        for p in self.peers.iter() {
            if p.device_id == self.me || !self.is_member(&p.device_id, wall) {
                continue;
            }
            match p.class(now) {
                PeerClass::LanGroup => {}
                PeerClass::UnicastOnly => {
                    uni.extend(p.unicast_targets(self.cfg.port));
                    if let Some(t) = p.tcp_targets().first() {
                        tcps.push(*t);
                    }
                }
                _ => uni.extend(p.unicast_targets(self.cfg.port)),
            }
        }
        uni.extend(self.extra_targets());
        uni.sort();
        uni.dedup();
        for a in uni {
            self.udp(UdpDest::Unicast(a), envs);
        }
        for t in tcps {
            self.tcp(t, envs);
        }
        self.relay_all(envs);
    }

    // ---- Raising and alarm events -------------------------------------

    pub(crate) fn raise(&mut self, now: Mono, wall: WallMs, source: Source, drill: bool) {
        if self.practice.is_none() || self.current.is_none() || !self.participating() {
            return;
        }
        let msg_id = MsgId(self.rng.bytes());
        let payload = AlarmRaised {
            alarm_id: msg_id,
            kind: AlarmKind::Help,
            drill,
            room: self.cfg.room.clone(),
            user_display: self.user_display.clone(),
            device_label: self.cfg.label.clone(),
            source,
        };
        let body = Body {
            msg_id,
            created_at: wall,
            sender: self.me,
            payload: Payload::AlarmRaised(payload.clone()),
        };
        let envs = self.seal_all(now, wall, &body);
        // 1. Send immediately on every transport, in parallel.
        self.send_lan_group(&envs);
        let mut uni: Vec<SocketAddrV4> = Vec::new();
        let mut tcps: Vec<SocketAddrV4> = Vec::new();
        for p in self.peers.iter() {
            if p.device_id == self.me || !self.is_member(&p.device_id, wall) {
                continue;
            }
            let class = p.class(now);
            if class != PeerClass::LanGroup {
                uni.extend(p.unicast_targets(self.cfg.port));
            }
            // TCP to every known peer that is not stale (lossy Wi-Fi).
            if class != PeerClass::Stale {
                tcps.extend(p.tcp_targets());
            }
        }
        for a in self.extra_targets() {
            uni.push(a);
            tcps.push(a);
        }
        uni.sort();
        uni.dedup();
        tcps.sort();
        tcps.dedup();
        for a in uni {
            self.udp(UdpDest::Unicast(a), &envs);
        }
        for a in tcps {
            self.tcp(a, &envs);
        }
        self.relay_all(&envs);
        // 2. Then record and persist.
        let entry = self.alarms.entry(msg_id).or_default();
        entry.known = true;
        entry.local.first_seen = wall;
        entry.msgs.raised = Some(RaisedRec {
            created_at: wall,
            origin: self.me,
            payload,
        });
        if let Some(e) = envs.first() {
            entry.envelopes.insert(msg_id, e.clone());
        }
        self.processed.insert(msg_id, wall);
        self.reflooded.insert(msg_id, now);
        self.own_alarms.insert(
            msg_id,
            OwnAlarm {
                t0: now,
                envelopes: envs.clone(),
                next_udp: now + next_retransmit_after(0),
                tcp_resends_done: 0,
                phase: SenderPhase::Requested,
                drill,
            },
        );
        if let Some(e) = envs.first() {
            self.persist(PersistOp::AddAlarmEnvelope {
                alarm_id: msg_id,
                msg_id,
                bytes: e.clone(),
            });
        }
        self.persist(PersistOp::Processed { msg_id, at: wall });
        self.log(AuditEvent::AlarmRaised {
            alarm_id: msg_id,
            source,
            drill,
        });
        if drill {
            let expected: Vec<DeviceId> = self
                .peers
                .iter()
                .filter(|p| {
                    p.device_id != self.me && wall.saturating_sub(p.last_seen_wall) <= 7 * DAY
                })
                .filter(|p| self.is_member(&p.device_id, wall))
                .map(|p| p.device_id)
                .collect();
            self.drills.insert(
                msg_id,
                DrillRun {
                    t0: now,
                    started_at: wall,
                    expected,
                    rtt: Default::default(),
                    close_at: now + DRILL_AUTO_CLOSE,
                },
            );
        } else if let Some(v) = self
            .alarms
            .get(&msg_id)
            .and_then(|e| crate::core::alarm::view(msg_id, &e.msgs, &self.me, None, wall))
        {
            // The sender's device makes no sound (ADR 0008): panel only.
            self.ui(UiEvent::SenderPanel {
                alarm: v.clone(),
                phase: SenderPhase::Requested,
            });
            if let Some(e) = self.alarms.get_mut(&msg_id) {
                e.last_view = Some(v);
            }
        }
    }

    /// Who colleagues see as coming, seen or resolving
    /// (`spec/03-protocol.md`, AlarmEvent): the signed-in person, or with
    /// nobody's name to give, this computer's name, else its room.
    pub(crate) fn event_name(&self) -> String {
        [&self.user_display, &self.cfg.label, &self.cfg.room]
            .into_iter()
            .find(|n| !n.trim().is_empty())
            .map(|n| n.trim().to_owned())
            .unwrap_or_default()
    }

    /// Builds, applies locally and sends one of our own alarm events.
    fn own_event(
        &mut self,
        now: Mono,
        wall: WallMs,
        alarm_id: MsgId,
        kind: EventKind,
        detail: EventDetail,
    ) -> Vec<Vec<u8>> {
        let body = self.body(
            wall,
            Payload::AlarmEvent(AlarmEvent {
                alarm_id,
                event: kind,
                user_display: self.event_name(),
                detail: detail.clone(),
            }),
        );
        let envs = self.seal_all(now, wall, &body);
        let rec = EventRec {
            msg_id: body.msg_id,
            created_at: wall,
            sender: self.me,
            kind,
            user_display: self.event_name(),
            detail,
        };
        let entry = self.alarms.entry(alarm_id).or_default();
        entry.msgs.events.insert(body.msg_id, rec);
        if let Some(e) = envs.first() {
            entry.envelopes.insert(body.msg_id, e.clone());
        }
        self.processed.insert(body.msg_id, wall);
        self.reflooded.insert(body.msg_id, now);
        if let Some(e) = envs.first() {
            self.persist(PersistOp::AddAlarmEnvelope {
                alarm_id,
                msg_id: body.msg_id,
                bytes: e.clone(),
            });
        }
        self.persist(PersistOp::Processed {
            msg_id: body.msg_id,
            at: wall,
        });
        envs
    }

    /// FR-RCV-007: automatic machine-level acknowledgement.
    pub(crate) fn send_receipt(
        &mut self,
        now: Mono,
        wall: WallMs,
        alarm_id: MsgId,
        origin: DeviceId,
        drill: bool,
        path: PathKind,
    ) {
        let detail = EventDetail::Receipt(ReceiptDetail {
            audio: self.audio,
            path: Some(path),
            displayed: Some(!drill),
            drill_rtt_ref: drill.then_some(alarm_id),
        });
        let envs = self.own_event(now, wall, alarm_id, EventKind::Receipt, detail);
        // To the origin directly first, by TCP as well, then flood, and
        // retransmit like other events: the receipt is what the sender's
        // "Delivered to N" counts.
        self.send_direct(now, &origin, &envs, true);
        self.send_flood(now, wall, &envs);
        self.schedule_event_retx(now, envs.clone());
        if let Some(e) = self.alarms.get_mut(&alarm_id) {
            e.receipt_env = envs.first().cloned();
            e.last_receipt_send = Some(now);
        }
    }

    pub(crate) fn local_ack(&mut self, now: Mono, wall: WallMs, alarm_id: MsgId, ack: LocalAck) {
        let Some(e) = self.alarms.get(&alarm_id) else {
            return;
        };
        if e.msgs.raised.is_none() {
            return;
        }
        // Respond can follow Seen; Seen after Respond changes nothing.
        if e.local.local_ack == Some(LocalAck::Responding) {
            return;
        }
        if e.local.local_ack == Some(ack) {
            return;
        }
        let kind = match ack {
            LocalAck::Seen => EventKind::Seen,
            LocalAck::Responding => EventKind::Responding,
        };
        let envs = self.own_event(now, wall, alarm_id, kind, EventDetail::None);
        self.send_flood(now, wall, &envs);
        self.schedule_event_retx(now, envs);
        if let Some(e) = self.alarms.get_mut(&alarm_id) {
            e.local.local_ack = Some(ack);
            let local = e.local;
            self.persist(PersistOp::SetAlarmLocal { alarm_id, local });
        }
        self.refresh_alarm(now, wall, alarm_id);
        self.update_sound();
    }

    pub(crate) fn close_alarm(&mut self, now: Mono, wall: WallMs, alarm_id: MsgId, cancel: bool) {
        let Some(e) = self.alarms.get(&alarm_id) else {
            return;
        };
        let Some(r) = &e.msgs.raised else { return };
        if e.msgs.derive().closed.is_some() {
            return;
        }
        // Only the origin may cancel (FR-ALM-005).
        if cancel && r.origin != self.me {
            return;
        }
        let (kind, detail) = if cancel {
            (
                EventKind::Cancelled,
                EventDetail::Cancelled {
                    reason: crate::proto::CANCEL_SENT_IN_ERROR,
                },
            )
        } else {
            (EventKind::Resolved, EventDetail::None)
        };
        let envs = self.own_event(now, wall, alarm_id, kind, detail);
        self.send_flood(now, wall, &envs);
        self.schedule_event_retx(now, envs);
        self.refresh_alarm(now, wall, alarm_id);
    }

    fn schedule_event_retx(&mut self, now: Mono, envelopes: Vec<Vec<u8>>) {
        self.event_retx.push(EventRetx {
            at: EVENT_RETRANSMIT.iter().map(|t| now + *t).collect(),
            envelopes,
        });
    }

    pub(crate) fn run_event_retx(&mut self, now: Mono) {
        let mut due: Vec<Vec<Vec<u8>>> = Vec::new();
        for r in &mut self.event_retx {
            if r.at.first().is_some_and(|t| *t <= now) {
                r.at.remove(0);
                due.push(r.envelopes.clone());
            }
        }
        self.event_retx.retain(|r| !r.at.is_empty());
        for envs in due {
            self.send_lan_group(&envs);
            // And to peers that do not hear the LAN group.
            let mut uni: Vec<SocketAddrV4> = Vec::new();
            for p in self.peers.iter() {
                if p.device_id == self.me
                    || !matches!(p.class(now), PeerClass::UnicastOnly | PeerClass::RelayOnly)
                {
                    continue;
                }
                uni.extend(p.unicast_targets(self.cfg.port));
            }
            uni.sort();
            uni.dedup();
            for a in uni {
                self.udp(UdpDest::Unicast(a), &envs);
            }
        }
    }

    pub(crate) fn run_own_alarm_timers(&mut self, now: Mono, wall: WallMs) {
        let ids: Vec<MsgId> = self.own_alarms.keys().copied().collect();
        for id in ids {
            let Some(o) = self.own_alarms.get(&id).cloned() else {
                continue;
            };
            let derived = self
                .alarms
                .get(&id)
                .map(|e| e.msgs.derive())
                .unwrap_or_default();
            let closed = derived.closed.is_some();
            let elapsed = now.since(o.t0);
            if closed || elapsed > ALARM_MAX_LIFE || (o.drill && elapsed > DRILL_AUTO_CLOSE) {
                self.own_alarms.remove(&id);
                continue;
            }
            let receipted: BTreeSet<DeviceId> = derived.receipts.keys().copied().collect();
            if now >= o.next_udp {
                self.send_lan_group(&o.envelopes);
                let mut uni = Vec::new();
                for p in self.peers.iter() {
                    if p.device_id == self.me
                        || receipted.contains(&p.device_id)
                        || p.class(now) == PeerClass::Stale && p.heartbeat.is_none()
                    {
                        continue;
                    }
                    if !self.is_member(&p.device_id, wall) {
                        continue;
                    }
                    uni.extend(p.unicast_targets(self.cfg.port));
                }
                uni.extend(self.extra_targets());
                uni.sort();
                uni.dedup();
                for a in uni {
                    self.udp(UdpDest::Unicast(a), &o.envelopes);
                }
                // Relays buffer and resume, but a reconnecting client may
                // have missed the first copy: resend while unreceipted.
                if self.peers.iter().any(|p| {
                    p.class(now) == PeerClass::RelayOnly && !receipted.contains(&p.device_id)
                }) {
                    self.relay_all(&o.envelopes);
                }
                if let Some(oa) = self.own_alarms.get_mut(&id) {
                    oa.next_udp = o.t0 + next_retransmit_after(elapsed);
                }
            }
            if let Some(t) = TCP_RESEND.get(o.tcp_resends_done)
                && elapsed >= *t
            {
                let mut tcps = Vec::new();
                for p in self.peers.iter() {
                    if p.device_id == self.me
                        || receipted.contains(&p.device_id)
                        || p.class(now) == PeerClass::Stale
                    {
                        continue;
                    }
                    if let Some(a) = p.tcp_targets().first() {
                        tcps.push(*a);
                    }
                }
                for a in tcps {
                    self.tcp(a, &o.envelopes);
                }
                if let Some(oa) = self.own_alarms.get_mut(&id) {
                    oa.tcp_resends_done += 1;
                }
            }
            // Honest sender panel (P10, H03).
            if !o.drill && receipted.is_empty() {
                let phase = if elapsed >= SENDER_COULD_NOT_REACH {
                    SenderPhase::CouldNotReach
                } else if elapsed >= SENDER_NOT_YET_DELIVERED {
                    SenderPhase::NotYetDelivered
                } else {
                    SenderPhase::Requested
                };
                if phase != o.phase {
                    if let Some(oa) = self.own_alarms.get_mut(&id) {
                        oa.phase = phase;
                    }
                    if let Some(v) = self
                        .alarms
                        .get(&id)
                        .and_then(|e| crate::core::alarm::view(id, &e.msgs, &self.me, None, wall))
                    {
                        self.ui(UiEvent::SenderPanel { alarm: v, phase });
                    }
                }
            }
        }
    }

    pub(crate) fn run_drills(&mut self, now: Mono, wall: WallMs) {
        let due: Vec<MsgId> = self
            .drills
            .iter()
            .filter(|(_, r)| now >= r.close_at)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            let Some(run) = self.drills.remove(&id) else {
                continue;
            };
            self.close_alarm(now, wall, id, false);
            let report = self.drill_report(&id, &run);
            self.log(AuditEvent::DrillCompleted {
                alarm_id: id,
                received: report.received(),
                expected: report.rows.len(),
            });
            self.ui(UiEvent::DrillReport(report));
        }
    }

    pub(crate) fn run_refloods(&mut self, now: Mono) {
        let due: Vec<MsgId> = self
            .refloods
            .values()
            .filter(|r| r.at <= now)
            .map(|r| r.msg_id)
            .collect();
        for id in due {
            let Some(mut r) = self.refloods.remove(&id) else {
                continue;
            };
            self.reflooded.insert(id, now);
            let has = self.known_has.get(&id).cloned().unwrap_or_default();
            let bridge = self.interfaces.len() > 1 || !r.heard_lan;
            // A single-interface device that has heard the message on the LAN
            // shares that segment with its sender, so every neighbour got the
            // same copy: re-flooding there only guards against loss, which
            // the origin's retransmissions also cover. Re-flood on the LAN
            // with probability about 3 / (LAN-group peers), so a big flat LAN
            // does not multiply every message (P7, ADR 0013).
            if !r.repeat && !r.suppressed && !bridge {
                let lan = self
                    .peers
                    .iter()
                    .filter(|p| p.class(now) == PeerClass::LanGroup)
                    .count() as u64;
                if lan > REFLOOD_LAN_TARGET && self.rng.below_incl(lan - 1) >= REFLOOD_LAN_TARGET {
                    r.suppressed = true;
                }
            }
            // Hearing a neighbour's copy says nothing about our other
            // segments, so a multi-homed device is never suppressed. Repeats
            // go on the LAN only where this device bridges segments, and by
            // UDP to unicast-only peers still lacking the message.
            let multi_homed = self.interfaces.len() > 1;
            if (!r.repeat && (!r.suppressed || multi_homed)) || (r.repeat && bridge) {
                self.send_lan_group(&r.envelopes);
            }
            let mut uni = Vec::new();
            let mut tcps = Vec::new();
            for p in self.peers.iter() {
                if p.device_id == self.me || p.device_id == r.origin || has.contains(&p.device_id) {
                    continue;
                }
                if !self
                    .trust
                    .as_ref()
                    .is_some_and(|t| t.is_member(&p.device_id, self.last_wall))
                {
                    continue;
                }
                if p.class(now) == PeerClass::UnicastOnly {
                    uni.extend(p.unicast_targets(self.cfg.port));
                    if let Some(t) = p.tcp_targets().first() {
                        tcps.push(*t);
                    }
                }
            }
            for a in uni {
                self.udp(UdpDest::Unicast(a), &r.envelopes);
            }
            if !r.repeat {
                for a in tcps {
                    self.tcp(a, &r.envelopes);
                }
                if !r.arrived_via_relay {
                    self.relay_all(&r.envelopes);
                }
            }
            if r.repeats_left > 0 {
                let i = REFLOOD_REPEAT.len() - r.repeats_left;
                let closed = self
                    .alarms
                    .get(&id)
                    .is_some_and(|e| e.msgs.derive().closed.is_some());
                if !closed {
                    let mut next = r.clone();
                    next.repeat = true;
                    next.repeats_left -= 1;
                    next.at = now + REFLOOD_REPEAT[i];
                    self.refloods.insert(id, next);
                }
            }
        }
    }

    // ---- Heartbeats, presence, certs ----------------------------------

    pub(crate) fn send_heartbeat(&mut self, now: Mono, wall: WallMs) {
        if self.current.is_none() {
            return;
        }
        let (ad, sd) = self.heartbeat_digests(wall);
        let mut capabilities = caps::CAN_ALERT | caps::CAN_RAISE;
        if self
            .trust
            .as_ref()
            .is_some_and(|t| t.is_admin(&self.me, wall))
        {
            capabilities |= caps::ADMIN;
        }
        if self.cfg.drill_coordinator == Some(self.me) {
            capabilities |= caps::DRILL_COORDINATOR;
        }
        if self.relays.values().any(|r| r.connected) {
            capabilities |= caps::RELAY_CONNECTED;
        }
        let addrs: Vec<[u8; 4]> = self
            .interfaces
            .iter()
            .map(|i| i.addr)
            .filter(|a: &Ipv4Addr| !a.is_loopback())
            .take(limits::HEARTBEAT_ADDRS_MAX)
            .map(|a| a.octets())
            .collect();
        let hb = Heartbeat {
            label: self.cfg.label.clone(),
            room: self.cfg.room.clone(),
            user_display: self.user_display.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            proto_version: limits::PROTO_VERSION,
            platform: Platform::Other(0),
            tcp_port: self.cfg.port,
            addrs,
            capabilities,
            audio: self.audio.unwrap_or_default(),
            session: self.session,
            status: self.status_level(),
            cert_serial: self.own_serial.unwrap_or_default(),
            epoch: self.current_epoch().unwrap_or(0),
            alarm_digest: ad,
            state_digest: sd,
            near_miss_24h: self.near_misses.len() as u64,
            uptime_s: now.since(self.started_at) / SECOND,
        };
        let body = self.body(wall, Payload::Heartbeat(hb));
        let envs = self.seal_all(now, wall, &body);
        self.processed.insert(body.msg_id, wall);
        self.send_lan_group(&envs);
        let mut uni: Vec<SocketAddrV4> = Vec::new();
        for p in self.peers.iter() {
            if p.device_id == self.me || p.class(now) == PeerClass::LanGroup {
                continue;
            }
            if wall.saturating_sub(p.last_seen_wall) > UNICAST_HEARTBEAT_WINDOW {
                continue;
            }
            uni.extend(p.unicast_targets(self.cfg.port));
        }
        uni.extend(self.extra_targets());
        uni.sort();
        uni.dedup();
        for a in uni {
            self.udp(UdpDest::Unicast(a), &envs);
        }
        self.relay_all(&envs);
    }

    pub(crate) fn send_presence(&mut self, now: Mono, wall: WallMs) {
        let direct: Vec<PresenceEntry> = self
            .peers
            .iter()
            .filter(|p| {
                p.device_id != self.me
                    && matches!(p.class(now), PeerClass::LanGroup | PeerClass::UnicastOnly)
            })
            .map(|p| PresenceEntry {
                device_id: p.device_id,
                last_seen_age_s: p.last_heard().map(|t| now.since(t) / SECOND).unwrap_or(0),
                path: p.best_path(now),
            })
            .collect();
        for chunk in direct.chunks(limits::PRESENCE_ENTRIES_MAX) {
            let body = self.body(
                wall,
                Payload::PresenceSummary(PresenceSummary {
                    peers: chunk.to_vec(),
                }),
            );
            let envs = self.seal_all(now, wall, &body);
            self.processed.insert(body.msg_id, wall);
            self.send_lan_group(&envs);
            self.relay_all(&envs);
        }
    }

    /// CertAnnounce envelopes carrying every cert we know, chunked.
    pub(crate) fn cert_announce_envelopes(&mut self, now: Mono, wall: WallMs) -> Vec<Vec<u8>> {
        let Some(t) = &self.trust else {
            return Vec::new();
        };
        let certs: Vec<Vec<u8>> = t.certs().map(|(c, _)| c.to_cbor()).collect();
        let mut out = Vec::new();
        // Three certs per announce keeps each envelope inside one datagram.
        for chunk in certs.chunks(3) {
            let body = self.body(
                wall,
                Payload::CertAnnounce(CertAnnounce {
                    certs: chunk.to_vec(),
                }),
            );
            self.processed.insert(body.msg_id, wall);
            out.extend(self.seal_all(now, wall, &body).into_iter().take(1));
        }
        out
    }

    /// Announces our own chain (on start-up, after waking, after joining).
    pub(crate) fn send_cert_announce(&mut self, now: Mono, wall: WallMs) {
        let Some(t) = &self.trust else { return };
        let Some(chain) = t.chain_for(&self.me, wall) else {
            return;
        };
        let certs: Vec<Vec<u8>> = chain.iter().map(|c| c.to_cbor()).collect();
        let body = self.body(wall, Payload::CertAnnounce(CertAnnounce { certs }));
        self.processed.insert(body.msg_id, wall);
        self.reflooded.insert(body.msg_id, now);
        let envs = self.seal_all(now, wall, &body);
        self.send_flood(now, wall, &envs);
        // Big announces go by TCP to known peers.
        if envs.first().is_some_and(|e| e.len() > limits::UDP_MAX) {
            let tcps: Vec<_> = self
                .peers
                .iter()
                .filter(|p| p.class(now) != PeerClass::Stale)
                .filter_map(|p| p.tcp_targets().first().copied())
                .collect();
            for a in tcps {
                self.tcp(a, &envs);
            }
        }
    }

    /// Builds a signed RelayAuth envelope answering a relay's challenge
    /// (`spec/06-relay.md`, Handshake). `None` if not enrolled or without a
    /// valid chain.
    pub fn relay_auth(
        &mut self,
        nonce: [u8; 32],
        resume_cursor: Option<u64>,
        wall: WallMs,
    ) -> Option<Vec<u8>> {
        let p = self.practice.clone()?;
        let chain = self.trust.as_ref()?.chain_for(&self.me, wall)?;
        let body = self.body(
            wall,
            Payload::RelayAuth(crate::proto::RelayAuth {
                practice_id: p.practice_id,
                chain: chain.iter().map(|c| c.to_cbor()).collect(),
                nonce,
                client_time: wall,
                resume_cursor,
            }),
        );
        Some(self.open_env(&body))
    }

    /// Sends our own cert chain back the way a message arrived.
    pub(crate) fn send_own_chain_via(&mut self, now: Mono, wall: WallMs, a: Arrival) {
        let Some(t) = &self.trust else { return };
        let Some(chain) = t.chain_for(&self.me, wall) else {
            return;
        };
        let certs: Vec<Vec<u8>> = chain.iter().map(|c| c.to_cbor()).collect();
        let body = self.body(wall, Payload::CertAnnounce(CertAnnounce { certs }));
        self.processed.insert(body.msg_id, wall);
        self.reflooded.insert(body.msg_id, now);
        let envs: Vec<_> = self
            .seal_all(now, wall, &body)
            .into_iter()
            .take(1)
            .collect();
        self.reply_via(now, a, &envs);
    }

    // ---- Sync ---------------------------------------------------------

    /// After waking, a network change or a relay reconnect: sync with every
    /// connected relay and up to 3 random direct peers.
    pub(crate) fn send_wake_sync(&mut self, now: Mono, wall: WallMs, relay_only: bool) {
        if self.current.is_none() {
            return;
        }
        let req = SyncRequest {
            want: want::OPEN_ALARMS
                | want::RECENT_ALARM_EVENTS
                | want::REVOCATIONS
                | want::KEYSHARES_FOR_ME,
            since: None,
            ids: vec![],
            to: None,
            alarm_digest: Some(self.heartbeat_digests(wall).0),
        };
        // Via relays: ask one present device, not everyone.
        let present: Vec<DeviceId> = self
            .relays
            .values()
            .filter(|r| r.connected)
            .flat_map(|r| r.presence.iter().copied())
            .collect();
        if !present.is_empty() {
            let i = self.rng.below_incl(present.len() as u64 - 1) as usize;
            let mut r = req.clone();
            r.to = Some(present[i]);
            let body = self.body(wall, Payload::SyncRequest(r));
            let envs = self.seal_all(now, wall, &body);
            self.processed.insert(body.msg_id, wall);
            self.relay_all(&envs);
        }
        if relay_only {
            return;
        }
        let mut cands: Vec<(DeviceId, SocketAddrV4)> = self
            .peers
            .iter()
            .filter(|p| p.device_id != self.me)
            .filter_map(|p| p.tcp_targets().first().map(|a| (p.device_id, *a)))
            .collect();
        for _ in 0..3 {
            if cands.is_empty() {
                break;
            }
            let i = self.rng.below_incl(cands.len() as u64 - 1) as usize;
            let (d, a) = cands.swap_remove(i);
            let mut r = req.clone();
            r.to = Some(d);
            let body = self.body(wall, Payload::SyncRequest(r));
            let envs = self.seal_all(now, wall, &body);
            self.processed.insert(body.msg_id, wall);
            self.tcp(a, &envs);
        }
    }

    pub(crate) fn send_sync_request_to(
        &mut self,
        now: Mono,
        wall: WallMs,
        d: DeviceId,
        req: SyncRequest,
    ) {
        let body = self.body(wall, Payload::SyncRequest(req));
        let envs = self.seal_all(now, wall, &body);
        self.processed.insert(body.msg_id, wall);
        let Some(p) = self.peers.get(&d) else { return };
        let class = p.class(now);
        let tcp = p.tcp_targets().first().copied();
        // TCP is trusted on the same LAN segment or once seen from them;
        // otherwise send by UDP too, since a known port does not mean TCP
        // is routable.
        let tcp_trusted = p.tcp_trusted(now);
        let ucast = p.unicast_targets(self.cfg.port);
        if class == PeerClass::RelayOnly {
            self.relay_all(&envs);
        }
        if !tcp_trusted || tcp.is_none() {
            for a in ucast {
                self.udp(UdpDest::Unicast(a), &envs);
            }
        }
        if let Some(a) = tcp {
            self.tcp(a, &envs);
        }
    }

    /// Sends a sync request back the way a message arrived.
    pub(crate) fn send_sync_request_via(
        &mut self,
        now: Mono,
        wall: WallMs,
        req: SyncRequest,
        a: Arrival,
    ) {
        if self.current.is_none() {
            return;
        }
        if let Some(to) = req.to
            && self.peers.get(&to).is_some()
            && a.path != PathKind::Relay
            && a.from.is_none()
        {
            return self.send_sync_request_to(now, wall, to, req);
        }
        let body = self.body(wall, Payload::SyncRequest(req));
        let envs = self.seal_all(now, wall, &body);
        self.processed.insert(body.msg_id, wall);
        self.reply_via(now, a, &envs);
    }

    /// Replies the way a message arrived: relay to relay, TCP to TCP, UDP to
    /// UDP. TCP is added for UDP arrivals only if that peer has reached us by
    /// TCP before, since a known TCP port does not mean TCP is routable.
    fn reply_via(&mut self, now: Mono, a: Arrival, envs: &[Vec<u8>]) {
        let Some(from) = a.from else {
            self.relay_all(envs);
            return;
        };
        if a.path == PathKind::Relay {
            self.relay_all(envs);
            return;
        }
        let peer = self.peers.by_ip(from.ip()).and_then(|d| self.peers.get(&d));
        let tcp_port = peer.map(|p| p.tcp_port).filter(|p| *p != 0);
        // Same LAN segment, or TCP seen from them: TCP is likely routable.
        let tcp_seen = peer.is_some_and(|p| p.tcp_trusted(now));
        // TCP to the address its TCP came from, if known.
        let tcp_ip = peer.and_then(|p| p.tcp_ip).unwrap_or(*from.ip());
        match a.path {
            PathKind::Tcp => {
                let port = tcp_port.unwrap_or(self.cfg.port);
                self.tcp(SocketAddrV4::new(*from.ip(), port), envs);
            }
            _ => {
                self.udp(UdpDest::Unicast(from), envs);
                if tcp_seen && let Some(port) = tcp_port {
                    self.tcp(SocketAddrV4::new(tcp_ip, port), envs);
                }
            }
        }
    }

    pub(crate) fn send_sync_response(
        &mut self,
        now: Mono,
        wall: WallMs,
        to: DeviceId,
        envs: Vec<Vec<u8>>,
        a: Arrival,
    ) {
        if envs.is_empty() {
            return;
        }
        // A UDP-only route back cannot carry a wrapped response larger than a
        // datagram: send each original envelope on its own as well. They are
        // independently sealed and signed, so this is equivalent.
        let relay_path = a.path == PathKind::Relay
            || self
                .peers
                .get(&to)
                .is_some_and(|p| p.class(now) == PeerClass::RelayOnly);
        let tcp_ok = a.path == PathKind::Tcp
            || self
                .peers
                .get(&to)
                .is_some_and(|p| p.tcp_port != 0 && p.tcp_trusted(now));
        if !relay_path && !tcp_ok {
            // Three times: these links are the lossy ones, and each envelope
            // has no other retransmission.
            for _ in 0..3 {
                for e in &envs {
                    self.reply_via(now, a, std::slice::from_ref(e));
                }
            }
            return;
        }
        let mut chunks: Vec<Vec<Vec<u8>>> = vec![Vec::new()];
        let mut size = 0;
        for e in envs {
            if size + e.len() > SYNC_CHUNK && !chunks.last().is_some_and(|c| c.is_empty()) {
                chunks.push(Vec::new());
                size = 0;
            }
            size += e.len();
            if let Some(c) = chunks.last_mut() {
                c.push(e);
            }
        }
        let n = chunks.len();
        for (i, c) in chunks.into_iter().enumerate() {
            let body = self.body(
                wall,
                Payload::SyncResponse(SyncResponse {
                    envelopes: c,
                    more: i + 1 < n,
                }),
            );
            self.processed.insert(body.msg_id, wall);
            let out = self.seal_all(now, wall, &body);
            let out: Vec<_> = out.into_iter().take(1).collect();
            let via = if a.path == PathKind::Relay
                || self
                    .peers
                    .get(&to)
                    .is_some_and(|p| p.class(now) == PeerClass::RelayOnly)
            {
                Arrival {
                    path: PathKind::Relay,
                    from: None,
                    relay: a.relay,
                }
            } else {
                a
            };
            self.reply_via(now, via, &out);
        }
    }

    /// Brings a peer on an older epoch up to date (flow B step 3).
    pub(crate) fn send_keyshare_to(&mut self, now: Mono, wall: WallMs, d: DeviceId) {
        if self
            .keyshare_sent
            .get(&d)
            .is_some_and(|t| now.since(*t) < SYNC_RATE_LIMIT)
        {
            return;
        }
        let Some((epoch, key)) = self.current.clone() else {
            return;
        };
        let Some(p) = self.practice.clone() else {
            return;
        };
        let Some(t) = &self.trust else { return };
        // Only share an epoch whose cause we know: the revocation that
        // rotated to it. Otherwise a KeyShare that overtook its Revocation
        // could hand the new key to the device being revoked (P6).
        let explained = epoch == 0
            || self.revocation_epochs.contains(&epoch)
            || self.rotated_epochs.contains(&epoch);
        if !explained || t.is_device_revoked(&d) {
            return;
        }
        let Some(kx) = t
            .valid_certs(&d, wall)
            .iter()
            .map(|c| c.subject_kx)
            .find(|k| *k != [0; 32])
        else {
            return;
        };
        let Ok(ks) = make_keyshare(
            self.rng.bytes(),
            self.rng.bytes(),
            &p.practice_id,
            d,
            &kx,
            epoch,
            &key,
        ) else {
            return;
        };
        self.keyshare_sent.insert(d, now);
        let body = self.body(wall, Payload::KeyShare(ks.clone()));
        let env = self.open_env(&body);
        self.processed.insert(body.msg_id, wall);
        self.reflooded.insert(body.msg_id, now);
        self.keyshare_envs
            .insert(body.msg_id, (d, epoch, env.clone()));
        self.persist(PersistOp::AddKeyShare(env.clone()));
        let envs = vec![env];
        self.send_direct(now, &d, &envs, true);
        self.send_lan_group(&envs);
    }

    pub(crate) fn flood_new(&mut self, now: Mono, wall: WallMs, envs: &[Vec<u8>]) {
        self.send_flood(now, wall, envs);
    }
}
