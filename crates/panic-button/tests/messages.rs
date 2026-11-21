// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Round-trip, determinism and golden-vector tests for every message type
//! (`spec/07-testing.md`, unit tests for the `proto` module).
//!
//! Golden vectors live in `tests/vectors/*.hex` so other implementations can
//! check compatibility. Regenerate with `PANIC_BUTTON_BLESS=1 cargo test -p
//! panic-button --test messages` and review the diff: a vector change is a
//! wire-format change.

use std::path::PathBuf;

use panic_button::proto::cbor::{Reader, Writer};
use panic_button::proto::*;
use proptest::prelude::*;

fn body(msg_id: u8, payload: Payload) -> Body {
    Body {
        msg_id: MsgId([msg_id; 16]),
        created_at: 1_763_115_000_000,
        sender: DeviceId([0x11; 32]),
        payload,
    }
}

fn samples() -> Vec<(&'static str, Body)> {
    let audio = AudioStatus {
        output_present: true,
        muted: false,
        volume_pct: 80,
    };
    let keyshare = KeyShare {
        recipient: DeviceId([0x22; 32]),
        epoch: 2,
        eph_pub: [0x33; 32],
        nonce: [0x44; 24],
        ct: vec![0x55; 48],
    };
    vec![
        (
            "heartbeat",
            body(
                1,
                Payload::Heartbeat(Heartbeat {
                    label: "ROOM4-PC".into(),
                    room: "Consulting Room 4".into(),
                    user_display: "Dr A Example".into(),
                    app_version: "0.1.0".into(),
                    proto_version: 1,
                    platform: Platform::Windows,
                    tcp_port: 47474,
                    addrs: vec![[192, 168, 1, 20]],
                    capabilities: caps::CAN_ALERT | caps::CAN_RAISE,
                    audio,
                    session: SessionState::Active,
                    status: StatusLevel::Green,
                    cert_serial: CertSerial([0x66; 16]),
                    epoch: 0,
                    alarm_digest: [0x77; 16],
                    state_digest: [0x88; 16],
                    near_miss_24h: 1,
                    uptime_s: 3600,
                }),
            ),
        ),
        (
            "alarm_raised",
            body(
                2,
                Payload::AlarmRaised(AlarmRaised {
                    alarm_id: MsgId([2; 16]),
                    kind: AlarmKind::Help,
                    drill: false,
                    room: "Consulting Room 4".into(),
                    user_display: "Dr A Example".into(),
                    device_label: "ROOM4-PC".into(),
                    source: Source::Button,
                }),
            ),
        ),
        (
            "alarm_event_receipt",
            body(
                3,
                Payload::AlarmEvent(AlarmEvent {
                    alarm_id: MsgId([2; 16]),
                    event: EventKind::Receipt,
                    user_display: "Jane Smith".into(),
                    detail: EventDetail::Receipt(ReceiptDetail {
                        audio: Some(audio),
                        path: Some(PathKind::Multicast),
                        displayed: Some(true),
                        drill_rtt_ref: None,
                    }),
                }),
            ),
        ),
        (
            "alarm_event_responding",
            body(
                4,
                Payload::AlarmEvent(AlarmEvent {
                    alarm_id: MsgId([2; 16]),
                    event: EventKind::Responding,
                    user_display: "Jane Smith".into(),
                    detail: EventDetail::None,
                }),
            ),
        ),
        (
            "alarm_event_cancelled",
            body(
                5,
                Payload::AlarmEvent(AlarmEvent {
                    alarm_id: MsgId([2; 16]),
                    event: EventKind::Cancelled,
                    user_display: "Dr A Example".into(),
                    detail: EventDetail::Cancelled {
                        reason: CANCEL_SENT_IN_ERROR,
                    },
                }),
            ),
        ),
        (
            "sync_request",
            body(
                6,
                Payload::SyncRequest(SyncRequest {
                    want: want::OPEN_ALARMS | want::CERTS,
                    since: Some(1_763_000_000_000),
                    ids: vec![MsgId([2; 16])],
                    to: Some(DeviceId([0x22; 32])),
                    alarm_digest: Some([0x33; 16]),
                }),
            ),
        ),
        (
            "sync_response",
            body(
                7,
                Payload::SyncResponse(SyncResponse {
                    envelopes: vec![vec![0xa0], vec![0xa1, 0x00, 0x01]],
                    more: false,
                }),
            ),
        ),
        (
            "cert_announce",
            body(
                8,
                Payload::CertAnnounce(CertAnnounce {
                    certs: vec![vec![0xa0]],
                }),
            ),
        ),
        (
            "practice_config",
            body(
                9,
                Payload::PracticeConfig(PracticeConfig {
                    version: 3,
                    settings: PracticeSettings {
                        hold_ms: Some(1500),
                        cue_mode: Some(CueMode::Soft),
                        sound_pattern: Some(SoundPattern::A),
                        override_mute: Some(true),
                        drill_schedule: Some("Tue 07:45".into()),
                        drill_mode: Some(DrillMode::Silent),
                        drill_coordinator: Some(DeviceId([0x11; 32])),
                        opening_hours: Some("Mon-Fri 08:00-18:30".into()),
                        relay_urls: Some(vec!["wss://relay.example/v1/ws".into()]),
                        seed_peers: None,
                    },
                }),
            ),
        ),
        (
            "presence_summary",
            body(
                10,
                Payload::PresenceSummary(PresenceSummary {
                    peers: vec![PresenceEntry {
                        device_id: DeviceId([0x22; 32]),
                        last_seen_age_s: 4,
                        path: PathKind::Broadcast,
                    }],
                }),
            ),
        ),
        (
            "device_settings",
            body(
                16,
                Payload::DeviceSettings(DeviceSettings {
                    target: DeviceId([0x22; 32]),
                    label: Some("RECEPTION-1".into()),
                    room: Some("Reception".into()),
                    assigned_at: 1_763_115_000_000,
                }),
            ),
        ),
        (
            "join_request",
            body(
                11,
                Payload::JoinRequest(JoinRequest {
                    kx_pub: [0x99; 32],
                    label: "RECEPTION-PC2".into(),
                    practice_hint: None,
                    code: "123456".into(),
                    tcp_port: Some(47474),
                }),
            ),
        ),
        (
            "join_approval",
            body(
                12,
                Payload::JoinApproval(JoinApproval {
                    recipient: DeviceId([0x22; 32]),
                    certs: vec![vec![0xa0], vec![0xa0]],
                    root_pub: [0xaa; 32],
                    practice_name: "Riverside Surgery".into(),
                    keyshare: keyshare.clone(),
                    config: vec![],
                }),
            ),
        ),
        ("key_share", body(13, Payload::KeyShare(keyshare))),
        (
            "revocation",
            body(
                14,
                Payload::Revocation(Revocation {
                    target_serial: CertSerial([0x66; 16]),
                    target_device: DeviceId([0x22; 32]),
                    cascade: false,
                    reason: "Laptop lost".into(),
                    new_epoch: 3,
                }),
            ),
        ),
        (
            "relay_auth",
            body(
                15,
                Payload::RelayAuth(RelayAuth {
                    practice_id: PracticeId([0xbb; 16]),
                    chain: vec![vec![0xa0]],
                    nonce: [0xcc; 32],
                    client_time: 1_763_115_000_000,
                    resume_cursor: Some(42),
                }),
            ),
        ),
    ]
}

