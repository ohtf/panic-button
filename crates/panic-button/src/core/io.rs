// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Inputs to and outputs from the core (`spec/02-architecture.md`, Sans-IO
//! core).

use std::net::{Ipv4Addr, SocketAddrV4};

use crate::proto::{AudioStatus, DeviceId, MsgId, PathKind, SessionState, Source, StatusLevel};

use crate::core::config::LocalConfig;
use crate::core::persist::PersistOp;
use crate::core::time::WallMs;

/// How a datagram arrived or is to be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UdpKind {
    Multicast,
    Broadcast,
    Unicast,
}

impl UdpKind {
    pub fn path(self) -> PathKind {
        match self {
            UdpKind::Multicast => PathKind::Multicast,
            UdpKind::Broadcast => PathKind::Broadcast,
            UdpKind::Unicast => PathKind::UdpUnicast,
        }
    }
}

/// Where to send a datagram. The runtime expands `Multicast` to the group
/// on every interface and `Broadcast` to the limited and every directed
/// broadcast address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UdpDest {
    Multicast,
    Broadcast,
    Unicast(SocketAddrV4),
}

/// Identifies a configured relay (index into the relay URL list).
pub type RelayId = u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelayLinkState {
    Connected,
    Disconnected,
}

/// One local IPv4 interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterfaceInfo {
    pub addr: Ipv4Addr,
    pub broadcast: Option<Ipv4Addr>,
}

/// Actions from the local user (or the CLI / control API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    /// Pointer down on the button, shortcut pressed, HID pressed.
    /// `practice` is the first-run practice hold: it never raises an alarm.
    HoldStart {
        source: Source,
        practice: bool,
    },
    HoldEnd {
        source: Source,
    },
    /// Raise immediately, bypassing the hold (CLI and tests; the control API
    /// gates this by policy).
    RaiseNow {
        source: Source,
    },
    Respond {
        alarm_id: MsgId,
    },
    Seen {
        alarm_id: MsgId,
    },
    Resolve {
        alarm_id: MsgId,
    },
    /// Cancel an own alarm as sent in error.
    Cancel {
        alarm_id: MsgId,
    },
    /// Admin: run a drill now (this device must be the coordinator or an
    /// admin).
    RunDrill,
    /// Admin: approve a pending join request.
    ApproveJoin {
        device: DeviceId,
    },
    RejectJoin {
        device: DeviceId,
    },
    /// Admin: make a member device an admin (issues it an admin cert).
    Promote {
        device: DeviceId,
    },
    /// Install a cert for this device made outside the core (admin
    /// recovery: the practice root, from the recovery kit, signs a new admin
    /// cert). Ignored unless it is valid and for this device.
    InstallOwnCert(Vec<u8>),
    /// Set this device's own label or room (`None` leaves it unchanged).
    SetOwnSettings {
        label: Option<String>,
        room: Option<String>,
    },
    /// Admin: set another device's label or room (ADR 0014).
    AssignDeviceSettings {
        device: DeviceId,
        label: Option<String>,
        room: Option<String>,
    },
    /// Admin: revoke a device. `cascade` = "compromised".
    Revoke {
        device: DeviceId,
        cascade: bool,
        reason: String,
    },
    /// Admin: publish new practice settings.
    PublishSettings(crate::proto::PracticeSettings),
    /// Unenrolled device: start broadcasting a join request.
    StartJoin {
        practice_hint: Option<crate::proto::PracticeId>,
    },
    /// Unenrolled device: accept the received approval ("Join <name>?").
    ConfirmJoin,
    /// Unenrolled device: stop joining.
    CancelJoin,
}

