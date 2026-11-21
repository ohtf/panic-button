// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Message bodies and type-specific payloads (`spec/03-protocol.md`,
//! Message types).

use crate::proto::cbor::{Cbor, MapBuilder, Reader, Writer, req};
use crate::proto::error::DecodeError;
use crate::proto::ids::{CertSerial, DeviceId, MsgId, PracticeId};
use crate::proto::limits;
use crate::proto::settings::PracticeSettings;

macro_rules! open_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $var:ident = $v:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($(#[$vm])* $var,)*
            /// A code this version does not know. Kept so it round-trips.
            Other(u64),
        }

        impl $name {
            pub fn code(self) -> u64 {
                match self {
                    $(Self::$var => $v,)*
                    Self::Other(c) => c,
                }
            }

            pub fn from_code(c: u64) -> Self {
                match c {
                    $($v => Self::$var,)*
                    c => Self::Other(c),
                }
            }
        }
    };
}
pub(crate) use open_enum;

open_enum!(
    /// Message type codes.
    MsgType {
        Heartbeat = 0x01,
        AlarmRaised = 0x02,
        AlarmEvent = 0x03,
        SyncRequest = 0x04,
        SyncResponse = 0x05,
        CertAnnounce = 0x06,
        PracticeConfig = 0x07,
        PresenceSummary = 0x08,
        DeviceSettings = 0x09,
        JoinRequest = 0x40,
        JoinApproval = 0x41,
        KeyShare = 0x42,
        Revocation = 0x43,
        RelayAuth = 0x44,
    }
);

impl MsgType {
    /// Whether the type travels in an open (not group-sealed) envelope.
    pub fn is_open(self) -> bool {
        self.code() >= 0x40
    }

    /// Whether receivers re-flood this type once (FR-NET-002).
    pub fn floods(self) -> bool {
        matches!(
            self,
            MsgType::AlarmRaised
                | MsgType::AlarmEvent
                | MsgType::CertAnnounce
                | MsgType::PracticeConfig
                | MsgType::DeviceSettings
                | MsgType::KeyShare
                | MsgType::Revocation
        )
    }
}

open_enum!(
    Platform {
        Windows = 1,
        MacOs = 2,
        Linux = 3,
        Android = 4,
        Ios = 5,
        Headless = 6,
    }
);

open_enum!(
    SessionState {
        Active = 0,
        Locked = 1,
        NoUser = 2,
        RemoteSessionDisabled = 3,
        /// Another user's session is the one at the screen (fast user
        /// switching). The device stops taking part until it is active.
        Inactive = 4,
    }
);

open_enum!(
    StatusLevel {
        Green = 0,
        Amber = 1,
        Red = 2,
    }
);

open_enum!(
    /// Where an activation came from (FR-ALM-011).
    Source {
        Button = 0,
        Hotkey = 1,
        Hid = 2,
        Mobile = 3,
        Cli = 4,
        Integration = 5,
    }
);

open_enum!(
    /// Alarm kind. Receivers treat unknown kinds as help (ADR 0007).
    AlarmKind {
        Help = 1,
    }
);

open_enum!(
    EventKind {
        Receipt = 0,
        Seen = 1,
        Responding = 2,
        Resolved = 3,
        Cancelled = 4,
    }
);

open_enum!(
    /// The path by which a message arrived.
    PathKind {
        Multicast = 0,
        Broadcast = 1,
        UdpUnicast = 2,
        Tcp = 3,
        Relay = 4,
        Sync = 5,
    }
);

/// Heartbeat capability bits.
pub mod caps {
    pub const CAN_ALERT: u64 = 1;
    pub const CAN_RAISE: u64 = 2;
    pub const ADMIN: u64 = 4;
    pub const DRILL_COORDINATOR: u64 = 8;
    pub const RELAY_CONNECTED: u64 = 16;
}

/// SyncRequest `want` bits.
pub mod want {
    pub const OPEN_ALARMS: u64 = 1;
    pub const RECENT_ALARM_EVENTS: u64 = 2;
    pub const CERTS: u64 = 4;
    pub const REVOCATIONS: u64 = 8;
    pub const CONFIG: u64 = 16;
    pub const KEYSHARES_FOR_ME: u64 = 32;
}

/// `{0: output_present, 1: muted, 2: volume_pct}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioStatus {
    pub output_present: bool,
    pub muted: bool,
    pub volume_pct: u8,
}

