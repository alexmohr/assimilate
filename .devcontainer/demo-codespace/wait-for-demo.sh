#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
#
# postStartCommand of the Codespaces live demo. The demo container seeds
# itself on its first start (../demo/start-demo.sh) and leaves a marker in
# the seed volume once it is done; this waits for that marker and for the
# server to answer, then says where to log in.
set -e

URL="${DEMO_URL:-http://localhost:8080}"
SEEDED_MARKER="${DEMO_SEEDED_MARKER:-/seeds/demo-seeded}"
TIMEOUT_SECS="${DEMO_WAIT_TIMEOUT_SECS:-900}"

echo "==> Waiting for the Assimilate demo to finish seeding (up to $((TIMEOUT_SECS / 60)) minutes)..."
elapsed=0
# The seed runs against a temporary server on the same port, so the health
# check alone could be answered by that one. The marker is written only after
# it has stopped.
until [ -f "$SEEDED_MARKER" ] && curl -sf -o /dev/null "$URL/api/health"; do
    if [ "$elapsed" -ge "$TIMEOUT_SECS" ]; then
        echo "The demo did not come up within $((TIMEOUT_SECS / 60)) minutes." >&2
        echo "Rebuild the codespace (Codespaces: Rebuild Container) to start it again." >&2
        exit 1
    fi
    sleep 5
    elapsed=$((elapsed + 5))
done

echo ""
echo "Assimilate demo ready: open the forwarded port 8080 (Ports tab)."
echo "Login: admin / admin"
echo "A guided tour is in DEMO.md."
echo ""