fn vector_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors")
        .join(format!("{name}.hex"))
}

fn check_golden(name: &str, bytes: &[u8]) {
    let path = vector_path(name);
    let hex = format!("{}\n", hex::encode(bytes));
    if std::env::var_os("PANIC_BUTTON_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, &hex).expect("write vector");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing vector {name}; run with PANIC_BUTTON_BLESS=1"));
    assert_eq!(hex.trim(), expected.trim(), "golden vector {name} changed");
}

#[test]
fn every_message_type_round_trips_and_matches_golden() {
    let samples = samples();
    // Every known type has a sample (join_approval etc. included).
    let mut types: Vec<_> = samples.iter().map(|(_, b)| b.msg_type()).collect();
    types.sort();
    types.dedup();
    assert_eq!(types.len(), 14, "one sample per message type");
    for (name, b) in samples {
        let bytes = b.to_cbor();
        let back = Body::from_cbor(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(back, b, "{name} round trip");
        assert_eq!(back.to_cbor(), bytes, "{name} deterministic");
        check_golden(name, &bytes);
    }
}

#[test]
fn heartbeat_fits_in_one_datagram_with_long_strings() {
    let (_, mut b) = samples().remove(0);
    if let Payload::Heartbeat(h) = &mut b.payload {
        h.label = "L".repeat(200);
        h.room = "R".repeat(200);
        h.user_display = "U".repeat(200);
        h.addrs = vec![[10, 0, 0, 1]; 8];
    }
    let signed = SignedMessage {
        body: b.to_cbor(),
        sig: [0; 64],
    };
    let env = Envelope::Sealed(SealedEnvelope {
        tag: PracticeTag([0; 8]),
        epoch: u64::MAX,
        nonce: [0; 24],
        ciphertext: vec![0; signed.to_cbor().len() + 16],
    });
    assert!(env.to_cbor().len() <= limits::UDP_MAX);
}

#[test]
fn unknown_message_type_is_rejected() {
    let mut w = Writer::new();
    w.map(5)
        .u64(0)
        .bytes(&[0; 16])
        .u64(1)
        .u64(0x30)
        .u64(2)
        .u64(0)
        .u64(3)
        .bytes(&[0; 32])
        .u64(4)
        .map(0);
    assert_eq!(
        Body::from_cbor(&w.into_bytes()),
        Err(DecodeError::UnknownType(0x30))
    );
}

#[test]
fn unknown_payload_keys_are_ignored() {
    // AlarmRaised with an extra key 40 (NFR-COMP-001).
    let mut w = Writer::new();
    w.map(8)
        .u64(0)
        .bytes(&[2; 16])
        .u64(1)
        .u64(1)
        .u64(2)
        .bool(false)
        .u64(3)
        .text("Room")
        .u64(4)
        .text("Name")
        .u64(5)
        .text("PC")
        .u64(6)
        .u64(0)
        .u64(40)
        .map(1)
        .u64(0)
        .text("future");
    let b = w.into_bytes();
    let mut r = Reader::new(&b);
    let a = <AlarmRaised as Cbor>::decode(&mut r).expect("decodes");
    assert_eq!(a.room, "Room");
}

#[test]
fn unknown_alarm_kind_survives_as_other() {
    let (_, mut b) = samples().remove(1);
    if let Payload::AlarmRaised(a) = &mut b.payload {
        a.kind = AlarmKind::Other(9);
    }
    let back = Body::from_cbor(&b.to_cbor()).expect("decodes");
    assert_eq!(back, b);
}

#[test]
fn oversized_strings_rejected_on_receive() {
    let mut w = Writer::new();
    w.map(7)
        .u64(0)
        .bytes(&[2; 16])
        .u64(1)
        .u64(1)
        .u64(2)
        .bool(false)
        .u64(3)
        .text_full(&"x".repeat(257))
        .u64(4)
        .text("")
        .u64(5)
        .text("")
        .u64(6)
        .u64(0);
    assert_eq!(
        AlarmRaised::from_cbor(&w.into_bytes()),
        Err(DecodeError::TooLarge)
    );
}

#[test]
fn cert_round_trip() {
    let body = CertBody {
        version: 1,
        practice_id: PracticeId([1; 16]),
        serial: CertSerial([2; 16]),
        subject: DeviceId([3; 32]),
        subject_kx: [4; 32],
        role: Role::Admin,
        label: "PM-LAPTOP".into(),
        issued_at: 1,
        expires_at: 0,
        issuer: [5; 32],
        token_id: Some(TokenId([6; 16])),
    };
    let cert = Cert {
        body: body.to_cbor(),
        sig: [7; 64],
    };
    let back = Cert::from_cbor(&cert.to_cbor()).expect("decodes");
    assert_eq!(back.body().expect("body"), body);
    check_golden("cert", &cert.to_cbor());
}

proptest! {
    #[test]
    fn decoders_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = Envelope::from_cbor(&bytes);
        let _ = SignedMessage::from_cbor(&bytes);
        let _ = Body::from_cbor(&bytes);
        let _ = Cert::from_cbor(&bytes);
        let _ = CertBody::from_cbor(&bytes);
        let _ = panic_button::proto::relay::Frame::from_bytes(&bytes);
        let _ = panic_button::proto::relay::decode_batch(&bytes);
    }

    #[test]
    fn mutated_messages_never_panic(idx in 0usize..16, pos in any::<usize>(), byte in any::<u8>()) {
        let (_, b) = samples().swap_remove(idx);
        let mut bytes = b.to_cbor();
        let p = pos % bytes.len();
        bytes[p] = byte;
        let _ = Body::from_cbor(&bytes);
        bytes.truncate(p);
        let _ = Body::from_cbor(&bytes);
    }

    #[test]
    fn alarm_raised_round_trips(
        id in any::<[u8; 16]>(),
        drill in any::<bool>(),
        room in "\\PC{0,20}",
        user in "\\PC{0,20}",
        source in 0u64..10,
    ) {
        let a = AlarmRaised {
            alarm_id: MsgId(id),
            kind: AlarmKind::Help,
            drill,
            room: panic_button::proto::cbor::truncate_utf8(&room, 64).to_owned(),
            user_display: panic_button::proto::cbor::truncate_utf8(&user, 64).to_owned(),
            device_label: "PC".into(),
            source: Source::from_code(source),
        };
        prop_assert_eq!(AlarmRaised::from_cbor(&a.to_cbor()), Ok(a));
    }
}

/// PresenceSummary carries at most 20 peers: more are not sent, and a
/// message with more is refused.
#[test]
fn presence_summary_holds_at_most_20_peers() {
    let entry = |i: u8| PresenceEntry {
        device_id: DeviceId([i; 32]),
        last_seen_age_s: 1,
        path: PathKind::Multicast,
    };
    let big = PresenceSummary {
        peers: (0..25).map(entry).collect(),
    };
    let back = PresenceSummary::from_cbor(&big.to_cbor()).expect("decodes");
    assert_eq!(back.peers.len(), 20);
    let mut w = Writer::new();
    w.map(1).u64(0).array(21);
    for i in 0..21 {
        w.map(3).u64(0).bytes(&[i; 32]).u64(1).u64(1).u64(2).u64(0);
    }
    assert!(PresenceSummary::from_cbor(&w.into_bytes()).is_err());
}
