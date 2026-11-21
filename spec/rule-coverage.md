# Rule coverage: protocol and security rules to tests

Every rule in [03-protocol.md](03-protocol.md) and [04-security.md](04-security.md), with the tests that check it (R9). Checked against the code on 2026-11-21.

- **Covered**: a test fails if the rule breaks.
- **Indirect**: only a broader test (a simulator property or scenario) would notice.
- **Not tested**: built, but nothing checks it.
- **Not built**: the spec asks for it and the code does not do it.

Test names are function names; search for them. Paths:

| Prefix | Location |
|---|---|
| `proto/`, `crypto/`, `core/`, `net/`, `store/`, `agent/`, `relay/` | `crates/panic-button/src/` |
| `tests/` | `crates/panic-button/tests/` |
| `sim:` | `crates/panic-button-sim/tests/scenarios.rs` |
| `prop:` | `crates/panic-button-sim/tests/properties.rs` (P1 to P12, `spec/07-testing.md`) |
| `desktop/` | `apps/desktop/src-tauri/src/` |
| `fuzz:` | `fuzz/fuzz_targets/` |
| `gui:` | `tests/gui/runthrough.py` (`s/gui-test`) |
| `msi:` | `tests/windows/msi-smoke.ps1` (Windows runner) |

Tests marked "Windows runner" run in the Installers workflow, not in `s/test`.

## 03 Protocol

### Conventions and size limits

| ID | Rule | Tests | Status |
|---|---|---|---|
| PR-01 | Deterministic CBOR: shortest-form integers, map keys ascending | `proto/cbor.rs` `shortest_form_integers`, `map_builder_sorts_keys`; `tests/messages.rs` `every_message_type_round_trips_and_matches_golden` | Covered |
| PR-02 | Unknown map keys are ignored | `proto/cbor.rs` `skips_unknown_keys`; `proto/envelope.rs` `ignores_unknown_keys`; `tests/messages.rs` `unknown_payload_keys_are_ignored` | Covered |
| PR-03 | Duplicate keys are rejected | `proto/cbor.rs` `rejects_duplicate_keys` | Covered |
| PR-04 | Indefinite-length items are rejected | `proto/cbor.rs` `rejects_indefinite_items` | Covered |
| PR-05 | Items over the size limits are rejected | `proto/cbor.rs` `rejects_huge_claimed_lengths`; `tests/messages.rs` `oversized_strings_rejected_on_receive` | Covered |
| PR-06 | A fixed-length byte string of the wrong length is a decode error | `proto/envelope.rs` `rejects_wrong_tag_length`; every decoder under `fuzz:` | Covered for the envelope tag; other fields only by fuzzing |
| PR-07 | DeviceId short form: first 8 characters of lowercase base32 | `proto/ids.rs` `short_form_is_8_lowercase_base32_chars` | Covered |
| PR-08 | Signatures are over `context`, a 0x00 byte, then the data | `proto/envelope.rs` `signing_input_has_separator`; `crypto/keys.rs` `sign_and_verify_with_context` | Covered |
| PR-09 | UDP payload at most 1200 bytes; larger messages only by TCP or relay | `tests/messages.rs` `heartbeat_fits_in_one_datagram_with_long_strings`; `core/time.rs` `constants_match_the_spec_tables` | Indirect: nothing checks that a large message is kept off UDP |
| PR-10 | TCP frame at most 256 KiB | `net/tcp.rs` `oversized_frames_close_the_connection` | Covered |
| PR-11 | Relay frame at most 64 KiB (WebSocket), 256 KiB (long-poll batch) | `proto/relay.rs` `oversized_frames_and_batches_are_refused` | Covered |
| PR-12 | Strings: truncated to 64 bytes on send, rejected over 256 on receive | `proto/cbor.rs` `text_is_truncated_on_char_boundary`; `tests/messages.rs` `oversized_strings_rejected_on_receive` | Covered |

### Envelope and SignedMessage