/// Everything the core reacts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Tick,
    Datagram {
        kind: UdpKind,
        from: SocketAddrV4,
        bytes: Vec<u8>,
    },
    /// One length-delimited frame from a TCP connection.
    StreamFrame {
        from: SocketAddrV4,
        bytes: Vec<u8>,
    },
    RelayFrame {
        relay: RelayId,
        bytes: Vec<u8>,
    },
    RelayState {
        relay: RelayId,
        state: RelayLinkState,
        presence: Option<Vec<DeviceId>>,
    },
    /// A peer address found by mDNS (practice tag matched).
    Discovered {
        addr: SocketAddrV4,
    },
    InterfacesChanged(Vec<InterfaceInfo>),
    /// macOS: the system refuses this app local network access (System
    /// Settings, Privacy & Security, Local Network), or allows it again.
    LocalNetworkBlocked(bool),
    AudioStatus {
        status: AudioStatus,
        override_available: bool,
    },
    SessionState(SessionState),
    /// The interactive user's display name changed.
    UserDisplay(String),
    User(UserAction),
    ConfigChanged(LocalConfig),
}

/// A view of one alarm for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmView {
    pub alarm_id: MsgId,
    pub origin: DeviceId,
    pub own: bool,
    pub drill: bool,
    pub room: String,
    pub user_display: String,
    pub device_label: String,
    pub raised_at: WallMs,
    pub source: Source,
    pub state: AlarmPhase,
    /// Distinct devices that sent a Receipt.
    pub delivered: usize,
    /// "Coming" names, in response order.
    pub coming: Vec<String>,
    pub seen: Vec<String>,
    pub local_ack: Option<LocalAck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlarmPhase {
    Open,
    Resolved { by: String, at: WallMs },
    Cancelled { at: WallMs },
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAck {
    Seen,
    Responding,
}

/// Sender panel delivery wording (`spec/05-desktop-agent.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderPhase {
    /// "Help requested".
    Requested,
    /// "Delivered to N devices" (and possibly "Coming: ...").
    Delivered,
    /// "Not yet delivered - trying all routes".
    NotYetDelivered,
    /// "Could not reach other devices. Call for help another way."
    CouldNotReach,
}

/// A status reason, rendered by the UI in plain English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StatusReason {
    NotEnrolled,
    CertRevoked,
    NoGroupKey,
    NoDevicesReachable,
    RemoteSessionDisabled,
    SessionInactive,
    LocalNetworkBlocked,
    AudioMuted,
    AudioMissing,
    RelayUnreachable,
    OnlyViaRelay,
    StaleEpoch,
    NoOtherDevices,
}