impl AudioStatus {
    /// Audio is usable for alerting.
    pub fn ok(&self) -> bool {
        self.output_present && !self.muted && self.volume_pct > 0
    }
}

impl Cbor for AudioStatus {
    fn encode(&self, w: &mut Writer) {
        w.map(3)
            .u64(0)
            .bool(self.output_present)
            .u64(1)
            .bool(self.muted)
            .u64(2)
            .u64(u64::from(self.volume_pct));
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut p, mut m, mut v) = (None, None, None);
        r.map(|k, r| {
            match k {
                0 => p = Some(r.bool()?),
                1 => m = Some(r.bool()?),
                2 => v = Some(r.u64()?.min(100) as u8),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(AudioStatus {
            output_present: req(p, 0)?,
            muted: req(m, 1)?,
            volume_pct: req(v, 2)?,
        })
    }
}

/// 0x01 Heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    pub label: String,
    pub room: String,
    pub user_display: String,
    pub app_version: String,
    pub proto_version: u64,
    pub platform: Platform,
    pub tcp_port: u16,
    pub addrs: Vec<[u8; 4]>,
    pub capabilities: u64,
    pub audio: AudioStatus,
    pub session: SessionState,
    pub status: StatusLevel,
    pub cert_serial: CertSerial,
    pub epoch: u64,
    pub alarm_digest: [u8; 16],
    pub state_digest: [u8; 16],
    pub near_miss_24h: u64,
    pub uptime_s: u64,
}

impl Cbor for Heartbeat {
    fn encode(&self, w: &mut Writer) {
        let addrs: Vec<_> = self
            .addrs
            .iter()
            .take(limits::HEARTBEAT_ADDRS_MAX)
            .collect();
        w.map(18);
        w.u64(0).text(&self.label);
        w.u64(1).text(&self.room);
        w.u64(2).text(&self.user_display);
        w.u64(3).text(&self.app_version);
        w.u64(4).u64(self.proto_version);
        w.u64(5).u64(self.platform.code());
        w.u64(6).u64(u64::from(self.tcp_port));
        w.u64(7).array(addrs.len());
        for a in addrs {
            w.bytes(a);
        }
        w.u64(8).u64(self.capabilities);
        w.u64(9);
        self.audio.encode(w);
        w.u64(10).u64(self.session.code());
        w.u64(11).u64(self.status.code());
        w.u64(12).bytes(&self.cert_serial.0);
        w.u64(13).u64(self.epoch);
        w.u64(14).bytes(&self.alarm_digest);
        w.u64(15).bytes(&self.state_digest);
        w.u64(16).u64(self.near_miss_24h);
        w.u64(17).u64(self.uptime_s);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let mut label = None;
        let mut room = None;
        let mut user = None;
        let mut app = None;
        let mut proto = None;
        let mut platform = None;
        let mut port = None;
        let mut addrs = None;
        let mut capabilities = None;
        let mut audio = None;
        let mut session = None;
        let mut status = None;
        let mut serial = None;
        let mut epoch = None;
        let mut ad = None;
        let mut sd = None;
        let mut nm = None;
        let mut up = None;
        r.map(|k, r| {
            match k {
                0 => label = Some(r.text()?),
                1 => room = Some(r.text()?),
                2 => user = Some(r.text()?),
                3 => app = Some(r.text()?),
                4 => proto = Some(r.u64()?),
                5 => platform = Some(Platform::from_code(r.u64()?)),
                6 => port = Some(r.u16()?),
                7 => addrs = Some(r.array_of(limits::HEARTBEAT_ADDRS_MAX, |r| r.fixed::<4>())?),
                8 => capabilities = Some(r.u64()?),
                9 => audio = Some(AudioStatus::decode(r)?),
                10 => session = Some(SessionState::from_code(r.u64()?)),
                11 => status = Some(StatusLevel::from_code(r.u64()?)),
                12 => serial = Some(CertSerial::decode(r)?),
                13 => epoch = Some(r.u64()?),
                14 => ad = Some(r.fixed::<16>()?),
                15 => sd = Some(r.fixed::<16>()?),
                16 => nm = Some(r.u64()?),
                17 => up = Some(r.u64()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(Heartbeat {
            label: req(label, 0)?,
            room: req(room, 1)?,
            user_display: req(user, 2)?,
            app_version: req(app, 3)?,
            proto_version: req(proto, 4)?,
            platform: req(platform, 5)?,
            tcp_port: req(port, 6)?,
            addrs: req(addrs, 7)?,
            capabilities: req(capabilities, 8)?,
            audio: req(audio, 9)?,
            session: req(session, 10)?,
            status: req(status, 11)?,
            cert_serial: req(serial, 12)?,
            epoch: req(epoch, 13)?,
            alarm_digest: req(ad, 14)?,
            state_digest: req(sd, 15)?,
            near_miss_24h: req(nm, 16)?,
            uptime_s: req(up, 17)?,
        })
    }
}

/// 0x02 AlarmRaised. `raised_at` is the body's `created_at`, and the origin
/// is the body's `sender`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmRaised {
    pub alarm_id: MsgId,
    pub kind: AlarmKind,
    pub drill: bool,
    pub room: String,
    pub user_display: String,
    pub device_label: String,
    pub source: Source,
}

impl Cbor for AlarmRaised {
    fn encode(&self, w: &mut Writer) {
        w.map(7);
        w.u64(0).bytes(&self.alarm_id.0);
        w.u64(1).u64(self.kind.code());
        w.u64(2).bool(self.drill);
        w.u64(3).text(&self.room);
        w.u64(4).text(&self.user_display);
        w.u64(5).text(&self.device_label);
        w.u64(6).u64(self.source.code());
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut id, mut kind, mut drill, mut room, mut user, mut label, mut source) =
            (None, None, None, None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => id = Some(MsgId::decode(r)?),
                1 => kind = Some(AlarmKind::from_code(r.u64()?)),
                2 => drill = Some(r.bool()?),
                3 => room = Some(r.text()?),
                4 => user = Some(r.text()?),
                5 => label = Some(r.text()?),
                6 => source = Some(Source::from_code(r.u64()?)),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(AlarmRaised {
            alarm_id: req(id, 0)?,
            kind: req(kind, 1)?,
            drill: req(drill, 2)?,
            room: req(room, 3)?,
            user_display: req(user, 4)?,
            device_label: req(label, 5)?,
            source: req(source, 6)?,
        })
    }
}

/// Receipt detail: `{0: audio, 1: path, 2: displayed, 3: drill_rtt_ref}`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiptDetail {
    pub audio: Option<AudioStatus>,
    pub path: Option<PathKind>,
    pub displayed: Option<bool>,
    pub drill_rtt_ref: Option<MsgId>,
}

/// The optional `detail` map of an AlarmEvent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EventDetail {
    #[default]
    None,
    Receipt(ReceiptDetail),
    /// `{0: reason}`: 0 sent_in_error, 1 other.
    Cancelled {
        reason: u64,
    },
}

/// Cancellation reason codes.
pub const CANCEL_SENT_IN_ERROR: u64 = 0;
pub const CANCEL_OTHER: u64 = 1;

/// 0x03 AlarmEvent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmEvent {
    pub alarm_id: MsgId,
    pub event: EventKind,
    pub user_display: String,
    pub detail: EventDetail,
}

impl Cbor for AlarmEvent {
    fn encode(&self, w: &mut Writer) {
        let mut m = MapBuilder::new();
        m.entry(0, |w| {
            w.bytes(&self.alarm_id.0);
        });
        m.entry(1, |w| {
            w.u64(self.event.code());
        });
        m.entry(2, |w| {
            w.text(&self.user_display);
        });
        match &self.detail {
            EventDetail::None => {}
            EventDetail::Receipt(d) => {
                m.entry(3, |w| {
                    let mut dm = MapBuilder::new();
                    if let Some(a) = &d.audio {
                        dm.entry(0, |w| a.encode(w));
                    }
                    if let Some(p) = d.path {
                        dm.entry(1, |w| {
                            w.u64(p.code());
                        });
                    }
                    if let Some(b) = d.displayed {
                        dm.entry(2, |w| {
                            w.bool(b);
                        });
                    }
                    if let Some(id) = &d.drill_rtt_ref {
                        dm.entry(3, |w| {
                            w.bytes(&id.0);
                        });
                    }
                    dm.write(w);
                });
            }
            EventDetail::Cancelled { reason } => {
                m.entry(3, |w| {
                    w.map(1).u64(0).u64(*reason);
                });
            }
        }
        m.write(w);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut id, mut event, mut user) = (None, None, None);
        let mut detail_raw: Option<&[u8]> = None;
        r.map(|k, r| {
            match k {
                0 => id = Some(MsgId::decode(r)?),
                1 => event = Some(EventKind::from_code(r.u64()?)),
                2 => user = Some(r.text()?),
                3 => detail_raw = Some(r.raw_item()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        let event = req(event, 1)?;
        let detail = match (event, detail_raw) {
            (_, None) => EventDetail::None,
            (EventKind::Receipt, Some(raw)) => {
                let mut d = ReceiptDetail::default();
                let mut r = Reader::new(raw);
                r.map(|k, r| {
                    match k {
                        0 => d.audio = Some(AudioStatus::decode(r)?),
                        1 => d.path = Some(PathKind::from_code(r.u64()?)),
                        2 => d.displayed = Some(r.bool()?),
                        3 => d.drill_rtt_ref = Some(MsgId::decode(r)?),
                        _ => return Ok(false),
                    }
                    Ok(true)
                })?;
                EventDetail::Receipt(d)
            }
            (EventKind::Cancelled, Some(raw)) => {
                let mut reason = None;
                let mut r = Reader::new(raw);
                r.map(|k, r| {
                    if k == 0 {
                        reason = Some(r.u64()?);
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                })?;
                EventDetail::Cancelled {
                    reason: reason.unwrap_or(CANCEL_OTHER),
                }
            }
            // Detail for other event kinds is not defined; ignore it.
            (_, Some(_)) => EventDetail::None,
        };
        Ok(AlarmEvent {
            alarm_id: req(id, 0)?,
            event,
            user_display: req(user, 2)?,
            detail,
        })
    }
}

/// 0x04 SyncRequest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncRequest {
    pub want: u64,
    pub since: Option<u64>,
    pub ids: Vec<MsgId>,
    /// Key 3 (extension): the device that should answer. Lets a request sent
    /// via a relay, which reaches every device, be answered by one.
    pub to: Option<DeviceId>,
    /// Key 4 (extension): the requester's alarm digest, so a responder whose
    /// digest differs can ask back (repair from either side).
    pub alarm_digest: Option<[u8; 16]>,
}

impl Cbor for SyncRequest {
    fn encode(&self, w: &mut Writer) {
        let mut m = MapBuilder::new();
        m.entry(0, |w| {
            w.u64(self.want);
        });
        if let Some(s) = self.since {
            m.entry(1, |w| {
                w.u64(s);
            });
        }
        if !self.ids.is_empty() {
            m.entry(2, |w| {
                w.array(self.ids.len());
                for id in &self.ids {
                    w.bytes(&id.0);
                }
            });
        }
        if let Some(to) = &self.to {
            m.entry(3, |w| to.encode(w));
        }
        if let Some(d) = &self.alarm_digest {
            m.entry(4, |w| {
                w.bytes(d);
            });
        }
        m.write(w);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut want, mut since, mut ids, mut to, mut ad) = (None, None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => want = Some(r.u64()?),
                1 => since = Some(r.u64()?),
                2 => ids = Some(r.array_of(limits::LIST_MAX, MsgId::decode)?),
                3 => to = Some(DeviceId::decode(r)?),
                4 => ad = Some(r.fixed::<16>()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(SyncRequest {
            want: req(want, 0)?,
            since,
            ids: ids.unwrap_or_default(),
            to,
            alarm_digest: ad,
        })
    }
}

fn encode_bstr_list(w: &mut Writer, items: &[Vec<u8>]) {
    w.array(items.len());
    for i in items {
        w.bytes(i);
    }
}

fn decode_bstr_list(r: &mut Reader<'_>, max_items: usize) -> Result<Vec<Vec<u8>>, DecodeError> {
    r.array_of(max_items, |r| Ok(r.bytes(limits::TCP_FRAME_MAX)?.to_vec()))
}

/// 0x05 SyncResponse: original raw envelopes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncResponse {
    pub envelopes: Vec<Vec<u8>>,
    pub more: bool,
}

impl Cbor for SyncResponse {
    fn encode(&self, w: &mut Writer) {
        w.map(2).u64(0);
        encode_bstr_list(w, &self.envelopes);
        w.u64(1).bool(self.more);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut env, mut more) = (None, None);
        r.map(|k, r| {
            match k {
                0 => env = Some(decode_bstr_list(r, limits::LIST_MAX)?),
                1 => more = Some(r.bool()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(SyncResponse {
            envelopes: req(env, 0)?,
            more: more.unwrap_or(false),
        })
    }
}

/// 0x06 CertAnnounce: raw signed certificates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CertAnnounce {
    pub certs: Vec<Vec<u8>>,
}

impl Cbor for CertAnnounce {
    fn encode(&self, w: &mut Writer) {
        w.map(1).u64(0);
        encode_bstr_list(w, &self.certs);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let mut certs = None;
        r.map(|k, r| {
            if k == 0 {
                certs = Some(decode_bstr_list(r, limits::LIST_MAX)?);
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        Ok(CertAnnounce {
            certs: req(certs, 0)?,
        })
    }
}

/// 0x07 PracticeConfig, signed by an admin device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PracticeConfig {
    pub version: u64,
    pub settings: PracticeSettings,
}

impl Cbor for PracticeConfig {
    fn encode(&self, w: &mut Writer) {
        w.map(2).u64(0).u64(self.version).u64(1);
        self.settings.encode(w);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut v, mut s) = (None, None);
        r.map(|k, r| {
            match k {
                0 => v = Some(r.u64()?),
                1 => s = Some(PracticeSettings::decode(r)?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(PracticeConfig {
            version: req(v, 0)?,
            settings: req(s, 1)?,
        })
    }
}

/// 0x09 DeviceSettings: an admin sets a device's label and room
/// (ADR 0014). Also the stored form of a device's own label and room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSettings {
    pub target: DeviceId,
    /// `None` leaves it unchanged.
    pub label: Option<String>,
    /// `None` leaves it unchanged.
    pub room: Option<String>,
    /// When the change was made (wall ms); the newest change wins.
    pub assigned_at: u64,
}

impl Cbor for DeviceSettings {
    fn encode(&self, w: &mut Writer) {
        let mut m = MapBuilder::new();
        m.entry(0, |w| self.target.encode(w));
        if let Some(l) = &self.label {
            m.entry(1, |w| {
                w.text(l);
            });
        }
        if let Some(r) = &self.room {
            m.entry(2, |w| {
                w.text(r);
            });
        }
        m.entry(3, |w| {
            w.u64(self.assigned_at);
        });
        m.write(w);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut target, mut label, mut room, mut at) = (None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => target = Some(DeviceId::decode(r)?),
                1 => label = Some(r.text()?),
                2 => room = Some(r.text()?),
                3 => at = Some(r.u64()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(DeviceSettings {
            target: req(target, 0)?,
            label,
            room,
            assigned_at: req(at, 3)?,
        })
    }
}

/// One entry of a PresenceSummary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceEntry {
    pub device_id: DeviceId,
    pub last_seen_age_s: u64,
    pub path: PathKind,
}

/// 0x08 PresenceSummary: direct peers only, at most 20 per message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PresenceSummary {
    pub peers: Vec<PresenceEntry>,
}

impl Cbor for PresenceSummary {
    fn encode(&self, w: &mut Writer) {
        let peers: Vec<_> = self
            .peers
            .iter()
            .take(limits::PRESENCE_ENTRIES_MAX)
            .collect();
        w.map(1).u64(0).array(peers.len());
        for p in peers {
            w.map(3)
                .u64(0)
                .bytes(&p.device_id.0)
                .u64(1)
                .u64(p.last_seen_age_s)
                .u64(2)
                .u64(p.path.code());
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let mut peers = None;
        r.map(|k, r| {
            if k != 0 {
                return Ok(false);
            }
            peers = Some(r.array_of(limits::PRESENCE_ENTRIES_MAX, |r| {
                let (mut id, mut age, mut path) = (None, None, None);
                r.map(|k, r| {
                    match k {
                        0 => id = Some(DeviceId::decode(r)?),
                        1 => age = Some(r.u64()?),
                        2 => path = Some(PathKind::from_code(r.u64()?)),
                        _ => return Ok(false),
                    }
                    Ok(true)
                })?;
                Ok(PresenceEntry {
                    device_id: req(id, 0)?,
                    last_seen_age_s: req(age, 1)?,
                    path: req(path, 2)?,
                })
            })?);
            Ok(true)
        })?;
        Ok(PresenceSummary {
            peers: req(peers, 0)?,
        })
    }
}

/// 0x40 JoinRequest, self-signed by the joining device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequest {
    pub kx_pub: [u8; 32],
    pub label: String,
    pub practice_hint: Option<PracticeId>,
    pub code: String,
    /// Key 4 (extension): the joining device's TCP port, so the approval
    /// (which may not fit a datagram) can be sent by TCP.
    pub tcp_port: Option<u16>,
}

impl Cbor for JoinRequest {
    fn encode(&self, w: &mut Writer) {
        let mut m = MapBuilder::new();
        m.entry(0, |w| {
            w.bytes(&self.kx_pub);
        });
        m.entry(1, |w| {
            w.text(&self.label);
        });
        if let Some(h) = &self.practice_hint {
            m.entry(2, |w| {
                w.bytes(&h.0);
            });
        }
        m.entry(3, |w| {
            w.text(&self.code);
        });
        if let Some(p) = self.tcp_port {
            m.entry(4, |w| {
                w.u64(u64::from(p));
            });
        }
        m.write(w);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut kx, mut label, mut hint, mut code, mut port) = (None, None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => kx = Some(r.fixed::<32>()?),
                1 => label = Some(r.text()?),
                2 => hint = Some(PracticeId::decode(r)?),
                3 => code = Some(r.text_max(16)?),
                4 => port = Some(r.u16()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(JoinRequest {
            kx_pub: req(kx, 0)?,
            label: req(label, 1)?,
            practice_hint: hint,
            code: req(code, 3)?,
            tcp_port: port,
        })
    }
}

/// 0x42 KeyShare: the group key for `epoch`, sealed to one recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyShare {
    pub recipient: DeviceId,
    pub epoch: u64,
    pub eph_pub: [u8; 32],
    pub nonce: [u8; 24],
    pub ct: Vec<u8>,
}

impl Cbor for KeyShare {
    fn encode(&self, w: &mut Writer) {
        w.map(5)
            .u64(0)
            .bytes(&self.recipient.0)
            .u64(1)
            .u64(self.epoch)
            .u64(2)
            .bytes(&self.eph_pub)
            .u64(3)
            .bytes(&self.nonce)
            .u64(4)
            .bytes(&self.ct);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut rc, mut ep, mut eph, mut n, mut ct) = (None, None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => rc = Some(DeviceId::decode(r)?),
                1 => ep = Some(r.u64()?),
                2 => eph = Some(r.fixed::<32>()?),
                3 => n = Some(r.fixed::<24>()?),
                4 => ct = Some(r.bytes(256)?.to_vec()),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(KeyShare {
            recipient: req(rc, 0)?,
            epoch: req(ep, 1)?,
            eph_pub: req(eph, 2)?,
            nonce: req(n, 3)?,
            ct: req(ct, 4)?,
        })
    }
}

/// 0x41 JoinApproval, signed by an admin device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinApproval {
    pub recipient: DeviceId,
    /// New member cert first, then the issuer chain.
    pub certs: Vec<Vec<u8>>,
    pub root_pub: [u8; 32],
    pub practice_name: String,
    pub keyshare: KeyShare,
    /// Latest PracticeConfig envelope (raw), or empty if none yet.
    pub config: Vec<u8>,
}

impl Cbor for JoinApproval {
    fn encode(&self, w: &mut Writer) {
        w.map(6).u64(0).bytes(&self.recipient.0).u64(1);
        encode_bstr_list(w, &self.certs);
        w.u64(2).bytes(&self.root_pub);
        w.u64(3).text(&self.practice_name);
        w.u64(4);
        self.keyshare.encode(w);
        w.u64(5).bytes(&self.config);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut rc, mut certs, mut root, mut name, mut ks, mut cfg) =
            (None, None, None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => rc = Some(DeviceId::decode(r)?),
                1 => certs = Some(decode_bstr_list(r, limits::CERT_CHAIN_MAX + 1)?),
                2 => root = Some(r.fixed::<32>()?),
                3 => name = Some(r.text()?),
                4 => ks = Some(KeyShare::decode(r)?),
                5 => cfg = Some(r.bytes(limits::TCP_FRAME_MAX)?.to_vec()),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(JoinApproval {
            recipient: req(rc, 0)?,
            certs: req(certs, 1)?,
            root_pub: req(root, 2)?,
            practice_name: req(name, 3)?,
            keyshare: req(ks, 4)?,
            config: cfg.unwrap_or_default(),
        })
    }
}

/// 0x43 Revocation, signed by an admin device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revocation {
    pub target_serial: CertSerial,
    pub target_device: DeviceId,
    pub cascade: bool,
    pub reason: String,
    pub new_epoch: u64,
}

impl Cbor for Revocation {
    fn encode(&self, w: &mut Writer) {
        w.map(5)
            .u64(0)
            .bytes(&self.target_serial.0)
            .u64(1)
            .bytes(&self.target_device.0)
            .u64(2)
            .bool(self.cascade)
            .u64(3)
            .text(&self.reason)
            .u64(4)
            .u64(self.new_epoch);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut s, mut d, mut c, mut re, mut e) = (None, None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => s = Some(CertSerial::decode(r)?),
                1 => d = Some(DeviceId::decode(r)?),
                2 => c = Some(r.bool()?),
                3 => re = Some(r.text()?),
                4 => e = Some(r.u64()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(Revocation {
            target_serial: req(s, 0)?,
            target_device: req(d, 1)?,
            cascade: req(c, 2)?,
            reason: req(re, 3)?,
            new_epoch: req(e, 4)?,
        })
    }
}

/// 0x44 RelayAuth (`spec/06-relay.md`, Handshake).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayAuth {
    pub practice_id: PracticeId,
    /// Raw signed certs: the device cert first, up to but not including root.
    pub chain: Vec<Vec<u8>>,
    pub nonce: [u8; 32],
    pub client_time: u64,
    pub resume_cursor: Option<u64>,
}

impl Cbor for RelayAuth {
    fn encode(&self, w: &mut Writer) {
        let mut m = MapBuilder::new();
        m.entry(0, |w| {
            w.bytes(&self.practice_id.0);
        });
        m.entry(1, |w| encode_bstr_list(w, &self.chain));
        m.entry(2, |w| {
            w.bytes(&self.nonce);
        });
        m.entry(3, |w| {
            w.u64(self.client_time);
        });
        if let Some(c) = self.resume_cursor {
            m.entry(4, |w| {
                w.u64(c);
            });
        }
        m.write(w);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut p, mut ch, mut n, mut t, mut c) = (None, None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => p = Some(PracticeId::decode(r)?),
                1 => ch = Some(decode_bstr_list(r, limits::CERT_CHAIN_MAX)?),
                2 => n = Some(r.fixed::<32>()?),
                3 => t = Some(r.u64()?),
                4 => c = Some(r.u64()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(RelayAuth {
            practice_id: req(p, 0)?,
            chain: req(ch, 1)?,
            nonce: req(n, 2)?,
            client_time: req(t, 3)?,
            resume_cursor: c,
        })
    }
}

/// A type-specific payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    Heartbeat(Heartbeat),
    AlarmRaised(AlarmRaised),
    AlarmEvent(AlarmEvent),
    SyncRequest(SyncRequest),
    SyncResponse(SyncResponse),
    CertAnnounce(CertAnnounce),
    PracticeConfig(PracticeConfig),
    PresenceSummary(PresenceSummary),
    DeviceSettings(DeviceSettings),
    JoinRequest(JoinRequest),
    JoinApproval(JoinApproval),
    KeyShare(KeyShare),
    Revocation(Revocation),
    RelayAuth(RelayAuth),
}

impl Payload {
    pub fn msg_type(&self) -> MsgType {
        match self {
            Payload::Heartbeat(_) => MsgType::Heartbeat,
            Payload::AlarmRaised(_) => MsgType::AlarmRaised,
            Payload::AlarmEvent(_) => MsgType::AlarmEvent,
            Payload::SyncRequest(_) => MsgType::SyncRequest,
            Payload::SyncResponse(_) => MsgType::SyncResponse,
            Payload::CertAnnounce(_) => MsgType::CertAnnounce,
            Payload::PracticeConfig(_) => MsgType::PracticeConfig,
            Payload::PresenceSummary(_) => MsgType::PresenceSummary,
            Payload::DeviceSettings(_) => MsgType::DeviceSettings,
            Payload::JoinRequest(_) => MsgType::JoinRequest,
            Payload::JoinApproval(_) => MsgType::JoinApproval,
            Payload::KeyShare(_) => MsgType::KeyShare,
            Payload::Revocation(_) => MsgType::Revocation,
            Payload::RelayAuth(_) => MsgType::RelayAuth,
        }
    }

    fn encode(&self, w: &mut Writer) {
        match self {
            Payload::Heartbeat(p) => p.encode(w),
            Payload::AlarmRaised(p) => p.encode(w),
            Payload::AlarmEvent(p) => p.encode(w),
            Payload::SyncRequest(p) => p.encode(w),
            Payload::SyncResponse(p) => p.encode(w),
            Payload::CertAnnounce(p) => p.encode(w),
            Payload::PracticeConfig(p) => p.encode(w),
            Payload::PresenceSummary(p) => p.encode(w),
            Payload::DeviceSettings(p) => p.encode(w),
            Payload::JoinRequest(p) => p.encode(w),
            Payload::JoinApproval(p) => p.encode(w),
            Payload::KeyShare(p) => p.encode(w),
            Payload::Revocation(p) => p.encode(w),
            Payload::RelayAuth(p) => p.encode(w),
        }
    }

    fn decode_typed(t: MsgType, raw: &[u8]) -> Result<Self, DecodeError> {
        Ok(match t {
            MsgType::Heartbeat => Payload::Heartbeat(Heartbeat::from_cbor(raw)?),
            MsgType::AlarmRaised => Payload::AlarmRaised(AlarmRaised::from_cbor(raw)?),
            MsgType::AlarmEvent => Payload::AlarmEvent(AlarmEvent::from_cbor(raw)?),
            MsgType::SyncRequest => Payload::SyncRequest(SyncRequest::from_cbor(raw)?),
            MsgType::SyncResponse => Payload::SyncResponse(SyncResponse::from_cbor(raw)?),
            MsgType::CertAnnounce => Payload::CertAnnounce(CertAnnounce::from_cbor(raw)?),
            MsgType::PracticeConfig => Payload::PracticeConfig(PracticeConfig::from_cbor(raw)?),
            MsgType::PresenceSummary => Payload::PresenceSummary(PresenceSummary::from_cbor(raw)?),
            MsgType::DeviceSettings => Payload::DeviceSettings(DeviceSettings::from_cbor(raw)?),
            MsgType::JoinRequest => Payload::JoinRequest(JoinRequest::from_cbor(raw)?),
            MsgType::JoinApproval => Payload::JoinApproval(JoinApproval::from_cbor(raw)?),
            MsgType::KeyShare => Payload::KeyShare(KeyShare::from_cbor(raw)?),
            MsgType::Revocation => Payload::Revocation(Revocation::from_cbor(raw)?),
            MsgType::RelayAuth => Payload::RelayAuth(RelayAuth::from_cbor(raw)?),
            MsgType::Other(c) => return Err(DecodeError::UnknownType(c)),
        })
    }
}

/// `Body = {0: msg_id, 1: type, 2: created_at, 3: sender, 4: payload}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub msg_id: MsgId,
    pub created_at: u64,
    pub sender: DeviceId,
    pub payload: Payload,
}

impl Body {
    pub fn msg_type(&self) -> MsgType {
        self.payload.msg_type()
    }
}

impl Cbor for Body {
    fn encode(&self, w: &mut Writer) {
        w.map(5)
            .u64(0)
            .bytes(&self.msg_id.0)
            .u64(1)
            .u64(self.msg_type().code())
            .u64(2)
            .u64(self.created_at)
            .u64(3)
            .bytes(&self.sender.0)
            .u64(4);
        self.payload.encode(w);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut id, mut t, mut at, mut sender) = (None, None, None, None);
        let mut raw: Option<&[u8]> = None;
        r.map(|k, r| {
            match k {
                0 => id = Some(MsgId::decode(r)?),
                1 => t = Some(MsgType::from_code(r.u64()?)),
                2 => at = Some(r.u64()?),
                3 => sender = Some(DeviceId::decode(r)?),
                4 => raw = Some(r.raw_item()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        let t = req(t, 1)?;
        let payload = Payload::decode_typed(t, req(raw, 4)?)?;
        Ok(Body {
            msg_id: req(id, 0)?,
            created_at: req(at, 2)?,
            sender: req(sender, 3)?,
            payload,
        })
    }
}
