# Home test on a few Windows and Linux computers

This is a guide for a small test before Panic Button goes near a practice. It covers getting the installers, setting up a test practice on one computer, adding the others, and what to try.

Use computers on the same home network. Panic Button finds the others on the network by itself, so nothing else needs setting up.

## Getting the installers

Download them from the [latest release](https://github.com/ohtf/panic-button/releases/latest), under Assets. `<version>` is the release number, for example 0.2.1:

- `panic-button-desktop_<version>_x64_en-US.msi` for Windows;
- `panic-button-desktop_<version>_amd64.AppImage` for any 64-bit Linux;
- `panic-button-desktop_<version>_amd64.deb` for Debian and Ubuntu.

The Windows `.zip` on that page is the command-line programs for IT staff, not the app.

On Arch you can install `panic-button-bin` from the AUR instead. [install.md](install.md) lists every other way.

## Installing on Windows

1. Before you start, make sure Windows treats your home network as private: Settings, Network and internet, then your Wi-Fi or Ethernet connection, and choose Private network. The installer only opens Panic Button's port on private and work networks.
2. Run the MSI. The installer is not signed yet, so Windows shows "Windows protected your PC". Click More info, then Run anyway. It asks for administrator permission.
3. On the last screen, leave Launch Panic Button ticked and click Finish. Panic Button also starts whenever anyone signs in.

To remove it, use Settings, Apps, Installed apps, Panic Button, Uninstall. That also removes its firewall rules. Your test practice stays in `C:\ProgramData\Panic Button`; delete that folder for a clean start.

Use one Windows account per computer for this test. Panic Button keeps its data in `C:\ProgramData\Panic Button`, and for now only the account that first ran it can write there.

## Installing on Linux

On Arch, or anything that is not Debian or Ubuntu, use the AppImage:

```sh
mkdir -p ~/Applications
mv ~/Downloads/panic-button-desktop_*_amd64.AppImage ~/Applications/panic-button-desktop.AppImage
chmod +x ~/Applications/panic-button-desktop.AppImage
~/Applications/panic-button-desktop.AppImage
```

If AppImageLauncher is installed, it asks whether to integrate the AppImage. Either answer works.

To start it when you sign in:

```sh
mkdir -p ~/.config/autostart
printf '[Desktop Entry]\nType=Application\nName=Panic Button\nExec=%s\n' \
  ~/Applications/panic-button-desktop.AppImage > ~/.config/autostart/panic-button.desktop
```

On Debian or Ubuntu, install the deb instead (`sudo apt install ./panic-button-desktop_*_amd64.deb`). It starts Panic Button at sign-in by itself.

If a firewall is running (for example `ufw` or `firewalld`), allow UDP and TCP port 47474.

On Wayland the desktop decides where windows go, so the red button may not sit in the corner. On niri, the window rules in [gui/README.md](gui/README.md#wayland) put it there.

## Setting up the test practice

Pick one computer to be the first admin.

1. In the Panic Button window, choose Set up a new practice.
2. Give it a name, a name for this computer, and a passphrase of at least 12 characters. Save the recovery kit where it suggests. For a test, any passphrase will do, but keep the kit until the test is over: it is the only way to get admin access back.
3. Choose a room, then try the practice hold. It does not alert anyone.
4. Open the Admin tab, and under Add a computer, click Create code. Copy the code.

On each of the other computers:

1. In the Panic Button window, choose Join an existing practice.
2. Paste the code, then choose a room.

Send the code to yourself in whatever way is handy (a note, a message to yourself). Anyone who has it can join your test practice until it expires, which does not matter at home but would in a practice.

Add at least one computer the other way, without a code. Its welcome screen should say Panic Button is running on the other computers within half a minute; click Ask to join. It shows a six-digit code. On the admin computer, the Admin tab lists it under Waiting to join; check the codes match and click Approve, then click Join on the new computer.

Within half a minute, the small shape on each button should turn into a green circle. The Status tab on any computer lists the others.

## Quitting during the test

Panic Button has no Quit option by default, because a computer that has quit cannot receive alarms. To add one, create a settings file with these two lines:

```toml
[ui]
allow_quit = true
```

Save it as `C:\ProgramData\Panic Button\config.toml` on Windows (Notepad needs to be run as administrator), or `/etc/panic-button/config.toml` on Linux. Then sign out and back in. The tray menu and the About tab then have Quit Panic Button, which asks you to confirm.

## What to try

Call for help from each computer in turn. Hold the red button until the ring fills (one and a half seconds). Every other computer should show the red Help needed window and sound the alarm within two seconds. Click I'm coming on one of them, and check the computer you called from shows who is coming. Then click Resolved.

Switch one computer off, or unplug its network cable. Within a minute, the others should count one device fewer, and if only two computers are in the test, the remaining one turns red. The Admin tab shows when each computer was last heard. Call for help while one is off, resolve it, then turn that computer back on. It should not ring for an alarm that has already been resolved.

Run a test alarm from the Admin tab. The others should show "Test alarm received - no action needed" without the alarm sound. After a minute the report on the Admin tab shows which computers answered.

Try a few things that went wrong for the commercial products: put a computer to sleep and wake it, turn its sound off, or move a laptop to another Wi-Fi network. Note what each status light says.

Mute a Windows computer, then call for help from another. The muted one should still sound the alarm. When you click I'm coming, it should go quiet and be muted again. On Linux, a muted computer should turn amber instead.

Hold Ctrl, Shift and F12 together for one and a half seconds, in another program. It should call for help the same way as the button. On some Wayland desktops it will not; the Settings tab says so.

Move the button with Shift and drag, then sign out and back in. It should come back where you left it.

Lock one computer's screen (Windows key and L, or your desktop's lock shortcut), then call for help from another. The locked computer should still sound the alarm.

If you can, connect to a Windows computer with Remote Desktop and open Panic Button there. The button should turn grey and say it cannot run inside a remote session.

Rename a computer or change its room from the Admin tab on the admin computer, and check the change appears on the others.

## What to send back

For anything odd, note the time, what you did and what you saw on each computer. The logs help:

- Windows: `C:\ProgramData\Panic Button\logs`
- Linux: `~/.local/share/panic-button/logs`

There is one file per day. They hold connection events and device names, not the content of alarms.

## Known gaps in this version

- The installer is unsigned, so Windows warns about it.
- On Windows, only one account per computer can use Panic Button.
- On Wayland desktops, the keyboard shortcut (hold Ctrl+Shift+F12) works only where the desktop provides shortcuts to apps (GNOME, KDE and Hyprland should; niri does not). Settings says whether it works on a computer.
- On Linux, a muted computer turns amber within about 10 seconds. On Windows, Panic Button unmutes the computer and turns the volume up to at least 60% while an alarm sounds, then puts both back; this has not been tried on a real Windows computer yet.

## Building the MSI on a Windows PC

To test a change that has not been released, build the MSI on one of the Windows computers. It takes about half an hour the first time.

1. Install [Rust](https://rustup.rs) (accept the offer to install the Visual Studio C++ build tools), [Node.js 24](https://nodejs.org), and [Git](https://git-scm.com).
2. In PowerShell:

   ```powershell
   git clone https://github.com/ohtf/panic-button.git
   cd panic-button\apps\desktop
   npm ci
   npx tauri build --bundles msi
   ```

3. The MSI is in `panic-button\target\release\bundle\msi`. Copy it to the other Windows computers.
