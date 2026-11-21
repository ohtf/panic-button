# Installing on a Windows computer

Install Panic Button on each computer that should call for help or receive alarms. It takes about five minutes per computer. You need an administrator account on the computer.

If your IT provider installs Panic Button for you, you do not need this page. Point them to [Deploying with IT tools](it-deployment.md).

## Before you start

Make sure Windows treats the practice network as private: open Settings, then Network and internet, choose your Wi-Fi or Ethernet connection, and choose Private network. Panic Button's firewall rules only apply to private and work networks, so on a public network alarms would not get through.

## Install

1. Open the [latest release](https://github.com/ohtf/panic-button/releases/latest) and, under Assets, download the file that starts `panic-button-desktop` and ends `x64_en-US.msi`. The `.zip` file on the same page is a set of tools for IT, not the app.
2. Open the downloaded file. Windows may say "Windows protected your PC", because the installer is not signed yet. Click More info, then Run anyway.
3. Click Yes when Windows asks for administrator permission.
4. On the last screen, leave Launch Panic Button ticked and click Finish.

Panic Button now starts whenever anyone signs in to this computer, and Windows checks every five minutes that it is still running.

## Set it up

The Panic Button window opens. What you do next depends on whether your practice already uses Panic Button.

If it does, the window usually says "Panic Button is running on N other computers on this network":

1. Click Ask to join. The window shows a six-digit code.
2. Ask your Practice Admin to open Panic Button on their computer and go to the Admin tab. They check the code matches, and click Approve.
3. Click Join, then choose the room this computer is in.

If your Practice Admin gave you an enrolment code instead, choose Join an existing practice and paste it.

If this is the first computer in your practice, choose Set up a new practice and follow the [Practice Admin guide](admin-guide.md#setting-up-the-practice).

Then try the practice hold: press and hold the red button until the ring is full. It does not alert anyone. Click Finish.

## Check it works

The red button sits in the bottom right corner of the screen. The small shape on it should be a green circle within a minute. If it is an amber triangle or a red square, see [the shape on the button](staff-guide.md#the-shape-on-the-button).

## Removing Panic Button

Open Settings, then Apps, then Installed apps, find Panic Button and choose Uninstall. This computer stays listed in your practice until a Practice Admin removes it on the Admin tab.
