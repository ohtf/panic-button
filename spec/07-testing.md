# 07 Testing and proving it works

Testing is part of the product. The claims "it works" and "it tells you when it doesn't" must both be backed by evidence that anyone can reproduce.

## Layers

1. Unit tests in every crate.
2. Deterministic simulation (`panic-button-sim`) with property-based tests. **The main line of defence.**
3. Fuzzing of every decoder.
4. Real-network integration tests in Linux network namespaces (CI).
5. Cross-platform build and smoke tests (CI matrix).
6. Hardware lab protocol (manual, recorded, before each release).
7. In-production evidence: status lights, drills and coverage reports.

## 1. Unit tests (examples, not exhaustive)

[rule-coverage.md](rule-coverage.md) lists every rule in `03-protocol.md` and `04-security.md` with the tests that check it, and the rules nothing checks yet.

- `proto` module: round-trip every message type; deterministic encoding; unknown keys ignored; duplicate keys rejected; size limits enforced; golden test vectors (committed hex files) so other implementations can check compatibility.
- `crypto` module: seal and open; tag derivation; KeyShare; cert chain validation (each rule in `04-security.md`, positive and negative); non-cascade vs cascade revocation; recovery kit encrypt and decrypt; zeroize on drop (via test hooks).
- `core` module: hold machine (every transition, including near miss boundaries at 299, 300, 1499 and 1500 ms); alarm state merging (permutations of event order give identical state); expiry; Cancelled only from the origin; status computation table tests (each red and amber reason).

## 2. Simulator (`panic-button-sim`)

### Model

- `World` holds nodes (each a real `Core` with its own simulated clock offset and seeded RNG), a discrete event queue ordered by virtual time, and a network model.
- Network model:
  - **Segments** (L2 broadcast domains). Each has flags: `multicast_ok`, `broadcast_ok`, `client_isolation` (nodes on the segment cannot reach each other at all, but can reach routed destinations), loss probability, latency distribution (min, median, p99), duplication probability and reordering.
  - **Routing matrix**: which segments can reach which by unicast (UDP and TCP separately, to model firewalls).
  - Nodes may be on several segments (dual-homed).
  - **Relays**: per-node reachability to each relay over time; relay up and down schedule.
  - **Events over time**: partitions, healing, node sleep and wake, node crash and restart (with persisted state retained), interface changes, clock jumps.
- Platform effects: audio status per node (present, muted, missing), session state.
- UI model: a simulated user per node with configurable behaviour (acknowledges after X s, never acknowledges, away).

### Properties (proptest; 256 cases in `s/check-deep`, 1,000 in `s/check-deep --long`)

Random worlds: 2 to 60 nodes, 1 to 4 segments, random flags, loss 0 to 30%, 0 to 2 relays, random event schedules. Shrinking is enabled. Every failing seed is saved as a regression test.

| ID | Property |
|---|---|
| P1 Delivery | If node R has a working path (any combination of transports, including multi-hop via re-flooding nodes and relays) from origin O for the whole interval [T0, T0 + B], R shows the alarm by T0 + B. B = 2 s for paths without loss, 5 s with loss up to 30%, 8 s via relay. |
| P2 Authenticity | No node ever shows an alarm or applies an event unless it was signed by a device with a valid, unrevoked cert in that node's view. Includes injected forged, replayed and cross-practice traffic. |
| P3 No duplicates | No node alerts twice for the same alarm id, including across crash and restart. |
| P4 Convergence | After the last event and a quiet period of 30 s with a connected network, all connected nodes have identical derived state for every alarm. |
| P5 No stale alerts | No node alerts for an alarm whose `raised_at` is more than ALARM_MAX_LIFE before its first receipt at that node (with true clocks; skewed-clock cases are checked separately in P9). |
| P6 Revocation | After a revocation and rotation have reached node N, N rejects all messages from the revoked device, and the revoked device cannot decrypt any message sealed in the new epoch. |
| P7 Bounded traffic | Total packets per alarm are no more than 10 x N plus retransmissions, for a fully connected LAN of N up to 250. Heartbeat traffic stays within NFR-RES-001. |
| P8 Status honesty | If node N shows green at time t, then an alarm raised by N at t would be delivered to at least one other node (checked by forking the world at t and raising one). If N cannot reach anyone, N shows red within PEER_STALE + 1 s. |
| P9 Clock skew | With per-node clock offsets up to plus or minus 10 minutes, P1 to P4 still hold. No alarm is dropped because of skew. |
| P10 Sender honesty | The sender panel never shows "Delivered to N" with N greater than the number of distinct devices that actually sent a Receipt. |
| P11 Cancel authority | Only the origin's Cancelled closes an alarm as cancelled. A Cancelled from any other device is ignored. |
| P12 Drill isolation | Drill alarms never cause a full alert on any node. |

