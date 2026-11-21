// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The simulated world: nodes running real [`Core`]s, a discrete event
//! queue in virtual time, and a network of segments, routes and relays
//! (`spec/07-testing.md`, Simulator model).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::net::{Ipv4Addr, SocketAddrV4};

use panic_button::core::config::LocalConfig;
use panic_button::core::io::{
    AudioCmd, AuditEvent, InterfaceInfo, RelayLinkState, UdpDest, UdpKind, UiEvent, UserAction,
};
use panic_button::core::persist::PersistedState;
use panic_button::core::rng::CoreRng;
use panic_button::core::{Core, Identity, Input, Mono, Output};
use panic_button::crypto::DeviceKeys;
use panic_button::proto::{AudioStatus, DeviceId, MsgId, SessionState, StatusLevel};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::{Rng, SeedableRng};

use crate::practice::{self, PracticeSetup};

pub const PORT: u16 = 47474;
/// Wall-clock time at virtual t = 0 (2026-11-14).
pub const WALL_BASE: u64 = 1_763_100_000_000;

/// An L2 broadcast domain.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub multicast_ok: bool,
    pub broadcast_ok: bool,
    /// Nodes on the segment cannot reach each other at all, but can reach
    /// routed destinations.
    pub client_isolation: bool,
    /// Per-packet, per-receiver loss probability in thousandths.
    pub loss_permille: u32,
    pub latency_min: u64,
    pub latency_max: u64,
    pub dup_permille: u32,
}

impl Default for Segment {
    fn default() -> Self {
        Segment {
            multicast_ok: true,
            broadcast_ok: true,
            client_isolation: false,
            loss_permille: 0,
            latency_min: 1,
            latency_max: 5,
            dup_permille: 0,
        }
    }
}

/// What the simulated person at a device does when an alert appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserModel {
    Respond { after: u64 },
    Seen { after: u64 },
    Never,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    pub segments: Vec<usize>,
    /// Wall clock offset from true time, ms.
    pub clock_offset: i64,
    /// Nodes configured as seed peers.
    pub seeds: Vec<usize>,
    /// Relays configured (and reachable unless an event says otherwise).
    pub relays: Vec<usize>,
    pub audio: AudioStatus,
    pub override_available: bool,
    pub user: UserModel,
    /// Which practice this node belongs to.
    pub practice: usize,
    /// Start time (virtual ms).
    pub start_at: u64,
    /// False: the node starts with keys but no practice, ready to join by
    /// approval (flow C).
    pub enrolled: bool,
}

