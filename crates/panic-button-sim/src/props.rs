// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Ground truth and property checks (`spec/07-testing.md`, Properties).
//!
//! The reachability model mirrors the simulator's network: who can reach
//! whom by LAN group, relay, or unicast once addresses are known. Property
//! P1 uses it to decide which nodes must receive an alarm, and by when.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use panic_button::core::io::{AlarmPhase, AuditEvent, UiEvent};
use panic_button::core::time::{ALARM_MAX_LIFE, PEER_STALE};
use panic_button::crypto::{DeviceKeys, seal, sign_body};
use panic_button::proto::{Body, Cbor, MsgId, Payload, StatusLevel};

use crate::world::{TraceEv, World};

/// Delivery bounds from P1.
pub const BOUND_LOSSLESS: u64 = 2_000;
pub const BOUND_LOSSY: u64 = 5_000;
pub const BOUND_RELAY: u64 = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EdgeClass {
    Lossless,
    Lossy,
    Relay,
}

/// Directed delivery edges between alive nodes of the same practice.
#[derive(Debug, Default)]
pub struct Graph {
    edges: BTreeMap<usize, Vec<(usize, EdgeClass)>>,
    pub alive: BTreeSet<usize>,
}

impl Graph {
    pub fn out_degree(&self, n: usize) -> usize {
        self.edges.get(&n).map(|e| e.len()).unwrap_or(0)
    }

    pub fn in_degree(&self, n: usize) -> usize {
        self.edges
            .values()
            .flatten()
            .filter(|(m, _)| *m == n)
            .count()
    }

    /// The P1 bound for each node reachable from `o`.
    pub fn bounds_from(&self, o: usize) -> BTreeMap<usize, u64> {
        let mut out = BTreeMap::new();
        for (class, bound) in [
            (EdgeClass::Lossless, BOUND_LOSSLESS),
            (EdgeClass::Lossy, BOUND_LOSSY),
            (EdgeClass::Relay, BOUND_RELAY),
        ] {
            let mut seen = BTreeSet::from([o]);
            let mut q = VecDeque::from([o]);
            while let Some(n) = q.pop_front() {
                for (m, c) in self.edges.get(&n).into_iter().flatten() {
                    if *c <= class && seen.insert(*m) {
                        q.push_back(*m);
                    }
                }
            }
            for n in seen {
                if n != o {
                    out.entry(n).or_insert(bound);
                }
            }
        }
        out
    }

    /// Edges out of `n`, for diagnostics.
    pub fn edges_of(&self, n: usize) -> Vec<(usize, String)> {
        self.edges
            .get(&n)
            .into_iter()
            .flatten()
            .map(|(m, c)| (*m, format!("{c:?}")))
            .collect()
    }

    /// Nodes connected to `n` in either direction, transitively.
    pub fn component(&self, n: usize) -> BTreeSet<usize> {
        let mut seen = BTreeSet::from([n]);
        let mut q = VecDeque::from([n]);
        while let Some(x) = q.pop_front() {
            for (a, es) in &self.edges {
                for (b, _) in es {
                    let next = if *a == x {
                        Some(*b)
                    } else if *b == x {
                        Some(*a)
                    } else {
                        None
                    };
                    if let Some(y) = next
                        && seen.insert(y)
                    {
                        q.push_back(y);
                    }
                }
            }
        }
        seen
    }
}

