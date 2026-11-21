# Deploying with IT tools

This page is for IT staff installing Panic Button on many computers with Intune, Configuration Manager, Group Policy or a script. To install on one computer by hand, see [Installing on a Windows computer](install-windows.md).

Panic Button is pre-release software. Test it in your environment before relying on it.

## What to install

- The desktop app on every computer where staff should call for help or receive alarms. On Windows that is the MSI from the [latest release](https://github.com/ohtf/panic-button/releases/latest): `panic-button-desktop_<version>_x64_en-US.msi`.
- Optionally, the headless agent on an always-on Linux machine, such as a server or a Raspberry Pi, to receive and pass on alarms (see [Linux](#linux)).

Each computer joins the practice with an enrolment code. A Practice Admin creates one on the Admin tab, under Add a computer. One code enrols any number of computers until it expires. Treat it like a password: anyone with it can add a computer to the practice.

## Silent install on Windows

Save the enrolment code to a file the installing account can read, then:

```
msiexec /i panic-button-desktop_<version>_x64_en-US.msi /qn ENROLMENT_TOKEN_FILE="\\server\share\panic-button\practice.token" ROOM="Consulting Room 4"
```

| Property | What it does |
|---|---|
| `ENROLMENT_TOKEN_FILE` | Path to a file holding the enrolment code. The installer copies it to `C:\ProgramData\Panic Button\enrol.token`. The install runs as SYSTEM, so on a network share the computer account needs read access. |
| `ENROLMENT_TOKEN` | The enrolment code itself, instead of a file. It is kept out of the installer log. |
| `ROOM` | The room this computer is in, shown to colleagues when someone calls for help. Set this way, staff cannot change it on that computer; leave it out to let them choose. |
| `RELAY_URL` | A relay address, if the practice uses one, for example `wss://relay.example.org/v1/ws`. |
| `SEED_PEERS` | Comma-separated `address:port` list of other Panic Button computers, for networks where computers cannot find each other by themselves. |
| `FIREWALL_PUBLIC` | `1` also opens the firewall on the Public network profile. By default only Domain and Private are opened. |

If the enrolment code is not valid, the install fails with exit code 1603 and nothing is installed. Add `/l*v install.log` to the command to see why.

The installer:

- installs to `C:\Program Files\Panic Button`;
- adds inbound firewall rules for `panic-button-desktop.exe` on UDP and TCP port 47474 (Domain and Private profiles);
- starts Panic Button at every sign-in, for every user (`HKLM\Software\Microsoft\Windows\CurrentVersion\Run`);
- installs the Microsoft Edge WebView2 runtime if it is missing (this needs internet access);
- writes the properties to `C:\ProgramData\Panic Button\config.toml`.

When Panic Button first starts, it registers a per-user scheduled task, "Panic Button watchdog (username)", which starts it again within five minutes if it stops.

Use one Windows account per computer for now: Panic Button keeps its data in `C:\ProgramData\Panic Button`, and only the account that first runs it can write there.

The installer is not code-signed yet, so SmartScreen warns when it is run by hand, and publisher rules in AppLocker or WDAC cannot allow it. Use a path or hash rule until signed releases are available.

## Settings

Settings live in `C:\ProgramData\Panic Button\config.toml`, which users can read but not change. For example:

```toml
[device]
room = "Reception"

[alarm]
hotkey = "Ctrl+Shift+F12"

[ui]
allow_quit = false
```

Settings that apply to the whole practice, such as the hold time, alarm sound and weekly test alarm, are set by a Practice Admin on the Admin tab and reach every computer by themselves. The full list of settings is in the [deployment specification](https://github.com/ohtf/panic-button/blob/main/spec/08-deployment.md#settings-reference).

The specification also describes policy settings in the registry under `HKLM\SOFTWARE\Policies\OHTF\Panic Button`. Panic Button does not read them yet; use `config.toml`.

## Upgrading

Install the new MSI the same way. Settings, the computer's membership of the practice and its history are kept. Enrolment properties are only needed on the first install.

## Uninstalling

```
msiexec /x panic-button-desktop_<version>_x64_en-US.msi /qn
```

This removes the program, the firewall rules, the sign-in entry and the watchdog tasks. It keeps `C:\ProgramData\Panic Button`, so a reinstall rejoins the practice. Add `PURGE=1` to delete that folder too. Either way, a Practice Admin should remove the computer on the Admin tab.

## Network requirements

- UDP and TCP port 47474 between the practice's computers.
- Multicast to 239.255.71.71 on port 47474 helps computers find each other, but is not required: broadcast and `SEED_PEERS` also work.
- If the practice uses a relay: outbound TCP 443 to the relay.

Computers on different VLANs, or on Wi-Fi that keeps clients apart, need a relay or seed peers that can be reached. The Status tab on each computer says how it reaches the others.

## Linux

The desktop app comes as a deb, an rpm, an AppImage and an AUR package (`panic-button-bin`); see [Installing](install.md). The deb and rpm start Panic Button at sign-in and check every five minutes that it is running. Machine settings go in `/etc/panic-button/config.toml`, and an enrolment code can be given with:

```toml
[enrolment]
token_file = "/etc/panic-button/enrol.token"
```

For the headless agent, install the command-line programs (see [Installing](install.md#command-line-programs)) and the systemd unit [`panic-button-agent.service`](https://github.com/ohtf/panic-button/blob/main/packaging/linux/panic-button-agent.service). The unit's comments explain where the files go.

## When something is wrong

- The Status tab on each computer explains an amber or red status.
- Logs, one file per day: `C:\ProgramData\Panic Button\logs` on Windows, `~/.local/share/panic-button/logs` on Linux. They hold connection events and device names, not the content of alarms.
