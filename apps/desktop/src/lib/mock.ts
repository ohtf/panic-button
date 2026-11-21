// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

// A stand-in agent for running the views in a plain browser: `?mock=` picks
// a scenario (idle, alert, sender, firstrun, amber, red). Never used inside
// the app itself.

import type { Alarm, PanicButtonEvent, Peer, Status } from "./types";

type Listener = (p: unknown) => void;

const scenario = new URLSearchParams(location.search).get("mock") ?? "idle";
const listeners = new Map<string, Set<Listener>>();
const now = Date.now();

function emit(name: string, payload: unknown) {
  for (const l of listeners.get(name) ?? []) l(payload);
}
const event = (e: PanicButtonEvent) => emit("panic-button", e);
const join = {
  nearby: [4],
  enrolled: false,
  practice: null as string | null,
  joining: false,
  code: "095245",
  approved: null as string | null,
};

const status: Status = {
  device: "a".repeat(64),
  device_short: "aaaaaaaa",
  admin: true,
  enrolled: scenario !== "firstrun",
  practice: "Riverside Surgery",
  level: scenario === "red" ? "red" : scenario === "amber" ? "amber" : "green",
  summary:
    scenario === "red"
      ? "Not connected to any devices"
      : scenario === "amber"
        ? "Connected to 3 devices (1 direct, 2 via relay)"
        : "Connected to 14 devices (12 direct, 2 via relay)",
  reasons:
    scenario === "red"
      ? ["No other devices reachable"]
      : scenario === "amber"
        ? ["Only reachable via relay for some devices"]
        : [],
  direct: 12,
  via_relay: 2,
};

function alarm(p: Partial<Alarm>): Alarm {
  return {
    alarm_id: "0123456789abcdef0123456789abcdef",
    origin: "b".repeat(64),
    own: false,
    drill: false,
    room: "Consulting Room 4",
    user: "Dr A Example",
    device_label: "ROOM4-PC",
    raised_at: now - 20_000,
    state: "open",
    resolved_by: null,
    closed_at: null,
    delivered: 13,
    coming: [],
    seen: [],
    local_ack: null,
    ...p,
  };
}

const alarms: Alarm[] = [];
if (scenario === "alert") {
  alarms.push(alarm({ coming: ["Jane Smith (Reception)"] }));
  alarms.push(
    alarm({
      alarm_id: "fedcba9876543210fedcba9876543210",
      room: "Treatment Room 1",
      user: "Nurse B Example",
      device_label: "TREAT1",
      raised_at: now - 4 * 60_000,
      local_ack: "responding",
      coming: ["You", "Dr Ali (Room 2)"],
    }),
  );
}
if (scenario === "sender") {
  alarms.push(
    alarm({
      own: true,
      room: "Reception",
      user: "Jane Smith",
      device_label: "RECEPTION-1",
      raised_at: now - 6000,
      delivered: 12,
      coming: ["Dr Ali (Room 2)", "Nurse B Example (Treatment Room 1)"],
    }),
  );
}

const peers: Peer[] = [
  ["Reception", "Jane Smith", "lan"],
  ["Room 2", "Dr Ali", "lan"],
  ["Treatment Room 1", "Nurse B Example", "relay"],
  ["Admin office", "Practice Manager", "lan"],
].map(([room, user, path], i) => ({
  device: String(i).repeat(64),
  device_short: String(i).repeat(8),
  member: true,
  label: `PC-${i + 1}`,
  room,
  user,
  path: path as Peer["path"],
  last_heard_ms: 1200 + i * 700,
  app_version: "0.1.0",
  audio_ok: true,
}));

let hold: ReturnType<typeof setInterval> | undefined;
let holdStart = 0;
let practice = false;

