// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Named scenarios S1 to S17 (`spec/07-testing.md`, Named scenarios).
//! Deterministic; always run.

#![allow(clippy::needless_range_loop)]

use std::net::{Ipv4Addr, SocketAddrV4};

use panic_button::core::io::{
    AlarmPhase, LocalAck, SenderPhase, StatusReason, UiEvent, UserAction,
};
use panic_button::core::time::MINUTE;
use panic_button::proto::{AudioStatus, MsgId, StatusLevel};
use panic_button_sim::*;

const WARMUP: u64 = 40_000;

fn status_of(w: &World, i: usize) -> (Option<StatusLevel>, Vec<StatusReason>) {
    let s = w.core(i).and_then(|c| c.status());
    (
        s.map(|s| s.level),
        s.map(|s| s.reasons.clone()).unwrap_or_default(),
    )
}

fn assert_status(w: &World, nodes: impl IntoIterator<Item = usize>, level: StatusLevel) {
    for i in nodes {
        assert_eq!(
            status_of(w, i).0,
            Some(level),
            "node {i}: {:?}",
            status_of(w, i)
        );
    }
}

fn delivered_within(
    w: &World,
    alarm: &MsgId,
    t0: u64,
    bound: u64,
    nodes: impl IntoIterator<Item = usize>,
) {
    let shown = w.shown(alarm);
    for i in nodes {
        let t = shown.get(&i).copied();
        assert!(
            t.is_some_and(|t| t <= t0 + bound),
            "node {i} shown at {t:?}, raised at {t0}, bound {bound}"
        );
    }
}

fn sender_delivered(w: &World, node: usize, alarm: &MsgId) -> usize {
    w.ui_events(node)
        .filter_map(|(_, e)| match e {
            UiEvent::SenderPanel { alarm: a, .. } if a.alarm_id == *alarm => Some(a.delivered),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

fn others(n: usize, except: &[usize]) -> Vec<usize> {
    (0..n).filter(|i| !except.contains(i)).collect()
}

fn view_of(w: &World, node: usize, alarm: &MsgId) -> Option<panic_button::core::io::AlarmView> {
    w.core(node)?
        .alarm_views(w.wall_of(node))
        .into_iter()
        .find(|v| v.alarm_id == *alarm)
}

fn two_vlans(a: usize, b: usize, dual: Option<usize>) -> WorldSpec {
    let mut s = WorldSpec::flat(a + b, 4);
    s.segments = vec![Segment::default(), Segment::default()];
    s.route_udp = WorldSpec::no_routes(2);
    s.route_tcp = WorldSpec::no_routes(2);
    for i in a..a + b {
        s.nodes[i].segments = vec![1];
    }
    if let Some(d) = dual {
        s.nodes[d].segments = vec![0, 1];
    }
    s
}

#[test]
fn s1_flat_lan_30_nodes() {
    let mut w = World::new(WorldSpec::flat(30, 1));
    w.run_until(WARMUP);
    assert_status(&w, 0..30, StatusLevel::Green);
    let t0 = w.now;
    let id = w.raise_now(5).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, others(30, &[5]));
    // The sender never alerts for its own alarm, and is told it was delivered.
    assert!(!w.shown(&id).contains_key(&5));
    assert_eq!(sender_delivered(&w, 5, &id), 29);
}

#[test]
fn s2_multicast_blocked_broadcast_allowed() {
    let mut s = WorldSpec::flat(10, 2);
    s.segments[0].multicast_ok = false;
    let mut w = World::new(s);
    w.run_until(WARMUP);
    assert_status(&w, 0..10, StatusLevel::Green);
    let t0 = w.now;
    let id = w.raise_now(0).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, 1..10);
}

#[test]
fn s3_unicast_only_with_seed_peer() {
    let mut s = WorldSpec::flat(8, 3);
    s.segments[0].multicast_ok = false;
    s.segments[0].broadcast_ok = false;
    for i in 1..8 {
        s.nodes[i].seeds = vec![0];
    }
    let mut w = World::new(s);
    w.run_until(WARMUP);
    assert_status(&w, 0..8, StatusLevel::Green);
    let t0 = w.now;
    let id = w.raise_now(3).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, others(8, &[3]));
}

