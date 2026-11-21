#!/bin/sh
# SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
# SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception
#
# deb postinst and rpm %post: enable the watchdog timer for every user. It
# takes effect at each user's next sign-in.
systemctl --global enable panic-button-watchdog.timer >/dev/null 2>&1 || true
