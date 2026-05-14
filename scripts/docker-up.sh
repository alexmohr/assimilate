#!/bin/sh

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
# Rebuild all images and (re)start the stack.
# Usage: ./scripts/docker-up.sh

set -eu

cd "$(dirname "$0")/.."

docker compose up --build --force-recreate --remove-orphans -d "$@"