#[test]
fn s4_two_vlans_bridged_by_dual_homed_node() {
    let mut w = World::new(two_vlans(5, 5, Some(4)));
    w.run_until(WARMUP);
    let t0 = w.now;
    let id = w.raise_now(0).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, 1..10);
    let id = w.raise_now(9).expect("raised");
    let t1 = w.now;
    w.run_for(5_000);
    delivered_within(&w, &id, t1, 2_000, 0..9);
}

#[test]
fn s5_two_vlans_via_relay() {
    let mut s = two_vlans(5, 1, None);
    s.relays = 1;
    for n in &mut s.nodes {
        n.relays = vec![0];
    }
    let mut w = World::new(s);
    w.run_until(WARMUP);
    assert_status(&w, 0..5, StatusLevel::Green);
    let (lvl, reasons) = status_of(&w, 5);
    assert_eq!(lvl, Some(StatusLevel::Amber));
    assert!(reasons.contains(&StatusReason::OnlyViaRelay), "{reasons:?}");
    let t0 = w.now;
    let id = w.raise_now(0).expect("raised");
    w.run_for(6_000);
    delivered_within(&w, &id, t0, 5_000, 1..6);
    let id = w.raise_now(5).expect("raised");
    let t1 = w.now;
    w.run_for(6_000);
    delivered_within(&w, &id, t1, 5_000, 0..5);
    assert_eq!(sender_delivered(&w, 5, &id), 5);
}

fn wifi_isolation(relay: bool) -> WorldSpec {
    let mut s = WorldSpec::flat(7, 6);
    s.segments = vec![
        Segment::default(),
        Segment {
            client_isolation: true,
            ..Segment::default()
        },
    ];
    s.route_udp = WorldSpec::no_routes(2);
    s.route_tcp = WorldSpec::no_routes(2);
    for i in 4..7 {
        s.nodes[i].segments = vec![1];
    }
    if relay {
        s.relays = 1;
        for n in &mut s.nodes {
            n.relays = vec![0];
        }
    }
    s
}

#[test]
fn s6_wifi_isolation_with_relay() {
    let mut w = World::new(wifi_isolation(true));
    w.run_until(WARMUP);
    let t0 = w.now;
    let id = w.raise_now(0).expect("raised");
    w.run_for(6_000);
    delivered_within(&w, &id, t0, 5_000, 1..7);
    // A laptop can raise and reach the wired PCs.
    let id = w.raise_now(5).expect("raised");
    let t1 = w.now;
    w.run_for(6_000);
    delivered_within(&w, &id, t1, 5_000, [0, 1, 2, 3, 4, 6]);
}

#[test]
fn s7_wifi_isolation_without_relay_is_red() {
    let mut w = World::new(wifi_isolation(false));
    w.run_until(WARMUP);
    assert_status(&w, 0..4, StatusLevel::Green);
    for i in 4..7 {
        let (lvl, reasons) = status_of(&w, i);
        assert_eq!(lvl, Some(StatusLevel::Red), "laptop {i}");
        assert!(reasons.contains(&StatusReason::NoDevicesReachable));
    }
    // The laptop's sender panel says it could not deliver (P8, H03).
    let id = w.raise_now(5).expect("raised");
    w.run_for(16_000);
    let phases: Vec<SenderPhase> = w
        .ui_events(5)
        .filter_map(|(_, e)| match e {
            UiEvent::SenderPanel { alarm, phase } if alarm.alarm_id == id => Some(*phase),
            _ => None,
        })
        .collect();
    assert!(phases.contains(&SenderPhase::NotYetDelivered), "{phases:?}");
    assert_eq!(phases.last(), Some(&SenderPhase::CouldNotReach));
    assert!(w.shown(&id).is_empty());
}

