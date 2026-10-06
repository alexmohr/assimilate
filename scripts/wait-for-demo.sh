#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

# Waits until the demo container (.devcontainer/demo) has finished seeding,
# for up to 40 minutes. Fails early, with the container's last log lines, if
# the demo container stops. Pass the compose files as $COMPOSE_FILES, e.g.
#
#   COMPOSE_FILES="-f .devcontainer/demo/docker-compose.demo.yml" \
#     scripts/wait-for-demo.sh

set -euo pipefail

: "${COMPOSE_FILES:?set COMPOSE_FILES to the demo compose file flags}"
read -r -a COMPOSE <<<"$COMPOSE_FILES"

demo_container() {
    docker compose "${COMPOSE[@]}" ps -qa demo 2>/dev/null | head -1
}

echo "Waiting for demo seed to complete (up to 40 min)..."
for _ in $(seq 1 480); do
    if docker compose "${COMPOSE[@]}" logs demo 2>&1 | grep -q "Demo ready:"; then
        echo "Demo is seeded and ready"
        exit 0
    fi
    CONTAINER_ID=$(demo_container)
    if [ -n "$CONTAINER_ID" ]; then
        STATE=$(docker inspect --format '{{.State.Status}}' "$CONTAINER_ID" 2>/dev/null || echo "unknown")
        if [ "$STATE" = "exited" ] || [ "$STATE" = "dead" ]; then
            echo "ERROR: Demo container stopped unexpectedly (state: $STATE)"
            docker logs "$CONTAINER_ID" --tail 100
            exit 1
        fi
    fi
    sleep 5
done

echo "ERROR: Timed out waiting for demo to be ready"
CONTAINER_ID=$(demo_container)
[ -n "$CONTAINER_ID" ] && docker logs "$CONTAINER_ID" --tail 100
exit 1
