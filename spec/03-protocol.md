# 03 Protocol (version 1)

This document is normative for interoperability. Anything not specified here is an implementation detail.

## Conventions

- Encoding: deterministic CBOR (RFC 8949 section 4.2.1 core deterministic encoding). Maps use **unsigned integer keys**. Decoders **must ignore unknown keys** and **must reject** duplicate keys, indefinite-length items and items exceeding the size limits.
- Byte strings are fixed length where stated. A wrong length is a decode error.
- Times are `u64` milliseconds since the Unix epoch (sender wall clock) unless stated.
- `DeviceId` = 32-byte Ed25519 public key. Short display form: the first 8 characters of its lowercase base32 encoding.
- All signatures are Ed25519 over `context || data`, where `context` is an ASCII string from the table below followed by a single 0x00 byte.

| Context | Used for |
|---|---|
| `panic-button/v1/sealed` | body of sealed messages |
| `panic-button/v1/open` | body of open messages |
| `panic-button/v1/cert` | certificates |
| `panic-button/v1/relay-auth` | relay handshake |

## Size limits

| Item | Limit |
|---|---|
| UDP datagram payload | 1200 bytes (never fragment). Messages that do not fit are sent by TCP or relay only |
| TCP frame | 256 KiB |
| Relay frame | 64 KiB (WebSocket) / 256 KiB (long-poll batch) |
| Strings (labels, rooms, names) | 64 UTF-8 bytes; truncate on send, reject over 256 on receive |

## Envelope

The outermost structure on every transport.

```
Envelope = {
  0: version      uint   (= 1)
  1: kind         uint   (0 = sealed, 1 = open)
  2: tag          bstr   (sealed: 8-byte practice tag; open: 16-byte practice id)
  3: epoch        uint   (sealed only)
  4: nonce        bstr   (sealed only, 24 bytes)
  5: payload      bstr   (sealed: ciphertext; open: SignedMessage CBOR bytes)
}
```

### Sealed envelopes

- `practice_tag(epoch) = BLAKE3::keyed_hash(group_key[epoch], b"panic-button/v1/tag")[0..8]`.
- `ciphertext = XChaCha20-Poly1305(key = group_key[epoch], nonce, aad = CBOR({0: version, 1: kind, 2: tag, 3: epoch}), plaintext = SignedMessage bytes)`.
- Receivers keep a map from tag to epoch for the current and previous epoch. An unknown tag means "not our practice": drop the envelope silently and cheaply. Do not attempt decryption.

### Open envelopes

Used only where the group key is unavailable or must not be relied upon: join, join approval, key share, revocation and relay auth. They are signed but not group-encrypted. Any confidential parts are sealed to a specific device (see KeyShare).

### SignedMessage

```
SignedMessage = {
  0: body   bstr   (CBOR bytes of Body)
  1: sig    bstr   (64 bytes; Ed25519 by body.sender over context || body)
}

Body = {
  0: msg_id      bstr  (16 random bytes)
  1: type        uint
  2: created_at  uint  (ms)
  3: sender      bstr  (DeviceId)
  4: payload     map   (type-specific)
}
```

## Message types

| Code | Name | Envelope | Flood once? | Typical transport |
|---|---|---|---|---|
| 0x01 | Heartbeat | sealed | no | multicast, broadcast, unicast UDP; relay |
| 0x02 | AlarmRaised | sealed | yes | all |
| 0x03 | AlarmEvent | sealed | yes | all |
| 0x04 | SyncRequest | sealed | no | TCP, relay |
| 0x05 | SyncResponse | sealed | no | TCP, relay |
| 0x06 | CertAnnounce | sealed | yes | UDP if it fits, otherwise TCP/relay |
| 0x07 | PracticeConfig | sealed | yes | TCP, relay, UDP if it fits |
| 0x08 | PresenceSummary | sealed | no | UDP, relay |
| 0x09 | DeviceSettings | sealed | yes | all; also sent directly to the target |
| 0x40 | JoinRequest | open | no | multicast, broadcast |
| 0x41 | JoinApproval | open | no | UDP unicast, TCP |
| 0x42 | KeyShare | open | yes | all |
| 0x43 | Revocation | open | yes | all |
| 0x44 | RelayAuth | open | n/a | relay only |

### 0x01 Heartbeat

Sent every `HEARTBEAT_INTERVAL` on multicast and broadcast, by unicast UDP to peers flagged `unicast_only`, and to every connected relay.

