#!/bin/sh
# SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
# SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception
#
# deb prerm and rpm %preun: disable the watchdog timer when the package is
# removed, not when it is upgraded (deb "upgrade", rpm "1"); the new
# version's postinst enables it again.
case "$1" in
  remove | purge | 0) systemctl --global disable panic-button-watchdog.timer >/dev/null 2>&1 || true ;;
esac
