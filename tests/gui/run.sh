#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
# SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception
#
# Runs tests/gui/runthrough.py (R35) against the release desktop app in an
# Arch Linux container with Xvfb and PulseAudio, driving it with tauri-driver
# and WebKitWebDriver. Arch, as tests/x11/run.sh, because binaries built on
# an up-to-date Arch host need its newer C library. Needs Docker, the
# release app (target/release/panic-button-desktop) and tauri-driver
# (cargo install tauri-driver --locked).

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${PANIC_BUTTON_BIN:-$ROOT/target/release}
DRIVER=${TAURI_DRIVER:-$(command -v tauri-driver || true)}
IMAGE=archlinux:base@sha256:d4594bb2300cba11f41c2f27923eb56f63d9988909c402f9c95517d1e362c899
[[ -x $BIN/panic-button-desktop ]] || { echo "missing $BIN/panic-button-desktop: run s/gui-test, which builds it" >&2; exit 2; }
[[ -x $DRIVER ]] || { echo "tauri-driver not found: cargo install tauri-driver --locked" >&2; exit 2; }
command -v docker >/dev/null || { echo "tests/gui needs Docker" >&2; exit 2; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$BIN/panic-button-desktop" "$DRIVER" "$ROOT/tests/gui/runthrough.py" "$work/"

# webkitgtk-6.0 provides WebKitWebDriver; it drives the app's
# webkit2gtk-4.1 web views, which are the same WebKit version.
exec docker run --rm -v "$work:/w:ro" "$IMAGE" bash -c '
  set -euo pipefail
  pacman -Syu --noconfirm --needed -q xorg-server-xvfb openbox python webkit2gtk-4.1 webkitgtk-6.0 \
    gtk3 alsa-lib libayatana-appindicator pulseaudio pulseaudio-alsa >/dev/null 2>&1
  Xvfb :99 -screen 0 1920x1080x24 >/dev/null 2>&1 &
  pulseaudio -D --exit-idle-time=-1 >/dev/null 2>&1
  sleep 2
  DISPLAY=:99 openbox >/dev/null 2>&1 &
  sleep 1
  timeout 900 python3 /w/runthrough.py
'