/// Builds the delivery graph for the world's current state.
pub fn graph(w: &World) -> Graph {
    let n = w.node_count();
    let spec = &w.spec;
    let mut g = Graph::default();
    for i in 0..n {
        if w.is_up(i) {
            g.alive.insert(i);
        }
    }
    let segs_of = |i: usize| -> Vec<usize> {
        spec.nodes[i]
            .segments
            .iter()
            .copied()
            .filter(|s| w.link_up(i, *s))
            .collect()
    };
    let same_practice = |a: usize, b: usize| spec.nodes[a].practice == spec.nodes[b].practice;
    let lan = |a: usize, b: usize| -> Option<EdgeClass> {
        let sa = segs_of(a);
        let sb = segs_of(b);
        let mut best: Option<EdgeClass> = None;
        for s in sa.iter().filter(|s| sb.contains(s)) {
            let seg = &w.segments[*s];
            if seg.client_isolation || !(seg.multicast_ok || seg.broadcast_ok) {
                continue;
            }
            let c = if seg.loss_permille == 0 {
                EdgeClass::Lossless
            } else {
                EdgeClass::Lossy
            };
            best = Some(best.map_or(c, |b: EdgeClass| b.min(c)));
        }
        best
    };
    let route = |a: usize, b: usize, tcp: bool| -> Option<EdgeClass> {
        let table = if tcp {
            &spec.route_tcp
        } else {
            &spec.route_udp
        };
        let mut best: Option<EdgeClass> = None;
        for sa in segs_of(a) {
            for sb in segs_of(b) {
                let ok = if sa == sb {
                    !w.segments[sb].client_isolation
                } else {
                    table[sa][sb]
                };
                if ok {
                    let lossy =
                        w.segments[sa].loss_permille > 0 || w.segments[sb].loss_permille > 0;
                    // Lossy even by TCP, which the simulator delivers
                    // reliably: the sender only uses TCP at once for peers
                    // it has heard lately, by UDP over the same lossy
                    // segment. P1 allows 5 s for paths with loss.
                    let c = if lossy {
                        EdgeClass::Lossy
                    } else {
                        EdgeClass::Lossless
                    };
                    best = Some(best.map_or(c, |b: EdgeClass| b.min(c)));
                }
            }
        }
        best
    };
    // LAN reachability under the initial segment flags: addresses learned
    // then are remembered after the flags change.
    let lan_initial = |a: usize, b: usize| -> bool {
        spec.nodes[a].segments.iter().any(|s| {
            let seg = &spec.segments[*s];
            spec.nodes[b].segments.contains(s)
                && !seg.client_isolation
                && (seg.multicast_ok || seg.broadcast_ok)
        })
    };
    let relay = |a: usize, b: usize| -> bool {
        (0..spec.relays).any(|r| w.relay_connected(a, r) && w.relay_connected(b, r))
    };
    // Who knows whose address (fixed point).
    let alive: Vec<usize> = g.alive.iter().copied().collect();
    let mut knows: BTreeSet<(usize, usize)> = BTreeSet::new();
    for &a in &alive {
        for &b in &alive {
            if a == b || !same_practice(a, b) {
                continue;
            }
            // A seed is configured as one address: b's first segment.
            let seed_ok = spec.nodes[a].seeds.contains(&b)
                && spec.nodes[b].segments.first().is_some_and(|sb| {
                    w.link_up(b, *sb)
                        && segs_of(a).iter().any(|sa| {
                            if sa == sb {
                                !w.segments[*sb].client_isolation
                            } else {
                                spec.route_udp[*sa][*sb]
                            }
                        })
                });
            if seed_ok
                || lan(b, a).is_some()
                || lan_initial(b, a)
                || relay(a, b)
                || spec.nodes[a]
                    .relays
                    .iter()
                    .any(|r| spec.nodes[b].relays.contains(r))
            {
                knows.insert((a, b));
            }
        }
    }
    loop {
        let mut changed = false;
        for &a in &alive {
            for &b in &alive {
                if a == b || knows.contains(&(a, b)) || !same_practice(a, b) {
                    continue;
                }
                // b unicasts its heartbeats to a, so a learns b's address.
                if knows.contains(&(b, a)) && route(b, a, false).is_some() {
                    knows.insert((a, b));
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    for &a in &alive {
        for &b in &alive {
            if a == b || !same_practice(a, b) {
                continue;
            }
            let mut best: Option<EdgeClass> = lan(a, b);
            if knows.contains(&(a, b)) {
                for c in [route(a, b, false), route(a, b, true)]
                    .into_iter()
                    .flatten()
                {
                    best = Some(best.map_or(c, |x| x.min(c)));
                }
            }
            if best.is_none() && relay(a, b) {
                best = Some(EdgeClass::Relay);
            }
            if let Some(c) = best {
                g.edges.entry(a).or_default().push((b, c));
            }
        }
    }
    g
}

pub type Check = Result<(), String>;

/// P1: every node with a path from the origin shows the alarm within the
/// bound. `g` is the graph at raise time (the topology is stable over the
/// window).
pub fn p1_delivery(w: &World, g: &Graph, alarm: &MsgId, origin: usize, t0: u64) -> Check {
    let shown = w.received(alarm);
    for (n, bound) in g.bounds_from(origin) {
        if !g.alive.contains(&n) || !w.is_up(n) {
            continue;
        }
        match shown.get(&n) {
            Some(t) if *t <= t0 + bound => {}
            other => {
                return Err(format!(
                    "P1: node {n} received at {other:?}, raised by {origin} at {t0}, bound {bound}"
                ));
            }
        }
    }
    Ok(())
}

/// P2: no node shows an alarm unless a legitimate member of its practice
/// raised it.
pub fn p2_authenticity(w: &World) -> Check {
    for e in &w.trace {
        if let TraceEv::Ui(UiEvent::AlarmShown { alarm, .. } | UiEvent::AlarmHistory(alarm)) = &e.ev
        {
            let Some((origin, _)) = w.raised.get(&alarm.alarm_id) else {
                return Err(format!(
                    "P2: node {} showed unknown alarm {}",
                    e.node, alarm.alarm_id
                ));
            };
            if w.spec.nodes[*origin].practice != w.spec.nodes[e.node].practice {
                return Err(format!(
                    "P2: node {} showed another practice's alarm",
                    e.node
                ));
            }
        }
    }
    Ok(())
}

/// P3: no node alerts twice for the same alarm (resumed alerts excluded,
/// ADR 0013).
pub fn p3_no_duplicates(w: &World) -> Check {
    let mut seen = BTreeSet::new();
    for e in &w.trace {
        if let TraceEv::Ui(UiEvent::AlarmShown {
            alarm,
            resumed: false,
        }) = &e.ev
            && !seen.insert((e.node, alarm.alarm_id))
        {
            return Err(format!(
                "P3: node {} alerted twice for {}",
                e.node, alarm.alarm_id
            ));
        }
    }
    Ok(())
}

type Fingerprint = (u8, usize, Vec<String>, Vec<String>);

fn fingerprint(v: &panic_button::core::io::AlarmView) -> Fingerprint {
    let phase = match v.state {
        AlarmPhase::Open | AlarmPhase::Expired => 0,
        AlarmPhase::Resolved { .. } => 1,
        AlarmPhase::Cancelled { .. } => 2,
    };
    let mut coming = v.coming.clone();
    coming.sort();
    let mut seen = v.seen.clone();
    seen.sort();
    (phase, v.delivered, coming, seen)
}

/// P4: connected nodes have identical derived state for every alarm.
pub fn p4_convergence(w: &World, g: &Graph) -> Check {
    let mut done = BTreeSet::new();
    for &n in &g.alive {
        if done.contains(&n) {
            continue;
        }
        let comp = g.component(n);
        done.extend(comp.iter().copied());
        let mut views: BTreeMap<MsgId, (usize, Fingerprint)> = BTreeMap::new();
        for &m in &comp {
            let Some(c) = w.core(m) else { continue };
            for v in c.alarm_views(w.wall_of(m)) {
                let f = fingerprint(&v);
                if let Some((other, f0)) = views.get(&v.alarm_id) {
                    if *f0 != f {
                        return Err(format!(
                            "P4: alarm {} differs: node {other} {f0:?} vs node {m} {f:?}",
                            v.alarm_id
                        ));
                    }
                } else {
                    views.insert(v.alarm_id, (m, f));
                }
            }
        }
        // Every node in the component knows every alarm the others know.
        for &m in &comp {
            let Some(c) = w.core(m) else { continue };
            let have: BTreeSet<MsgId> = c
                .alarm_views(w.wall_of(m))
                .iter()
                .map(|v| v.alarm_id)
                .collect();
            for id in views.keys() {
                if !have.contains(id) {
                    return Err(format!("P4: node {m} lacks alarm {id}"));
                }
            }
        }
    }
    Ok(())
}

/// P5: no alert for an alarm raised more than ALARM_MAX_LIFE before its
/// first receipt (true clocks).
pub fn p5_no_stale(w: &World) -> Check {
    for e in &w.trace {
        if let TraceEv::Ui(UiEvent::AlarmShown {
            alarm,
            resumed: false,
        }) = &e.ev
            && let Some((_, t)) = w.raised.get(&alarm.alarm_id)
            && e.t > t + ALARM_MAX_LIFE
        {
            return Err(format!("P5: node {} alerted for a stale alarm", e.node));
        }
    }
    Ok(())
}

/// P8: a green node can deliver to someone; a node that can reach nobody
/// (stably for PEER_STALE + 1 s) is red. Checks the node's status now.
pub fn p8_status_honesty(w: &World, g: &Graph, stable_since: u64) -> Check {
    for &n in &g.alive {
        let Some(st) = w.core(n).and_then(|c| c.status()) else {
            continue;
        };
        let practice_size = w
            .spec
            .nodes
            .iter()
            .filter(|x| x.practice == w.spec.nodes[n].practice)
            .count();
        if w.now >= stable_since + PEER_STALE + 1_000
            && st.level == StatusLevel::Green
            && g.out_degree(n) == 0
        {
            return Err(format!("P8: node {n} green but can deliver to nobody"));
        }
        if w.now >= stable_since + PEER_STALE + 1_000
            && g.out_degree(n) == 0
            && g.in_degree(n) == 0
            && practice_size > 1
            && st.level != StatusLevel::Red
        {
            return Err(format!("P8: node {n} isolated but {:?}", st.level));
        }
    }
    Ok(())
}

/// P10: the sender's "Delivered to N" never exceeds the number of devices
/// that had received the alarm (and so could have sent a Receipt).
pub fn p10_sender_honesty(w: &World) -> Check {
    let mut received: BTreeMap<MsgId, BTreeSet<usize>> = BTreeMap::new();
    for e in &w.trace {
        match &e.ev {
            TraceEv::Log(AuditEvent::AlarmReceived { alarm_id, .. }) => {
                received.entry(*alarm_id).or_default().insert(e.node);
            }
            TraceEv::Ui(UiEvent::SenderPanel { alarm, .. }) => {
                let n = received.get(&alarm.alarm_id).map(|s| s.len()).unwrap_or(0);
                if alarm.delivered > n {
                    return Err(format!(
                        "P10: sender showed {} delivered, only {n} received",
                        alarm.delivered
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// P11: an alarm shows as cancelled only if its origin cancelled it.
pub fn p11_cancel_authority(w: &World, origin_cancelled: &BTreeSet<MsgId>) -> Check {
    for e in &w.trace {
        if let TraceEv::Ui(
            UiEvent::AlarmUpdated(v) | UiEvent::AlarmClosed(v) | UiEvent::AlarmHistory(v),
        ) = &e.ev
            && matches!(v.state, AlarmPhase::Cancelled { .. })
            && !origin_cancelled.contains(&v.alarm_id)
        {
            return Err(format!(
                "P11: node {} shows {} cancelled",
                e.node, v.alarm_id
            ));
        }
    }
    Ok(())
}

/// P12: drills never cause a full alert.
pub fn p12_drill_isolation(w: &World) -> Check {
    for e in &w.trace {
        if let TraceEv::Ui(UiEvent::AlarmShown { alarm, .. }) = &e.ev
            && alarm.drill
        {
            return Err(format!("P12: node {} alerted for a drill", e.node));
        }
    }
    Ok(())
}

/// Builds a sealed envelope for `payload`, signed by `signer`, as an
/// attacker or a misbehaving insider would.
pub fn forge(
    w: &World,
    practice: usize,
    signer: &DeviceKeys,
    msg_id: MsgId,
    payload: Payload,
) -> Vec<u8> {
    let body = Body {
        msg_id,
        created_at: crate::world::WALL_BASE + w.now,
        sender: signer.device_id(),
        payload,
    };
    let signed = sign_body(signer, &body);
    seal(&w.practices[practice].group_key, 0, [7; 24], &signed).to_cbor()
}
