#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
#
# Runs the server's install.sh against a live server and checks what it leaves
# behind: an enrollment token is created through the API, the script installs
# and enrolls, a second run with the same token is refused, and the agent
# connects with the token the script stored.
#
#   scripts/test-install-script.sh http://127.0.0.1:8080
#   scripts/test-install-script.sh http://127.0.0.1:8080 debian:13
#
# Without an image the script runs on this host, as root. With one, it runs in
# a container on the host network, which is how CI tests it on Debian 13.
set -euo pipefail

SERVER="${1:?usage: $0 <server-url> [container-image]}"
IMAGE="${2:-}"
ADMIN_USER="${ADMIN_USER:-admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin}"
NEW_PASSWORD="${NEW_PASSWORD:-install-test-password-1}"
HOSTNAME_UNDER_TEST="install-test-host"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

COOKIES="$(mktemp)"
trap 'rm -f "$COOKIES"' EXIT

api() {
    # api METHOD PATH [JSON BODY]
    local method="$1" path="$2"
    shift 2
    if [ "$#" -gt 0 ]; then
        curl -fsS -b "$COOKIES" -c "$COOKIES" -X "$method" "$SERVER$path" \
            -H 'Content-Type: application/json' -d "$1"
    else
        curl -fsS -b "$COOKIES" -c "$COOKIES" -X "$method" "$SERVER$path"
    fi
}

login() {
    curl -fsS -c "$COOKIES" -X POST "$SERVER/api/auth/login" \
        -H 'Content-Type: application/json' \
        -d "$(jq -n --arg u "$ADMIN_USER" --arg p "$1" '{username: $u, password: $p}')"
}

echo "==> Logging in"
if login "$ADMIN_PASSWORD" | jq -e '.user.must_change_password == true' >/dev/null; then
    api POST /api/auth/change-password "$(jq -n --arg p "$NEW_PASSWORD" '{new_password: $p}')" >/dev/null
    login "$NEW_PASSWORD" >/dev/null
fi

SERVER_KEY="$(api GET /api/system/agent-channel-key | jq -r .public_key)"
[ -n "$SERVER_KEY" ] || fail "the server returned no agent channel key"

echo "==> Creating an enrollment token"
CREATED="$(api POST /api/enrollment-tokens "$(jq -n --arg h "$HOSTNAME_UNDER_TEST" '{lifetime_secs: 900, hostname: $h}')")"
TOKEN="$(jq -r .plaintext <<<"$CREATED")"
TOKEN_ID="$(jq -r .token.id <<<"$CREATED")"
case "$TOKEN" in enr_*) ;; *) fail "the token does not start with enr_" ;; esac

# What runs on the host being installed: the script, its results, a second run
# with the same token, and the agent starting with the stored token.
INNER=$(cat <<'INNER_EOF'
set -eu
if ! command -v curl >/dev/null 2>&1; then
    apt-get update -qq && apt-get install -y -qq curl ca-certificates >/dev/null
fi
RUN="curl -fsSL $SERVER/install.sh | sh -s -- --server $SERVER --token $TOKEN --server-key $SERVER_KEY --skip-service"
sh -c "$RUN"

test -x /usr/local/bin/assimilate-agent || { echo "FAIL: the agent is not installed"; exit 1; }
test "$(stat -c %a /etc/assimilate-agent.env)" = 600 || { echo "FAIL: the token file is not mode 600"; exit 1; }
grep -Eq "^BORG_AGENT_TOKEN=[0-9a-f]{64}$" /etc/assimilate-agent.env || { echo "FAIL: no agent token stored"; exit 1; }
! grep -q "$TOKEN" /etc/assimilate-agent.env || { echo "FAIL: the enrollment token was stored"; exit 1; }
grep -q "^Environment=BORG_SERVER_KEY=$SERVER_KEY" /etc/systemd/system/assimilate-agent.service \
    || { echo "FAIL: the unit does not pin the server key"; exit 1; }
grep -q "^WantedBy=multi-user.target" /etc/systemd/system/assimilate-agent.service \
    || { echo "FAIL: the unit does not start at boot"; exit 1; }

# The token is spent: running again, even with --force, must be refused.
if sh -c "$RUN --force" >/dev/null 2>&1; then
    echo "FAIL: a used enrollment token was accepted"; exit 1
fi

# Start the agent the way the unit would, long enough for it to connect.
set -a; . /etc/assimilate-agent.env; set +a
BORG_SERVER_URL="$SERVER" BORG_SERVER_KEY="$SERVER_KEY" timeout 25 /usr/local/bin/assimilate-agent >/dev/null 2>&1 &
sleep 22
INNER_EOF
)

echo "==> Running install.sh${IMAGE:+ in $IMAGE}"
if [ -n "$IMAGE" ]; then
    docker run --rm --network host -e SERVER="$SERVER" -e TOKEN="$TOKEN" -e SERVER_KEY="$SERVER_KEY" \
        -e BORG_HOSTNAME="$HOSTNAME_UNDER_TEST" "$IMAGE" sh -c "$INNER" &
else
    SERVER="$SERVER" TOKEN="$TOKEN" SERVER_KEY="$SERVER_KEY" BORG_HOSTNAME="$HOSTNAME_UNDER_TEST" \
        sh -c "$INNER" &
fi
INSTALL_PID=$!

echo "==> Waiting for $HOSTNAME_UNDER_TEST to connect"
CONNECTED=no
for _ in $(seq 1 40); do
    if api GET /api/agents | jq -e --arg h "$HOSTNAME_UNDER_TEST" '.[] | select(.hostname == $h and .is_connected)' >/dev/null; then
        CONNECTED=yes
        break
    fi
    sleep 1
done
wait "$INSTALL_PID" || fail "install.sh or its checks failed"
[ "$CONNECTED" = yes ] || fail "$HOSTNAME_UNDER_TEST never connected with the token install.sh stored"

STATUS="$(api GET /api/enrollment-tokens | jq -r --argjson id "$TOKEN_ID" '.tokens[] | select(.id == $id) | .status + ":" + (.used_by // "")')"
[ "$STATUS" = "used:$HOSTNAME_UNDER_TEST" ] || fail "the token is listed as '$STATUS', expected used:$HOSTNAME_UNDER_TEST"

echo "PASS: enrolled $HOSTNAME_UNDER_TEST, the token is used, and a second use was refused"