#[test]
fn s8_asleep_through_alarm_sees_history_only() {
    let mut s = WorldSpec::flat(5, 8);
    s.nodes[1].user = UserModel::Respond { after: 2_000 };
    let mut w = World::new(s);
    w.run_until(WARMUP);
    w.at(w.now, WorldEvent::Sleep(4));
    w.run_for(100);
    let id = w.raise_now(0).expect("raised");
    w.run_for(10_000);
    w.act_now(1, UserAction::Resolve { alarm_id: id });
    w.run_for(10_000);
    w.at(w.now, WorldEvent::Wake(4));
    w.run_for(20_000);
    assert!(
        !w.shown(&id).contains_key(&4),
        "must not alert after resolution"
    );
    let history = w
        .ui_events(4)
        .any(|(_, e)| matches!(e, UiEvent::AlarmHistory(v) if v.alarm_id == id));
    assert!(history, "the alarm is in history");
    assert!(matches!(
        view_of(&w, 4, &id).map(|v| v.state),
        Some(AlarmPhase::Resolved { .. })
    ));
}

#[test]
fn s9_wakes_during_open_alarm_raised_10_minutes_ago() {
    let mut w = World::new(WorldSpec::flat(5, 9));
    w.run_until(WARMUP);
    w.at(w.now, WorldEvent::Sleep(4));
    w.run_for(100);
    let raised_wall = w.wall_of(0);
    let id = w.raise_now(0).expect("raised");
    w.run_for(10 * MINUTE);
    w.at(w.now, WorldEvent::Wake(4));
    let woke = w.now;
    w.run_for(10_000);
    let shown = w.shown(&id);
    assert!(
        shown.get(&4).is_some_and(|t| *t <= woke + 5_000),
        "{shown:?}"
    );
    let v = view_of(&w, 4, &id).expect("view");
    assert_eq!(v.raised_at, raised_wall);
    assert!(w.wall_of(4) - v.raised_at >= 10 * MINUTE);
}