| ID | Rule | Tests | Status |
|---|---|---|---|
| PR-13 | Envelope fields and encoding | `proto/envelope.rs` `round_trips`, `known_encoding`, `rejects_trailing_bytes` | Covered |
| PR-14 | Practice tag is keyed BLAKE3 of the group key, 8 bytes | `crypto/keys.rs` `tag_differs_by_key`; golden vectors | Covered |
| PR-15 | XChaCha20-Poly1305 with the envelope header as associated data | `crypto/seal.rs` `seal_open_round_trip`, `wrong_key_or_tampering_fails` | Covered |
| PR-16 | Unknown tag: drop without trying to decrypt | `sim:` `s10_two_practices_on_one_segment`; `prop:` `p2_p11_attacks` (cross-practice traffic) | Covered |
| PR-17 | Open envelopes only for join, approval, key share, revocation, relay auth | `crypto/seal.rs` `envelope_kind_must_match_type` | Covered |
| PR-18 | SignedMessage signed by `body.sender` | `crypto/seal.rs` `forged_sender_is_rejected`; `prop:` `p2_p11_attacks` | Covered |

### Message types

| ID | Rule | Tests | Status |
|---|---|---|---|
| PR-19 | Every message type encodes as specified | `tests/messages.rs` `every_message_type_round_trips_and_matches_golden`, `alarm_raised_round_trips`, `cert_round_trip` | Covered |
| PR-20 | Unknown message types are dropped | `tests/messages.rs` `unknown_message_type_is_rejected` | Covered |
| PR-21 | Heartbeats go by multicast, broadcast, unicast to `unicast_only` peers, and every relay | `sim:` `s1_flat_lan_30_nodes`, `s2_multicast_blocked_broadcast_allowed`, `s3_unicast_only_with_seed_peer`, `s6_wifi_isolation_with_relay` (each needs heartbeats by that path to turn green) | Covered |
| PR-22 | Unknown alarm kinds are treated as help | `tests/messages.rs` `unknown_alarm_kind_survives_as_other` | Decoding covered; nothing checks that the core alerts for one |
| PR-23 | Only the origin may cancel; others' Cancelled is ignored | `core/alarm.rs` `p11_only_origin_cancels`; `prop:` `p2_p11_attacks`, P11 in every random world | Covered |
| PR-24 | Resolved may come from any enrolled device | `sim:` `s8_asleep_through_alarm_sees_history_only` (node 1 resolves node 0's alarm) | Covered |
| PR-25 | AlarmEvent names the sender: the person, else the computer's label, else its room | `sim:` `a_responder_without_a_user_name_is_named_by_its_computer` | Covered |
| PR-26 | SyncResponse carries original envelopes, processed as if direct, path `sync` | `sim:` `s8_asleep_through_alarm_sees_history_only`, `s9_wakes_during_open_alarm_raised_10_minutes_ago`; P4 | Covered |
| PR-27 | CertAnnounce when a device joins | `sim:` `s15_token_enrolment_with_stale_epoch`, `join_by_approval_on_the_lan` | Covered |
| PR-28 | PracticeConfig: last writer wins by version, ties by higher msg_id; admins only | `sim:` `concurrent_practice_settings_converge_on_one` | Covered, except that a member's PracticeConfig is ignored on receipt (not tested) |
| PR-29 | PresenceSummary: direct peers, at most 20 per message, split if needed | `tests/messages.rs` `presence_summary_holds_at_most_20_peers`; `sim:` `presence_summaries_are_split_at_20_peers` | Covered |
| PR-30 | DeviceSettings: only the target applies it, newest wins, machine config wins, re-sent until shown; the setup name is the first admin's label | `sim:` `admin_assigns_label_and_room`; `agent/setup.rs` `the_setup_name_becomes_this_computers_name`; `gui:` (room change, "Changing to") | Covered |
| PR-31 | JoinRequest: six-digit code from the keys; re-sent every 5 s for at most 10 minutes; practice id or zero id | `crypto/keys.rs` `join_code_is_six_digits`; `sim:` `join_by_approval_on_the_lan`; `core/time.rs` `constants_match_the_spec_tables` | Covered, except the 10-minute stop (not tested) |
| PR-32 | JoinApproval carries cert chain, root key, name, key share and config | `sim:` `join_by_approval_on_the_lan`; `gui:` (C joins by approval) | Covered |
| PR-33 | KeyShare key derivation, bound to recipient and epoch | `crypto/keyshare.rs` `round_trip_and_binding`, `rejects_low_order_point` | Covered |
| PR-34 | A member shares an epoch only with a valid, unrevoked recipient | `sim:` `s15_token_enrolment_with_stale_epoch`; `prop:` `p6_revocation` | Covered |
| PR-35 | Revocation is signed by an admin | `sim:` `promote_member_to_admin` (a new admin's revocation is honoured); `relay/state/tests.rs` `a_member_cannot_revoke` | Relay covered; a member's revocation being ignored by the core is not tested |

### Delivery

| ID | Rule | Tests | Status |
|---|---|---|---|
| PR-36 | UDP socket on the port, joins the multicast group on every interface, TTL 1, bind retried for 30 s | `tests/netns/run.sh` (real sockets, S1, S2) | Socket and group covered by netns; bind retry not tested |
| PR-37 | Broadcast to 255.255.255.255 and each directed broadcast address | `sim:` `s2_multicast_blocked_broadcast_allowed`; netns S2 | Covered |
| PR-38 | TCP: length-prefixed frames, ephemeral port if busy, advertised in the heartbeat, 64 outbound connections | `net/tcp.rs` `frames_round_trip_over_loopback`; netns | Framing covered; port fallback and connection limit not tested |
| PR-39 | mDNS discovery | none (netns runs with mDNS off) | Not tested |
| PR-40 | Seed peers get unicast heartbeats and TCP | `sim:` `s3_unicast_only_with_seed_peer`; `gui:` (three copies on loopback by seed peers) | Covered |
| PR-41 | Path classes: `lan_group`, `unicast_only`, `relay_only`, indirect | `core/peers.rs` `classes`; `net/udp.rs` `classification` | Covered |
| PR-42 | Raising sends by every transport at once | `sim:` `s1` to `s6`; P1 | Covered |
| PR-43 | UDP retransmit schedule | `core/time.rs` `retransmit_schedule_matches_spec` | Covered |
| PR-44 | TCP resend at 2 s and 10 s to peers without a Receipt | P1 in random worlds with loss | Indirect |
| PR-45 | Undecodable envelopes are dropped and counted | `tests/messages.rs` `decoders_never_panic`, `mutated_messages_never_panic`; `prop:` `p2_p11_attacks` (garbage, bit flips); `fuzz:` `core_handle_datagram` | Dropping covered; counting covered only for other versions (PR-66) |
| PR-46 | Unknown sender cert: hold the message up to 30 s and ask for certs | `sim:` `s15_token_enrolment_with_stale_epoch` | Indirect |
| PR-47 | Dedupe by msg_id, kept across restarts for alarm types | `sim:` `s11_replays_never_re_alert`; P3 (across crash and restart) | Covered |
| PR-48 | A Receipt for each AlarmRaised that is not a drill, expired or our own | `core/alarm.rs` `p10_delivered_counts_distinct_receipt_senders_not_origin`; P10 | Covered |
| PR-49 | Re-flood once, skipping segments someone else covered | `sim:` `s4_two_vlans_bridged_by_dual_homed_node`; `prop:` `p7_bounded_traffic` | Covered |
| PR-50 | Never re-flood a message twice | `prop:` `p7_bounded_traffic` | Covered |

### Alarm state, drills, anti-entropy, status

| ID | Rule | Tests | Status |
|---|---|---|---|
| PR-51 | Alarm state is a function of the set of messages | `core/alarm.rs` `derivation_is_order_independent`; P4 | Covered |
| PR-52 | Events before their AlarmRaised are held, and the alarm is requested | P4 in random worlds with loss and reordering | Indirect |
| PR-53 | Closed is the earliest valid Resolved or origin's Cancelled, and stays closed | `core/alarm.rs` `earliest_closure_wins` | Covered |
| PR-54 | Expired after `ALARM_MAX_LIFE` if not closed | `core/alarm.rs` `fr_alm_008_expiry_and_skew` | Covered |
| PR-55 | Alerting rules; first seen after expiry goes to history without alerting | `sim:` `s8_asleep_through_alarm_sees_history_only`, `s9_wakes_during_open_alarm_raised_10_minutes_ago`; P5 | Covered |
| PR-56 | Clock skew: age 0 if more than 5 minutes ahead; never dropped | `core/alarm.rs` `fr_alm_008_expiry_and_skew`; `prop:` `p9_clock_skew_up_to_10_minutes` | Covered |
| PR-57 | Drills: Receipt with the alarm id, toast, no alert, RTT measured locally, resolved at 60 s | P12; `agent/schedule.rs` `coordinator_runs_and_admins_record_skips`; `gui:` (report "2 of 2") | No alert and the report covered; the 60 s auto-resolve is indirect |
| PR-58 | Alarm digest and state digest | `core/digest.rs` `alarm_digest_is_order_independent` | Alarm digest covered; state digest not tested on its own |
| PR-59 | A differing digest starts a rate-limited sync | `sim:` `s8`, `s9`; P4 | Covered |
| PR-60 | After wake, network change or relay reconnect: heartbeat, then sync | `sim:` `s9_wakes_during_open_alarm_raised_10_minutes_ago`, `s12_relay_down_mid_alarm`, `inactive_session_takes_no_part_until_active_again` | Covered |
| PR-61 | Red reasons | `core/status.rs` `fr_sts_003_red_reasons`; `sim:` `s7_wifi_isolation_without_relay_is_red`; P8 | Covered. "Core error" has no status reason |
| PR-62 | Amber reasons | `core/status.rs` `amber_reasons`, `mute_override_clears_amber`; `sim:` `s5_two_vlans_via_relay`, `s17_muted_receiver_without_override` | Covered, except two reasons that are not built: "fewer devices than usual" (FR-STS-006) and "the only reachable path is indirect" |
| PR-63 | Green otherwise | `core/status.rs` `green_when_all_good` | Covered |
| PR-64 | A single-device practice is amber, "No other devices found" | `core/status.rs` `amber_reasons` | Covered |
| PR-65 | Timers and constants as in the table | `core/time.rs` `constants_match_the_spec_tables`, `retransmit_schedule_matches_spec` | Covered, except `RELAY_PING` and `RELAY_BACKOFF` (relay rules, `06-relay.md`) |

### Versioning and clarifications

| ID | Rule | Tests | Status |
|---|---|---|---|
| PR-66 | Other envelope versions are dropped and counted | `proto/envelope.rs` `rejects_other_versions`; `sim:` `other_envelope_versions_are_dropped_and_counted` | Covered |
| PR-67 | New types and keys need no version change: unknown types dropped, keys ignored | as PR-02 and PR-20 | Covered |
| PR-68 | Reject CBOR tags, non-integer keys, nesting deeper than 16; accept keys in any order | `proto/cbor.rs` `rejects_tags`, `rejects_non_integer_keys`, `rejects_deep_nesting`, `skips_unknown_keys` (keys 0, 99, 1) | Covered |
| PR-69 | Embedded certificates are byte strings of the Cert's CBOR | `tests/messages.rs` `cert_round_trip`; `tests/certs.rs` `token_enrolment_end_to_end` | Covered |
| PR-70 | PracticeConfig settings keys | golden vectors; `agent/views.rs` `settings_merge_and_validate` | Covered |
| PR-71 | Token network overrides (key 10) | `proto/token.rs` `string_round_trip` (with overrides) | Covered |
| PR-72 | Relay frame types | `proto/relay.rs` `frames_round_trip` | Covered |
| PR-73 | Unknown enumerated values decode and are kept; unknown roles carry no authority | `tests/messages.rs` `unknown_alarm_kind_survives_as_other`; `tests/certs.rs` `unknown_roles_carry_no_authority` | Covered for alarm kinds and roles |
| PR-74 | SyncRequest `to`: only the named device answers | `sim:` `s5_two_vlans_via_relay`, `s6_wifi_isolation_with_relay` | Indirect |
| PR-75 | SyncRequest `alarm_digest`: the responder repairs the other way too | P4 | Indirect |
| PR-76 | JoinRequest `tcp_port`, for an approval too big for a datagram | `sim:` `join_by_approval_on_the_lan` | Indirect |
| PR-77 | Token `issuer_chain` | `tests/certs.rs` `token_enrolment_end_to_end`; `sim:` `s15_token_enrolment_with_stale_epoch` | Covered |
| PR-78 | Alarm digest covers the AlarmRaised and every AlarmEvent | `core/digest.rs` `alarm_digest_is_order_independent` | Covered |

## 04 Security

### Threat controls

| ID | Rule | Tests | Status |
|---|---|---|---|
| SE-01 | Outsiders cannot read traffic (group encryption) | `crypto/seal.rs` `seal_open_round_trip`, `wrong_key_or_tampering_fails` | Covered |
| SE-02 | Outsiders cannot forge: signatures and cert validation | `prop:` `p2_p11_attacks` (forgeries by a non-member holding the group key); P2 | Covered |
| SE-03 | Replays never re-alert | `sim:` `s11_replays_never_re_alert`; P3 | Covered |
| SE-04 | Neighbouring practices are kept apart: practice id, root key, tag | `sim:` `s10_two_practices_on_one_segment`; `tests/certs.rs` `rule2_practice_id_must_match` | Covered |
| SE-05 | Insiders: every action signed and audited on every device; only the originator cancels | `store/tests.rs` `batches_survive_reopen` (audit kept); `agent/views.rs` `audit_csv_quotes_and_dates`; PR-23 | Covered, except that each event type is audited (not tested) |
| SE-06 | Stolen laptop: revoke, which rotates the key; the relay drops it | `sim:` `s14_revoke_laptop_during_open_alarm`; `prop:` `p6_revocation`; `relay/state/tests.rs` `admin_revocation_closes_and_refuses_the_device` | Covered |
| SE-07 | Compromised admin: cascade revocation, recovery from the root | `tests/certs.rs` `cascade_revocation_removes_descendants`; `sim:` `a_recovered_admin_cert_makes_a_member_an_admin` | Covered |
| SE-08 | The relay never holds group keys | `tests/boundaries.rs` `modules_keep_their_boundaries` (the relay uses `proto` and `crypto` only) | Covered by structure |
| SE-09 | Relay: TLS, and the device signs a challenge | `relay/state/tests.rs` `members_authenticate_and_forward`, `bad_auth_is_refused` | Challenge covered; TLS settings not tested (relay rules are in `06-relay.md`) |
| SE-10 | LAN flooding: cheap tag check first; 200 datagrams a second per source before decryption; bounded queues; alarms before heartbeats | `sim:` `s10_two_practices_on_one_segment`; `net/udp.rs` `at_most_200_datagrams_a_second_from_one_source` | Tag check and per-source limit covered (the limit was built 2026-11-21). Queue bounds not tested. Alarm priority not built: messages are sealed, so their type is unknown until decrypted |

### Certificate validation

| ID | Rule | Tests | Status |
|---|---|---|---|
| SE-11 | 1. The signature verifies under the issuer | `tests/certs.rs` `rule1_signature_must_verify` | Covered |
| SE-12 | 2. The practice id matches | `tests/certs.rs` `rule2_practice_id_must_match` | Covered |
| SE-13 | 3. Issuer roles; chain depth at most 8; enroller lifetime at most 365 days | `tests/certs.rs` `rule3_root_issues_admin_only`, `rule3_role_constraints_on_issuers`, `rule3_enroller_issues_members_only_and_has_bounded_lifetime`, `chain_depth_is_bounded` | Covered |
| SE-14 | 4. Not expired, with 24 h tolerance | `tests/certs.rs` `rule4_expiry_with_tolerance` | Covered |
| SE-15 | 5. Not revoked, nor any issuer by cascade | `tests/certs.rs` `rule5_revoked_leaf_is_invalid`, `cascade_revocation_removes_descendants` | Covered |
| SE-16 | Non-cascading revocation keeps issued certs; issuers judged at the child's `issued_at`; a revocation covers every cert of the device | `tests/certs.rs` `non_cascade_revocation_keeps_issued_certs`, `members_enrolled_by_an_expired_token_stay_valid`, `revoked_device_cannot_return_with_new_cert`, `validity_is_order_independent`, `serial_conflict_is_rejected` | Covered |
| SE-17 | The admin UI explains "Remove" and "compromised" | none | Not built: the app offers Remove only; cascade is `panic-button ctl revoke --compromised` |

### Tokens and enrolment

| ID | Rule | Tests | Status |
|---|---|---|---|
| SE-18 | Devices listed by the token that enrolled them | `gui:` (shown, not checked) | Not tested |
| SE-19 | Tokens short-lived, 30 days by default | none | The CLI's default is 30 days, the app's 7; not tested |
| SE-20 | MSI token properties kept out of install logs | `msi:` "token kept out of the MSI log" | Covered (Windows runner) |
| SE-21 | Flow A: practice id, root, epoch 0 key, admin cert for this device | `agent/setup.rs` `practice_then_token`; `gui:` | Covered |
| SE-22 | Flow A: passphrase at least 12 characters, typed twice, strength meter | `crypto/recovery.rs` `short_passphrase_refused`; `gui:` (typed twice) | Covered, except the strength meter (not built) |
| SE-23 | Flow A: kit is XChaCha20-Poly1305 under Argon2id (256 MiB, t 3, p 1); file plus printable page with QR code | `crypto/recovery.rs` `round_trip`, `wrong_passphrase_fails`, `weak_params_refused_by_open_kit` | Kit file covered; the printable page is not built |
| SE-24 | Flow A: the root key is kept on no device | `agent/setup.rs` `the_root_key_is_not_kept_after_setup` | Covered |
| SE-25 | Flow A: the passphrase is checked to open the kit before continuing | `agent/setup.rs` `practice_then_token` (setup fails otherwise) | Indirect |
| SE-26 | Flow B: token format and prefix; secrets not printed | `proto/token.rs` `string_round_trip`, `rejects_bad_tokens`, `debug_redacts_secrets` | Covered |
| SE-27 | Flow B: the device self-issues a member cert with the token id | `tests/certs.rs` `token_enrolment_end_to_end` | Covered |
| SE-28 | Flow B: a device with a stale epoch is sent the current key | `sim:` `s15_token_enrolment_with_stale_epoch` | Covered |
| SE-29 | Flow B: the token is blanked after enrolment where possible | `msi:` "token blanked after enrolment" | Covered (Windows runner; added 2026-11-21) |
| SE-30 | Flow C: count practices nearby from tags only | `sim:` `join_by_approval_on_the_lan`; `gui:` ("running on 2 other computers") | Covered |
| SE-31 | Flow C: code shown on both computers, approve, confirm the practice name | `sim:` `join_by_approval_on_the_lan`; `gui:` | Covered |
| SE-32 | Flow D: the root signs a new admin cert for this device | `sim:` `a_recovered_admin_cert_makes_a_member_an_admin`; `agent/setup.rs` `new_recovery_kit_opens_with_the_new_passphrase_only` | Covered in the core; the agent's kit-to-cert step is not tested |

### Revocation, key storage, privacy, supply chain

| ID | Rule | Tests | Status |
|---|---|---|---|
| SE-33 | Revocation flooded; new epoch shared with every valid device; revoked devices rejected and cannot read | `sim:` `s14_revoke_laptop_during_open_alarm`; `prop:` `p6_revocation`; P6 | Covered |
| SE-34 | The previous epoch accepted for `EPOCH_GRACE`, from unrevoked signers only | `sim:` `s14_revoke_laptop_during_open_alarm` | Unrevoked-only covered; the grace period's end is not tested |
| SE-35 | Concurrent rotations: the key with the higher BLAKE3 hash wins | `sim:` `s16_concurrent_rotations_converge` | Covered |
| SE-36 | Windows: keys encrypted with DPAPI in machine scope | `store/tests.rs` `secrets_are_encrypted_on_windows`, `unencrypted_keys_from_older_versions_load_and_are_encrypted`, `keys_encrypted_on_windows_are_refused_elsewhere`, `store_matches_persisted_state_apply` | Covered (Windows runner) |
| SE-37 | Linux, macOS, headless: an owner-only file | `store/tests.rs` `the_database_file_is_owner_only` | Covered. Keychain and Secret Service are not built |
| SE-38 | Secret key material is zeroized on drop | `crypto/keys.rs` `debug_does_not_leak_secrets` | By review: `zeroize` types; nothing checks memory |
| SE-39 | Cryptographic primitives as listed | `crypto/keys.rs` `rfc8032_test_vector_1`, `rfc7748_x25519_vector`; golden vectors | Covered |
| SE-40 | `user_display_source`: full name, user name, or none | `desktop/lib.rs` `room_only_sends_no_name` | Covered (built 2026-11-21: before, the setting was read and ignored) |
| SE-41 | No data goes to OHTF | none; there is no telemetry code | By review |
| SE-42 | cargo-deny and cargo-audit in CI; signed releases; SHA-256 sums; SBOM | `.github/workflows/ci.yml` (deny, audit) | CI covers deny and audit; signing waits for certificates (R40, R41) |

## What is left

Not built: SE-10 alarm priority, SE-17 the cascade choice in the app, SE-22 the strength meter, SE-23 the printable kit page, PR-62 two amber reasons, SE-37 keychain and Secret Service. The app's token default (SE-19) is shorter than the spec's.

Not tested, or only indirectly: PR-06 (beyond the tag), PR-09, PR-22 (core), PR-28 and PR-35 (a member's message ignored), PR-31 (10-minute stop), PR-36 (bind retry), PR-38 (port fallback, connection limit), PR-39, PR-44, PR-46, PR-52, PR-57 (auto-resolve), PR-58 (state digest), PR-74 to PR-76, SE-05 (each event audited), SE-18, SE-25, SE-32 (agent step), SE-34 (grace end).