function endHold(triggered: boolean) {
  clearInterval(hold);
  hold = undefined;
  // As the core does: a full ring on trigger, then the end, then the
  // practice result.
  if (triggered) event({ type: "hold_progress", fraction: 1 });
  event({ type: "hold_ended" });
  if (triggered && practice) event({ type: "practice_complete" });
  if (triggered && !practice) {
    const own = alarm({ own: true, room: "Reception", user: "Jane Smith", raised_at: Date.now(), delivered: 0 });
    alarms.unshift(own);
    event({ type: "sender_panel", alarm: own, phase: "requested" });
    setTimeout(() => {
      own.delivered = 13;
      event({ type: "sender_panel", alarm: { ...own }, phase: "delivered" });
    }, 600);
    setTimeout(() => {
      own.coming = ["Dr Ali (Room 2)"];
      event({ type: "alarm_updated", alarm: { ...own } });
    }, 2500);
  }
}

export const mock = {
  listen(name: string, cb: Listener): () => void {
    if (!listeners.has(name)) listeners.set(name, new Set());
    listeners.get(name)!.add(cb);
    return () => listeners.get(name)?.delete(cb);
  },
  async invoke(cmd: string, args: Record<string, unknown>): Promise<unknown> {
    switch (cmd) {
      case "app_info":
        return {
          enrolled: scenario !== "firstrun",
          running: scenario !== "firstrun",
          version: "0.1.0",
          data_dir: "~/.local/share/panic-button",
          allow_quit: true,
          audio_ok: true,
          wayland: scenario === "amber",
          token_configured: false,
          home_dir: "/home/user",
          sounds_dir: "~/.local/share/panic-button/sounds",
          shortcut:
            scenario === "amber"
              ? {
                  keys: "Ctrl+Shift+F12",
                  active: false,
                  problem:
                    "Keyboard shortcuts for other apps do not work on this desktop (Wayland). Use the on-screen button.",
                }
              : { keys: "Ctrl+Shift+F12", active: true, problem: null },
        };
      case "new_recovery_kit":
        await new Promise((r) => setTimeout(r, 1500));
        return { practice: "Riverside Surgery", recovery_kit: args.newPath };
      case "setup_practice":
        await new Promise((r) => setTimeout(r, 1500));
        status.enrolled = true;
        return {
          practice: args.name,
          practice_id: "00112233445566778899aabbccddeeff",
          root_pub: "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
          recovery_kit: args.kitPath,
        };
      case "join_with_token":
        status.enrolled = true;
        return null;
      case "start_join":
        join.joining = true;
        // An admin approves a few seconds later.
        setTimeout(() => {
          join.approved = "Riverside Surgery";
          event({ type: "join_approved", practice: "Riverside Surgery" });
        }, 4000);
        return { ...join };
      case "create_token":
        return { token_id: "1234", expires_at: now + 86_400_000, text: "PANIC_BUTTON1-EXAMPLE-TOKEN-TEXT" };
      case "agent_call":
        return mock.call(args.method as string, (args.params ?? {}) as Record<string, unknown>);
      default:
        return null;
    }
  },
  call(method: string, p: Record<string, unknown>): unknown {
    const find = () => alarms.find((a) => a.alarm_id === p.alarm_id);
    switch (method) {
      case "status":
        return { ...status };
      case "alarms":
        return alarms.map((a) => ({ ...a }));
      case "peers":
        return peers;
      case "devices":
        return [
          {
            device: status.device,
            device_short: status.device_short,
            this_device: true,
            label: "RECEPTION-1",
            room: "Reception",
            pending_label: null,
            pending_room: null,
            user: "Jane Smith",
            role: "admin",
            enrolled_at: now - 9e8,
            token_id: null,
            path: null,
            last_heard_ms: null,
            status: "green",
          },
          ...peers.map((p, i) => ({
            device: p.device,
            device_short: p.device_short,
            this_device: false,
            label: p.label,
            room: p.room,
            pending_label: null,
            pending_room: null,
            user: p.user,
            role: "member",
            enrolled_at: now - 8e8,
            token_id: i < 2 ? "7".repeat(32) : null,
            path: p.path,
            last_heard_ms: p.last_heard_ms,
            status: "green",
          })),
          {
            device: "9".repeat(64),
            device_short: "99999999",
            this_device: false,
            label: "OLD-LAPTOP",
            room: null,
            pending_label: null,
            pending_room: "Consulting Room 2",
            user: null,
            role: "member",
            enrolled_at: now - 7e9,
            token_id: null,
            path: null,
            last_heard_ms: null,
            status: null,
          },
        ];
      case "audit.export":
        return { csv: "time_utc,kind,detail\n", entries: 0 };
      case "practice.settings":
        return {
          version: 3,
          hold_ms: 1500,
          cue_mode: "soft",
          sound_pattern: "A",
          override_mute: null,
          drill_mode: "silent",
          drill_schedule: "Tue 07:45",
          drill_coordinator: "a".repeat(64),
          opening_hours: "Mon-Fri 08:00-18:30",
          relay_urls: ["wss://relay.example-pcn.org.uk/v1/ws"],
          seed_peers: null,
        };
      case "tokens":
        return [
          {
            token_id: "7".repeat(32),
            label: "Reception PCs",
            issued_at: now - 9e8,
            expires_at: now + 9e8,
            expired: false,
            revoked: false,
            enrolled: [
              { device: "0".repeat(64), label: "PC-1", enrolled_at: now - 8e8 },
              { device: "1".repeat(64), label: "PC-2", enrolled_at: now - 8e8 },
            ],
          },
          {
            token_id: "8".repeat(32),
            label: "Locum laptop",
            issued_at: now - 9e9,
            expires_at: now - 1e9,
            expired: true,
            revoked: false,
            enrolled: [],
          },
        ];
      case "drill.reports":
        return [
          { alarm_id: "5".repeat(32), started_at: now - 86_400_000, received: 4, expected: 5, skipped: false },
          { alarm_id: "6".repeat(32), started_at: now - 8 * 86_400_000, received: 0, expected: 0, skipped: true },
        ];
      case "drill.report":
        return {
          alarm_id: "5".repeat(32),
          started_at: now - 86_400_000,
          received: 4,
          expected: 5,
          csv: "device,label,room,received\n",
        };
      case "config":
        return {
          label: "RECEPTION-1",
          room: "Reception",
          hold_ms: 1500,
          cue_mode: "soft",
          sound_pattern: "A",
          volume_pct: 80,
          relay_urls: [],
          label_locked: false,
          room_locked: false,
        };
      case "hold.start":
        if (hold) return { ok: true };
        practice = Boolean(p.practice);
        holdStart = Date.now();
        hold = setInterval(() => {
          const f = Math.min(1, (Date.now() - holdStart) / 1500);
          event({ type: "hold_progress", fraction: f });
          if (f >= 1) endHold(true);
        }, 33);
        return { ok: true };
      case "hold.end":
        if (hold) endHold(false);
        return { ok: true };
      case "respond":
      case "seen": {
        const a = find();
        if (a) {
          a.local_ack = method === "respond" ? "responding" : "seen";
          if (method === "respond") a.coming = ["You", ...a.coming];
          event({ type: "alarm_updated", alarm: { ...a } });
        }
        return { ok: true };
      }
      case "resolve":
      case "cancel": {
        const a = find();
        if (a) {
          a.state = method === "resolve" ? "resolved" : "cancelled";
          a.resolved_by = method === "resolve" ? "Jane Smith" : null;
          a.closed_at = Date.now();
          event({ type: "alarm_closed", alarm: { ...a } });
        }
        return { ok: true };
      }
      case "join.status":
        return { ...join };
      case "join.confirm":
        Object.assign(join, { enrolled: true, practice: join.approved, joining: false, code: null, approved: null });
        status.enrolled = true;
        return { ...join };
      case "join.cancel":
        Object.assign(join, { joining: false, approved: null });
        return { ok: true };
      case "admin.join_requests":
        return [];
      default:
        return { ok: true };
    }
  },
};
