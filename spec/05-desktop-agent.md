# 05 Desktop agent: UX and platform behaviour

## Visual identity

- Do not imitate any commercial product (no green circular "button" trade dress). The button is a big red button, the generic emergency-stop shape everyone recognises, with an original glyph (a simple raised hand) in white.
- The desktop launcher and GitHub README share the [perspective logo](../docs/assets/panic-button-logo.svg). The tray and floating button keep the flat red button with a white hand.
- **Red means alarm, and only alarm.** Red is used for the button, the alert window and alarm states. Everything else (main window, admin, sender panel) is neutral, with blue for actions, so red never becomes background noise.

| Token | Hex | Use | Contrast |
|---|---|---|---|
| Alarm Red | `#C8102E` | Button face, alarm headings on white | White glyph 5.9:1 |
| Alarm Red Pressed | `#9E0B23` | Button during hold; hold ring track | White 8.3:1 |
| Alert Deep Red | `#7A0019` | Alert window background | White text 11.4:1 |
| Ink | `#1B1F24` | Text | On Paper 16.6:1 |
| Slate | `#4B5563` | Secondary text | On Paper 7.6:1 |
| Mist | `#F3F4F6` | Panel and window backgrounds | |
| Paper | `#FFFFFF` | Surfaces, glyphs on red | |
| Action Blue | `#1F4E8C` | Buttons and links outside alarm states | White 8.3:1 |
| Status Green | `#1A7F37` | Status: OK (filled circle) | |
| Status Amber | `#F59E0B` | Status: limited (triangle), with Ink glyph | Ink 7.7:1 |
| Status Red | `#B91C1C` | Status: nobody reachable (square) | |

- On the red button, the status dot has a 2 px Paper outline so it stays visible, and its shape (circle, triangle, square) carries the meaning without colour (NFR-ACC-001).
- All sounds are generated programmatically (`apps/desktop/src-tauri/src/audio.rs`, run with `s/sounds`). The alarm patterns and drill chime are committed as WAV files under CC0 in `sounds/` (`alarm-a.wav`, `alarm-b.wav`, `alarm-c.wav`, `drill-chime.wav`) and built into the app. An alarm file holds one cycle; the app loops it and applies the volume ramp.
- A practice can replace a sound by putting a WAV file with the same name in the `sounds` folder of the data directory (for example if pattern A resembles their fire alarm). A replacement that cannot be read, or is larger than 16 MB, is logged and the built-in sound plays instead, so a bad file never silences an alarm.
- The hold cue is synthesised at run time, because its pitch rises over the configured hold time.

## Floating button

| Property | Value |
|---|---|
| Size | 44 x 44 px at 100% scaling (scales with DPI); min hit target 44 px |
| Default position | Bottom-right of the primary monitor, 24 px above the taskbar or dock and 24 px from the edge |
| Idle opacity | 75%; 100% on hover or during hold |
| Window | Transparent, frameless, always-on-top, skip taskbar, no focus steal on show |
| Status dot | 10 px circle at the top-right edge: green, amber or red. The shape also differs (filled circle, triangle, square) so it does not rely on colour alone |
| Tooltip | "Press and hold to call for help" plus the status line |

### Hold interaction

State machine in the `core` module (`HoldMachine`). The UI only reports pointer events:

```
Idle --HoldStart(src)--> Holding{start, src}
Holding --HoldEnd, elapsed < 300ms--> Idle                     (nothing logged)
Holding --HoldEnd, 300ms <= elapsed < hold_ms--> Idle          (NearMiss logged)
Holding --Tick, elapsed >= hold_ms--> Triggered --> raise alarm --> Idle
```

