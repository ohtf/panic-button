# 04 Security

## Assets

1. **Availability of alarm delivery** (the primary asset; a security control must never silently reduce it).
2. **Integrity of alarms**: no forged or replayed alarms, no forged cancellations or resolutions.
3. **Confidentiality** of who raised an alarm, and where and when (minor but real: it reveals incidents and staffing).
4. Practice keys and the recovery kit.

## Threat actors and controls

| Actor | Capability | Controls |
|---|---|---|
| Outsider on the same LAN or Wi-Fi (patient on guest Wi-Fi, neighbouring practice, contractor) | Sniff, inject or replay packets | Group encryption (cannot read); signatures plus cert validation (cannot forge); dedupe with persisted ids plus ALARM_MAX_LIFE (replays never re-alert); practice tag filtering |
| Neighbouring practice sharing infrastructure | Same as outsider, plus a valid cert for *their* practice | Practice id and root key separation; tag mismatch drops traffic; certs chain to a different root |
| Malicious or careless insider (enrolled device) | Raise false alarms, resolve others' alarms | Every action is signed and attributable; audit log on every device; only the originator can cancel; this is accepted residual risk, managed by accountability |
| Stolen or lost laptop | Holds a device key and the group key | Admin revokes the device, which rotates the group key; the relay drops it; DPAPI machine scope stops the key file being copied off-box (Windows) |
| Compromised admin device | Can issue certs and revocations | Revocation with cascade; recovery via root key; at least 2 admins recommended |
| Compromised or curious relay operator | Sees envelopes, metadata, presence | Relay never holds group keys; content is end-to-end encrypted; metadata exposure is documented |
| Network attacker on the internet path to the relay | MITM, block | TLS (rustls, system roots plus optional pinned CA); relay auth is mutual at the application layer (device signs a challenge; relay identity via TLS); blocking only removes one path |
| Denial of service on the LAN (flooding) | Exhaust CPU or bandwidth | Cheap tag check before crypto; per-source rate limits (200 datagrams/s per source IP before decrypt); bounded queues; alarms take priority over heartbeats in processing queues |

## Key hierarchy

```
Practice root key (Ed25519)          offline: only in the recovery kit (passphrase-encrypted)
  |
  +-- Admin certs (role=admin)       issued by root or by another admin
  |     |
  |     +-- Member certs             issued by an admin (interactive join)
  |     +-- Enroller certs           issued by an admin, embedded in an enrolment token; expiring
  |           |
  |           +-- Member certs       self-issued by new devices using the token's enroller key
  |
Group key per epoch (32 random bytes) for sealed envelopes
Device keys: Ed25519 (signing, = DeviceId) + X25519 (key exchange), generated on device, never leave it
```

### Certificate validation

A device cert is valid if:
1. Its signature verifies under `issuer`.
2. `practice_id` matches.
3. The issuer is the root public key, **or** the issuer has a valid cert (recursively, maximum chain depth 8) with an appropriate role:
   - admin certs: issued by root or admin;
   - member certs: issued by admin or enroller;
   - enroller certs: issued by admin, must have `expires_at` no more than 365 days after `issued_at`.
4. It is not expired (enroller and other certs with `expires_at` set). Expiry is judged against local wall time with 24 h of tolerance.
5. Neither it nor any issuer in its chain is revoked with `cascade = true`, and the cert itself is not revoked.

**Non-cascading revocation of an issuer does not invalidate certs it issued.** This lets a practice manager leave (their admin cert is revoked) without de-enrolling every PC they set up. Use `cascade = true` only for suspected compromise. Issuers in a chain are judged as of the child's `issued_at` for expiry and non-cascading revocation, and a revocation applies to every cert of the target device (ADR 0012). The admin UI must explain the difference in plain English:
- "Remove this device" (non-cascade)
- "This device or account is compromised - also remove everything it enrolled" (cascade)

### Known limitation: backdated self-issued certs

A token's enroller key can be used to self-issue member certs until the token expires or is revoked. Someone holding a leaked token can backdate `issued_at`. Mitigations:
- Admin view lists all devices by token id. Revoking a token prompts the admin to review and revoke unknown devices enrolled with it.
- Tokens are short-lived by default (30 days).
- Token revocation is implemented as a cascade revocation of the enroller cert **only if the admin chooses**. The default (non-cascade) stops new enrolments only after propagation, and relies on the review step.
- MSI token properties are marked hidden so they are not written to install logs (see `08-deployment.md`).

`max_uses` on tokens is advisory. It cannot be enforced offline, so it is displayed but not relied upon.

## Enrolment flows

### A. New practice (first device)

1. Generate practice id (16 random bytes), root keypair, epoch 0 group key, and device keys.
2. The root signs an admin cert for this device.
3. The user sets a recovery passphrase (minimum 12 characters; strength meter; must type it twice).
4. Produce the **recovery kit**: `{practice_id, practice_name, root_secret_key, created_at}` encrypted with XChaCha20-Poly1305 under a key from Argon2id (m = 256 MiB, t = 3, p = 1, random 16-byte salt). It is saved as a `.panic-button-recovery` file plus a printable HTML page with a QR code of the same blob and instructions.
5. Wipe the root secret key from memory and disk (zeroize). It is **not** kept on any device.
6. The user must confirm they have saved the kit, and the app checks the passphrase decrypts it before continuing.

