// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The core state machine.

mod admin;
mod recv;
mod send;

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddrV4;

use crate::crypto::{DeviceKeys, GroupKey, TrustStore};
use crate::proto::{
    AudioStatus, Cbor, CertSerial, DeviceId, JoinRequest, MsgId, PathKind, PracticeSettings,
    PracticeTag, SessionState, Source, StatusLevel,
};

use crate::core::alarm::{self, AlarmMessages};
use crate::core::config::{EffectiveConfig, LocalConfig};
use crate::core::drill::DrillReport;
use crate::core::hold::HoldMachine;
use crate::core::io::{
    AlarmPhase, AlarmView, AudioCmd, AuditEvent, DropReason, Input, InterfaceInfo, Output, RelayId,
    RelayLinkState, SenderPhase, StatusView, UiEvent, UserAction,
};
use crate::core::peers::PeerTable;
use crate::core::persist::{AlarmLocal, PersistOp, PersistedState, PracticeInfo};
use crate::core::rng::CoreRng;
use crate::core::status::{self, StatusInputs};
use crate::core::time::*;

/// The device identity the core runs as.
#[derive(Debug, Clone)]
pub struct Identity {
    pub keys: DeviceKeys,
}

/// One alarm known to this device.
#[derive(Debug, Clone, Default)]
pub(crate) struct AlarmEntry {
    pub msgs: AlarmMessages,
    pub envelopes: BTreeMap<MsgId, Vec<u8>>,
    pub local: AlarmLocal,
    pub known: bool,
    pub last_view: Option<AlarmView>,
    pub receipt_env: Option<Vec<u8>>,
    pub last_receipt_send: Option<Mono>,
    pub closed_shown: bool,
    pub raise_requested: Option<Mono>,
    pub skew_logged: bool,
}