```
{
  0: label          tstr
  1: room           tstr
  2: user_display   tstr    (empty if no interactive user)
  3: app_version    tstr    (semver)
  4: proto_version  uint    (1)
  5: platform       uint    (1 windows, 2 macos, 3 linux, 4 android, 5 ios, 6 headless)
  6: tcp_port       uint
  7: addrs          [bstr]  (sender's IPv4 interface addresses, 4 bytes each, max 8)
  8: capabilities   uint    (bitset: 1 can_alert, 2 can_raise, 4 admin, 8 drill_coordinator, 16 relay_connected)
  9: audio          map     ({0: output_present bool, 1: muted bool, 2: volume_pct uint})
  10: session       uint    (0 active, 1 locked, 2 no_user, 3 remote_session_disabled, 4 inactive: another user's session has the screen)
  11: status        uint    (0 green, 1 amber, 2 red)
  12: cert_serial   bstr    (16 bytes)
  13: epoch         uint
  14: alarm_digest  bstr    (16 bytes, see Anti-entropy)
  15: state_digest  bstr    (16 bytes, see Anti-entropy)
  16: near_miss_24h uint
  17: uptime_s      uint
}
```

### 0x02 AlarmRaised

```
{
  0: alarm_id      bstr  (= body.msg_id)
  1: kind          uint  (1 = help; receivers treat unknown kinds as help)
  2: drill         bool
  3: room          tstr
  4: user_display  tstr
  5: device_label  tstr
  6: source        uint  (0 button, 1 hotkey, 2 hid, 3 mobile, 4 cli, 5 integration)
}
```
`raised_at` is `body.created_at`. The origin device is `body.sender`.

### 0x03 AlarmEvent

```
{
  0: alarm_id      bstr
  1: event         uint  (0 Receipt, 1 Seen, 2 Responding, 3 Resolved, 4 Cancelled)
  2: user_display  tstr  (who: the signed-in person; with no name, the sender's device label, else its room)
  3: detail        map   (optional)
}
```
- Receipt detail: `{0: audio map (as heartbeat), 1: path uint (0 multicast, 1 broadcast, 2 udp_unicast, 3 tcp, 4 relay, 5 sync), 2: displayed bool, 3: drill_rtt_ref bstr (msg_id of the AlarmRaised, for RTT matching)}`.
- Cancelled detail: `{0: reason uint (0 sent_in_error, 1 other)}`. **Only the origin device may cancel.** Receivers ignore a Cancelled event from any other sender.
- Resolved may come from any enrolled device.

### 0x04 SyncRequest / 0x05 SyncResponse

```
SyncRequest  = { 0: want uint bitset (1 open_alarms, 2 recent_alarm_events, 4 certs, 8 revocations, 16 config, 32 keyshares_for_me), 1: since uint (ms, optional), 2: ids [bstr] (optional specific msg_ids) }
SyncResponse = { 0: envelopes [bstr] (original raw Envelope bytes), 1: more bool }
```
Responses carry **original signed envelopes**, so authenticity is preserved end to end. Receivers process each contained envelope exactly as if it had arrived directly, with path `sync`.

### 0x06 CertAnnounce

`{ 0: certs [bstr] }`: raw signed certificates. Sent by a device when it first joins, and on request.

### 0x07 PracticeConfig

`{ 0: version uint (monotonic), 1: settings map }`, signed by an admin device. Last writer wins by `version`, with ties broken by the higher `msg_id`. The settings keys are listed in `08-deployment.md`.

### 0x08 PresenceSummary

Every 60 s: `{ 0: peers [ {0: device_id, 1: last_seen_age_s uint, 2: path uint} ] }`, listing direct peers only (max 20 entries per message so it fits in one datagram; split if needed). Not flooded. Used to build a 2-hop view for the coverage screen.

### 0x09 DeviceSettings

`{ 0: target DeviceId, 1: label tstr (optional), 2: room tstr (optional), 3: assigned_at uint }`, signed by an admin device. Sets the target device's label or room; a missing field leaves it unchanged. Only the target applies it, and only if it is newer than the device's current own label and room; machine configuration and policy still win. The admin re-sends it directly when the target's heartbeat does not yet show it, and its device list shows the change as pending ("Changing to ...") until then. The first admin's computer starts with the name given at practice setup as its own label, as if assigned. See ADR 0014.

### 0x40 JoinRequest (interactive enrolment)

