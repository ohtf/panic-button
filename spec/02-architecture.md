# 02 Architecture

## System view

```
            Practice LAN (one or more VLANs, wired + Wi-Fi)
  +-----------+     multicast/broadcast/unicast UDP, TCP     +-----------+
  | Desktop   |<-------------------------------------------->| Desktop   |
  | agent A   |<-----------+                    +----------->| agent B   |
  +-----+-----+            |                    |            +-----+-----+
        |                  v                    v                  |
        |            +-----------+        +-----------+            |
        |            | Desktop   |        | Headless  |            |
        |            | agent C   |        | agent     | (optional, |
        |            +-----------+        +-----------+  Linux box)|
        |                                                          |
        |  wss:// or HTTPS long-poll (optional, outbound 443)      |
        +----------------------->+-----------+<--------------------+
                                 |  Relay    |  practice box, PCN, or OHTF hosted
                                 | (blind)   |  forwards sealed envelopes only
                                 +-----------+
```

Every device runs the same core. The relay is a separate, much simpler program that never holds practice keys.

## Repository layout

```
panic-button/
  Cargo.toml                 workspace
  crates/
    panic-button/            the one published crate (ADR 0016); modules in src/:
      proto/                 message types, CBOR encode/decode, versioning (no I/O)
      crypto/                keys, certs, sealing, signatures, recovery kit (no I/O)
      core/                  sans-IO state machine: peers, alarms, hold logic, status, sync
      store/                 SQLite persistence (rusqlite) for identity, certs, alarms, audit
      net/                   tokio I/O drivers: UDP, TCP, mDNS, relay client, interface watch
      agent/                 runtime glue: wires core + net + store + platform; control API
      relay/                 relay server (`relay` feature, on by default)
      cli/                   the `panic-button` admin and diagnostics CLI
      main.rs, bin/          programs: panic-button, panic-button-agent, panic-button-relay
    panic-button-sim/        deterministic network simulator and property tests (unpublished)
  apps/
    desktop/                 Tauri 2 app (Rust side embeds the agent module; Svelte 5 frontend)
  packaging/
    windows/                 WiX fragments (firewall, properties), signing scripts
    macos/                   pkg scripts, entitlements, LaunchAgent plist, config profile example
    linux/                   .deb control, systemd user unit, AppImage recipe
    relay/                   Dockerfile, docker-compose example, systemd unit, Caddyfile example
  sounds/                    generator script + generated CC0 WAVs
  tests/
    netns/                   Linux network-namespace integration tests
  fuzz/                      cargo-fuzz targets
  spec/                      this spec and ADRs (spec/adr/)
  safety/                    hazard log and clinical safety records (entry point: SAFETY.md)
  docs/                      user, admin, IT and relay operator guides; testing records
  .github/workflows/         CI
```

## Sans-IO core

The `core` module contains all protocol and alarm logic and performs **no I/O, reads no clocks and uses no global randomness**. This is what makes the simulator deterministic and the properties testable.

```rust
pub struct Core { /* state */ }

impl Core {
    pub fn new(identity: Identity, state: PersistedState, config: EffectiveConfig, rng_seed: [u8; 32]) -> Self;
    /// Feed one input; returns the effects the runtime must perform, in order.
    pub fn handle(&mut self, now: Instant, wall: SystemTimeMs, input: Input) -> Vec<Output>;
    /// Earliest time at which `handle(.., Input::Tick)` must be called.
    pub fn next_deadline(&self) -> Option<Instant>;
}

pub enum Input {
    Tick,
    Datagram { transport: Transport, from: SocketAddr, bytes: Bytes },
    StreamFrame { conn: ConnId, peer_hint: Option<DeviceId>, bytes: Bytes },
    RelayFrame { relay: RelayId, bytes: Bytes },
    RelayState { relay: RelayId, state: RelayLinkState, presence: Option<Vec<DeviceId>> },
    InterfacesChanged(Vec<InterfaceInfo>),
    AudioStatus(AudioStatus),
    SessionState(SessionState),          // active, locked, no_user, remote_session
    User(UserAction),                    // HoldStart{source}, HoldEnd{source}, Respond, Seen, Resolve, Cancel, admin actions...
    ConfigChanged(EffectiveConfig),
}

pub enum Output {
    SendDatagram { transport: Transport, to: SocketAddr, bytes: Bytes },
    SendStream { to: PeerAddr, bytes: Bytes },
    SendRelay { relay: Option<RelayId>, bytes: Bytes },   // None = all connected relays
    Persist(PersistOp),
    Ui(UiEvent),                          // AlarmShown, AlarmUpdated, AlarmClosed, StatusChanged, HoldProgress, NearMiss...
    Audio(AudioCmd),                      // StartCue, StopCue, StartAlarm{pattern}, StopAlarm, Chime
    SetSystemVolumeOverride(bool),
    Log(AuditEvent),
}
```