/// An alarm raised by this device.
#[derive(Debug, Clone)]
pub(crate) struct OwnAlarm {
    pub t0: Mono,
    pub envelopes: Vec<Vec<u8>>,
    pub next_udp: Mono,
    pub tcp_resends_done: usize,
    pub phase: SenderPhase,
    pub drill: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Reflood {
    pub at: Mono,
    pub msg_id: MsgId,
    pub envelopes: Vec<Vec<u8>>,
    pub first_from: Option<SocketAddrV4>,
    pub arrived_via_relay: bool,
    pub suppressed: bool,
    pub origin: DeviceId,
    /// Re-floods of alarm raises repeat on bridge paths (ADR 0013).
    pub repeats_left: usize,
    pub repeat: bool,
    /// Heard by multicast or broadcast (so the LAN segment has it).
    pub heard_lan: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Pending {
    pub sender: DeviceId,
    pub heartbeat: bool,
    pub bytes: Vec<u8>,
    pub path: PathKind,
    pub from: Option<SocketAddrV4>,
    pub relay: Option<RelayId>,
    pub until: Mono,
}

#[derive(Debug, Clone)]
pub(crate) struct EventRetx {
    pub at: Vec<Mono>,
    pub envelopes: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub(crate) struct DrillRun {
    pub t0: Mono,
    pub started_at: WallMs,
    pub expected: Vec<DeviceId>,
    pub rtt: BTreeMap<DeviceId, u64>,
    pub close_at: Mono,
}

#[derive(Debug, Clone)]
pub(crate) struct JoinReq {
    pub req: JoinRequest,
    pub from: Option<SocketAddrV4>,
    pub at: Mono,
}

#[derive(Debug, Clone)]
pub(crate) struct Joining {
    pub started: Mono,
    pub next: Mono,
    pub hint: Option<crate::proto::PracticeId>,
    pub approval: Option<admin::PendingApproval>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RelayInfo {
    pub connected: bool,
    pub presence: BTreeSet<DeviceId>,
}

/// The current practice config message.
#[derive(Debug, Clone)]
pub(crate) struct ConfigMsg {
    pub version: u64,
    pub msg_id: MsgId,
    pub settings: PracticeSettings,
    pub envelope: Vec<u8>,
}

/// The Panic Button core: all protocol and alarm logic, with no I/O, no clocks
/// and no global randomness (ADR 0002).
pub struct Core {
    pub(crate) keys: DeviceKeys,
    pub(crate) me: DeviceId,
    pub(crate) rng: CoreRng,
    pub(crate) local: LocalConfig,
    pub(crate) cfg: EffectiveConfig,

    pub(crate) practice: Option<PracticeInfo>,
    pub(crate) trust: Option<TrustStore>,
    pub(crate) own_serial: Option<CertSerial>,
    /// All held group keys, by tag.
    pub(crate) keys_by_tag: BTreeMap<PracticeTag, (u64, GroupKey)>,
    /// When each epoch was first held.
    pub(crate) epoch_since: BTreeMap<u64, WallMs>,
    /// The epoch and key used for sending.
    pub(crate) current: Option<(u64, GroupKey)>,
    pub(crate) higher_epoch_seen: Option<u64>,
    pub(crate) config_msg: Option<ConfigMsg>,
    /// This device's own label and room, set by the person or an admin.
    pub(crate) own_settings: Option<crate::proto::DeviceSettings>,
    /// Admin: label and room changes not yet seen in the target's
    /// heartbeat, re-sent until they are (ADR 0014).
    pub(crate) pending_settings: BTreeMap<DeviceId, admin::PendingSettings>,

    pub(crate) peers: PeerTable,
    pub(crate) alarms: BTreeMap<MsgId, AlarmEntry>,
    pub(crate) own_alarms: BTreeMap<MsgId, OwnAlarm>,
    pub(crate) processed: BTreeMap<MsgId, WallMs>,
    /// Envelopes already verified: hash -> (msg id, the sender if the
    /// message is one that tracks paths, first seen).
    pub(crate) env_seen: BTreeMap<[u8; 16], (MsgId, Option<DeviceId>, Mono)>,
    pub(crate) pending: Vec<Pending>,
    pub(crate) refloods: BTreeMap<MsgId, Reflood>,
    pub(crate) reflooded: BTreeMap<MsgId, Mono>,
    pub(crate) known_has: BTreeMap<MsgId, BTreeSet<DeviceId>>,
    pub(crate) event_retx: Vec<EventRetx>,
    pub(crate) revocation_envs: BTreeMap<MsgId, Vec<u8>>,
    pub(crate) keyshare_envs: BTreeMap<MsgId, (DeviceId, u64, Vec<u8>)>,
    pub(crate) drills: BTreeMap<MsgId, DrillRun>,
    pub(crate) join_requests: BTreeMap<DeviceId, JoinReq>,
    /// Unenrolled devices: sealed messages heard on the LAN, by practice
    /// tag and sender address, for "Panic Button is running on N computers
    /// on this network" in first run.
    pub(crate) nearby: BTreeMap<PracticeTag, BTreeMap<SocketAddrV4, Mono>>,
    pub(crate) joining: Option<Joining>,

    pub(crate) relays: BTreeMap<RelayId, RelayInfo>,
    pub(crate) interfaces: Vec<InterfaceInfo>,
    pub(crate) discovered: BTreeMap<SocketAddrV4, Mono>,
    pub(crate) sync_sent: BTreeMap<DeviceId, Mono>,
    pub(crate) cert_sync_sent: BTreeMap<DeviceId, (Mono, u32)>,
    /// Peers whose last heartbeat had a different state digest.
    pub(crate) sd_mismatch: BTreeSet<DeviceId>,
    pub(crate) cert_wanted: BTreeMap<DeviceId, (Mono, recv::Arrival)>,
    pub(crate) last_announce: Option<Mono>,
    pub(crate) deferred_raises: Option<Vec<(MsgId, PathKind)>>,
    /// Epochs announced by revocations we hold, and epochs we rotated to.
    pub(crate) revocation_epochs: BTreeSet<u64>,
    pub(crate) rotated_epochs: BTreeSet<u64>,
    pub(crate) ad_mismatch: BTreeSet<DeviceId>,
    pub(crate) keyshare_sent: BTreeMap<DeviceId, Mono>,
    pub(crate) skew_logged: BTreeSet<DeviceId>,

    pub(crate) audio: Option<AudioStatus>,
    pub(crate) override_available: bool,
    pub(crate) session: SessionState,
    /// macOS Local Network permission refused (`Input::LocalNetworkBlocked`).
    pub(crate) local_network_blocked: bool,
    pub(crate) user_display: String,
    pub(crate) hold: HoldMachine,
    pub(crate) sound_on: bool,
    pub(crate) override_on: bool,
    pub(crate) near_misses: Vec<WallMs>,

    pub(crate) next_heartbeat: Mono,
    pub(crate) next_presence: Mono,
    pub(crate) next_status: Mono,
    pub(crate) next_prune: Mono,
    pub(crate) started_at: Mono,
    pub(crate) last_input: Option<Mono>,
    pub(crate) last_wall: WallMs,
    pub(crate) status: Option<StatusView>,
    pub(crate) few_admins_warned: Option<usize>,
    pub(crate) diagnostics: BTreeMap<DropReason, u64>,

    pub(crate) restore: Option<PersistedState>,
    pub(crate) restoring: bool,
    pub(crate) out: Vec<Output>,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core")
            .field("me", &self.me)
            .field("enrolled", &self.practice.is_some())
            .finish_non_exhaustive()
    }
}

impl Core {
    pub fn new(
        identity: Identity,
        state: PersistedState,
        local: LocalConfig,
        rng_seed: [u8; 32],
    ) -> Self {
        let me = identity.keys.device_id();
        let cfg = EffectiveConfig::resolve(&local, &PracticeSettings::default());
        Core {
            keys: identity.keys,
            me,
            rng: CoreRng::from_seed(rng_seed),
            local,
            cfg,
            practice: None,
            trust: None,
            own_serial: None,
            keys_by_tag: BTreeMap::new(),
            epoch_since: BTreeMap::new(),
            current: None,
            higher_epoch_seen: None,
            config_msg: None,
            own_settings: None,
            pending_settings: BTreeMap::new(),
            peers: PeerTable::default(),
            alarms: BTreeMap::new(),
            own_alarms: BTreeMap::new(),
            processed: BTreeMap::new(),
            env_seen: BTreeMap::new(),
            pending: Vec::new(),
            refloods: BTreeMap::new(),
            reflooded: BTreeMap::new(),
            known_has: BTreeMap::new(),
            event_retx: Vec::new(),
            revocation_envs: BTreeMap::new(),
            keyshare_envs: BTreeMap::new(),
            drills: BTreeMap::new(),
            join_requests: BTreeMap::new(),
            nearby: BTreeMap::new(),
            joining: None,
            relays: BTreeMap::new(),
            interfaces: Vec::new(),
            discovered: BTreeMap::new(),
            sync_sent: BTreeMap::new(),
            cert_sync_sent: BTreeMap::new(),
            sd_mismatch: BTreeSet::new(),
            cert_wanted: BTreeMap::new(),
            last_announce: None,
            deferred_raises: None,
            revocation_epochs: BTreeSet::new(),
            rotated_epochs: BTreeSet::new(),
            ad_mismatch: BTreeSet::new(),
            keyshare_sent: BTreeMap::new(),
            skew_logged: BTreeSet::new(),
            audio: None,
            override_available: false,
            session: SessionState::Active,
            local_network_blocked: false,
            user_display: String::new(),
            hold: HoldMachine::new(),
            sound_on: false,
            override_on: false,
            near_misses: Vec::new(),
            next_heartbeat: Mono::ZERO,
            next_presence: Mono::ZERO,
            next_status: Mono::ZERO,
            next_prune: Mono::ZERO,
            started_at: Mono::ZERO,
            last_input: None,
            last_wall: 0,
            status: None,
            few_admins_warned: None,
            diagnostics: BTreeMap::new(),
            restore: Some(state),
            restoring: false,
            out: Vec::new(),
        }
    }

    pub fn device_id(&self) -> DeviceId {
        self.me
    }

    pub fn is_enrolled(&self) -> bool {
        self.practice.is_some()
    }

    pub fn practice(&self) -> Option<&PracticeInfo> {
        self.practice.as_ref()
    }

    /// Admin devices: join requests heard in the last 20 s (a joining
    /// device repeats its request every 5 s until it stops), as
    /// `(device, label, code)`.
    pub fn pending_joins(&self, now: Mono) -> Vec<(DeviceId, String, String)> {
        self.join_requests
            .iter()
            .filter(|(_, j)| now.since(j.at) <= 4 * JOIN_RESEND)
            .map(|(d, j)| (*d, j.req.label.clone(), j.req.code.clone()))
            .collect()
    }

    /// Unenrolled devices: how many computers of each practice were heard
    /// on the network in the last minute, largest first. Empty once
    /// enrolled.
    pub fn nearby(&self, now: Mono) -> Vec<usize> {
        if self.practice.is_some() {
            return Vec::new();
        }
        let mut v: Vec<usize> = self
            .nearby
            .values()
            .map(|a| {
                a.values()
                    .filter(|at| now.since(**at) <= NEARBY_WINDOW)
                    .count()
            })
            .filter(|n| *n > 0)
            .collect();
        v.sort_unstable_by(|a, b| b.cmp(a));
        v
    }

    /// While joining by approval (flow C): this device's six-digit code,
    /// and the practice name once an admin has approved.
    pub fn joining(&self) -> Option<(String, Option<String>)> {
        self.joining.as_ref().map(|j| {
            (
                crate::crypto::join_code(&self.me.0, &self.keys.kx_pub()),
                j.approval.as_ref().map(|a| a.info.name.clone()),
            )
        })
    }

    pub fn trust(&self) -> Option<&TrustStore> {
        self.trust.as_ref()
    }

    /// The practice-wide settings (the latest PracticeConfig), and its
    /// version (0 if none has been published).
    pub fn practice_settings(&self) -> (crate::proto::PracticeSettings, u64) {
        self.config_msg
            .as_ref()
            .map(|c| (c.settings.clone(), c.version))
            .unwrap_or_default()
    }

    pub fn config(&self) -> &EffectiveConfig {
        &self.cfg
    }

    pub fn status(&self) -> Option<&StatusView> {
        self.status.as_ref()
    }

    pub fn current_epoch(&self) -> Option<u64> {
        self.current.as_ref().map(|(e, _)| *e)
    }

    /// The practice tag of the current epoch (for mDNS advertising).
    pub fn practice_tag(&self) -> Option<PracticeTag> {
        self.current.as_ref().map(|(_, k)| k.tag())
    }

    pub fn peers(&self) -> &PeerTable {
        &self.peers
    }

    /// A label or room this admin assigned to `device` that its heartbeat
    /// does not show yet (ADR 0014).
    pub fn pending_settings(&self, device: &DeviceId) -> Option<&crate::proto::DeviceSettings> {
        self.pending_settings.get(device).map(|p| &p.settings)
    }

    /// The alarm and state digests this device would put in a heartbeat now
    /// (diagnostics and tests).
    #[doc(hidden)]
    pub fn digests(&self, wall: WallMs) -> ([u8; 16], [u8; 16]) {
        self.heartbeat_digests(wall)
    }

    /// Devices known to have a message (diagnostics and tests).
    #[doc(hidden)]
    pub fn known_has(&self, id: &MsgId) -> Vec<DeviceId> {
        self.known_has
            .get(id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn diagnostics(&self) -> &BTreeMap<DropReason, u64> {
        &self.diagnostics
    }

    /// Current views of every known alarm.
    pub fn alarm_views(&self, wall: WallMs) -> Vec<AlarmView> {
        self.alarms
            .iter()
            .filter_map(|(id, e)| alarm::view(*id, &e.msgs, &self.me, e.local.local_ack, wall))
            .collect()
    }

    /// Fresh entropy from the OS RNG (the runtime calls this when
    /// [`Core::needs_reseed`] is true).
    pub fn reseed(&mut self, seed: [u8; 32]) {
        self.rng.reseed(seed);
    }

    pub fn needs_reseed(&self) -> bool {
        self.rng.needs_reseed()
    }

    /// Feeds one input; returns the effects the runtime must perform, in
    /// order.
    pub fn handle(&mut self, now: Mono, wall: WallMs, input: Input) -> Vec<Output> {
        self.last_wall = wall;
        if let Some(state) = self.restore.take() {
            self.started_at = now;
            self.restore_state(now, wall, state);
            self.on_wake(now, wall);
        } else if let Some(last) = self.last_input
            && now.since(last) > WAKE_GAP
        {
            self.on_wake(now, wall);
        }
        self.last_input = Some(now);
        self.peers.set_now(now);
        match input {
            Input::Tick => {}
            Input::Datagram { kind, from, bytes } => {
                if self.participating() {
                    self.receive(now, wall, &bytes, kind.path(), Some(from), None, false);
                }
            }
            Input::StreamFrame { from, bytes } => {
                if self.participating() {
                    self.receive(now, wall, &bytes, PathKind::Tcp, Some(from), None, false);
                }
            }
            Input::RelayFrame { relay, bytes } => {
                if self.participating() {
                    self.receive(now, wall, &bytes, PathKind::Relay, None, Some(relay), false);
                }
            }
            Input::RelayState {
                relay,
                state,
                presence,
            } => self.on_relay_state(now, wall, relay, state, presence),
            Input::Discovered { addr } => {
                self.discovered.insert(addr, now);
            }
            Input::LocalNetworkBlocked(b) => {
                let was = self.local_network_blocked;
                self.local_network_blocked = b;
                if was && !b {
                    self.on_wake(now, wall);
                }
            }
            Input::InterfacesChanged(ifs) => {
                self.interfaces = ifs;
                self.on_wake(now, wall);
            }
            Input::AudioStatus {
                status,
                override_available,
            } => {
                self.audio = Some(status);
                self.override_available = override_available;
            }
            Input::SessionState(s) => {
                let was = self.participating();
                self.session = s;
                // Back in use (the user switched back, or a remote session
                // ended): catch up at once rather than at the next sync.
                if !was && self.participating() {
                    self.on_wake(now, wall);
                }
            }
            Input::UserDisplay(n) => self.user_display = n.trim().to_owned(),
            Input::User(a) => self.on_user(now, wall, a),
            Input::ConfigChanged(local) => {
                self.local = local;
                self.recompute_config();
            }
        }
        self.tick(now, wall);
        std::mem::take(&mut self.out)
    }

    /// Earliest time at which `handle(.., Input::Tick)` must be called.
    pub fn next_deadline(&self) -> Option<Mono> {
        let mut d = vec![self.next_status, self.next_prune];
        if self.practice.is_some() {
            d.push(self.next_heartbeat);
            d.push(self.next_presence);
        }
        if let Some(h) = self.hold.deadline(self.cfg.hold_ms) {
            d.push(h);
        }
        for o in self.own_alarms.values() {
            d.push(o.next_udp);
            if o.phase != SenderPhase::Delivered && o.phase != SenderPhase::CouldNotReach {
                d.push(o.t0 + SENDER_NOT_YET_DELIVERED);
                d.push(o.t0 + SENDER_COULD_NOT_REACH);
            }
            if let Some(t) = TCP_RESEND.get(o.tcp_resends_done) {
                d.push(o.t0 + *t);
            }
        }
        d.extend(self.refloods.values().map(|r| r.at));
        d.extend(self.event_retx.iter().flat_map(|e| e.at.iter().copied()));
        d.extend(self.pending.iter().map(|p| p.until));
        d.extend(self.drills.values().map(|r| r.close_at));
        if let Some(j) = &self.joining {
            d.push(j.next);
        }
        d.into_iter().min()
    }

    pub(crate) fn participating(&self) -> bool {
        match self.session {
            SessionState::RemoteSessionDisabled => self.cfg.allow_remote_session,
            // Fast user switching: the session at the screen takes part.
            SessionState::Inactive => false,
            _ => true,
        }
    }

    pub(crate) fn emit(&mut self, o: Output) {
        if self.restoring {
            match &o {
                Output::Ui(UiEvent::AlarmShown { resumed: true, .. })
                | Output::Audio(AudioCmd::StartAlarm { .. })
                | Output::SetSystemVolumeOverride(_) => {}
                _ => return,
            }
        }
        self.out.push(o);
    }

    pub(crate) fn ui(&mut self, e: UiEvent) {
        self.emit(Output::Ui(e));
    }

    pub(crate) fn log(&mut self, e: AuditEvent) {
        self.emit(Output::Log(e));
    }

    pub(crate) fn persist(&mut self, op: PersistOp) {
        self.emit(Output::Persist(op));
    }

    pub(crate) fn drop_msg(&mut self, r: DropReason) {
        *self.diagnostics.entry(r).or_default() += 1;
    }

    pub(crate) fn recompute_config(&mut self) {
        let practice = self
            .config_msg
            .as_ref()
            .map(|c| c.settings.clone())
            .unwrap_or_default();
        self.cfg = EffectiveConfig::resolve(&self.local, &practice);
        if let Some(own) = &self.own_settings {
            if let Some(l) = own.label.as_ref().filter(|_| !self.local.label_locked) {
                self.cfg.label = l.clone();
            }
            if let Some(r) = own.room.as_ref().filter(|_| !self.local.room_locked) {
                self.cfg.room = r.clone();
            }
        }
    }

    fn on_relay_state(
        &mut self,
        now: Mono,
        wall: WallMs,
        relay: RelayId,
        state: RelayLinkState,
        presence: Option<Vec<DeviceId>>,
    ) {
        let info = self.relays.entry(relay).or_default();
        let was = info.connected;
        info.connected = state == RelayLinkState::Connected;
        if !info.connected {
            // We know at once that these devices are no longer reachable
            // through this relay (FR-STS-003).
            let gone: Vec<DeviceId> = std::mem::take(&mut info.presence).into_iter().collect();
            for d in gone {
                self.peers.relay_gone(&d);
            }
        }
        if let Some(p) = presence {
            info.presence = p.into_iter().filter(|d| *d != self.me).collect();
        }
        if info.connected && !was {
            // Relay (re)connect: heartbeat and sync (`spec/03-protocol.md`,
            // Anti-entropy).
            self.next_heartbeat = now;
            self.send_wake_sync(now, wall, true);
        }
    }

    /// Wake from sleep, a network change, or start-up: heartbeat now and
    /// sync with relays and a few direct peers.
    pub(crate) fn on_wake(&mut self, now: Mono, wall: WallMs) {
        self.next_heartbeat = now;
        if self.practice.is_some() && self.last_announce.is_none_or(|t| now.since(t) >= SECOND) {
            self.last_announce = Some(now);
            self.send_cert_announce(now, wall);
            self.send_wake_sync(now, wall, false);
        }
    }

    pub(crate) fn tick(&mut self, now: Mono, wall: WallMs) {
        // Hold machine.
        let hold_ms = self.cfg.hold_ms;
        let outcome = self.hold.tick(now, hold_ms);
        self.on_hold_outcome(now, wall, outcome);
        if let Some(p) = self.hold.progress_due(now, hold_ms) {
            self.ui(UiEvent::HoldProgress {
                fraction_permille: p,
            });
        }

        if let Some(j) = &self.joining
            && now >= j.next
        {
            self.send_join_request(now, wall);
        }

        if self.practice.is_some() && self.participating() {
            for (relay, info) in &self.relays {
                let _ = relay;
                if info.connected {
                    for d in info.presence.clone() {
                        self.peers.relay_present(d, now);
                    }
                }
            }
            if now >= self.next_heartbeat {
                self.send_heartbeat(now, wall);
                let jitter = self.rng.below_incl(2 * HEARTBEAT_JITTER);
                self.next_heartbeat = now + HEARTBEAT_INTERVAL - HEARTBEAT_JITTER + jitter;
            }
            if now >= self.next_presence {
                if now > self.started_at {
                    self.send_presence(now, wall);
                }
                self.next_presence = now + PRESENCE_INTERVAL;
            }
            self.run_own_alarm_timers(now, wall);
            self.run_refloods(now);
            self.run_event_retx(now);
            self.run_drills(now, wall);
            if !self.cert_wanted.is_empty() {
                self.run_cert_requests(now, wall);
            }
        }
        self.expire_pending(now);
        if now >= self.next_prune {
            self.prune(now, wall);
            self.next_prune = now + MINUTE;
        }
        if now >= self.next_status {
            self.update_status(now, wall);
            self.next_status = now + SECOND;
        }
    }

    fn expire_pending(&mut self, now: Mono) {
        let before = self.pending.len();
        self.pending.retain(|p| p.until > now);
        let dropped = before - self.pending.len();
        for _ in 0..dropped {
            self.drop_msg(DropReason::UnknownCert);
        }
    }

    fn prune(&mut self, now: Mono, wall: WallMs) {
        let cutoff = wall.saturating_sub(PROCESSED_ID_TTL);
        let before = self.processed.len();
        self.processed.retain(|_, at| *at >= cutoff);
        if self.processed.len() != before {
            self.persist(PersistOp::PruneProcessedBefore(cutoff));
        }
        // Bound the in-memory id set (50,000 entries).
        while self.processed.len() > 50_000 {
            let oldest = self
                .processed
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(id, _)| *id);
            match oldest {
                Some(id) => {
                    self.processed.remove(&id);
                }
                None => break,
            }
        }
        self.env_seen
            .retain(|_, (_, _, at)| now.since(*at) <= 5 * MINUTE);
        self.reflooded.retain(|_, at| now.since(*at) <= DAY);
        self.known_has
            .retain(|id, _| self.processed.contains_key(id));
        self.sync_sent.retain(|_, t| now.since(*t) <= HOUR);
        self.cert_sync_sent
            .retain(|_, (t, _)| now.since(*t) <= HOUR);
        self.keyshare_sent.retain(|_, t| now.since(*t) <= HOUR);
        self.discovered.retain(|_, t| now.since(*t) <= HOUR);
        self.join_requests
            .retain(|_, j| now.since(j.at) <= JOIN_MAX);
        for addrs in self.nearby.values_mut() {
            addrs.retain(|_, at| now.since(*at) <= NEARBY_WINDOW);
        }
        self.nearby.retain(|_, addrs| !addrs.is_empty());
        self.near_misses.retain(|t| wall.saturating_sub(*t) <= DAY);
        self.peers.prune(wall);
        // Alarms: forget envelopes 24 h after close or expiry.
        let mut gone = Vec::new();
        for (id, e) in &self.alarms {
            let Some(r) = &e.msgs.raised else {
                if wall.saturating_sub(e.local.first_seen) > DAY {
                    gone.push(*id);
                }
                continue;
            };
            let end = match e.msgs.derive().closed {
                Some(c) => c.at,
                None => r.created_at.saturating_add(ALARM_MAX_LIFE),
            };
            if wall.saturating_sub(end) > ENVELOPE_KEEP_AFTER_CLOSE {
                gone.push(*id);
            }
        }
        for id in gone {
            self.alarms.remove(&id);
            self.own_alarms.remove(&id);
            self.persist(PersistOp::PruneAlarm(id));
        }
        if let Some(t) = self.trust.as_mut() {
            t.prune_revoked(wall, 30 * DAY);
        }
    }

    pub(crate) fn status_inputs(&self, now: Mono, wall: WallMs) -> StatusInputs {
        let enrolled = self.practice.is_some();
        let (own_valid, own_revoked, other_members) = match &self.trust {
            Some(t) => {
                let valid = t.is_member(&self.me, wall);
                let revoked = t.is_device_revoked(&self.me);
                // Any cert for another subject (member, admin or enroller)
                // means this is not a single-device practice.
                let others = t.certs().filter(|(_, b)| b.subject != self.me).count();
                (valid, revoked, others)
            }
            None => (false, false, 0),
        };
        let mut direct = 0;
        let mut via_relay = 0;
        for p in self.peers.iter() {
            if p.device_id == self.me {
                continue;
            }
            if let Some(t) = &self.trust
                && !t.is_member(&p.device_id, wall)
            {
                continue;
            }
            match p.class(now) {
                crate::core::peers::PeerClass::LanGroup
                | crate::core::peers::PeerClass::UnicastOnly => direct += 1,
                crate::core::peers::PeerClass::RelayOnly => via_relay += 1,
                _ => {}
            }
        }
        let stale_epoch = match (self.higher_epoch_seen, self.current_epoch()) {
            (Some(h), Some(c)) => h > c,
            _ => false,
        };
        StatusInputs {
            enrolled,
            own_cert_valid: own_valid,
            own_revoked,
            have_group_key: self.current.is_some(),
            remote_session_disabled: self.session == SessionState::RemoteSessionDisabled
                && !self.cfg.allow_remote_session,
            session_inactive: self.session == SessionState::Inactive,
            local_network_blocked: self.local_network_blocked,
            other_members,
            direct,
            via_relay,
            audio: self.audio,
            override_available: self.override_available,
            override_mute: self.cfg.override_mute,
            relays_configured: usize::from(self.cfg.relay_count),
            relays_connected: self.relays.values().filter(|r| r.connected).count(),
            stale_epoch,
        }
    }

    fn update_status(&mut self, now: Mono, wall: WallMs) {
        let v = status::compute(&self.status_inputs(now, wall));
        if self.status.as_ref() != Some(&v) {
            self.status = Some(v.clone());
            self.ui(UiEvent::StatusChanged(v));
        }
        if let Some(t) = &self.trust
            && t.is_admin(&self.me, wall)
        {
            let admins = t
                .members(wall)
                .iter()
                .filter(|d| t.is_admin(d, wall))
                .count();
            if admins < 2 && self.few_admins_warned != Some(admins) {
                self.few_admins_warned = Some(admins);
                self.ui(UiEvent::FewAdmins(admins));
            }
        }
    }

    pub(crate) fn status_level(&self) -> StatusLevel {
        self.status
            .as_ref()
            .map(|s| s.level)
            .unwrap_or(StatusLevel::Red)
    }

    fn on_user(&mut self, now: Mono, wall: WallMs, a: UserAction) {
        match a {
            UserAction::HoldStart { source, practice } => {
                if !practice && (!self.participating() || self.practice.is_none()) {
                    // Still run the hold so the gesture feels the same, but
                    // only practice holds can complete without enrolment.
                }
                if self.hold.start(now, source, practice) == crate::core::hold::HoldOutcome::Started
                {
                    if self.cfg.cue_mode != crate::proto::CueMode::Silent {
                        self.emit(Output::Audio(AudioCmd::StartCue {
                            mode: self.cfg.cue_mode,
                            hold_ms: self.cfg.hold_ms,
                        }));
                    }
                    if let Some(p) = self.hold.progress_due(now, self.cfg.hold_ms) {
                        self.ui(UiEvent::HoldProgress {
                            fraction_permille: p,
                        });
                    }
                }
            }
            UserAction::HoldEnd { source } => {
                let o = self.hold.end(now, source, self.cfg.hold_ms);
                self.on_hold_outcome(now, wall, o);
            }
            UserAction::RaiseNow { source } => {
                self.raise(now, wall, source, false);
            }
            UserAction::Respond { alarm_id } => {
                self.local_ack(now, wall, alarm_id, crate::core::io::LocalAck::Responding)
            }
            UserAction::Seen { alarm_id } => {
                self.local_ack(now, wall, alarm_id, crate::core::io::LocalAck::Seen)
            }
            UserAction::Resolve { alarm_id } => self.close_alarm(now, wall, alarm_id, false),
            UserAction::Cancel { alarm_id } => self.close_alarm(now, wall, alarm_id, true),
            UserAction::RunDrill => {
                self.raise(now, wall, Source::Cli, true);
            }
            UserAction::ApproveJoin { device } => self.approve_join(now, wall, device),
            UserAction::RejectJoin { device } => {
                self.join_requests.remove(&device);
            }
            UserAction::Promote { device } => self.promote(now, wall, device),
            UserAction::InstallOwnCert(raw) => self.install_own_cert(now, wall, raw),
            UserAction::SetOwnSettings { label, room } => {
                self.apply_own_settings(label, room, wall, None);
            }
            UserAction::AssignDeviceSettings {
                device,
                label,
                room,
            } => self.assign_device_settings(now, wall, device, label, room),
            UserAction::Revoke {
                device,
                cascade,
                reason,
            } => self.revoke(now, wall, device, cascade, &reason),
            UserAction::PublishSettings(s) => self.publish_settings(now, wall, s),
            UserAction::StartJoin { practice_hint } => self.start_join(now, wall, practice_hint),
            UserAction::ConfirmJoin => self.confirm_join(now, wall),
            UserAction::CancelJoin => self.joining = None,
        }
    }

    fn on_hold_outcome(&mut self, now: Mono, wall: WallMs, o: crate::core::hold::HoldOutcome) {
        use crate::core::hold::HoldOutcome;
        match o {
            HoldOutcome::None | HoldOutcome::Started => {}
            HoldOutcome::Released => {
                self.emit(Output::Audio(AudioCmd::StopCue));
                self.ui(UiEvent::HoldEnded);
            }
            HoldOutcome::NearMiss {
                source,
                duration_ms,
            } => {
                self.emit(Output::Audio(AudioCmd::StopCue));
                self.ui(UiEvent::HoldEnded);
                self.near_misses.push(wall);
                self.ui(UiEvent::NearMiss {
                    source,
                    duration_ms,
                });
                self.log(AuditEvent::NearMiss {
                    source,
                    duration_ms,
                });
            }
            HoldOutcome::Triggered { source, practice } => {
                // Stop the cue before anything else (within 20 ms of trigger).
                self.emit(Output::Audio(AudioCmd::StopCue));
                // The hold machine is already idle, so the regular progress
                // events have stopped short of full: show the ring full
                // before it resets.
                self.ui(UiEvent::HoldProgress {
                    fraction_permille: 1000,
                });
                self.ui(UiEvent::HoldEnded);
                if practice {
                    self.ui(UiEvent::PracticeComplete);
                } else {
                    self.raise(now, wall, source, false);
                }
            }
        }
    }

    /// Recomputes an alarm's view; emits UI events for changes, and starts
    /// or stops the alarm sound.
    pub(crate) fn refresh_alarm(&mut self, now: Mono, wall: WallMs, id: MsgId) {
        let _ = now;
        let Some(e) = self.alarms.get(&id) else {
            return;
        };
        let Some(v) = alarm::view(id, &e.msgs, &self.me, e.local.local_ack, wall) else {
            return;
        };
        if e.last_view.as_ref() == Some(&v) {
            return;
        }
        let closed = matches!(
            v.state,
            AlarmPhase::Resolved { .. } | AlarmPhase::Cancelled { .. }
        );
        let alerted = e.local.alerted;
        let closed_shown = e.closed_shown;
        let own = v.own;
        if let Some(e) = self.alarms.get_mut(&id) {
            e.last_view = Some(v.clone());
            if closed {
                e.closed_shown = true;
            }
        }
        if own {
            let phase = self
                .own_alarms
                .get(&id)
                .map(|o| o.phase)
                .unwrap_or(SenderPhase::Requested);
            let phase = if v.delivered > 0 {
                SenderPhase::Delivered
            } else {
                phase
            };
            if let Some(o) = self.own_alarms.get_mut(&id) {
                o.phase = phase;
            }
            if !v.drill {
                self.ui(UiEvent::SenderPanel { alarm: v, phase });
            }
        } else if alerted {
            if closed && !closed_shown {
                self.ui(UiEvent::AlarmClosed(v));
            } else {
                self.ui(UiEvent::AlarmUpdated(v));
            }
        }
        self.update_sound();
    }

    /// Plays the alarm while any alarm is alerting locally.
    pub(crate) fn update_sound(&mut self) {
        let alerting = self.alarms.values().any(|e| {
            e.local.alerted && e.local.local_ack.is_none() && e.msgs.derive().closed.is_none()
        });
        if alerting && !self.sound_on {
            self.sound_on = true;
            self.emit(Output::Audio(AudioCmd::StartAlarm {
                pattern: self.cfg.sound_pattern,
            }));
            if self.cfg.override_mute && self.override_available && !self.override_on {
                self.override_on = true;
                self.emit(Output::SetSystemVolumeOverride(true));
                self.log(AuditEvent::VolumeOverride { on: true });
            }
        } else if !alerting && self.sound_on {
            self.sound_on = false;
            self.emit(Output::Audio(AudioCmd::StopAlarm));
            if self.override_on {
                self.override_on = false;
                self.emit(Output::SetSystemVolumeOverride(false));
                self.log(AuditEvent::VolumeOverride { on: false });
            }
        }
    }

    pub(crate) fn drill_report(&self, id: &MsgId, run: &DrillRun) -> DrillReport {
        let d = self
            .alarms
            .get(id)
            .map(|e| e.msgs.derive())
            .unwrap_or_default();
        let rows = run
            .expected
            .iter()
            .map(|dev| {
                let peer = self.peers.get(dev);
                let rc = alarm::receipt_of(&d, dev);
                crate::core::drill::DrillRow {
                    device: *dev,
                    label: peer
                        .and_then(|p| p.heartbeat.as_ref())
                        .map(|h| h.label.clone())
                        .unwrap_or_default(),
                    room: peer
                        .and_then(|p| p.heartbeat.as_ref())
                        .map(|h| h.room.clone())
                        .unwrap_or_default(),
                    received: d.receipts.contains_key(dev),
                    rtt_ms: run.rtt.get(dev).copied(),
                    path: rc.and_then(|r| r.path),
                    audio: rc.and_then(|r| r.audio),
                }
            })
            .collect();
        DrillReport {
            alarm_id: *id,
            started_at: run.started_at,
            rows,
        }
    }

    pub(crate) fn heartbeat_digests(&self, wall: WallMs) -> ([u8; 16], [u8; 16]) {
        let mut pairs = Vec::new();
        for (id, e) in &self.alarms {
            let Some(r) = &e.msgs.raised else { continue };
            let closed_at = e.msgs.derive().closed.map(|c| c.at);
            let recent_raise = wall.saturating_sub(r.created_at) <= ALARM_MAX_LIFE;
            let recent_close = closed_at.is_some_and(|c| wall.saturating_sub(c) <= ALARM_MAX_LIFE);
            if recent_raise || recent_close {
                for m in e.msgs.message_ids(*id) {
                    pairs.push((*id, m));
                }
            }
        }
        let ad = crate::core::digest::alarm_digest(pairs);
        let sd = match &self.trust {
            Some(t) => crate::core::digest::state_digest(
                t.serials(),
                t.revocation_ids(),
                self.config_msg.as_ref().map(|c| c.version).unwrap_or(0),
                self.current_epoch().unwrap_or(0),
            ),
            None => [0; 16],
        };
        (ad, sd)
    }

    pub(crate) fn restore_state(&mut self, now: Mono, wall: WallMs, s: PersistedState) {
        self.restoring = true;
        if let Some(p) = s.practice.clone() {
            self.install_practice(p, s.own_serial);
        }
        for k in &s.group_keys {
            self.add_group_key(k.epoch, k.key.clone(), k.at);
        }
        if let Some(t) = self.trust.as_mut() {
            for raw in &s.certs {
                if let Ok(c) = crate::proto::Cert::from_cbor(raw) {
                    let _ = t.add_cert(c);
                }
            }
        }
        for env in &s.revocations {
            self.receive(now, wall, env, PathKind::Sync, None, None, true);
        }
        for env in &s.keyshares {
            self.receive(now, wall, env, PathKind::Sync, None, None, true);
        }
        if let Some(c) = &s.config {
            self.receive(now, wall, c, PathKind::Sync, None, None, true);
        }
        if let Some(own) = s
            .own_settings
            .as_ref()
            .and_then(|b| crate::proto::DeviceSettings::from_cbor(b).ok())
        {
            self.own_settings = Some(own);
            self.recompute_config();
        }
        for (id, stored) in &s.alarms {
            let entry = self.alarms.entry(*id).or_default();
            entry.local = stored.local;
            for env in stored.envelopes.values() {
                self.receive_alarm_restore(now, wall, env);
            }
        }
        // Load persisted ids last, so restoring the alarms above is not
        // mistaken for duplicates (FR-ALM-010).
        for (id, at) in &s.processed {
            self.processed.entry(*id).or_insert(*at);
        }
        // Resume interrupted, unacknowledged, open alerts (ADR 0013).
        let ids: Vec<MsgId> = self.alarms.keys().copied().collect();
        for id in ids {
            let Some(e) = self.alarms.get(&id) else {
                continue;
            };
            let Some(v) = alarm::view(id, &e.msgs, &self.me, e.local.local_ack, wall) else {
                continue;
            };
            let resume = e.local.alerted
                && e.local.local_ack.is_none()
                && v.state == AlarmPhase::Open
                && !v.own
                && !v.drill;
            if let Some(e) = self.alarms.get_mut(&id) {
                e.last_view = Some(v.clone());
                e.known = true;
            }
            if v.own && v.state == AlarmPhase::Open {
                // Keep retransmitting our own open alarm after a restart.
                let envelopes: Vec<Vec<u8>> = self
                    .alarms
                    .get(&id)
                    .and_then(|e| e.envelopes.get(&id).cloned())
                    .into_iter()
                    .collect();
                self.own_alarms.insert(
                    id,
                    OwnAlarm {
                        t0: now,
                        envelopes,
                        next_udp: now,
                        tcp_resends_done: 0,
                        phase: if v.delivered > 0 {
                            SenderPhase::Delivered
                        } else {
                            SenderPhase::Requested
                        },
                        drill: v.drill,
                    },
                );
            }
            if resume {
                self.ui(UiEvent::AlarmShown {
                    alarm: v,
                    resumed: true,
                });
            }
        }
        self.update_sound();
        self.restoring = false;
        self.recompute_config();
    }
}