Self-signed by the joining device:
`{ 0: kx_pub bstr(32), 1: label tstr, 2: practice_hint bstr(16) optional, 3: code tstr (6 digits) }`.
`code = decimal(BLAKE3(sign_pub || kx_pub)[0..4] as u32 mod 1_000_000)`, zero-padded. Re-sent every 5 s while the join screen is open, for at most 10 minutes. The envelope tag carries the practice id being joined, or 16 zero bytes for "any". Admin devices list requests that match their practice id, or the zero id.

### 0x41 JoinApproval

Signed by an admin device. Addressed to a joining device:
`{ 0: recipient DeviceId, 1: certs [bstr] (new member cert + issuer chain), 2: root_pub bstr(32), 3: practice_name tstr, 4: keyshare (as KeyShare payload), 5: config bstr (latest PracticeConfig envelope) }`.

### 0x42 KeyShare

Signed by an admin device (on rotation) or any member (when bringing a newly enrolled device up to date):
`{ 0: recipient DeviceId, 1: epoch uint, 2: eph_pub bstr(32), 3: nonce bstr(24), 4: ct bstr }`
where `k = HKDF-SHA256(ikm = X25519(eph_priv, recipient_kx_pub), salt = practice_id, info = "panic-button/v1/keyshare" || recipient || epoch_be64)` and `ct = XChaCha20-Poly1305(k, nonce, aad = recipient || epoch_be64, plaintext = group_key)`.
A member may share epoch E only if it holds E and the recipient's cert is valid and unrevoked.

### 0x43 Revocation

Signed by an admin device:
`{ 0: target_serial bstr(16), 1: target_device DeviceId, 2: cascade bool, 3: reason tstr, 4: new_epoch uint (the epoch the admin is about to distribute) }`.

### 0x44 RelayAuth

See `06-relay.md`.

## Certificates

```
Cert = { 0: body bstr (CBOR CertBody), 1: sig bstr(64) }
CertBody = {
  0: version     uint (1)
  1: practice_id bstr(16)
  2: serial      bstr(16)
  3: subject     DeviceId (signing pub)
  4: subject_kx  bstr(32) (X25519 pub)
  5: role        uint (0 member, 1 admin, 2 enroller)
  6: label       tstr
  7: issued_at   uint
  8: expires_at  uint (0 = none; required for enroller)
  9: issuer      bstr(32) (root pub, or the issuing device's signing pub)
  10: token_id   bstr(16) (optional; set when issued by an enroller)
}
```
Validation rules are in `04-security.md`.

## Delivery algorithm

### Transports and addressing

- UDP socket bound to `0.0.0.0:PORT` (default 47474) with `SO_REUSEADDR` (and `SO_REUSEPORT` on Unix). It joins `MCAST_GROUP` (default `239.255.71.71`) on every up, non-loopback IPv4 interface. Multicast TTL is 1 by default (`multicast_ttl` configurable to 8 for routed multicast). Loopback is enabled, and own messages are discarded by sender id. If the port is busy at start (on Linux it is inside the range used for outgoing connections), the bind is retried every second for up to 30 s.
- Broadcast: to `255.255.255.255:PORT` and to each interface's directed broadcast address.
- TCP listener on `0.0.0.0:PORT`, falling back to an ephemeral port if busy. The actual port is advertised in the heartbeat. Frames are `u32` big-endian length plus an Envelope. Connections are opened on demand with a 1 s connect timeout, kept for 60 s idle, and limited to 64 concurrent outbound.
- mDNS: service `_panic-button._udp.local.`, TXT `t=<hex practice tag>`, `p=<tcp port>`, `v=1`. Used to find peers when multicast to the group address is filtered but mDNS is permitted.
- Seed peers: configured `host:port` list. They are always sent heartbeats by unicast UDP and are used for TCP.

### Per-peer path tracking

For each peer, record the transports by which its messages arrived in the last `PEER_STALE`: `{multicast, broadcast, udp_unicast, tcp, relay, indirect}`. A peer is **`lan_group`** if it was heard via multicast or broadcast, **`unicast_only`** if it was heard only by unicast UDP or TCP, **`relay_only`** if it was heard only via the relay, and **indirect** if it is known only from a PresenceSummary.

### Raising an alarm (originator)

At trigger time T0:
1. Build AlarmRaised, seal it in the current epoch, and persist it asynchronously.
2. Send immediately, in parallel:
   - multicast (1 packet) and broadcast (1 packet per interface);
   - UDP unicast to every peer that is not `lan_group`;
   - TCP to **every** known peer that is not stale (reliability over lossy Wi-Fi);
   - every connected relay.