Rules:
- Time comes in as parameters. Randomness (message ids, nonces, jitter) comes from a `ChaCha20Rng` seeded at construction. The runtime seeds it from `OsRng`. The simulator seeds it deterministically.
- Nonces for encryption come from the same RNG. In production it is seeded from `OsRng` and reseeded every 2^20 draws. Uniqueness of 24-byte random nonces is fine at this rate.
- Crypto operations are called from the core through the `crypto` module, which is pure.
- The core enforces all limits (sizes, rates, timers) so the simulator tests them.

## Runtime (`panic-button-agent`)

- Tokio multi-threaded runtime with a single core task that owns `Core` and receives inputs over an mpsc channel. Outputs are dispatched to I/O tasks.
- I/O tasks: UDP socket (one socket bound to `0.0.0.0:port`, joined to the multicast group on every IPv4 interface), TCP listener plus connection pool, mDNS (`mdns-sd`), relay clients (one per configured URL), interface watcher (poll every 5 s plus OS notifications where available), audio status poller (every 10 s), session state watcher.
- Store: the `store` module, with SQLite in WAL mode. Persist ops are applied in order and fsynced for security-relevant state (identity, certs, revocations, epochs, processed alarm ids) before the corresponding outputs are released. Alarm sending is **not** delayed by persistence: the alarm is sent first, then persisted.
- Local control API: JSON-RPC 2.0 over a Unix domain socket (`$XDG_RUNTIME_DIR/panic-button/control.sock`, mode 0600) or a Windows named pipe (`\\.\pipe\panic-button-<SID>`, ACL restricted to the current user and SYSTEM). The CLI and integration tests use it. Methods: `status`, `peers`, `alarms`, `raise` (test builds and headless only, unless policy allows), `respond`, `resolve`, `cancel`, `drill.run`, `drill.report`, admin methods, `diagnostics.export`.

## Desktop process model

- One Tauri 2 process per user session, embedding `panic-button-agent` as a library. Windows:
  - `button`: small, transparent, frameless, always-on-top, skip-taskbar, not focusable where possible.
  - `panel`: sender confirmation panel, anchored to the button.
  - `alert`: receiver window, topmost.
  - `main`: settings, status and admin. Opened from the tray.
- Tray icon with status colour and menu.
- The frontend is a thin view. All decisions (hold timing, state, status) happen in Rust and reach the frontend as events. The frontend sends `HoldStart`/`HoldEnd` and user actions as Tauri commands.
- Single instance per session (`tauri-plugin-single-instance`). Session participation rules are in `05-desktop-agent.md`.

## Identity scope

- **Windows:** one device identity per machine, stored in `%ProgramData%\Panic Button\panic-button.db` and protected with DPAPI machine scope (ADR 0010, ADR 0017). The heartbeat reports the current interactive user's display name.
- **macOS and Linux (v1):** one identity per user account, in the user's keychain or secret service where available, or otherwise in a 0600 file in the user data directory.

## Data stored per device (SQLite)

| Table | Contents | Retention |
|---|---|---|
| identity | signing and key-exchange keys (encrypted at rest per OS), own cert | permanent |
| practice | practice id, root pubkey, name, current and previous epoch keys | permanent |
| certs | known certs (raw signed bytes) | permanent; pruned 30 days after revocation |
| revocations | raw signed revocations | permanent |
| peers | device id, label, room, last addrs, last seen, path flags, version | 7 days after last seen |
| envelopes | raw envelopes for open alarms and recent events (for sync) | 24 h after alarm close |
| alarms | derived alarm records for history | 400 days |
| processed_ids | message ids of alarm-related messages | 24 h (alarm-related ids: 2 x ALARM_MAX_LIFE minimum) |
| audit | audit events | per FR-LOG-002 |
| drills | drill reports | 400 days |
| config | signed practice config, local prefs | permanent |

## Module dependency direction

`proto` <- `crypto` <- `core` <- (`store`, `net`) <- `agent` <- (`apps/desktop`, `cli`). `relay` uses `proto` and `crypto` only (it validates certs and revocations but never holds group keys or alarm state). `proto`, `crypto` and `core` do no I/O. These were separate crates until ADR 0016; `crates/panic-button/tests/boundaries.rs` now enforces them. The simulator (`panic-button-sim`) and the desktop app are separate, unpublished crates that use the library without the `relay` feature.

## Suggested crates (check licences with cargo-deny)

`tokio`, `socket2`, `bytes`, `ciborium` or `minicbor` (choose one, ADR 0006), `ed25519-dalek`, `x25519-dalek`, `chacha20poly1305`, `blake3`, `hkdf`, `sha2`, `argon2`, `rand_chacha`, `rand_core`, `zeroize`, `rusqlite` (bundled), `mdns-sd`, `if-addrs`, `tokio-tungstenite` + `rustls`, `reqwest` (rustls, for long-poll), `axum` (relay), `tracing`, `tracing-appender`, `thiserror`, `anyhow`, `clap`, `serde`, `toml`, `proptest`, `rodio` or `cpal` (audio), `windows` (Win32 APIs), `tauri` 2 + plugins (`single-instance`, `global-shortcut`, `autostart`).