impl StatusReason {
    pub fn text(self) -> &'static str {
        match self {
            StatusReason::NotEnrolled => "This computer is not set up yet",
            StatusReason::CertRevoked => "This computer has been removed from the practice",
            StatusReason::NoGroupKey => "Missing the practice key - ask an admin",
            StatusReason::NoDevicesReachable => "No other devices reachable",
            StatusReason::RemoteSessionDisabled => {
                "Panic Button cannot run inside a remote session. Ask IT to install it on this computer itself."
            }
            StatusReason::SessionInactive => {
                "Paused while someone else is signed in and using this computer"
            }
            StatusReason::LocalNetworkBlocked => {
                "Local network access is turned off for Panic Button. Turn it on in System Settings, Privacy & Security, Local Network."
            }
            StatusReason::AudioMuted => "Sound is muted on this computer",
            StatusReason::AudioMissing => "No speakers or headphones found",
            StatusReason::RelayUnreachable => "Relay unreachable",
            StatusReason::OnlyViaRelay => "Only reachable via relay",
            StatusReason::StaleEpoch => "Waiting for the new practice key",
            StatusReason::NoOtherDevices => "No other devices found",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusView {
    pub level: StatusLevel,
    pub reasons: Vec<StatusReason>,
    pub direct: usize,
    pub via_relay: usize,
}

impl StatusView {
    /// For example "Connected to 14 devices (12 direct, 2 via relay)".
    pub fn summary(&self) -> String {
        let n = self.direct + self.via_relay;
        if n == 0 {
            return "Not connected to any devices".into();
        }
        let noun = if n == 1 { "device" } else { "devices" };
        if self.via_relay == 0 {
            format!("Connected to {n} {noun}")
        } else {
            format!(
                "Connected to {n} {noun} ({} direct, {} via relay)",
                self.direct, self.via_relay
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// Start showing a receiver alert. `resumed` is true when this resumes
    /// an alert interrupted by a restart (ADR 0013).
    AlarmShown {
        alarm: AlarmView,
        resumed: bool,
    },
    AlarmUpdated(AlarmView),
    /// The alarm closed or expired; the window shows the outcome for 15 s.
    AlarmClosed(AlarmView),
    /// An alarm went straight to history without alerting (expired on
    /// arrival, or closed before it reached us).
    AlarmHistory(AlarmView),
    /// Sender panel for our own alarm.
    SenderPanel {
        alarm: AlarmView,
        phase: SenderPhase,
    },
    DrillToast {
        alarm_id: MsgId,
        audible: bool,
    },
    StatusChanged(StatusView),
    HoldProgress {
        fraction_permille: u16,
    },
    HoldEnded,
    /// A practice hold (first run) reached `hold_ms`. Nothing was raised.
    PracticeComplete,
    NearMiss {
        source: Source,
        duration_ms: u64,
    },
    /// Admin devices: a device wants to join.
    JoinRequested {
        device: DeviceId,
        label: String,
        code: String,
    },
    /// Joining device: our code, to display.
    JoinCode(String),
    /// Joining device: an admin approved; ask the user to confirm.
    JoinApproved {
        practice_name: String,
    },
    Enrolled {
        practice_name: String,
    },
    DrillReport(crate::core::drill::DrillReport),
    /// The practice has fewer than two admins (FR-ADM-004).
    FewAdmins(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCmd {
    StartCue {
        mode: crate::proto::CueMode,
        hold_ms: u32,
    },
    StopCue,
    StartAlarm {
        pattern: crate::proto::SoundPattern,
    },
    StopAlarm,
    Chime,
}

/// Audit log entries (FR-LOG-001). Never contain secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEvent {
    AlarmRaised {
        alarm_id: MsgId,
        source: Source,
        drill: bool,
    },
    AlarmReceived {
        alarm_id: MsgId,
        origin: DeviceId,
        path: PathKind,
    },
    AlarmEvent {
        alarm_id: MsgId,
        by: DeviceId,
        event: crate::proto::EventKind,
    },
    NearMiss {
        source: Source,
        duration_ms: u64,
    },
    Enrolled {
        practice_name: String,
    },
    CertAdded {
        device: DeviceId,
        role: crate::proto::Role,
    },
    /// An admin set this device's label or room (ADR 0014).
    DeviceSettingsAssigned {
        by: DeviceId,
    },
    Revoked {
        device: DeviceId,
        cascade: bool,
        by: DeviceId,
    },
    EpochChanged {
        epoch: u64,
    },
    ConfigChanged {
        version: u64,
        by: DeviceId,
    },
    DrillCompleted {
        alarm_id: MsgId,
        received: usize,
        expected: usize,
    },
    VolumeOverride {
        on: bool,
    },
    ClockSkew {
        device: DeviceId,
        ahead_ms: u64,
    },
    Dropped {
        reason: DropReason,
    },
}

/// Diagnostics counters for dropped input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DropReason {
    Decode,
    UnsupportedVersion,
    UnknownTag,
    DecryptFailed,
    BadSignature,
    UnknownType,
    Revoked,
    UnknownCert,
    NotAuthorised,
    RateLimited,
    WrongPractice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    SendDatagram {
        dest: UdpDest,
        bytes: Vec<u8>,
    },
    SendStream {
        to: SocketAddrV4,
        bytes: Vec<u8>,
    },
    /// `None` = every connected relay.
    SendRelay {
        relay: Option<RelayId>,
        bytes: Vec<u8>,
    },
    Persist(PersistOp),
    Ui(UiEvent),
    Audio(AudioCmd),
    SetSystemVolumeOverride(bool),
    Log(AuditEvent),
}
