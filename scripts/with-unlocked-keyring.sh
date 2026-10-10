#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
#
# Runs a command inside a throwaway D-Bus session with an unlocked
# gnome-keyring, the Secret Service desktop-core's keychain test needs. CI only:
# the keyring's password is a fixed, meaningless one.
set -euo pipefail

exec dbus-run-session -- bash -c \
    'echo -n ci | gnome-keyring-daemon --unlock --components=secrets >/dev/null && exec "$@"' \
    with-unlocked-keyring "$@"