#[test]
fn s10_two_practices_on_one_segment() {
    let mut s = WorldSpec::flat(8, 10);
    s.practices = 2;
    for i in 4..8 {
        s.nodes[i].practice = 1;
    }
    let mut w = World::new(s);
    w.run_until(WARMUP);
    assert_status(&w, 0..8, StatusLevel::Green);
    for i in 0..8 {
        let c = w.core(i).expect("core");
        let st = c.status().expect("status");
        assert_eq!(st.direct, 3, "node {i} sees only its own practice");
    }
    let id = w.raise_now(0).expect("raised");
    w.run_for(5_000);
    let shown = w.shown(&id);
    assert_eq!(shown.keys().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
}

#[test]
fn s11_replays_never_re_alert() {
    let mut s = WorldSpec::flat(5, 11);
    for n in &mut s.nodes {
        n.user = UserModel::Seen { after: 3_000 };
    }
    let mut w = World::new(s);
    w.run_until(WARMUP);
    w.capture = true;
    let id = w.raise_now(0).expect("raised");
    w.run_for(1_000);
    w.capture = false;
    let raises: Vec<Vec<u8>> = w
        .captured
        .iter()
        .filter(|(_, b)| w.classify(b) == "AlarmRaised")
        .map(|(_, b)| b.clone())
        .collect();
    assert!(!raises.is_empty());
    let attacker = SocketAddrV4::new(Ipv4Addr::new(10, 0, 9, 9), 47474);
    let t = w.now + 5 * MINUTE;
    for b in &raises {
        w.inject_segment(t, 0, attacker, b.clone());
    }
    w.run_for(6 * MINUTE);
    w.at(w.now, WorldEvent::Crash(3));
    w.run_for(1_000);
    w.at(w.now, WorldEvent::Restart(3));
    w.run_for(MINUTE);
    let t = w.now + 2 * 60 * MINUTE - w.now % MINUTE;
    for b in &raises {
        w.inject_segment(t, 0, attacker, b.clone());
    }
    w.run_until(t + MINUTE);
    for i in 1..5 {
        let n = w
            .ui_events(i)
            .filter(|(_, e)| matches!(e, UiEvent::AlarmShown { alarm, resumed: false } if alarm.alarm_id == id))
            .count();
        assert_eq!(n, 1, "node {i} alerted {n} times");
    }
}

#[test]
fn s12_relay_down_mid_alarm() {
    let mut s = two_vlans(5, 1, None);
    s.relays = 1;
    for n in &mut s.nodes {
        n.relays = vec![0];
    }
    s.nodes[1].user = UserModel::Respond { after: 3_000 };
    let mut w = World::new(s);
    w.run_until(WARMUP);
    let t0 = w.now;
    let id = w.raise_now(0).expect("raised");
    w.at(t0 + 1_000, WorldEvent::RelayUp(0, false));
    w.run_for(5_000);
    // LAN delivery is unaffected.
    delivered_within(&w, &id, t0, 2_000, 1..5);
    assert!(view_of(&w, 5, &id).is_some_and(|v| v.coming.is_empty()));
    w.at(t0 + 20_000, WorldEvent::RelayUp(0, true));
    w.run_until(t0 + 30_000);
    let v = view_of(&w, 5, &id).expect("relay-only node has the alarm");
    assert_eq!(
        v.coming,
        vec!["User 1".to_string()],
        "events arrive after reconnect"
    );
}

fn s13_size() -> usize {
    if std::env::var_os("PANIC_BUTTON_FULL_SCALE").is_some() {
        250
    } else {
        60
    }
}

#[test]
fn s13_large_flat_lan_all_respond() {
    let n = s13_size();
    let mut s = WorldSpec::flat(n, 13);
    for (i, node) in s.nodes.iter_mut().enumerate() {
        node.user = UserModel::Respond {
            after: 2_000 + (i as u64 * 37) % 3_000,
        };
        node.start_at = (i as u64 * 53) % 5_000;
    }
    let mut w = World::new(s);
    w.classify_traffic = std::env::var_os("CLASSIFY").is_some();
    w.run_until(WARMUP);
    let before = w.counters.by_type.clone();
    let base = w.first_packets_between(WARMUP - 10_000, WARMUP);
    let t0 = w.now;
    let id = w.raise_now(0).expect("raised");
    w.run_for(10_000);
    delivered_within(&w, &id, t0, 2_000, 1..n);
    let during = w.first_packets_between(t0, t0 + 10_000);
    for (k, v) in &w.counters.by_type {
        eprintln!("{k}: {}", v - before.get(k).copied().unwrap_or(0));
    }
    let alarm_packets = during.saturating_sub(base);
    // P7: no more than 10 x N first transmissions (retransmissions are
    // allowed on top), with background heartbeat traffic subtracted.
    assert!(
        alarm_packets <= 10 * n as u64,
        "alarm traffic {alarm_packets} for {n} nodes (background {base}, total {})",
        w.packets_between(t0, t0 + 10_000)
    );
    let v = view_of(&w, 0, &id).expect("view");
    assert_eq!(v.delivered, n - 1);
    assert_eq!(v.coming.len(), n - 1);
}

#[test]
fn s14_revoke_laptop_during_open_alarm() {
    let mut s = WorldSpec::flat(6, 14);
    s.nodes[2].user = UserModel::Respond { after: 4_000 };
    let mut w = World::new(s);
    w.run_until(WARMUP);
    let t0 = w.now;
    let id = w.raise_now(1).expect("raised");
    w.run_for(1_000);
    let laptop = w.device_id(5);
    w.act_now(
        0,
        UserAction::Revoke {
            device: laptop,
            cascade: false,
            reason: "Laptop lost".into(),
        },
    );
    w.run_for(1_000);
    // The laptop tries to respond after revocation: everyone rejects it.
    w.act_now(5, UserAction::Respond { alarm_id: id });
    w.run_for(5_000);
    for i in 0..5 {
        let v = view_of(&w, i, &id).expect("view");
        assert_eq!(v.state, AlarmPhase::Open, "alarm continues at {i}");
        assert!(
            !v.coming.contains(&"User 5".to_string()),
            "node {i}: {:?}",
            v.coming
        );
        assert!(
            v.coming.contains(&"User 2".to_string()),
            "node {i}: {:?}",
            v.coming
        );
    }
    // After rotation the laptop cannot read new traffic.
    w.run_for(20_000);
    let e = w.core(0).and_then(|c| c.current_epoch());
    assert_eq!(e, Some(1));
    assert_eq!(w.core(5).and_then(|c| c.current_epoch()), Some(0));
    let id2 = w.raise_now(3).expect("raised");
    w.run_for(5_000);
    assert!(!w.received(&id2).contains_key(&5));
    delivered_within(&w, &id2, t0 + 27_000, 3_000, [0, 1, 2, 4]);
    assert_eq!(status_of(&w, 5).0, Some(StatusLevel::Red));
}

#[test]
fn s15_token_enrolment_with_stale_epoch() {
    let mut s = WorldSpec::flat(6, 15);
    s.nodes[5].start_at = 60_000;
    let mut w = World::new(s);
    w.run_until(WARMUP);
    let gone = w.device_id(4);
    w.act_now(
        0,
        UserAction::Revoke {
            device: gone,
            cascade: false,
            reason: "Left".into(),
        },
    );
    w.run_until(59_000);
    assert_eq!(w.core(1).and_then(|c| c.current_epoch()), Some(1));
    // Node 5 starts with the epoch-0 key from its token.
    w.run_until(60_000 + 20_000);
    assert_eq!(
        w.core(5).and_then(|c| c.current_epoch()),
        Some(1),
        "got the current key"
    );
    assert_eq!(
        status_of(&w, 5).0,
        Some(StatusLevel::Green),
        "{:?}",
        status_of(&w, 5)
    );
    let id = w.raise_now(5).expect("raised");
    let t0 = w.now;
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, [0, 1, 2, 3]);
}

