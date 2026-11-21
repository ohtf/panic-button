# 09 Roadmap

Legend: `[x]` done, `[~]` in progress, `[ ]` not started. Item codes (`R1`, `R2`, ...) are stable: never renumbered or reused (see [house-style specs.md](https://github.com/pacharanero/house-style/blob/main/specs.md#stable-roadmap-item-codes)). Exit criteria are tasks too.

## v1 = phase 1 (desktop + LAN mesh) + phase 2 (optional relay)

Build in this order. Each milestone ends with green CI and a short demo note in the PR (or, while the repo is committing direct to `main`, in the commit body).

### M0 Repository and CI

- [x] **R1 - Workspace skeleton** per `02-architecture.md`.
- [x] **R2 - Licence, security and conduct files**: fetch the AGPL licence text; write `SECURITY.md`; add `CODE_OF_CONDUCT.md` (fetched text).
- [x] **R3 - CI**: fmt, clippy, test, cargo-deny, frontend lint placeholders, build matrix (Linux, Windows, macOS).
- [x] **R4 - Commit the ADRs** in `spec/adr/`.
- [x] **R5 - M0 exit**: empty crates build on all three OSes. CI green.

### M1 Protocol and crypto

- [x] **R6 - `panic-button-proto`**: all message types, deterministic CBOR, limits, golden vectors.
- [x] **R7 - `panic-button-crypto`**: keys, certs, chain validation, seal and open, KeyShare, recovery kit, tokens.
- [x] **R8 - Fuzz targets** for all decoders, plus `core_handle_datagram` (random bytes into an enrolled core by every transport).
- [~] **R9 - M1 exit**: unit tests for every rule in `03-protocol.md` and `04-security.md`. Fuzzers run clean for 10 minutes each.

### M2 Core and simulator

- [x] **R10 - Core: peers and path tracking, heartbeats.**
- [x] **R11 - Core: alarm raise and receive pipeline**, re-flood with suppression, receipts.
- [x] **R12 - Core: alarm state merge and expiry.**
- [x] **R13 - Core: hold machine** (FR-ACT-002, FR-ACT-005, FR-ACT-008, FR-ACT-009).
- [x] **R14 - Core: status computation** (FR-STS-003).
- [x] **R15 - Core: anti-entropy** (digests, SyncRequest, SyncResponse).
- [x] **R16 - Core: enrolment flows** (token and interactive).
- [x] **R17 - Core: revocation and rotation.**
- [x] **R18 - Core: drills.**
- [x] **R19 - Core: practice config.**
- [x] **R20 - `panic-button-sim` world model**: segments, routing, relays, schedules.
- [x] **R21 - Simulator scenarios S1 to S17.**
- [x] **R22 - Simulator properties P1 to P12.**
- [x] **R23 - M2 exit**: all properties and scenarios pass at 256 cases in CI and 1,000 cases in one nightly run (scaled down from 10,000 on 2026-11-14 by Marcus Baw, to raise again once the nightly is reliable). **Do not start M4 until this passes.** Met 2026-11-14: CI run 43621103526 (256 cases) and nightly run 43642345393 (1,000 cases per property, S13 at 250 devices, 10 minutes of fuzzing per target).

### M3 Runtime, store, headless agent, CLI

- [x] **R24 - `panic-button-store`**: SQLite schema and migrations.
- [x] **R25 - `panic-button-net`**: UDP, TCP, mDNS, interface watch.
- [x] **R26 - `panic-button-agent`**: runtime, control API, headless binary.
- [x] **R27 - `panic-button`**: `practice init`, `token create/revoke/list`, `device list/revoke/rename/room`, `drill run/report`, `status`, `peers`, `ctl` commands. Also `device promote`. Rename and room use the DeviceSettings message (ADR 0014); drill reports are kept in the store (FR-DRL-007).
- [x] **R28 - netns integration tests** (S1, S2, S4, S7 without relay).
- [x] **R29 - M3 exit**: three headless agents in netns raise and receive alarms. The CLI can set up a practice end to end.

### M4 Desktop app

- [~] **R30 - Tauri 2 + Svelte 5 app** embedding the agent: floating button, hold ring and cue, sender panel, receiver alert window, tray, status, main window (status, history, settings), first run, practice run. Prototype working end to end on Linux (`docs/gui/`). Done since: the global shortcut (Windows, macOS, X11) and button position memory per monitor layout, both checked on a real X server by `s/x11-test`; the button and panel windows are their real size on Linux (they were held at 200 x 200, tao#561). Also ESLint and Prettier for the frontend, in `s/desktop-check` and a quick CI job, and joining by approval code (flow C: the computer shows a six-digit code, an admin approves it on the Admin tab, the person confirms the practice name), with `join_by_approval_on_the_lan` in the simulator and the `join` netns scenario. A running agent with no practice can also be enrolled in place (`enrol`), so someone who tries asking first can still paste a code or set up a practice. Easier joining (2026-11-19, from home testing): the app runs its agent from first launch, and the welcome screen says how many Panic Button computers it hears on the network, with Ask to join; checked by the simulator, the `join` netns scenario and `s/x11-test`. Fixed the same day: the practice hold never enabled Finish, because the core stopped sending progress just before the end (`practice_hold_completes_without_raising`).
- [~] **R31 - Generated sounds** (CC0). Patterns A to C and the drill chime are generated WAV files in `sounds/` (`s/sounds`), replaceable per device; the hold cue is synthesised. Needs a listening check on real speakers.
- [~] **R32 - Platform modules**: Windows audio endpoint (status and override), session detection, DPAPI; macOS Local Network detection; Linux Wayland fallbacks. Done: the Windows control API named pipe (Q7); mute and volume detection on Linux (`pactl`, `wpctl`, `amixer`; checked by `s/x11-test`) and macOS (`osascript`, not yet run on a Mac); the Windows audio endpoint, with the mute override during alarms (`platform::windows_audio`; compiled and linted by `s/windows-check`, not yet run on Windows); session detection, locked and remote, on Windows (`platform::windows_session`, not yet run on Windows) and Linux (logind and xrdp; the remote case is checked by `s/x11-test`). Still to do: fast user switching on Windows, DPAPI, macOS session and Local Network, and the XDG GlobalShortcuts portal so the shortcut works on Wayland.
- [~] **R33 - M4 exit**: manual run-through of every screen on all three OSes. Accessibility check (keyboard only, screen reader labels, contrast). First pass 2026-11-20 in a browser against the mock agent: axe-core 4.14 (WCAG 2.2 AA and best practice) over the main window's five tabs, first run, the alert, the sender panel and the button finds no violations after fixing empty table headers, a missing Admin heading, first run's missing main landmark and the button window's missing heading; keyboard alone reaches every control, answers an alert, and completes the practice hold with the space bar. Still to do: the run-through on Windows, macOS and Linux, and listening with real screen readers (NVDA, VoiceOver, Orca).

### M5 Admin and drills in the app

- [~] **R34 - Admin section**: coverage, devices (approve, revoke, rename, rooms, promote), tokens, drills (schedule, run, reports, CSV), practice settings, admins and recovery, audit export. Done: devices with a needs-attention filter, rename, room, promote and remove; join approvals; enrolment codes (create, copy, revoke, devices per code); test alarms with reports and CSV copy; a warning when there is only one admin; practice settings (hold time, cue, alarm pattern, drill mode, opening hours, relay URLs; also `panic-button practice settings`). Also audit log export as CSV (`panic-button ctl audit-export`). Also weekly test alarms run from a chosen coordinator, with skips recorded on admin devices (FR-DRL-001, FR-DRL-006), and "Recover admin access" with the recovery kit on an enrolled device (Settings, or `panic-button practice recover`). Also "Create a new recovery kit" with a new passphrase (Admin tab, or `panic-button practice new-kit`). Not yet: recovery on a device that is not enrolled (needs a protocol decision, Q9).
- [ ] **R35 - M5 exit**: a practice can be set up, enrolled, drilled and administered entirely from the GUI.

### M6 Relay

- [x] **R36 - `panic-button-relay`**: WebSocket and long-poll, tenants, auth, forwarding, buffer and resume, presence, revocation enforcement, `/source`, metrics.
- [x] **R37 - Relay client** in `panic-button-net`, including proxy support and long-poll fallback. NTLM and Kerberos proxy auth are not supported (S for v1, see `06-relay.md`).
- [x] **R38 - Relay netns tests** S5, S6, S12, S14 with a real relay. Docker image and compose example. Also S5 over long-poll; `packaging/relay/` has the image, compose with Caddy, and a hardened systemd unit.
- [x] **R39 - M6 exit**: L3, L5 and L8 equivalents pass in netns. L3 is S5 (two VLANs, relay; delivery within 5 s, amber where relay-only), L5 is S6 (Wi-Fi isolation plus relay; laptops receive and raise within 5 s), L8 is S12 (relay down mid-alarm; LAN unaffected, relay-only device catches up).

### M7 Packaging

- [~] **R40 - Windows MSI** with properties, firewall, Run key, WebView2, signing (with a placeholder certificate in CI), Scheduled Task watchdog registration on first run. Done: firewall and Run key (R53); WebView2, which Tauri's MSI downloads and installs silently when missing (its default `webviewInstallMode`; computers without internet access would need the offline installer, about 127 MB more); the watchdog task (2026-11-20, compiled for Windows, not yet run there). Also the MSI properties for IT (`ENROLMENT_TOKEN`, `ENROLMENT_TOKEN_FILE`, `ROOM`, `RELAY_URL`, `SEED_PEERS`), written into `config.toml` by `panic-button-desktop.exe --install-settings` (tested on Linux; the MSI wiring not yet run on Windows). Still to do: signing, which needs a certificate, and `PURGE=1` on uninstall.
- [ ] **R41 - macOS pkg** (signing and notarisation on tags), LaunchAgent.
- [x] **R42 - Linux** deb, AppImage and systemd units. The deb and AppImage are done (R54). Systemd units added 2026-11-21: `packaging/linux/panic-button-agent.service`, a hardened system service for the headless agent, and the desktop watchdog (`panic-button-watchdog.timer` and `.service`), shipped and enabled for every user by the deb, rpm and AUR packages; install, upgrade and removal checked in Ubuntu 24.04 and Fedora 41 containers. The AppImage has no watchdog.
- [ ] **R43 - M7 exit**: silent MSI install smoke test in CI. L9 passes in the lab.

### M7a Home test (Windows and Linux installers first)

Priority set by Marcus Baw on 2026-11-15: a Windows installer and Linux packages good enough for a limited test on several Windows and Linux machines at home, ahead of the rest of M7. The full R40 to R43 items still apply afterwards.

- [x] **R52 - Installer builds in CI** (superseded by R64; `installers.yml` stays for test builds without a release): a workflow that builds the Windows MSI and the Linux deb and AppImage from any commit (manual run) and on tags, and keeps them as downloadable artifacts. Unsigned for now: Windows SmartScreen will warn. Workflow written (`.github/workflows/installers.yml`); its first run was refused by GitHub billing.
- [x] **R53 - Windows MSI basics**: per-machine install to Program Files, Windows Firewall rules for UDP and TCP 47474 (so the first alarm is not blocked behind a firewall prompt), starts at sign-in for every user, a clean uninstall that removes the rules. Signing, policies, the WebView2 bootstrapper check and the watchdog stay in R40. `packaging/windows/panic-button.wxs`. Installed on Windows in home testing (v0.2.0, 2026-11-19), and checked on every change by the smoke test (R43).
- [x] **R54 - Linux packages basics**: a deb and an AppImage that start the app at sign-in (XDG autostart), with the same sounds and icons. Deb and AppImage built in an Ubuntu 22.04 container; the AppImage runs on Arch (CachyOS).
- [~] **R55 - Home test guide**: a short guide for installing on a few machines, setting up a practice on one, adding the others with an enrolment code, and the checks to run (alarm both ways, a computer switched off, a test alarm). `docs/home-test.md`.

### Distribution

Set up on 2026-11-15. One command, `s/version++`, releases to every channel (house style `distribution.md`).

- [x] **R62 - Release pipeline**: `s/version++` bumps the version (Cargo, npm), regenerates `CHANGELOG.md` with git-cliff and commits; `auto-tag.yml` tags and calls `release.yml` and `publish-crates.yml`. First release v0.1.0 on 2026-11-15; v0.2.0 and v0.2.1 went out on every channel from `s/version++` alone.
- [x] **R63 - Command-line programs** (`panic-button`, `panic-button-agent`, `panic-button-relay`) for Linux, macOS and Windows on x86-64 and ARM via cargo-dist, with install scripts, `SHA256SUMS` and Homebrew formulae in `ohtf/homebrew-tap`. Published for v0.1.0.
- [x] **R64 - Desktop packages** on every release: MSI, deb, rpm, AppImage and a universal macOS dmg, with stable file names, a Homebrew cask and the AUR package `panic-button-bin`. v0.1.0: all five built and published, the cask is in the tap, the deb and rpm install cleanly on Ubuntu 24.04 and Fedora 41, and the AUR package builds from the release. `panic-button-bin` was published to the AUR on 2026-11-16 (CI key on Marcus Baw's AUR account) and builds from the release; the rpm lacked the autostart entry until 0.1.1.
- [x] **R65 - crates.io**: one crate, `panic-button` (library and the three programs), publishes on each release by trusted publishing (ADR 0016, 2026-11-18; before that, eight crates). v0.1.0 of all eight crates was published by hand from the tag on 2026-11-16 (crates.io allows five new crates at once, then one per 10 minutes). The publish job runs in the `crates-io` GitHub environment, which only `main` may deploy to (2026-11-17). v0.2.0 was the first release published by CI, through one trusted publisher entry (`auto-tag.yml`, environment `crates-io`), on 2026-11-18. The seven former library crates' 0.1.0 releases were yanked the same day. Trusted publishing is the only route since 2026-11-19: the `CARGO_REGISTRY_TOKEN` fallback was removed. Package metadata says `AGPL-3.0-or-later`, because crates.io rejects our licence addition after `WITH`; the addition still applies.

### Documentation

The guides for people who use Panic Button, in plain English for practice staff. Every page is checked against the "Signs of AI writing" patterns (Wikipedia; also the humanizer skill) before it is committed, and screenshots come from the real app.

- [x] **R56 - Writing checklist**: the AI-writing patterns to avoid, as a short checklist in `docs/` that every page is checked against. Fix the patterns already in `README.md`. `docs/writing-checklist.md`; README fixed.
- [~] **R57 - Staff quick guide**: one printable page. How to call for help, what the status shapes mean, answering an alert, cancelling a mistake. Draft in `docs/staff-guide.md` with screenshots. Printing it from the docs site gives one A4 page (print styles in `docs/stylesheets/extra.css`, checked with Chromium's PDF output). Still needs a read-through by practice staff.
- [~] **R58 - Practice Admin guide**: setting up the practice, the recovery kit, adding and removing computers, rooms and names, test alarms and their reports, what to do when a computer shows amber or red. Draft in `docs/admin-guide.md`; needs screenshots and a read-through by a practice manager.
- [~] **R59 - Docs site** with Zensical (house style `docs.md`), published to GitHub Pages, with a check that every page is in the navigation. Built 2026-11-16: `mkdocs.yml`, a home page, navigation by reader (staff, Practice Admins, developers and IT), `s/docs`, `s/check-docs-nav`, and `.github/workflows/docs.yml`, which builds and checks every docs change. It deploys to https://ohtf.github.io/panic-button/ on every docs change on `main` (since 2026-11-20).
- [ ] **R60 - Install guides**, written with the installers and tested against them: for practice staff installing on one computer (Windows first), and for IT deploying the MSI silently with policy settings. They become part of R46.

### M8 Hardening and release candidate

- [ ] **R61 - Adversarial and long runs back in CI** for release candidates: 1,000-case properties, S13 at full scale, long fuzzing including hostile LAN traffic, Windows and macOS tests. Moved to local runs (`s/check-deep`) on 2026-11-15 to save Actions minutes until the home test (M7a) has passed.
- [ ] **R44 - Hardware lab protocol** L1 to L14 executed and recorded.
- [ ] **R45 - Hazard log** updated with evidence and residual risks.
- [ ] **R46 - Documentation set** from `08-deployment.md`.
- [ ] **R47 - CSO review and sign-off.**
- [ ] **R48 - M8 exit**: v1.0.0-rc.1 tagged and pilot started.

### Pre-release checks (from the original spec notes)

- [ ] **R49 - Legal review** of the app store additional permission before the first external contribution (ADR 0001).
- [x] **R50 - Check port 47474** against the IANA service name and port registry. Checked 2026-11-16: unassigned (in the gap 47101-47556; the nearest assignment is `dbbrowse` on 47557). It lies in Linux's default range for outgoing connections (32768-60999), so the agent retries the UDP bind for up to 30 s, and TCP already falls back to another port. Registering the port with IANA is optional and can wait for v1.
- [ ] **R51 - Trade mark check** on "Panic Button" (renamed from Muster on 2026-11-15, ADR 0015). The name is descriptive, so it may be hard to register; the check is mainly that using it does not infringe anyone else's mark.

## Post-v1

### 1.1 Integration node and home profile

- Headless agent "integration outputs": ntfy, Gotify, Matrix, generic webhook, MQTT (Home Assistant discovery). These give mobile *receiving* via existing apps before native apps exist, and let self-hosters drive lights and sirens.
- Inputs: a Home Assistant or MQTT trigger can raise an alarm (`source = integration`; disabled by default; requires an explicit admin setting).
- Home profile defaults tested. Docker image for "relay + integration node" in one compose file.

### 1.2 OHTF USB button (hardware)

- Open hardware under `hardware/` (CERN-OHL-S-2.0). A low-cost microcontroller (for example RP2040 or CH552) with a large momentary switch.
- Presents a **vendor-defined HID interface** (read via `hidapi`; no driver on Windows or macOS; udev rule on Linux) with reports for press, release and periodic presence. An optional fallback keyboard interface sends F24, a key almost nothing uses, only if configured.
- The agent applies the same hold machine (`source = hid`). Button presence is reported in the heartbeat and coverage ("button connected").
- Compliance for sale: UKCA/CE (EMC), RoHS, WEEE producer registration. OHTF-branded units are sold to fund development. The trade mark policy controls the branding.

### Basic mobile app

Requested by Marcus Baw on 2026-11-19: an exceedingly basic Android and iOS app, well before the full 2.0 mobile work below. Open the app, press the alarm.

- [ ] **R66 - Basic mobile app (Tauri mobile)**: one screen with the big red button and the hold ring, built from the existing Svelte frontend and the agent with Tauri's Android and iOS targets. It joins the practice like a desktop computer (enrolment code or approval code) and raises alarms over the practice Wi-Fi while the app is open, showing the same sender panel. It does not receive alarms in the background; that needs the push forwarder in 2.0. To check first: that Tauri mobile can run the agent's UDP, TCP and mDNS networking, and what iOS needs (the Local Network permission, and Apple's multicast entitlement or seed peers instead), and how the app says clearly that it cannot receive alarms.

### 2.0 Mobile

- Android first: FCM high-priority messages via the push forwarder, plus an optional foreground-service mode that listens directly on practice Wi-Fi. UnifiedPush support so a practice can avoid Google and OHTF entirely.
- iOS: APNs via the OHTF push forwarder. Apply for the Critical Alerts entitlement early. Raising alarms from widgets, the Action Button and Apple Watch.
- **Push forwarder:** run by OHTF. Content-blind: it receives `{device push token, opaque wake id}` from the practice relay and sends a wake-only push. The app then fetches the envelopes from the practice relay. No alarm content passes through OHTF.
- PWA with Web Push via the relay as an interim, no-app-store option.
- Mobile enrolment by QR code from any enrolled desktop.

### Later

- ADMX/ADML Group Policy templates.
- Welsh language (cy).
- IPv6.
- Citrix and AVD investigation: a virtual channel or endpoint helper so thin-client users are covered.
- Multi-site: practices with branch surgeries (several practice ids, or sites within one practice, with site-scoped alerting).
- An alternative independent implementation of the protocol from the spec and golden vectors, to validate the spec.