#### Implementation notes

- The properties live in `crates/panic-button-sim/tests/properties.rs` and the scenarios in `crates/panic-button-sim/tests/scenarios.rs`. `cargo test` runs 8 cases per property, on every push in CI. `s/check-deep` runs 256 in release mode and `s/check-deep --long` runs 1,000, locally; the Deep checks and Long runs workflows do the same by hand. Failing seeds are saved in `crates/panic-button-sim/tests/properties.proptest-regressions` and replayed first on every run.
- P1 and P8 are judged against a ground-truth reachability model (`crates/panic-button-sim/src/props.rs`) built from the simulated network. Devices learn addresses from heartbeats, so random worlds change topology only before 22 s and raise the first alarm at 90 s, leaving time for status (`PEER_STALE`) and address re-learning to settle. P8 is checked only once the topology has been stable for `PEER_STALE + 1 s`.
- P3 excludes alerts resumed after a restart (ADR 0013). P7 counts first transmissions only; retransmissions are allowed on top.
- To investigate, `cargo run --release -p panic-button-sim --example explore -- world 0 200 30` sweeps seeds and prints the first failing case; `CASE='<Params from the regressions file>' ... -- p4 0 0` re-runs one with diagnostics.

### Named scenarios (deterministic, always run)

| ID | Scenario | Expected |
|---|---|---|
| S1 | Flat LAN, 30 nodes | P1 at 2 s. All green |
| S2 | Multicast blocked, broadcast allowed | Delivery via broadcast |
| S3 | Multicast and broadcast blocked; unicast allowed; seed peer configured | Delivery via unicast and TCP after the first heartbeat exchange |
| S4 | Two VLANs, no routing, one dual-homed node | Delivery across via re-flood |
| S5 | Two VLANs, no routing, relay reachable by both | Delivery via relay. Nodes amber ("Only reachable via relay") where applicable |
| S6 | Wi-Fi segment with client isolation plus wired segment, relay present | Laptops receive via relay |
| S7 | Wi-Fi client isolation, no relay | Laptops show **red** ("No other devices reachable"). P8 holds |
| S8 | Laptop asleep for the whole alarm; wakes after it is resolved | History only, no alert |
| S9 | Laptop wakes during an open alarm raised 10 min ago | Alerts, showing "raised 10 minutes ago" |
| S10 | Two practices on one segment | No cross-talk; tags differ |
| S11 | Attacker replays a captured AlarmRaised 5 min and 2 h later; crash and restart in between | Never re-alerts |
| S12 | Relay goes down mid-alarm; LAN fine | No impact on LAN delivery. Relay-only nodes get events after reconnect via resume |
| S13 | 250 nodes flat LAN, raise, all respond | P7 holds; latency targets met |
| S14 | Admin revokes a laptop during an open alarm | Alarm continues for everyone else. Laptop's subsequent events rejected |
| S15 | Token enrolment with stale epoch | New device obtains the current key via KeyShare within one heartbeat round |
| S16 | Concurrent rotations by two admins | Converges to one epoch key. No message loss during conflict |
| S17 | Receiver muted, override off | Receipt reports muted. Receiver amber. Sender still gets the receipt |

## 3. Fuzzing

`cargo-fuzz` targets: `envelope_decode`, `signed_message_decode`, `cert_decode_and_validate`, `token_decode`, `relay_frame_decode`, `core_handle_datagram` (random bytes into a configured core). `s/check-deep` runs each for 60 s, and `--long` for 10 minutes. Crashes become regression tests.

## 4. Network-namespace integration (Linux CI)