impl Default for NodeSpec {
    fn default() -> Self {
        NodeSpec {
            segments: vec![0],
            clock_offset: 0,
            seeds: vec![],
            relays: vec![],
            audio: AudioStatus {
                output_present: true,
                muted: false,
                volume_pct: 80,
            },
            override_available: false,
            user: UserModel::Never,
            practice: 0,
            start_at: 0,
            enrolled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorldEvent {
    /// A node's link to a segment goes down or up.
    Link {
        node: usize,
        segment: usize,
        up: bool,
    },
    Sleep(usize),
    Wake(usize),
    Crash(usize),
    Restart(usize),
    RelayUp(usize, bool),
    /// A node's reachability to a relay.
    RelayReach {
        node: usize,
        relay: usize,
        up: bool,
    },
    /// Change a node's wall clock offset.
    ClockJump {
        node: usize,
        offset: i64,
    },
    SegmentFlags {
        segment: usize,
        multicast_ok: bool,
        broadcast_ok: bool,
        client_isolation: bool,
    },
    Audio {
        node: usize,
        status: AudioStatus,
    },
    /// The node's session state (locked, remote, another user's session).
    Session {
        node: usize,
        state: SessionState,
    },
    /// The signed-in person's display name on the node (empty: none).
    UserDisplay {
        node: usize,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldSpec {
    pub seed: u64,
    pub segments: Vec<Segment>,
    /// `route_udp[a][b]`: unicast UDP from segment a can reach segment b.
    pub route_udp: Vec<Vec<bool>>,
    pub route_tcp: Vec<Vec<bool>>,
    pub nodes: Vec<NodeSpec>,
    pub relays: usize,
    pub relay_latency: (u64, u64),
    pub practices: usize,
    /// Extra admin nodes (the first node of each practice is always admin).
    pub admins: Vec<usize>,
    pub events: Vec<(u64, WorldEvent)>,
}

impl WorldSpec {
    /// A flat LAN of `n` nodes on one segment.
    pub fn flat(n: usize, seed: u64) -> Self {
        WorldSpec {
            seed,
            segments: vec![Segment::default()],
            route_udp: vec![vec![true]],
            route_tcp: vec![vec![true]],
            nodes: (0..n).map(|_| NodeSpec::default()).collect(),
            relays: 0,
            relay_latency: (10, 40),
            practices: 1,
            admins: vec![],
            events: vec![],
        }
    }

    pub fn no_routes(segments: usize) -> Vec<Vec<bool>> {
        (0..segments)
            .map(|a| (0..segments).map(|b| a == b).collect())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TraceEv {
    Ui(UiEvent),
    Audio(AudioCmd),
    Log(AuditEvent),
    Volume(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraceEntry {
    pub t: u64,
    pub node: usize,
    pub ev: TraceEv,
}

#[derive(Debug, Clone)]
enum Ev {
    Deliver {
        node: usize,
        epoch: u64,
        input: Input,
    },
    Tick {
        node: usize,
        epoch: u64,
    },
    World(WorldEvent),
    User {
        node: usize,
        action: UserAction,
    },
    Start(usize),
}

pub(crate) struct SimNode {
    pub keys: DeviceKeys,
    pub core: Option<Core>,
    pub state: PersistedState,
    pub awake: bool,
    /// Incremented on crash/restart, so in-flight events for the old
    /// process are discarded.
    pub epoch: u64,
    pub next_tick: Option<u64>,
    pub ips: Vec<(usize, Ipv4Addr)>,
    pub link_up: Vec<bool>,
    pub relay_reach: Vec<bool>,
    pub relay_connected: Vec<bool>,
    pub relay_cursor: Vec<usize>,
    pub clock_offset: i64,
    pub audio: AudioStatus,
    pub seed_counter: u64,
    pub local: LocalConfig,
}

#[derive(Debug, Default)]
pub(crate) struct SimRelay {
    pub up: bool,
    /// (practice, from node, bytes)
    pub buffer: Vec<(usize, usize, Vec<u8>)>,
}

/// Packet counters.
#[derive(Debug, Default, Clone)]
pub struct Counters {
    pub packets: u64,
    pub udp: u64,
    pub tcp: u64,
    pub relay: u64,
    /// Packets per 100 ms bucket, for traffic windows.
    pub timeline: BTreeMap<u64, u64>,
    /// Packets by message type (when the simulator can open them).
    pub by_type: BTreeMap<String, u64>,
    /// First transmissions per 100 ms bucket: a packet whose (sender,
    /// envelope, transport) has not been sent before. Repeats are
    /// retransmissions (P7 excludes them).
    pub first_timeline: BTreeMap<u64, u64>,
    seen: std::collections::BTreeSet<(usize, [u8; 16], u8)>,
}

pub struct World {
    pub spec: WorldSpec,
    pub now: u64,
    queue: BinaryHeap<Reverse<(u64, u64, usize)>>,
    events: BTreeMap<usize, Ev>,
    next_ev: usize,
    seq: u64,
    rng: ChaCha20Rng,
    pub(crate) nodes: Vec<SimNode>,
    pub(crate) relays: Vec<SimRelay>,
    pub segments: Vec<Segment>,
    pub trace: Vec<TraceEntry>,
    pub counters: Counters,
    pub practices: Vec<PracticeSetup>,
    /// Alarms raised by legitimate nodes: id -> (origin node, true time).
    pub raised: BTreeMap<MsgId, (usize, u64)>,
    /// Status changes per node: (t, level).
    pub status_log: Vec<Vec<(u64, StatusLevel)>>,
    /// Copies of every envelope sent, for replay attacks: (t, bytes).
    pub captured: Vec<(u64, Vec<u8>)>,
    pub capture: bool,
    /// Decrypt and count every packet by type (slow; for analysis).
    pub classify_traffic: bool,
    /// Record every send by these nodes: (t, node, description).
    pub watch: Vec<usize>,
    pub watched: Vec<(u64, usize, String)>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "World(t={}, nodes={})", self.now, self.nodes.len())
    }
}

fn node_ip(segment: usize, node: usize) -> Ipv4Addr {
    let n = node + 1;
    Ipv4Addr::new(10, segment as u8, (n >> 8) as u8, (n & 0xff) as u8)
}

impl World {
    pub fn new(spec: WorldSpec) -> Self {
        let mut rng = ChaCha20Rng::seed_from_u64(spec.seed);
        let mut core_rng = CoreRng::from_seed({
            let mut s = [0u8; 32];
            rng.fill_bytes(&mut s);
            s
        });
        let practices = practice::setup(&mut core_rng, &spec);
        let mut nodes = Vec::new();
        for (i, ns) in spec.nodes.iter().enumerate() {
            let (keys, state) = practices[ns.practice].node_state(i);
            let ips: Vec<(usize, Ipv4Addr)> =
                ns.segments.iter().map(|s| (*s, node_ip(*s, i))).collect();
            let seed_addrs = ns
                .seeds
                .iter()
                .filter_map(|s| {
                    spec.nodes
                        .get(*s)
                        .and_then(|n| n.segments.first())
                        .map(|seg| SocketAddrV4::new(node_ip(*seg, *s), PORT))
                })
                .collect();
            let local = LocalConfig {
                label: format!("N{i}"),
                room: format!("Room {i}"),
                seed_addrs,
                relay_count: ns.relays.len() as u16,
                ..Default::default()
            };
            nodes.push(SimNode {
                keys,
                core: None,
                state,
                awake: true,
                epoch: 0,
                next_tick: None,
                ips,
                link_up: vec![true; spec.segments.len()],
                relay_reach: vec![true; spec.relays],
                relay_connected: vec![false; spec.relays],
                relay_cursor: vec![0; spec.relays],
                clock_offset: ns.clock_offset,
                audio: ns.audio,
                seed_counter: i as u64,
                local,
            });
        }
        let n = nodes.len();
        let mut w = World {
            segments: spec.segments.clone(),
            relays: (0..spec.relays)
                .map(|_| SimRelay {
                    up: true,
                    buffer: Vec::new(),
                })
                .collect(),
            spec,
            now: 0,
            queue: BinaryHeap::new(),
            events: BTreeMap::new(),
            next_ev: 0,
            seq: 0,
            rng,
            nodes,
            trace: Vec::new(),
            counters: Counters::default(),
            practices,
            raised: BTreeMap::new(),
            status_log: vec![Vec::new(); n],
            captured: Vec::new(),
            capture: false,
            classify_traffic: false,
            watch: Vec::new(),
            watched: Vec::new(),
        };
        for i in 0..n {
            let at = w.spec.nodes[i].start_at;
            w.push(at, Ev::Start(i));
        }
        let evs = w.spec.events.clone();
        for (t, e) in evs {
            w.push(t, Ev::World(e));
        }
        w
    }

    fn push(&mut self, t: u64, ev: Ev) {
        let id = self.next_ev;
        self.next_ev += 1;
        self.events.insert(id, ev);
        self.seq += 1;
        self.queue.push(Reverse((t, self.seq, id)));
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn device_id(&self, node: usize) -> DeviceId {
        self.nodes[node].keys.device_id()
    }

    pub fn node_of(&self, d: &DeviceId) -> Option<usize> {
        self.nodes.iter().position(|n| n.keys.device_id() == *d)
    }

    pub fn core(&self, node: usize) -> Option<&Core> {
        self.nodes[node].core.as_ref()
    }

    pub fn wall_of(&self, node: usize) -> u64 {
        (WALL_BASE as i64 + self.now as i64 + self.nodes[node].clock_offset) as u64
    }

    pub fn is_up(&self, node: usize) -> bool {
        self.nodes[node].core.is_some() && self.nodes[node].awake
    }

    pub fn ip(&self, node: usize, segment: usize) -> Option<Ipv4Addr> {
        self.nodes[node]
            .ips
            .iter()
            .find(|(s, _)| *s == segment)
            .map(|(_, ip)| *ip)
    }

    /// Schedules a user action.
    pub fn user(&mut self, at: u64, node: usize, action: UserAction) {
        self.push(at, Ev::User { node, action });
    }

    /// Schedules a world event.
    pub fn at(&mut self, t: u64, e: WorldEvent) {
        self.push(t, Ev::World(e));
    }

    /// Injects raw bytes at a node as if received by UDP multicast from
    /// `from`.
    pub fn inject(&mut self, at: u64, node: usize, from: SocketAddrV4, bytes: Vec<u8>) {
        let epoch = self.nodes[node].epoch;
        self.push(
            at,
            Ev::Deliver {
                node,
                epoch,
                input: Input::Datagram {
                    kind: UdpKind::Multicast,
                    from,
                    bytes,
                },
            },
        );
    }

    /// Injects bytes to every node on a segment.
    pub fn inject_segment(&mut self, at: u64, segment: usize, from: SocketAddrV4, bytes: Vec<u8>) {
        for i in 0..self.nodes.len() {
            if self.nodes[i].ips.iter().any(|(s, _)| *s == segment) {
                self.inject(at, i, from, bytes.clone());
            }
        }
    }

    pub fn run_until(&mut self, t: u64) {
        while let Some(Reverse((et, _, id))) = self.queue.peek().copied() {
            if et > t {
                break;
            }
            self.queue.pop();
            self.now = et;
            let Some(ev) = self.events.remove(&id) else {
                continue;
            };
            self.dispatch(ev);
        }
        self.now = t;
    }

    pub fn run_for(&mut self, ms: u64) {
        let t = self.now + ms;
        self.run_until(t);
    }

    fn dispatch(&mut self, ev: Ev) {
        match ev {
            Ev::Start(i) => self.start_node(i),
            Ev::Deliver { node, epoch, input } => {
                if self.nodes[node].epoch == epoch && self.is_up(node) {
                    self.handle(node, input);
                }
            }
            Ev::Tick { node, epoch } => {
                if self.nodes[node].epoch == epoch
                    && self.nodes[node].next_tick == Some(self.now)
                    && self.is_up(node)
                {
                    self.nodes[node].next_tick = None;
                    self.handle(node, Input::Tick);
                }
            }
            Ev::User { node, action } => {
                if self.is_up(node) {
                    self.handle(node, Input::User(action));
                }
            }
            Ev::World(e) => self.world_event(e),
        }
    }

    fn start_node(&mut self, i: usize) {
        let n = &mut self.nodes[i];
        n.seed_counter += 1_000_003;
        let mut seed = [0u8; 32];
        seed[..8].copy_from_slice(&self.spec.seed.to_le_bytes());
        seed[8..16].copy_from_slice(&n.seed_counter.to_le_bytes());
        seed[16..24].copy_from_slice(&(i as u64).to_le_bytes());
        let core = Core::new(
            Identity {
                keys: n.keys.clone(),
            },
            n.state.clone(),
            n.local.clone(),
            seed,
        );
        n.core = Some(core);
        n.awake = true;
        n.next_tick = None;
        let ifs: Vec<InterfaceInfo> = n
            .ips
            .iter()
            .map(|(_, ip)| InterfaceInfo {
                addr: *ip,
                broadcast: None,
            })
            .collect();
        let audio = n.audio;
        let ov = self.spec.nodes[i].override_available;
        self.handle(i, Input::InterfacesChanged(ifs));
        self.handle(
            i,
            Input::AudioStatus {
                status: audio,
                override_available: ov,
            },
        );
        self.handle(i, Input::SessionState(SessionState::Active));
        self.handle(i, Input::UserDisplay(format!("User {i}")));
        self.update_relays();
        self.refresh_relay_state(i);
    }

    fn world_event(&mut self, e: WorldEvent) {
        match e {
            WorldEvent::Link { node, segment, up } => {
                if let Some(l) = self.nodes[node].link_up.get_mut(segment) {
                    *l = up;
                }
                let ifs = self.interfaces(node);
                if self.is_up(node) {
                    self.handle(node, Input::InterfacesChanged(ifs));
                }
            }
            WorldEvent::Sleep(n) => {
                self.nodes[n].awake = false;
                self.update_relays();
            }
            WorldEvent::Wake(n) => {
                if self.nodes[n].core.is_some() && !self.nodes[n].awake {
                    self.nodes[n].awake = true;
                    self.handle(n, Input::Tick);
                    self.update_relays();
                    self.refresh_relay_state(n);
                }
            }
            WorldEvent::Crash(n) => {
                self.nodes[n].core = None;
                self.nodes[n].epoch += 1;
                self.nodes[n].relay_connected.fill(false);
                self.update_relays();
            }
            WorldEvent::Restart(n) => {
                self.nodes[n].epoch += 1;
                self.start_node(n);
            }
            WorldEvent::RelayUp(r, up) => {
                self.relays[r].up = up;
                self.update_relays();
            }
            WorldEvent::RelayReach { node, relay, up } => {
                self.nodes[node].relay_reach[relay] = up;
                self.update_relays();
            }
            WorldEvent::ClockJump { node, offset } => self.nodes[node].clock_offset = offset,
            WorldEvent::SegmentFlags {
                segment,
                multicast_ok,
                broadcast_ok,
                client_isolation,
            } => {
                let s = &mut self.segments[segment];
                s.multicast_ok = multicast_ok;
                s.broadcast_ok = broadcast_ok;
                s.client_isolation = client_isolation;
            }
            WorldEvent::Audio { node, status } => {
                self.nodes[node].audio = status;
                let ov = self.spec.nodes[node].override_available;
                if self.is_up(node) {
                    self.handle(
                        node,
                        Input::AudioStatus {
                            status,
                            override_available: ov,
                        },
                    );
                }
            }
            WorldEvent::Session { node, state } => {
                if self.is_up(node) {
                    self.handle(node, Input::SessionState(state));
                }
            }
            WorldEvent::UserDisplay { node, name } => {
                if self.is_up(node) {
                    self.handle(node, Input::UserDisplay(name));
                }
            }
        }
    }

    fn interfaces(&self, node: usize) -> Vec<InterfaceInfo> {
        let n = &self.nodes[node];
        n.ips
            .iter()
            .filter(|(s, _)| n.link_up.get(*s).copied().unwrap_or(false))
            .map(|(_, ip)| InterfaceInfo {
                addr: *ip,
                broadcast: None,
            })
            .collect()
    }

    /// Recomputes relay sessions and sends RelayState inputs on change.
    fn update_relays(&mut self) {
        for r in 0..self.relays.len() {
            let mut changed = false;
            for i in 0..self.nodes.len() {
                let configured = self.spec.nodes[i].relays.contains(&r);
                let want = configured
                    && self.relays[r].up
                    && self.is_up(i)
                    && self.nodes[i].relay_reach[r];
                if self.nodes[i].relay_connected[r] != want {
                    self.nodes[i].relay_connected[r] = want;
                    changed = true;
                    if want {
                        // Replay buffered envelopes after the cursor
                        // (`spec/06-relay.md`, resume).
                        let practice = self.spec.nodes[i].practice;
                        let cursor = self.nodes[i].relay_cursor[r];
                        let items: Vec<Vec<u8>> = self.relays[r].buffer
                            [cursor.min(self.relays[r].buffer.len())..]
                            .iter()
                            .filter(|(p, from, _)| *p == practice && *from != i)
                            .map(|(_, _, b)| b.clone())
                            .collect();
                        self.nodes[i].relay_cursor[r] = self.relays[r].buffer.len();
                        let rid = self.relay_id(i, r);
                        for b in items {
                            let lat = self.relay_leg();
                            let epoch = self.nodes[i].epoch;
                            let t = self.now + lat;
                            self.push(
                                t,
                                Ev::Deliver {
                                    node: i,
                                    epoch,
                                    input: Input::RelayFrame {
                                        relay: rid,
                                        bytes: b,
                                    },
                                },
                            );
                        }
                    }
                }
            }
            if changed {
                for i in 0..self.nodes.len() {
                    if !self.spec.nodes[i].relays.contains(&r) || !self.is_up(i) {
                        continue;
                    }
                    let practice = self.spec.nodes[i].practice;
                    let presence: Vec<DeviceId> = (0..self.nodes.len())
                        .filter(|j| {
                            *j != i
                                && self.nodes[*j].relay_connected[r]
                                && self.spec.nodes[*j].practice == practice
                        })
                        .map(|j| self.device_id(j))
                        .collect();
                    let state = if self.nodes[i].relay_connected[r] {
                        RelayLinkState::Connected
                    } else {
                        RelayLinkState::Disconnected
                    };
                    let rid = self.relay_id(i, r);
                    let epoch = self.nodes[i].epoch;
                    let t = self.now + 1;
                    self.push(
                        t,
                        Ev::Deliver {
                            node: i,
                            epoch,
                            input: Input::RelayState {
                                relay: rid,
                                state,
                                presence: Some(presence),
                            },
                        },
                    );
                }
            }
        }
    }

    /// Tells a node the current state of each configured relay (the runtime
    /// always knows whether its own sessions are up).
    fn refresh_relay_state(&mut self, i: usize) {
        for r in self.spec.nodes[i].relays.clone() {
            let practice = self.spec.nodes[i].practice;
            let connected = self.nodes[i].relay_connected[r];
            let presence: Vec<DeviceId> = (0..self.nodes.len())
                .filter(|j| {
                    *j != i
                        && self.nodes[*j].relay_connected[r]
                        && self.spec.nodes[*j].practice == practice
                })
                .map(|j| self.device_id(j))
                .collect();
            let rid = self.relay_id(i, r);
            let epoch = self.nodes[i].epoch;
            let t = self.now + 1;
            self.push(
                t,
                Ev::Deliver {
                    node: i,
                    epoch,
                    input: Input::RelayState {
                        relay: rid,
                        state: if connected {
                            RelayLinkState::Connected
                        } else {
                            RelayLinkState::Disconnected
                        },
                        presence: Some(presence),
                    },
                },
            );
        }
    }

    fn relay_id(&self, node: usize, relay: usize) -> u16 {
        self.spec.nodes[node]
            .relays
            .iter()
            .position(|r| *r == relay)
            .unwrap_or(0) as u16
    }

    fn relay_leg(&mut self) -> u64 {
        let (a, b) = self.spec.relay_latency;
        a + self.rand(b.saturating_sub(a) + 1)
    }

    fn rand(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.rng.next_u64() % n }
    }

    fn roll(&mut self, permille: u32) -> bool {
        permille > 0 && self.rand(1000) < u64::from(permille)
    }

    fn latency(&mut self, seg: usize) -> u64 {
        let s = &self.segments[seg];
        let (a, b) = (s.latency_min, s.latency_max.max(s.latency_min));
        a + self.rand(b - a + 1)
    }

    fn handle(&mut self, node: usize, input: Input) {
        let now = self.now;
        let wall = self.wall_of(node);
        let Some(core) = self.nodes[node].core.as_mut() else {
            return;
        };
        let outs = core.handle(Mono(now), wall, input);
        let next = core.next_deadline();
        if let Some(status) = core.status() {
            let lvl = status.level;
            if self.status_log[node].last().map(|(_, l)| *l) != Some(lvl) {
                self.status_log[node].push((now, lvl));
            }
        }
        for o in outs {
            self.apply_output(node, o);
        }
        if let Some(d) = next {
            let t = d.0.max(now + 1);
            let n = &mut self.nodes[node];
            if n.next_tick.is_none_or(|x| t < x || x <= now) {
                n.next_tick = Some(t);
                let epoch = n.epoch;
                self.push(t, Ev::Tick { node, epoch });
            }
        }
    }

    fn bucket(&mut self) {
        *self.counters.timeline.entry(self.now / 100).or_default() += 1;
    }

    fn first(&mut self, node: usize, bytes: &[u8], kind: u8) {
        let h = hash16(bytes);
        if self.counters.seen.insert((node, h, kind)) {
            *self
                .counters
                .first_timeline
                .entry(self.now / 100)
                .or_default() += 1;
        }
    }

    /// Message type of an envelope, for traffic analysis.
    pub fn classify(&self, bytes: &[u8]) -> String {
        use panic_button::proto::{Cbor, Envelope, SignedMessage};
        match Envelope::from_cbor(bytes) {
            Ok(Envelope::Sealed(s)) => {
                for p in &self.practices {
                    if let Ok(m) = panic_button::crypto::open(&p.group_key, &s)
                        && let Ok(b) = panic_button::proto::Body::from_cbor(&m.body)
                    {
                        if let panic_button::proto::Payload::AlarmEvent(e) = &b.payload {
                            return format!("AlarmEvent/{:?}/{}", e.event, b.msg_id);
                        }
                        return format!("{:?}", b.msg_type());
                    }
                }
                "sealed?".into()
            }
            Ok(Envelope::Open(o)) => SignedMessage::from_cbor(&o.payload)
                .ok()
                .and_then(|m| panic_button::proto::Body::from_cbor(&m.body).ok())
                .map(|b| match &b.payload {
                    panic_button::proto::Payload::KeyShare(k) => format!(
                        "KeyShare/to {:?}/epoch {}/from {:?}",
                        self.node_of(&k.recipient),
                        k.epoch,
                        self.node_of(&b.sender)
                    ),
                    _ => format!("{:?}", b.msg_type()),
                })
                .unwrap_or_else(|| "open?".into()),
            Err(_) => "garbage".into(),
        }
    }

    fn count_type(&mut self, kind: &str, bytes: &[u8]) {
        if self.classify_traffic {
            let t = format!("{kind}:{}", self.classify(bytes));
            if std::env::var_os("CLASSIFY_TIMES").is_some() && t == "ucast:CertAnnounce" {
                *self
                    .counters
                    .by_type
                    .entry(format!("t{}", self.now / 1000))
                    .or_default() += 1;
            }
            *self.counters.by_type.entry(t).or_default() += 1;
        }
    }

    fn apply_output(&mut self, node: usize, o: Output) {
        if self.watch.contains(&node) {
            let d = match &o {
                Output::SendDatagram { dest, bytes } => {
                    Some(format!("udp {dest:?} {}", self.classify(bytes)))
                }
                Output::SendStream { to, bytes } => {
                    Some(format!("tcp {to} {}", self.classify(bytes)))
                }
                Output::SendRelay { bytes, .. } => Some(format!("relay {}", self.classify(bytes))),
                _ => None,
            };
            if let Some(d) = d {
                self.watched.push((self.now, node, d));
            }
        }
        match o {
            Output::SendDatagram { dest, bytes } => {
                if self.capture {
                    self.captured.push((self.now, bytes.clone()));
                }
                match dest {
                    UdpDest::Multicast | UdpDest::Broadcast => {
                        let kind = if dest == UdpDest::Multicast {
                            UdpKind::Multicast
                        } else {
                            UdpKind::Broadcast
                        };
                        let ips = self.nodes[node].ips.clone();
                        for (seg, ip) in ips {
                            if !self.nodes[node].link_up[seg] {
                                continue;
                            }
                            self.count_type(
                                if kind == UdpKind::Multicast {
                                    "mcast"
                                } else {
                                    "bcast"
                                },
                                &bytes,
                            );
                            self.first(
                                node,
                                &bytes,
                                if kind == UdpKind::Multicast { 0 } else { 1 },
                            );
                            self.counters.packets += 1;
                            self.counters.udp += 1;
                            self.bucket();
                            let s = self.segments[seg].clone();
                            let ok = if kind == UdpKind::Multicast {
                                s.multicast_ok
                            } else {
                                s.broadcast_ok
                            };
                            if !ok || s.client_isolation {
                                continue;
                            }
                            for j in 0..self.nodes.len() {
                                if j == node
                                    || !self.nodes[j].ips.iter().any(|(sg, _)| *sg == seg)
                                    || !self.nodes[j].link_up[seg]
                                {
                                    continue;
                                }
                                self.deliver_udp(seg, j, kind, SocketAddrV4::new(ip, PORT), &bytes);
                            }
                        }
                    }
                    UdpDest::Unicast(to) => {
                        self.count_type("ucast", &bytes);
                        let k = 2 + (u32::from(*to.ip()) as u8 % 200);
                        self.first(node, &bytes, k);
                        self.counters.packets += 1;
                        self.counters.udp += 1;
                        self.bucket();
                        if let Some((j, dst_seg, src)) = self.route(node, *to.ip(), false) {
                            self.deliver_udp(
                                dst_seg,
                                j,
                                UdpKind::Unicast,
                                SocketAddrV4::new(src, PORT),
                                &bytes,
                            );
                        }
                    }
                }
            }
            Output::SendStream { to, bytes } => {
                self.count_type("tcp", &bytes);
                self.first(node, &bytes, 250);
                self.counters.packets += 1;
                self.counters.tcp += 1;
                self.bucket();
                if let Some((j, dst_seg, src)) = self.route(node, *to.ip(), true) {
                    // TCP is reliable: loss shows up as retransmission delay.
                    let mut lat = self.latency(dst_seg);
                    let loss = self.segments[dst_seg].loss_permille;
                    let mut tries = 0;
                    while self.roll(loss) && tries < 5 {
                        lat += 200 << tries;
                        tries += 1;
                    }
                    let epoch = self.nodes[j].epoch;
                    let from = SocketAddrV4::new(src, 40000 + node as u16);
                    let t = self.now + lat;
                    self.push(
                        t,
                        Ev::Deliver {
                            node: j,
                            epoch,
                            input: Input::StreamFrame { from, bytes },
                        },
                    );
                }
            }
            Output::SendRelay { relay, bytes } => {
                if self.capture {
                    self.captured.push((self.now, bytes.clone()));
                }
                let practice = self.spec.nodes[node].practice;
                for r in 0..self.relays.len() {
                    if !self.nodes[node].relay_connected[r] {
                        continue;
                    }
                    if let Some(rid) = relay
                        && self.relay_id(node, r) != rid
                    {
                        continue;
                    }
                    self.first(node, &bytes, 251);
                    self.counters.packets += 1;
                    self.counters.relay += 1;
                    self.bucket();
                    self.relays[r].buffer.push((practice, node, bytes.clone()));
                    let up_leg = self.relay_leg();
                    for j in 0..self.nodes.len() {
                        if j == node
                            || !self.nodes[j].relay_connected[r]
                            || self.spec.nodes[j].practice != practice
                        {
                            continue;
                        }
                        self.nodes[j].relay_cursor[r] = self.relays[r].buffer.len();
                        let lat = up_leg + self.relay_leg();
                        let epoch = self.nodes[j].epoch;
                        let rid = self.relay_id(j, r);
                        let t = self.now + lat;
                        self.push(
                            t,
                            Ev::Deliver {
                                node: j,
                                epoch,
                                input: Input::RelayFrame {
                                    relay: rid,
                                    bytes: bytes.clone(),
                                },
                            },
                        );
                    }
                }
            }
            Output::Persist(op) => self.nodes[node].state.apply(op),
            Output::Ui(e) => {
                if let UiEvent::AlarmShown { resumed: false, .. } = &e {
                    self.schedule_user_response(node, &e);
                }
                if let UiEvent::SenderPanel { alarm, .. } = &e
                    && !self.raised.contains_key(&alarm.alarm_id)
                {
                    self.raised.insert(alarm.alarm_id, (node, self.now));
                }
                self.trace.push(TraceEntry {
                    t: self.now,
                    node,
                    ev: TraceEv::Ui(e),
                });
            }
            Output::Audio(a) => self.trace.push(TraceEntry {
                t: self.now,
                node,
                ev: TraceEv::Audio(a),
            }),
            Output::SetSystemVolumeOverride(b) => self.trace.push(TraceEntry {
                t: self.now,
                node,
                ev: TraceEv::Volume(b),
            }),
            Output::Log(l) => {
                if let AuditEvent::AlarmRaised { alarm_id, .. } = &l {
                    self.raised.entry(*alarm_id).or_insert((node, self.now));
                }
                self.trace.push(TraceEntry {
                    t: self.now,
                    node,
                    ev: TraceEv::Log(l),
                });
            }
        }
    }

    fn schedule_user_response(&mut self, node: usize, e: &UiEvent) {
        let UiEvent::AlarmShown { alarm, .. } = e else {
            return;
        };
        let action = match self.spec.nodes[node].user {
            UserModel::Respond { after } => Some((
                after,
                UserAction::Respond {
                    alarm_id: alarm.alarm_id,
                },
            )),
            UserModel::Seen { after } => Some((
                after,
                UserAction::Seen {
                    alarm_id: alarm.alarm_id,
                },
            )),
            UserModel::Never => None,
        };
        if let Some((after, a)) = action {
            let t = self.now + after;
            self.push(t, Ev::User { node, action: a });
        }
    }

    /// Finds the destination node for an IP and whether the sender can
    /// reach it. Returns (node, destination segment, source IP).
    fn route(&self, from: usize, ip: Ipv4Addr, tcp: bool) -> Option<(usize, usize, Ipv4Addr)> {
        let (j, dst_seg) = self
            .nodes
            .iter()
            .enumerate()
            .find_map(|(j, n)| n.ips.iter().find(|(_, a)| *a == ip).map(|(s, _)| (j, *s)))?;
        if j == from || !self.nodes[j].link_up[dst_seg] {
            return None;
        }
        let table = if tcp {
            &self.spec.route_tcp
        } else {
            &self.spec.route_udp
        };
        for (src_seg, src_ip) in &self.nodes[from].ips {
            if !self.nodes[from].link_up[*src_seg] {
                continue;
            }
            if *src_seg == dst_seg {
                if !self.segments[dst_seg].client_isolation {
                    return Some((j, dst_seg, *src_ip));
                }
            } else if table
                .get(*src_seg)
                .and_then(|r| r.get(dst_seg))
                .copied()
                .unwrap_or(false)
            {
                return Some((j, dst_seg, *src_ip));
            }
        }
        None
    }

    fn deliver_udp(
        &mut self,
        seg: usize,
        j: usize,
        kind: UdpKind,
        from: SocketAddrV4,
        bytes: &[u8],
    ) {
        let s = self.segments[seg].clone();
        if self.roll(s.loss_permille) {
            return;
        }
        let copies = if self.roll(s.dup_permille) { 2 } else { 1 };
        for _ in 0..copies {
            let lat = self.latency(seg);
            let epoch = self.nodes[j].epoch;
            let t = self.now + lat;
            self.push(
                t,
                Ev::Deliver {
                    node: j,
                    epoch,
                    input: Input::Datagram {
                        kind,
                        from,
                        bytes: bytes.to_vec(),
                    },
                },
            );
        }
    }

    // ---- Queries used by scenarios and properties ----------------------

    /// First time each node showed (alerted for) an alarm.
    pub fn shown(&self, alarm: &MsgId) -> BTreeMap<usize, u64> {
        let mut m = BTreeMap::new();
        for e in &self.trace {
            if let TraceEv::Ui(UiEvent::AlarmShown { alarm: a, .. }) = &e.ev
                && a.alarm_id == *alarm
            {
                m.entry(e.node).or_insert(e.t);
            }
        }
        m
    }

    /// First time each node received an alarm (shown, toast or history).
    pub fn received(&self, alarm: &MsgId) -> BTreeMap<usize, u64> {
        let mut m = BTreeMap::new();
        for e in &self.trace {
            if let TraceEv::Log(AuditEvent::AlarmReceived { alarm_id, .. }) = &e.ev
                && alarm_id == alarm
            {
                m.entry(e.node).or_insert(e.t);
            }
        }
        m
    }

    pub fn status(&self, node: usize) -> Option<StatusLevel> {
        self.core(node).and_then(|c| c.status()).map(|s| s.level)
    }

    /// The most recent alarm raised by `node`.
    pub fn last_alarm_of(&self, node: usize) -> Option<MsgId> {
        self.raised
            .iter()
            .filter(|(_, (n, _))| *n == node)
            .max_by_key(|(_, (_, t))| *t)
            .map(|(id, _)| *id)
    }

    /// Raises an alarm at `node` now (bypassing the hold) and returns its id
    /// after processing.
    pub fn raise_now(&mut self, node: usize) -> Option<MsgId> {
        let before = self.last_alarm_of(node);
        self.handle(
            node,
            Input::User(UserAction::RaiseNow {
                source: panic_button::proto::Source::Button,
            }),
        );
        let after = self.last_alarm_of(node);
        if after != before { after } else { None }
    }

    /// Changes a node's machine config or policy, as a restart or an IT
    /// change would, and tells its core.
    pub fn configure(&mut self, node: usize, f: impl FnOnce(&mut LocalConfig)) {
        f(&mut self.nodes[node].local);
        if self.is_up(node) {
            let local = self.nodes[node].local.clone();
            self.handle(node, Input::ConfigChanged(local));
        }
    }

    pub fn act_now(&mut self, node: usize, action: UserAction) {
        if self.is_up(node) {
            self.handle(node, Input::User(action));
        }
    }

    pub fn ui_events(&self, node: usize) -> impl Iterator<Item = (u64, &UiEvent)> {
        self.trace
            .iter()
            .filter(move |e| e.node == node)
            .filter_map(|e| match &e.ev {
                TraceEv::Ui(u) => Some((e.t, u)),
                _ => None,
            })
    }

    /// First transmissions (excluding retransmissions) in [a, b).
    pub fn first_packets_between(&self, a: u64, b: u64) -> u64 {
        self.counters
            .first_timeline
            .range(a / 100..b.div_ceil(100))
            .map(|(_, n)| *n)
            .sum()
    }

    pub fn packets_between(&self, a: u64, b: u64) -> u64 {
        self.counters
            .timeline
            .range(a / 100..b.div_ceil(100))
            .map(|(_, n)| *n)
            .sum()
    }

    pub fn link_up(&self, node: usize, segment: usize) -> bool {
        self.nodes[node]
            .link_up
            .get(segment)
            .copied()
            .unwrap_or(false)
    }

    pub fn relay_connected(&self, node: usize, relay: usize) -> bool {
        self.nodes[node]
            .relay_connected
            .get(relay)
            .copied()
            .unwrap_or(false)
    }

    pub fn keys(&self, node: usize) -> &DeviceKeys {
        &self.nodes[node].keys
    }

    pub fn persisted(&self, node: usize) -> &PersistedState {
        &self.nodes[node].state
    }
}

fn hash16(b: &[u8]) -> [u8; 16] {
    let mut h = [0u8; 16];
    let mut x: u64 = 0xcbf29ce484222325;
    let mut y: u64 = 0x100000001b3;
    for (i, byte) in b.iter().enumerate() {
        x = (x ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        y = y.rotate_left(5) ^ u64::from(*byte) ^ (i as u64);
        y = y.wrapping_mul(0x9e3779b97f4a7c15);
    }
    h[..8].copy_from_slice(&x.to_le_bytes());
    h[8..].copy_from_slice(&y.to_le_bytes());
    h
}