### B. Token enrolment (managed deployment)

1. An admin creates a token: generate an enroller keypair; the admin issues an enroller cert (expiry per token); bundle:
   `Token = base64url(CBOR {0: version, 1: practice_id, 2: practice_name, 3: root_pub, 4: enroller_cert, 5: enroller_secret, 6: epoch, 7: group_key, 8: relay_urls, 9: seed_peers, 10: network overrides, 11: token_id, 12: expires_at})`, with string prefix `panic-button-enrol-1:`.
2. A device with the token generates its own keys and self-issues a member cert signed by the enroller key (`token_id` set). It stores the practice details and group key, then joins: it sends CertAnnounce with its cert and the enroller cert, and heartbeats.
3. If the token's epoch is older than the current one, peers detect the stale epoch from heartbeats. Any member sends the new device a KeyShare for the current epoch once it has validated the new cert.
4. The token file or property value is deleted or blanked from local config after successful enrolment, where the agent has permission. Admins are told the token remains valid elsewhere until expiry or revocation.

### C. Interactive enrolment (LAN)

0. Before joining, the device can count the practices on its network from the practice tags of sealed envelopes it cannot open (`Core::nearby`). This reveals nothing a passive listener on the LAN does not already see: no names, ids or keys.
1. The joining device broadcasts JoinRequest and shows its 6-digit code.
2. Admin devices show "RECEPTION-PC2 wants to join (code 123 456). Check this code is shown on that computer." with Approve and Reject.
3. On Approve, the admin issues a member cert and sends JoinApproval (cert chain, root pub, practice name, KeyShare of the current group key, current config).
4. The joining device verifies the chain to the root pub in the approval. **Trust on first use:** the user on the joining device sees the practice name and confirms "Join Riverside Surgery?".

### D. Recovery

On any enrolled or fresh device: "Recover admin access" -> load the kit -> passphrase -> decrypt the root key in memory -> the root signs a new admin cert for this device -> zeroize. The recovered root can also issue a cascade revocation of compromised admins.

## Revocation and key rotation

1. The admin issues a Revocation (open envelope, flooded, sent to relays).
2. The admin generates the epoch E+1 group key and sends a KeyShare for E+1 to every device with a valid cert (one per recipient; flooded; also retained for sync with `want: keyshares_for_me`).
3. Devices switch to sending in E+1 when they hold it. They accept E (previous) for `EPOCH_GRACE` from **unrevoked** signers only.
4. Revoked devices: their messages are rejected immediately (signature from a revoked cert). After rotation they cannot read new traffic. The relay drops their sessions.
5. If two admins rotate concurrently (two different E+1 keys), the key with the higher `BLAKE3(key)` wins deterministically. Devices that adopted the loser switch when they see the winner's KeyShare. Senders include the epoch and tag, so receivers can hold both keys during the conflict and decrypt both.

## Key storage

| Platform | Device keys | Practice and group keys |
|---|---|---|
| Windows | DPAPI `CRYPTPROTECT_LOCAL_MACHINE` blob in `%ProgramData%\Panic Button\identity`, ACL: SYSTEM and Administrators full, Users read | same |
| macOS (v1) | user login keychain item | same |
| Linux (v1) | Secret Service (libsecret) if available, otherwise a 0600 file under `$XDG_DATA_HOME/panic-button` | same |
| Headless | 0600 file owned by the service user | same |

As built (2026-11-21): on every platform the keys are rows in the agent's SQLite database, `panic-button.db` in the data directory (`%ProgramData%\Panic Button` on Windows), not separate files. On Windows the device key pair and each group key are encrypted with DPAPI as above (`store/protect.rs`, ADR 0017), stored as `pb-dpapi-1:` and the blob; keys written unencrypted by v0.2.1 or earlier are encrypted when first loaded. On Linux, macOS and headless, the database is a 0600 file owned by its user; the keychain and Secret Service are not used yet. The Windows folder ACL is not set yet either: only the account that first runs Panic Button can write there.

All secret key material in memory uses `zeroize` on drop.

## Cryptographic choices (ADR 0005)

Ed25519 (signatures), X25519 (key agreement), XChaCha20-Poly1305 (AEAD), BLAKE3 (hashing, keyed tag), HKDF-SHA256 (key derivation), Argon2id (passphrase KDF). All from audited RustCrypto or dalek crates. No custom primitives.

## Privacy

- Data processed: device label, room name, user display name, timestamps, IP addresses. No patient data.
- Display name source: the OS full name of the logged-in user. Policy `user_display_source` can be `os_full_name` (default), `os_username` or `room_only`.
- No data goes to OHTF or anyone else unless the practice configures a relay operated by them. Relay metadata exposure (device ids, connection times, IP addresses, message sizes and timing) is documented in the relay operator guidance.
- Practices should cover Panic Button in their own data protection documentation. A template DPIA paragraph will be provided with v1 docs.

## Supply chain and releases

- `cargo-deny` (advisories, licences, bans, sources) and `cargo-audit` in CI. Dependabot or Renovate for updates.
- Signed releases: Windows Authenticode (OV/EV certificate or Azure Trusted Signing), macOS Developer ID plus notarisation, Linux artefacts signed with minisign or Sigstore. Publish SHA-256 sums.
- Aim for reproducible builds of the relay and headless agent (post-v1 goal; document the steps).
- SBOM (CycloneDX) attached to each release.