`tests/netns/` contains scripts (run with `sudo` in GitHub Actions) that build topologies with `ip netns`, bridges and veth pairs. They use `nft` to drop multicast or broadcast per bridge, emulate client isolation (drop traffic between ports on a bridge), and use `tc netem` for loss and latency. They run real `panic-button-agent --headless` binaries plus a real `panic-button-relay`. Tests drive agents through the control API (`panic-button ctl raise`, `panic-button ctl wait-alarm --timeout 3s`, `panic-button ctl status --json`). Implement at least S1, S2, S4, S5, S6, S7, S12 and S14 here.

## 5. CI matrix

Decided 2026-11-15 (Marcus Baw): GitHub Actions minutes are limited, and until the basic install-and-use test has passed, the slow and adversarial checks run locally. Every push runs only the quick Linux checks (fmt, clippy, unit tests with 8 property cases, cargo-deny, cargo-audit, REUSE, zizmor); changes to docs and spec alone run nothing. `s/check-deep` runs the rest locally before core changes are pushed. The matrix below is the target for release candidates (M8), run by hand from the Deep checks, Long runs and Installers workflows.

- Linux (x64): fmt, clippy, unit tests, sim properties, fuzz smoke, netns tests, cargo-deny, frontend checks, relay Docker build.
- Windows (windows-latest): build, unit tests, desktop build, MSI build. A silent install and uninstall smoke test (`msiexec /i ... /qn ENROLMENT_TOKEN_FILE=...`) that checks the agent can be launched headlessly against the installed config, the firewall rule exists and the control API responds.
- macOS (macos-latest): build, unit tests, desktop build (unsigned in PRs, signed and notarised on release tags).
- Long runs: 1,000-case property runs, 10 minutes of fuzzing per target, S13 at full scale.

## 6. Hardware lab protocol (before each release)

### Equipment

- Managed switch with VLAN support and IGMP snooping (able to block multicast and broadcast per VLAN).
- Wi-Fi access point with a client isolation toggle and an SSID mapped to a separate VLAN.
- At least 3 Windows 11 machines (one domain-joined if possible, one laptop on Wi-Fi), 1 Windows 10 22H2, 1 Mac, 1 Linux desktop.
- 1 Linux box running the relay and a headless agent.
- One machine with no speakers and one with audio muted.

### Test cases (record each result in `docs/testing/results/YYYY-MM-DD-vX.Y.Z.md`)

| ID | Setup | Pass criteria |
|---|---|---|
| L1 | All wired, one VLAN | Alarm shown everywhere within 2 s. All green |
| L2 | Multicast blocked on the switch | As L1 |
| L3 | Two VLANs without routing, relay reachable from both | Shown everywhere within 5 s. Amber where relay-only |
| L4 | Laptop on isolated Wi-Fi, no relay | Laptop red, with the correct reason. Sender panel on the laptop says it could not deliver |
| L5 | L4 plus relay | Laptop receives within 5 s and can raise alarms that reach wired PCs |
| L6 | Muted machine and no-speaker machine | Both amber with the correct reason. Override unmutes the muted Windows machine |
| L7 | Laptop asleep through alarm, then woken | No stale alert. History shows it |
| L8 | Pull the relay's network cable mid-alarm | Wired LAN unaffected. Relay-only clients catch up on reconnect |
| L9 | Silent MSI install with token on a fresh Windows 11 | Enrolled and green with no user interaction except the room prompt (or none if ROOM set) |
| L10 | Accidental press test: 10 people, 5 quick taps and 5 leans on the shortcut each | 0 alarms raised. Near misses logged |
| L11 | Deliberate press under distraction: 10 people raise an alarm while talking | 10 alarms raised. Median time from intent to trigger recorded |
| L12 | Weekly drill scheduled | Runs on time. Report matches reality |
| L13 | Revoke a laptop | It turns red within 1 minute. It cannot raise alarms others see |
| L14 | Fire alarm comparison: play patterns A to C next to the building fire alarm test | Nobody in the room confuses them (record the panel's view) |

## 7. In-production evidence

The status light, coverage view and weekly drill reports are the continuing proof. The pilot practice exports drill reports weekly to the project (with consent, identifiers removed) during the v1 pilot.
