// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

// Shapes returned by the agent's control API and UI events
// (crates/panic-button/src/agent/views.rs).

export type Level = "green" | "amber" | "red";

export interface Status {
  device: string;
  device_short: string;
  admin: boolean;
  enrolled: boolean;
  practice: string | null;
  level: Level | null;
  summary: string | null;
  reasons: string[];
  direct: number | null;
  via_relay: number | null;
}

export interface Alarm {
  alarm_id: string;
  origin: string;
  own: boolean;
  drill: boolean;
  room: string;
  user: string;
  device_label: string;
  raised_at: number;
  state: "open" | "resolved" | "cancelled" | "expired";
  resolved_by: string | null;
  closed_at: number | null;
  delivered: number;
  coming: string[];
  seen: string[];
  local_ack: "seen" | "responding" | null;
}

export interface Peer {
  device: string;
  device_short: string;
  member: boolean;
  label: string | null;
  room: string | null;
  user: string | null;
  path: "lan" | "unicast" | "relay" | "indirect" | "stale";
  last_heard_ms: number | null;
  app_version: string | null;
  audio_ok: boolean | null;
}

export interface Device {
  device: string;
  device_short: string;
  this_device: boolean;
  label: string;
  room: string | null;
  /** Assigned by this admin, not yet confirmed by the device. */
  pending_label: string | null;
  pending_room: string | null;
  user: string | null;
  role: "member" | "admin" | "enroller" | "unknown";
  enrolled_at: number;
  token_id: string | null;
  path: Peer["path"] | null;
  last_heard_ms: number | null;
  status: Level | null;
}

export interface Token {
  token_id: string;
  label: string;
  issued_at: number;
  expires_at: number;
  expired: boolean;
  revoked: boolean;
  enrolled: { device: string; label: string; enrolled_at: number }[];
}

export interface DrillSummary {
  alarm_id: string;
  started_at: number;
  received: number;
  expected: number;
  skipped?: boolean;
}

export interface PracticeSettings {
  version: number;
  hold_ms: number | null;
  cue_mode: "silent" | "soft" | "alarm_ramp" | null;
  sound_pattern: "A" | "B" | "C" | null;
  override_mute: boolean | null;
  drill_mode: "silent" | "audible" | null;
  drill_schedule: string | null;
  drill_coordinator: string | null;
  opening_hours: string | null;
  relay_urls: string[] | null;
  seed_peers: string[] | null;
}

export interface AgentConfig {
  label: string;
  room: string;
  hold_ms: number;
  cue_mode: string;
  sound_pattern: string;
  volume_pct: number;
  relay_urls: string[];
  label_locked: boolean;
  room_locked: boolean;
}

export interface AppInfo {
  enrolled: boolean;
  running: boolean;
  version: string;
  data_dir: string;
  allow_quit: boolean;
  audio_ok: boolean;
  wayland: boolean;
  token_configured: boolean;
  home_dir: string;
  sounds_dir: string;
  shortcut: ShortcutInfo | null;
}

/** Joining by approval (`join.status`). */
export interface JoinView {
  /** Not in a practice yet: computers heard on this network, per practice. */
  nearby: number[];
  enrolled: boolean;
  practice: string | null;
  joining: boolean;
  code: string | null;
  approved: string | null;
}

export interface ShortcutInfo {
  keys: string;
  active: boolean;
  problem: string | null;
}

export type SenderPhase = "requested" | "delivered" | "not_yet_delivered" | "could_not_reach";

export type PanicButtonEvent =
  | { type: "alarm_shown"; alarm: Alarm; resumed: boolean }
  | { type: "alarm_updated"; alarm: Alarm }
  | { type: "alarm_closed"; alarm: Alarm }
  | { type: "alarm_history"; alarm: Alarm }
  | { type: "sender_panel"; alarm: Alarm; phase: SenderPhase }
  | { type: "drill_toast"; alarm_id: string; audible: boolean }
  | {
      type: "status";
      level: Level;
      summary: string;
      reasons: string[];
      direct: number;
      via_relay: number;
    }
  | { type: "hold_progress"; fraction: number }
  | { type: "hold_ended" }
  | { type: "near_miss"; duration_ms: number }
  | { type: "practice_complete" }
  | { type: "join_requested"; device: string; label: string; code: string }
  | { type: "join_code"; code: string }
  | { type: "join_approved"; practice: string }
  | { type: "enrolled"; practice: string }
  | { type: "drill_report"; alarm_id: string; rows: number }
  | { type: "few_admins"; admins: number };