3. Retransmit by UDP (multicast + broadcast + unicast to peers with no Receipt yet) at T0 + 250 ms, 500 ms, 1 s and 2 s, then every 2 s until T0 + 30 s, then every 10 s while the alarm is open.
4. Resend via TCP at T0 + 2 s and T0 + 10 s to peers with no Receipt.

### Receiving an alarm or alarm event

Pipeline for every received envelope:
1. Decode the envelope. Drop it on error (counted in diagnostics).
2. Sealed: look up the tag (unknown: drop), decrypt (failure: drop), decode SignedMessage, verify the signature against `body.sender`, check the sender has a valid, unrevoked cert (unknown cert: hold the message in a 30 s pending queue and send a SyncRequest for certs to the source; if the cert never arrives, drop).
3. Dedupe by `msg_id` (in-memory LRU of 50,000 entries plus the persisted `processed_ids` for alarm types). A duplicate updates path tracking only.
4. Apply it to the core state (see Alarm state) and emit UI and audio outputs.
5. For AlarmRaised (not drill, not expired, not own): send a Receipt immediately.
6. **Re-flood once** (for types marked "flood once"), after a random jitter of 0 to `REFLOOD_JITTER`:
   - multicast + broadcast once, **unless** during the jitter the same `msg_id` was heard again via multicast or broadcast from another peer (someone else covered this segment);
   - UDP unicast and TCP to peers that are `unicast_only` and are not known to have this `msg_id` (they sent it to us, or sent a Receipt for it);
   - all connected relays, unless the message arrived via a relay.
7. Never re-flood a message more than once per device.

### Alarm state (order-independent)

An alarm's state is a function of the set of valid messages about it:
- `raised`: the AlarmRaised message (required. Events that arrive before it are held, and a SyncRequest for the id is sent).
- `receipts`, `seen`, `responding`: sets keyed by sender device (latest per device and type).
- `closed`: the earliest by `(created_at, msg_id)` among the valid Resolved events and the Cancelled events from the origin device. Once closed, the alarm stays closed.
- `expired`: true if not closed and `now_wall - raised_at > ALARM_MAX_LIFE`.

Local alerting state:
- **Alerting** = not own, not drill, not closed, not expired at first receipt, and the local user has not sent Seen or Responding.
- An alarm whose first receipt happens after expiry goes straight to history as "Expired - not resolved" without alerting (FR-ALM-008).
- Clock skew: if `raised_at` is more than 5 minutes in the future, compute age as 0 and flag the sender clock as skewed in diagnostics. **Never drop an alarm because of clock skew.**

### Drills

An AlarmRaised with `drill = true`, sent by the coordinator. Receivers send a Receipt with `detail.3 = alarm_id`, show the drill toast, and do not alert. The originator measures RTT locally (from send to Receipt arrival), so drill timing does not depend on clocks being in sync. The coordinator sends Resolved at T0 + `DRILL_AUTO_CLOSE`.

## Anti-entropy

- `alarm_digest = BLAKE3("panic-button/v1/ad" || sorted concat of (alarm_id || latest_event_msg_id) for alarms open or closed within the last ALARM_MAX_LIFE)[0..16]`.
- `state_digest = BLAKE3("panic-button/v1/sd" || sorted cert serials || sorted revocation msg_ids || config version || current epoch)[0..16]`.
- When a peer's heartbeat digest differs from ours, send that peer a SyncRequest by TCP (or via relay if it is `relay_only`), rate-limited to one per peer per `SYNC_RATE_LIMIT`. Pick the `want` bits by which digest differed.
- After wake from sleep, a network change or a relay reconnect: an immediate heartbeat, then a SyncRequest (open alarms, recent events, revocations, keyshares_for_me) to every connected relay and to up to 3 random direct peers.

## Status computation

Evaluated every second and on every relevant input:

- **Red** if any of: not enrolled; own cert revoked; no current group key; zero peers reachable by any path in the last `PEER_STALE` and no relay connected; session is a disabled remote session; core error.
- **Amber** if not red and any of: audio output missing or muted (and `override_mute` is unavailable or off); relay configured but none connected; all reachable peers are `relay_only`; holding a stale epoch (rotation is known but we lack the key); fewer peers than usual (FR-STS-006); the only reachable path is indirect.
- **Green** otherwise.

A single-device practice is amber with "No other devices found". This is intended.

## Timers and constants