#[test]
fn s16_concurrent_rotations_converge() {
    let mut s = WorldSpec::flat(8, 16);
    s.admins = vec![1];
    let mut w = World::new(s);
    w.run_until(WARMUP);
    let a = w.device_id(6);
    let b = w.device_id(7);
    w.act_now(
        0,
        UserAction::Revoke {
            device: a,
            cascade: false,
            reason: String::new(),
        },
    );
    w.act_now(
        1,
        UserAction::Revoke {
            device: b,
            cascade: false,
            reason: String::new(),
        },
    );
    // Alarms keep flowing during the conflict.
    w.run_for(200);
    let id = w.raise_now(2).expect("raised");
    let t0 = w.now;
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, [0, 1, 3, 4, 5]);
    w.run_for(60_000);
    let tags: Vec<_> = (0..6)
        .map(|i| {
            let c = w.core(i).expect("core");
            (
                c.current_epoch(),
                c.trust().map(|t| t.revocations().count()),
            )
        })
        .collect();
    assert!(tags.iter().all(|t| *t == tags[0]), "{tags:?}");
    assert_eq!(tags[0], (Some(1), Some(2)));
    let id = w.raise_now(3).expect("raised");
    let t1 = w.now;
    w.run_for(5_000);
    delivered_within(&w, &id, t1, 2_000, [0, 1, 2, 4, 5]);
}

#[test]
fn s17_muted_receiver_without_override() {
    let mut s = WorldSpec::flat(4, 17);
    s.nodes[2].audio = AudioStatus {
        output_present: true,
        muted: true,
        volume_pct: 80,
    };
    let mut w = World::new(s);
    w.run_until(WARMUP);
    let (lvl, reasons) = status_of(&w, 2);
    assert_eq!(lvl, Some(StatusLevel::Amber));
    assert!(reasons.contains(&StatusReason::AudioMuted));
    let id = w.raise_now(0).expect("raised");
    w.run_for(3_000);
    assert_eq!(
        sender_delivered(&w, 0, &id),
        3,
        "sender still gets the receipt"
    );
    let peer = w
        .core(0)
        .and_then(|c| c.peers().get(&w.device_id(2)).cloned());
    assert!(peer.and_then(|p| p.audio()).is_some_and(|a| a.muted));
    let _ = LocalAck::Seen;
}

