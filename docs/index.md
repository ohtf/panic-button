# Panic Button

Panic Button puts a red button on the screen of every computer in a GP practice. Someone who needs help holds it for one and a half seconds, and every other computer in the building shows who needs help and where, and sounds an alarm. The person who called sees that the alarm got through and who is coming.

It runs on Windows, Mac and Linux, over the practice's own network. It needs no server, no subscription and no internet connection. It is free and open source, from the [Open Health Technology Foundation](https://openhealthtechnology.org).

![The red button, and the alert on a colleague's screen](images/alert.png)

Panic Button is not ready for real practices yet. Version 0.1 is for testing.

## Where to start

- Practice staff: [Calling for help](staff-guide.md), one page on using the button and answering an alert.
- Practice Admins: [Installing on Windows](install-windows.md) (or [all downloads](install.md)), then the [Practice Admin guide](admin-guide.md) for setting up the practice, adding computers and running test alarms.
- Trying it at home first: the [home test](home-test.md) sets up a test practice on two or three computers.
- IT: [Deploying with IT tools](it-deployment.md), for silent installs and settings.
- Developers: [the desktop app](gui/README.md), and the [specification](https://github.com/ohtf/panic-button/tree/main/spec) for how it works.

## What it does not do

Panic Button calls colleagues in the building. It does not call 999 or anyone outside the practice, so follow your practice's emergency procedures as well.

## Getting involved

The code, issues and plans are on [GitHub](https://github.com/ohtf/panic-button). If you test it, tell us what happened by [opening an issue](https://github.com/ohtf/panic-button/issues).