| Name | Value |
|---|---|
| `PORT` | 47474 (UDP and TCP; unassigned in the IANA registry, checked 2026-11-16) |
| `MCAST_GROUP` | 239.255.71.71 |
| `HEARTBEAT_INTERVAL` | 15 s plus or minus 2 s random jitter |
| `PEER_STALE` | 45 s |
| `PEER_FORGET` | 7 days |
| `PRESENCE_INTERVAL` | 60 s |
| `ALARM_MAX_LIFE` | 60 min |
| `REFLOOD_JITTER` | 150 ms |
| `SYNC_RATE_LIMIT` | 30 s per peer |
| `PENDING_CERT_WAIT` | 30 s |
| `EPOCH_GRACE` | 7 days |
| `DRILL_AUTO_CLOSE` | 60 s |
| `RELAY_PING` | 20 s |
| `RELAY_BACKOFF` | 1 s doubling to 30 s max, with full jitter |
| `HOLD_MS` default | 1500 (range 1000-3000) |
| `NEAR_MISS_MIN_MS` | 300 |

## Versioning

- `Envelope.version` changes only for incompatible envelope changes. Receivers drop versions they do not support, and diagnostics count them so the admin view can say "devices on an incompatible version".
- New message types and new payload keys are added without changing the version. Unknown types are dropped. Unknown keys are ignored.
- The heartbeat `app_version` lets the admin view flag mixed versions.

## Clarifications (added during implementation)

These fill gaps the original text left open. They are normative for version 1.

- **Decoding strictness.** Receivers reject CBOR tags, non-integer map keys and nesting deeper than 16 levels, in addition to the rules in Conventions. They accept map keys in any order (senders always write ascending order), so a non-canonical but otherwise valid encoding from another implementation is not dropped. Signatures are always verified over the bytes as received.
- **Embedded certificates.** Wherever a structure carries certificates (`CertAnnounce`, `JoinApproval.certs`, `RelayAuth.chain`, tokens), each is a byte string containing the CBOR encoding of a `Cert`, so the signed bytes are preserved exactly.
- **PracticeConfig settings keys.** `0 hold_ms uint`, `1 cue_mode uint (0 silent, 1 soft, 2 alarm_ramp)`, `2 sound_pattern uint (0 A, 1 B, 2 C)`, `3 override_mute bool`, `4 drill_schedule tstr`, `5 drill_mode uint (0 silent, 1 audible)`, `6 drill_coordinator DeviceId`, `7 opening_hours tstr`, `8 relay_urls [tstr]`, `9 seed_peers [tstr]`. Every key is optional; an absent key means the practice has not set it.
- **Token network overrides** (token key 10): `{0: port uint, 1: multicast_group bstr(4), 2: multicast_ttl uint}`, every key optional.
- **Relay frame types** (first byte of each WebSocket message): `0x00` envelope, `0x01` presence, `0x02` challenge (server to client), `0x03` auth result (server to client). The client's RelayAuth is sent as an envelope frame.
- **Unknown enumerated values** (platform, session, source, path, alarm kind, event kind) decode successfully and are preserved. Unknown alarm kinds are treated as help. Unknown event kinds are ignored by the alarm state. Unknown certificate roles carry no authority.
- **SyncRequest key 3 `to` (DeviceId, optional).** The device that should answer. A request sent via a relay reaches every device of the practice; only the named device answers. Requests without `to` are answered by every receiver.
- **SyncRequest key 4 `alarm_digest` (bstr(16), optional).** The requester's alarm digest. A responder whose own digest differs also sends its own SyncRequest back (subject to rate limits), so one heartbeat heard in either direction is enough to start repair both ways.
- **JoinRequest key 4 `tcp_port` (uint, optional).** The joining device's TCP port, so a JoinApproval that does not fit a datagram can be delivered by TCP.
- **Token key 13 `issuer_chain` ([bstr], optional).** Raw certs from the enroller cert's issuer up to but not including the root, so a newly enrolled device can validate its own chain before it hears from an admin (otherwise it would show red, and its alarms would be held by peers that lack the admin cert, until an admin is reachable).
- **Alarm digest** (replaces the formula in Anti-entropy): `BLAKE3("panic-button/v1/ad" || sorted concat of (alarm_id || msg_id) for the AlarmRaised and every AlarmEvent of each alarm raised, or closed, within the last ALARM_MAX_LIFE)[0..16]` (ADR 0013, item 9).
- **Implementation decisions** affecting delivery, status and traffic are recorded in ADR 0013.
- **Golden vectors** for every message type are in `crates/panic-button/tests/vectors/`.