/// FR-ADM-002: an admin promotes a member, which can then act as an admin
/// (here, revoke another device) everywhere.
#[test]
fn promote_member_to_admin() {
    let mut w = World::new(WorldSpec::flat(5, 21));
    w.run_until(WARMUP);
    let promoted = w.device_id(2);
    w.act_now(0, UserAction::Promote { device: promoted });
    w.run_for(5_000);
    for i in 0..5 {
        let c = w.core(i).expect("core");
        let wall = w.wall_of(i);
        assert!(
            c.trust().is_some_and(|t| t.is_admin(&promoted, wall)),
            "node {i} sees node 2 as an admin"
        );
    }
    // The new admin's revocation is honoured.
    let gone = w.device_id(4);
    w.act_now(
        2,
        UserAction::Revoke {
            device: gone,
            cascade: false,
            reason: "Left".into(),
        },
    );
    w.run_for(5_000);
    for i in 0..4 {
        let c = w.core(i).expect("core");
        assert!(
            c.trust().is_some_and(|t| !t.is_member(&gone, w.wall_of(i))),
            "node {i} removed node 4"
        );
    }
    // A member cannot promote.
    let other = w.device_id(3);
    w.act_now(1, UserAction::Promote { device: other });
    w.run_for(5_000);
    let c = w.core(0).expect("core");
    assert!(!c.trust().is_some_and(|t| t.is_admin(&other, w.wall_of(0))));
}

/// ADR 0014: an admin sets another device's label and room. Everyone sees
/// them in that device's heartbeat; a device asleep at the time gets them
/// when it wakes; the person's own later change wins; and a room set by IT
/// configuration cannot be changed.
#[test]
fn admin_assigns_label_and_room() {
    let mut w = World::new(WorldSpec::flat(5, 22));
    w.run_until(WARMUP);
    let heard = |w: &World, at: usize, of: usize| {
        let c = w.core(at).expect("core");
        let p = c.peers().get(&w.device_id(of)).expect("peer");
        let hb = p.heartbeat.as_ref().expect("heartbeat");
        (hb.label.clone(), hb.room.clone())
    };
    let d2 = w.device_id(2);
    w.act_now(
        0,
        UserAction::AssignDeviceSettings {
            device: d2,
            label: Some("ROOM4-PC".into()),
            room: Some("Consulting Room 4".into()),
        },
    );
    w.run_for(25_000);
    for i in [0, 1, 3, 4] {
        assert_eq!(
            heard(&w, i, 2),
            ("ROOM4-PC".to_owned(), "Consulting Room 4".to_owned()),
            "node {i}"
        );
    }

    // Node 4 is asleep when its room is set; it gets it on waking.
    w.at(w.now, WorldEvent::Sleep(4));
    w.run_for(1_000);
    let d4 = w.device_id(4);
    w.act_now(
        0,
        UserAction::AssignDeviceSettings {
            device: d4,
            label: None,
            room: Some("Treatment Room 1".into()),
        },
    );
    w.run_for(20_000);
    w.at(w.now, WorldEvent::Wake(4));
    w.run_for(90_000);
    assert_eq!(heard(&w, 1, 4).1, "Treatment Room 1");

    // The person on node 2 changes their room later: theirs wins.
    w.act_now(
        2,
        UserAction::SetOwnSettings {
            label: None,
            room: Some("Room 2".into()),
        },
    );
    w.run_for(25_000);
    assert_eq!(heard(&w, 1, 2).1, "Room 2");

    // A room from IT configuration on node 3 cannot be changed.
    w.configure(3, |l| {
        l.room = "Reception".into();
        l.room_locked = true;
    });
    let d3 = w.device_id(3);
    w.act_now(
        0,
        UserAction::AssignDeviceSettings {
            device: d3,
            label: None,
            room: Some("Elsewhere".into()),
        },
    );
    w.run_for(25_000);
    assert_eq!(heard(&w, 1, 3).1, "Reception");

    // A member cannot set another device's room.
    w.act_now(
        1,
        UserAction::AssignDeviceSettings {
            device: d2,
            label: None,
            room: Some("Nowhere".into()),
        },
    );
    w.run_for(25_000);
    assert_eq!(heard(&w, 0, 2).1, "Room 2");
}