- The UI calls `setPointerCapture` on `pointerdown` and sends `HoldStart{source: button}`. It sends `HoldEnd` on `pointerup`, `pointercancel` or `lostpointercapture`, or when the window loses visibility.
- **Pointer movement never cancels a hold** (ADR 0009). Under stress, hands move.
- Only one hold can be active. A second source starting during a hold is ignored. Its `HoldEnd` is ignored too.
- The core emits `UiEvent::HoldProgress{fraction}` at 30 Hz so the ring animation stays in sync with the real timer. The UI may interpolate.
- Touch long-press uses the same path.
- The shortcut (`tauri-plugin-global-shortcut`, which reports Pressed and Released) sends `HoldStart{source: hotkey}` on the first Pressed and `HoldEnd` on Released. Repeated Pressed events are ignored. If Released is unsupported on the platform, switch to double-press mode (FR-ACT-007) and say so in settings.
- The shortcut works on Windows, macOS and X11. In a Wayland session it cannot work without the XDG GlobalShortcuts portal (not yet supported), even under XWayland, so Settings says it is unavailable there. `alarm.hotkey = ""` or `"none"` turns it off; a shortcut that cannot be registered (another program has it) is logged and shown in Settings, and the app still starts.

### Hold cue

| `cue_mode` | Behaviour during hold |
|---|---|
| `silent` | Ring only |
| `soft` (default, practice) | Soft rising tone: sine, 500 Hz rising to 800 Hz over `hold_ms`, with a gentle 4 Hz amplitude pulse, starting at -30 dBFS and rising to -20 dBFS. Stops within 20 ms of release. Must not resemble the alarm patterns |
| `alarm_ramp` (default, home profile) | The selected alarm pattern, at volume rising from 5% to 40% over `hold_ms` |

After a trigger, **no sound** plays on the sender's device in any mode (ADR 0008).

### Repositioning

