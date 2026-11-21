# 08 Deployment and configuration

## Configuration sources and precedence

Highest wins, per key:

1. **Machine policy:** Windows registry `HKLM\SOFTWARE\Policies\OHTF\Panic Button`; macOS managed preferences (`/Library/Managed Preferences/org.ohtf.panic-button.plist`, from MDM configuration profiles); Linux `/etc/panic-button/policy.toml`.
2. **Machine config file:** Windows `%ProgramData%\Panic Button\config.toml`; macOS `/Library/Application Support/Panic Button/config.toml`; Linux `/etc/panic-button/config.toml`.
3. **Signed practice config** (PracticeConfig messages from admins).
4. **User preferences** (only keys marked user-editable and not locked by 1 to 3).
5. **Profile defaults** (`practice` or `home`).

`relay_urls` and `seed_peers` are merged as the union of sources 1 to 3 rather than overridden, so a local addition never removes a practice-wide relay.

## Settings reference

| Key | Type | Default (practice) | Default (home) | Sources | Notes |
|---|---|---|---|---|---|
| `profile` | enum | `practice` | | 1, 2 | |
| `enrolment.token` | string | | | 1, 2 | Secret; blanked after use if writable |
| `enrolment.token_file` | path | | | 1, 2 | Preferred over inline token |
| `device.room` | string | | | 1, 2, 4 | User-editable unless set by 1 or 2 |
| `device.label` | string | hostname | hostname | 1, 2 | |
| `device.user_display_source` | enum | `os_full_name` | `os_full_name` | 1, 2 | `os_full_name`, `os_username`, `room_only` |
| `network.port` | u16 | 47474 | 47474 | 1, 2 | |
| `network.multicast_group` | IPv4 | 239.255.71.71 | same | 1, 2 | |
| `network.multicast_ttl` | u8 | 1 | 1 | 1, 2 | |
| `network.seed_peers` | [host:port] | [] | [] | 1, 2, 3 | Union |
| `network.relay_urls` | [url] | [] | [] | 1, 2, 3 | Union |
| `network.relay_ca_pem` | path | | | 1, 2 | |
| `alarm.hold_ms` | u32 | 1500 | 1500 | 1, 3 | 1000-3000 |
| `alarm.cue_mode` | enum | `soft` | `alarm_ramp` | 1, 3 | `silent`, `soft`, `alarm_ramp` |
| `alarm.sound_pattern` | enum | `A` | `A` | 1, 3 | `A`, `B`, `C` |
| `alarm.volume_pct` | u8 | 100 | 100 | 1, 4 | |
| `alarm.override_mute` | bool | true | false | 1, 3 | Windows only in v1 |
| `alarm.hotkey` | string | `Ctrl+Shift+F12` | same | 1, 4 | `""` or `none` turns it off. Not available on Wayland |
| `ui.floating_button.allow_hide` | bool | false | true | 1 | |
| `ui.allow_quit` | bool | unset | unset | 1 | Unset: Quit in Settings; the home profile also has it in the tray. `true`: in the tray too. `false`: no Quit at all |
| `drill.schedule` | string | | | 3 | e.g. `Tue 07:45` (local time) |
| `drill.mode` | enum | `silent` | `audible` | 3 | |
| `drill.coordinator` | DeviceId | | | 3 | |
| `practice.opening_hours` | string | `Mon-Fri 08:00-18:30` | | 3 | For FR-STS-006 and coverage filters |
| `policy.allow_remote_session` | bool | false | false | 1 | |
| `policy.updates` | enum | `managed` | `auto` | 1 | |
| `policy.allow_cli_raise` | bool | false | false | 1 | Allows `panic-button ctl raise` on non-test builds |

Registry value types: strings as `REG_SZ`, booleans and integers as `REG_DWORD`, lists as `REG_MULTI_SZ`. Key names use the dotted path with dots replaced by backslash subkeys, for example `HKLM\SOFTWARE\Policies\OHTF\Panic Button\alarm\cue_mode`.

## Example machine config

```toml
# C:\ProgramData\Panic Button\config.toml
profile = "practice"

[enrolment]
token_file = 'C:\ProgramData\Panic Button\enrol.token'

[device]
room = "Reception"

[network]
seed_peers = ["10.20.1.10:47474"]
relay_urls = ["wss://relay.example-pcn.org.uk/v1/ws"]
```

## Windows

### MSI

- Built with the Tauri bundler (WiX 3/4 backend) plus custom fragments in `packaging/windows/`.
- Per-machine install to `%ProgramFiles%\Panic Button`. Requires elevation (expected under SCCM, Intune or GPO).
- Public properties:

| Property | Meaning |
|---|---|
| `ENROLMENT_TOKEN` | Inline token (marked in `MsiHiddenProperties` so it is not logged) |
| `ENROLMENT_TOKEN_FILE` | Path to a token file (preferred) |
| `RELAY_URL` | One relay URL (more via policy) |
| `SEED_PEERS` | Comma-separated host:port list |
| `ROOM` | Room for this machine |
| `FIREWALL_PUBLIC` | `1` to also open the port on the Public firewall profile (default `0`; Domain and Private only) |

- The MSI writes the properties into `%ProgramData%\Panic Button\config.toml`. ACL: Administrators and SYSTEM full, Users read. The token is written to `enrol.token` with Users read access (needed for per-user first run). The agent deletes it after enrolment.
- As built: a deferred custom action runs `panic-button-desktop.exe --install-settings` as SYSTEM, which merges the properties into any existing config (`setup::write_install_settings`): a token, inline or read from `ENROLMENT_TOKEN_FILE`, is checked and copied to `enrol.token`, and the config names that file; `RELAY_URL` and `SEED_PEERS` are added to any already there. An invalid token fails the install. `ENROLMENT_TOKEN` and the action's command line are hidden from the MSI log. The files inherit `%ProgramData%`'s permissions, so users can read but not delete `enrol.token`; it stays until it expires or an administrator removes it.
- Firewall: inbound rule for `panic-button-desktop.exe`, UDP and TCP on `network.port`, Domain and Private profiles (Public optional).
- Autostart: `HKLM\Software\Microsoft\Windows\CurrentVersion\Run\Panic Button`.
- WebView2: evergreen bootstrapper if missing.
- Upgrades: major upgrade with the same UpgradeCode. Preserves `%ProgramData%\Panic Button`.
- Uninstall removes the program, firewall rule and Run key. It keeps `%ProgramData%\Panic Button` unless `PURGE=1`.

Silent install example:
```
msiexec /i Panic Button-1.0.0-x64.msi /qn ENROLMENT_TOKEN_FILE="\\server\share\panic-button\practice.token" ROOM="Consulting Room 4"
```

### For IT

- Code-signed with the OHTF certificate. AppLocker or WDAC publisher rules can allow by publisher.
- An ADMX/ADML template is planned for post-v1. In v1, policies are plain registry values (document a `.reg` example and an Intune OMA-URI/registry CSP example).
- Network requirements: UDP and TCP 47474 within the practice LAN; multicast 239.255.71.71 if allowed (optional); outbound TCP 443 to the relay host if used.

## macOS

- Signed and notarised `.pkg` installing `/Applications/Panic Button.app`, a LaunchAgent at `/Library/LaunchAgents/org.ohtf.panic-button.plist` (KeepAlive) and the optional config file.
- MDM: configuration profile with a `org.ohtf.panic-button` payload (same keys as the settings reference) and a PPPC profile where useful. Local Network permission cannot be pre-granted by MDM, so first run must prompt (see `05-desktop-agent.md`).

## Linux

- `.deb` (and AppImage for others). The desktop app is installed with an XDG autostart entry.
- Headless agent: `panic-button-agent` binary with a systemd unit (`panic-button-agent.service`, `User=panic-button`, `Restart=always`), config at `/etc/panic-button/config.toml`, state at `/var/lib/panic-button`.

## Relay hosting

See `06-relay.md`. A `packaging/relay/docker-compose.yml` example:

```yaml
services:
  relay:
    image: ghcr.io/<org>/panic-button-relay:1
    restart: unless-stopped
    volumes:
      - ./relay.toml:/etc/panic-button-relay/relay.toml:ro
      - relay-data:/var/lib/panic-button-relay
  caddy:
    image: caddy:2
    restart: unless-stopped
    ports: ["443:443"]
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy-data:/data
volumes:
  relay-data: {}
  caddy-data: {}
```

`Caddyfile`:
```
relay.example.org.uk {
  reverse_proxy relay:8443
}
```

## Documentation to ship with v1

- **Quick guide for staff** (one page, printable): how to raise an alarm, what the status light means, how to respond, how to cancel if sent in error.
- **Practice Admin guide:** setup, recovery kit storage, adding and removing devices, drills, reading coverage, what to do when amber or red.
- **IT deployment guide:** MSI, policies, firewall, relay options, troubleshooting (with a network diagnostics export).
- **Relay operator guide:** install, tenants, metadata exposure, AGPL obligations.
- **Clinical safety case summary** and the hazard log.