fn join_codes(w: &World, node: usize) -> Vec<String> {
    w.ui_events(node)
        .filter_map(|(_, e)| match e {
            UiEvent::JoinCode(c) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

fn join_requests_seen(w: &World, node: usize) -> Vec<(panic_button::proto::DeviceId, String)> {
    w.ui_events(node)
        .filter_map(|(_, e)| match e {
            UiEvent::JoinRequested { device, code, .. } => Some((*device, code.clone())),
            _ => None,
        })
        .collect()
}

fn approved(w: &World, node: usize) -> Option<String> {
    w.ui_events(node).find_map(|(_, e)| match e {
        UiEvent::JoinApproved { practice_name } => Some(practice_name.clone()),
        _ => None,
    })
}

/// Flow C (`spec/04-security.md`): a computer with no enrolment code joins
/// on the LAN. It shows a six-digit code, an admin sees the same code and
/// approves, the joining user confirms the practice name, and it then sends
/// and receives alarms. Non-admins never see the request; a rejected or
/// cancelled join enrols nothing.
#[test]
fn join_by_approval_on_the_lan() {
    let mut spec = WorldSpec::flat(6, 18);
    for i in 3..6 {
        spec.nodes[i].enrolled = false;
    }
    let mut w = World::new(spec);
    w.run_until(WARMUP);
    assert!(!w.core(3).expect("core").is_enrolled());
    // Before asking, a new computer counts the enrolled ones it can hear
    // (one practice of three); the other new computers send nothing sealed.
    assert_eq!(
        w.core(3)
            .expect("core")
            .nearby(panic_button::core::Mono(w.now)),
        vec![3]
    );

    for i in 3..6 {
        w.act_now(
            i,
            UserAction::StartJoin {
                practice_hint: None,
            },
        );
    }
    w.run_for(6_000);
    let code = join_codes(&w, 3)
        .into_iter()
        .next()
        .expect("join code shown");
    assert_eq!(code.len(), 6);
    let seen = join_requests_seen(&w, 0);
    assert!(
        seen.contains(&(w.device_id(3), code)),
        "admin sees node 3's code: {seen:?}"
    );
    for i in [1, 2] {
        assert!(
            join_requests_seen(&w, i).is_empty(),
            "non-admin {i} saw a join request"
        );
    }

    // Approve 3, reject 4, and 5 cancels.
    w.act_now(
        0,
        UserAction::ApproveJoin {
            device: w.device_id(3),
        },
    );
    w.act_now(
        0,
        UserAction::RejectJoin {
            device: w.device_id(4),
        },
    );
    w.act_now(5, UserAction::CancelJoin);
    w.run_for(6_000);
    assert_eq!(approved(&w, 3).as_deref(), Some("Practice 0"));
    assert!(approved(&w, 4).is_none());
    // Approval alone does not enrol: the user confirms first.
    assert!(!w.core(3).expect("core").is_enrolled());
    w.act_now(3, UserAction::ConfirmJoin);
    w.run_for(30_000);
    assert!(w.core(3).expect("core").is_enrolled());
    assert!(
        w.core(3)
            .expect("core")
            .nearby(panic_button::core::Mono(w.now))
            .is_empty()
    );
    assert!(!w.core(4).expect("core").is_enrolled());
    assert!(!w.core(5).expect("core").is_enrolled());
    assert_status(&w, 0..4, StatusLevel::Green);

    // The new device's alarms reach everyone, and it receives theirs.
    let t0 = w.now;
    let id = w.raise_now(3).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, [0, 1, 2]);
    let t1 = w.now;
    let id = w.raise_now(1).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t1, 2_000, [0, 2, 3]);
    for i in [4, 5] {
        assert!(
            !w.shown(&id).contains_key(&i),
            "unenrolled node {i} alerted"
        );
    }
}

/// The first-run practice hold: when it completes, the ring is shown full,
/// the UI is told the practice succeeded, and nothing is raised anywhere. A
/// real hold also ends with a full ring.
#[test]
fn practice_hold_completes_without_raising() {
    let mut w = World::new(WorldSpec::flat(3, 21));
    w.run_until(WARMUP);
    let t0 = w.now;
    w.act_now(
        0,
        UserAction::HoldStart {
            source: panic_button::proto::Source::Button,
            practice: true,
        },
    );
    w.run_for(3_000);
    let after: Vec<&UiEvent> = w
        .ui_events(0)
        .filter(|(t, _)| *t >= t0)
        .map(|(_, e)| e)
        .collect();
    let full = after.iter().position(
        |e| matches!(e, UiEvent::HoldProgress { fraction_permille } if *fraction_permille == 1000),
    );
    let ended = after.iter().position(|e| matches!(e, UiEvent::HoldEnded));
    let done = after
        .iter()
        .position(|e| matches!(e, UiEvent::PracticeComplete));
    assert!(
        full.is_some() && ended.is_some() && done.is_some(),
        "{after:?}"
    );
    assert!(full < ended && ended < done, "order: {after:?}");
    assert!(
        w.last_alarm_of(0).is_none(),
        "a practice hold raised an alarm"
    );
    for i in 1..3 {
        assert!(
            !w.ui_events(i)
                .any(|(t, e)| t >= t0 && matches!(e, UiEvent::AlarmShown { .. })),
            "node {i} alerted for a practice hold"
        );
    }

    // A real hold ends with a full ring too, and raises.
    let t1 = w.now;
    w.act_now(
        1,
        UserAction::HoldStart {
            source: panic_button::proto::Source::Button,
            practice: false,
        },
    );
    w.run_for(3_000);
    assert!(w.ui_events(1).any(|(t, e)| t >= t1
        && matches!(e, UiEvent::HoldProgress { fraction_permille } if *fraction_permille == 1000)));
    assert!(
        !w.ui_events(1)
            .any(|(t, e)| t >= t1 && matches!(e, UiEvent::PracticeComplete))
    );
    assert!(w.last_alarm_of(1).is_some());
}

/// Fast user switching (`spec/05-desktop-agent.md`, Session and lifecycle):
/// while another user's session has the screen, the device takes no part:
/// it does not alert, others stop counting it, and its status says why.
/// Switched back, it takes part again at once.
#[test]
fn inactive_session_takes_no_part_until_active_again() {
    use panic_button::proto::SessionState;
    let mut w = World::new(WorldSpec::flat(4, 23));
    w.run_until(WARMUP);
    assert_status(&w, 0..4, StatusLevel::Green);

    let t = w.now;
    w.at(
        t,
        WorldEvent::Session {
            node: 3,
            state: SessionState::Inactive,
        },
    );
    w.run_for(2 * MINUTE);
    let (level, reasons) = status_of(&w, 3);
    assert_eq!(level, Some(StatusLevel::Red));
    assert!(
        reasons.contains(&StatusReason::SessionInactive),
        "{reasons:?}"
    );
    assert!(
        !reasons.contains(&StatusReason::RemoteSessionDisabled),
        "{reasons:?}"
    );

    let t0 = w.now;
    let id = w.raise_now(1).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t0, 2_000, [0, 2]);
    assert!(
        !w.shown(&id).contains_key(&3),
        "the inactive session alerted"
    );
    assert_eq!(
        sender_delivered(&w, 1, &id),
        2,
        "the inactive session was counted"
    );
    w.act_now(1, UserAction::Resolve { alarm_id: id });

    let t = w.now;
    w.at(
        t,
        WorldEvent::Session {
            node: 3,
            state: SessionState::Active,
        },
    );
    w.run_for(40_000);
    assert_status(&w, 0..4, StatusLevel::Green);
    let t1 = w.now;
    let id = w.raise_now(0).expect("raised");
    w.run_for(5_000);
    delivered_within(&w, &id, t1, 2_000, [1, 2, 3]);
}