- Tray menu "Move button": the button gets a dashed outline and can be dragged freely for 30 s, or until clicked once. In this mode holding does nothing.
- Shift+drag at any time also moves it (Shift held at `pointerdown` means move, not hold).
- Positions are saved keyed by the monitor layout (each monitor's size, position and scale; names are left out because some systems change them), in `button-position.json` in the data folder, at most 16 layouts. A saved position is pulled wholly onto the monitor holding its centre; if no monitor holds it, the button goes to the default position on the primary monitor.
- Every 5 s the app checks the layout. If it changed (a laptop docked or undocked, a screen unplugged) or the button is no longer wholly on a screen, the button is placed again for the new layout.
- On Wayland the compositor places windows, so positions are neither saved nor restored.

## Sender panel (after trigger)

A small panel (about 260 x 120 px) anchored next to the button. Neutral colours, no flashing, no red.

| Time | Content |
|---|---|
| Immediately | "Help requested" with a small spinner |
| On first Receipt | "Delivered to N devices" (live count) |
| On Responding | "Coming: Jane Smith (Reception), Dr Ali (Room 2)" |
| No Receipt after 5 s | "Not yet delivered - trying all routes" (amber text). This is honest; it must be shown |
| No Receipt after 15 s | "Could not reach other devices. Call for help another way." (red text, with a persistent warning icon on the button) |

Controls:
- "Cancel - sent in error" -> confirm dialog: "Cancel the alarm and tell everyone it was sent in error?" [Cancel alarm] [Keep alarm]. "Keep alarm" is the default focus.
- "Resolved" -> confirm: "Mark as resolved so everyone can stand down?" [Resolve] [Back].
- The panel can be collapsed to the button. A small badge on the button shows the responder count while the alarm is open.

## Receiver alert window

- Topmost, centred on the primary monitor, about 520 x 360 px (scaled); it cannot be minimised. On other monitors, a thin flashing border overlay window is optional (S).
- Shown without activating focus where the OS allows (Windows `WS_EX_NOACTIVATE` and `SW_SHOWNOACTIVATE`; macOS non-activating panel). Flash the taskbar or dock.
- No default button. Enter and Space do nothing until a button has focus via Tab or click (FR-RCV-002).

Layout (text sizes at 100% scaling):
```
HELP NEEDED                                   (32 px bold)
Consulting Room 4                             (28 px)
Dr A Example  -  ROOM4-PC  -  09:12 (just now)  (16 px)

Coming: Jane Smith (Reception)                (16 px, live)

[ I'm coming ]     [ Seen - can't come ]      (large buttons, 48 px tall)
```
- Colours: high-contrast amber-on-black or white-on-deep-red (WCAG AA). No reliance on colour: the heading text carries the meaning.
- Multiple alarms: stacked cards, newest first, each with its own buttons.
- After the local user acts, the sound stops and the card shows "You're coming" or "Seen". The card stays until the alarm closes, so the user still sees updates.
- Closed: "Resolved by <name> at HH:MM" or "Cancelled - sent in error". It closes after 15 s.
- Expired on first receipt: no window. It goes to history only.

### Alarm sounds

- At least 3 patterns, generated programmatically. Default pattern A: two tones of 880 Hz and 660 Hz, 200 ms each with 5 ms ramps, repeated 3 times, then 600 ms of silence, looping. Patterns B and C differ in rhythm and pitch.
- Volume: 50% of the alarm volume setting, rising to 100% over 8 s, looping until acknowledged.
- Played via `rodio`/`cpal` on the default output device. If playback fails, report it in the heartbeat audio status and show amber.
- The default output's mute and volume are read every 10 s and sent to the core as the audio status, so muted or zero volume shows amber ("Sound is muted on this computer") and appears in test alarm reports. Linux reads them with `pactl`, then `wpctl`, then `amixer`; macOS with `osascript`; Windows with `IAudioEndpointVolume` (`platform::windows_audio`). A level that cannot be read counts as not muted, so a computer is never shown amber for a reading it could not make.
- `override_mute` (Windows, v1): use `IAudioEndpointVolume` to unmute and set master volume to at least 60% while alerting, then restore exactly. Log each override. The mute and volume are saved before anything changes and put back on the same device, even if the default output changed during the alarm. Because the alarm will be heard anyway, a muted Windows computer with `override_mute` on stays green.
- The hazard log tracks confusion with fire alarms. Practices are told to listen to the default pattern next to their fire alarm test and to change it if there is any resemblance.

## Drill toast

A small non-modal toast near the button: "Test alarm received - no action needed", for 10 s. Audible drill mode adds a 2 s soft chime.

## Tray icon and menu

Icon shows the status colour and shape. Menu:
- Status line (disabled item): "Connected to 14 devices"
- Open Panic Button (main window: status details, history, settings)
- Move button
- Room: Consulting Room 4 (opens room editor if allowed)
- Run test (admin only): runs a drill now
- Admin (admin only)
- About and source (AGPL)
- Quit is **hidden** unless policy `allow_quit = true` (default false in practice profile). Even then quitting requires confirmation: "If you quit, you will not receive alarms on this computer."

## Main window

Tabs: **Status** (status explanation, peers count by path, audio check with a "Play test sound" button, relay state), **History** (alarms and drills on this device), **Settings** (room, shortcut, alarm volume, user-level prefs allowed by policy), **Admin** (admin devices only, see below), **About**.

### Admin section

- **Coverage:** table per FR-STS-005, sortable and filterable. "Needs attention" filter (amber, red, offline over 1 day during opening hours, near misses over 3 in 24 h).
- **Devices:** pending join requests (approve or reject with code); device list with rename, set room, make admin, remove (non-cascade), compromised (cascade).
- **Enrolment tokens:** create (expiry, label), copy, save to file, revoke; list of devices per token.
- **Drills:** run now; weekly schedule; coordinator device; mode; reports with CSV export.
- **Practice settings:** cue mode, hold time, alarm pattern, relay URLs, seed peers, opening hours, drill settings.
- **Admins and recovery:** list of admins with a warning if fewer than 2; "Recover admin access"; "Create a new recovery kit" (requires the current kit, rotates nothing).
- **Audit and export:** CSV export of history and audit.

## First run

1. Welcome: "Panic Button lets you call colleagues for help from this computer." The agent is already running with no practice. It counts the computers it hears on the network by practice tag (`Core::nearby`, the last minute) and, if there are any, the welcome screen leads with "Panic Button is running on N other computers on this network" and an Ask to join button, which starts flow C. The practice name is not shown until an admin approves (it is only in sealed messages). Otherwise it says it is looking, then after 20 s that it found none.
2. If a token or config exists: enrol automatically and skip to step 4.
3. Choose: "Join an existing practice" (shows join code and waits for admin approval, or "Paste an enrolment code") or "Set up a new practice" (name, passphrase, save kit, confirm).
4. Room: "Which room is this computer in?" A text field with suggestions from rooms already known in the practice. Skipped if a room is set by policy.
5. Practice run: "Try it now: press and hold the button for 1.5 seconds. This is a practice and will not alert anyone." This runs the hold machine locally and does not raise an alarm. It teaches the gesture and the cue. When the hold completes, the core shows the ring full (`HoldProgress` 1000, then `HoldEnded`) and sends `PracticeComplete`, which enables Finish. The cue stops at that moment, as it does for a real alarm (ADR 0008).
6. Done. Show the button.

## Session and lifecycle

- **Autostart:** Windows `HKLM\...\Run` entry written by the MSI (per machine, all users); macOS LaunchAgent; Linux XDG autostart, or a systemd user unit for the headless agent.
- **Watchdog:**
  - Windows: on every start, the app creates or updates a per-user Scheduled Task, "Panic Button watchdog (<user>)", repeating every 5 minutes, that runs `panic-button-desktop.exe --ensure-running` (`platform::windows_watchdog`). The Run key starts the app at sign-in, so the task needs no logon trigger (which would need administrator rights). If an instance is running, the new process hands over to it and exits; the running app ignores a hand-over with `--ensure-running` instead of showing its window. Task names include the user because they are shared by every account. The MSI deletes all `Panic Button watchdog*` tasks on uninstall, but not on upgrade.
  - macOS: LaunchAgent `KeepAlive = true`.
  - Linux: XDG autostart plus a user timer. `panic-button-watchdog.timer` runs `panic-button-desktop --ensure-running` 2 minutes after sign-in and then 5 minutes after each check ends, only in a graphical session (`packaging/linux/`). The deb, rpm and AUR packages enable it for every user (`systemctl --global enable`) and disable it on removal, not on upgrade. A `Restart=always` service was rejected: when autostart has already started the app, the service's copy exits at once and systemd would restart it in a loop.
- **Remote sessions (Windows):** if `GetSystemMetrics(SM_REMOTESESSION)` is non-zero, or the session is not the console session, and `allow_remote_session = false`: show the button greyed with the tooltip "Panic Button cannot run inside a remote session. Ask IT to install it on this computer itself." Do not heartbeat as a participating device. Report `session = remote_session_disabled` if running.
- **Fast user switching (Windows):** only the agent in the active console session (`WTSGetActiveConsoleSessionId` equal to own session id, checked every 5 s and on `WM_WTSSESSION_CHANGE`) opens network sockets. Others stay dormant.
- **Locked screen:** keep running and keep sound. Heartbeat `session = locked`.
- **Detection:** the app checks the session every 5 s. Windows uses `SM_REMOTESESSION` and `WTSSessionInfoEx` (`platform::windows_session`); Linux uses logind's `LockedHint` and `Remote` for the graphical session (`loginctl show-session auto`) and treats `XRDP_SESSION` as remote; macOS reports active until it is implemented. Anything that cannot be read counts as active.
- **Sleep and wake:** on resume (OS power events or a wall-clock jump of more than 30 s between ticks), trigger anti-entropy (see protocol).
- **Updates:** managed installs (policy `updates = managed`) never self-update. Unmanaged installs may use the Tauri updater with signed update manifests from the OHTF release channel, off by default in v1.

## Platform notes

- **Windows:** WebView2 runtime is required (evergreen, normally present on Windows 10/11; the MSI checks and bootstraps it). The firewall rule is added by the MSI. Test at 100%, 125%, 150% and 200% scaling and with multiple monitors.
- **macOS:** `NSLocalNetworkUsageDescription` in Info.plist ("Panic Button uses the local network to send and receive alarms from other computers in your practice."). The first-run flow must trigger the Local Network permission prompt and detect denial (red status: "Local network access is turned off for Panic Button in System Settings > Privacy & Security > Local Network"). Button window level is `NSStatusWindowLevel`, visible on all Spaces.
- **Linux:** X11 is fully supported. On Wayland, always-on-top and global shortcuts depend on the compositor. Use the XDG GlobalShortcuts portal where available. If the button cannot be kept on top, fall back to tray plus shortcut and show amber: "On-screen button may be hidden behind windows on this desktop".
