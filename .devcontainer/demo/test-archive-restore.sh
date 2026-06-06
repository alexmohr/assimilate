#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -eu

BASE_URL="${BASE_URL:-http://localhost:8080}"
WORK_DIR=$(mktemp -d)
RESTORE_DIR=/tmp/assimilate-archive-restore-test
trap 'rm -rf "$WORK_DIR" "$RESTORE_DIR"' EXIT

curl -fsS -c "$WORK_DIR/cookies" \
    -H 'Content-Type: application/json' \
    -d '{"username":"admin","password":"admin"}' \
    "$BASE_URL/api/auth/login" > /dev/null

REPO_ID=$(curl -fsS -b "$WORK_DIR/cookies" "$BASE_URL/api/repos" \
    | jq -er '.[] | select(.name == "server-daily") | .id' | head -n 1)
ARCHIVE_NAME=$(curl -fsS -b "$WORK_DIR/cookies" "$BASE_URL/api/repos/$REPO_ID/archives" \
    | jq -er '[.[] | select(.hostname == "web-server-01")] | sort_by(.start) | last | .name')
ARCHIVE_ENCODED=$(jq -rn --arg value "$ARCHIVE_NAME" '$value|@uri')

curl -fsS -b "$WORK_DIR/cookies" \
    "$BASE_URL/api/repos/$REPO_ID/archives/$ARCHIVE_ENCODED/export" \
    -o "$WORK_DIR/archive.tar.lz4"
test "$(od -An -tx1 -N4 "$WORK_DIR/archive.tar.lz4" | tr -d ' \n')" = "04224d18"

mkdir -p "$RESTORE_DIR"
curl -fsS -b "$WORK_DIR/cookies" \
    -H 'Content-Type: application/json' \
    -d "{\"paths\":[],\"target_path\":\"$RESTORE_DIR\",\"hostname\":\"web-server-01\"}" \
    "$BASE_URL/api/repos/$REPO_ID/archives/$ARCHIVE_ENCODED/restore" \
    | jq -e '.success == true' > /dev/null

test -f "$RESTORE_DIR/restore-example.txt"
grep -q 'Restore this file from the archive browser.' "$RESTORE_DIR/restore-example.txt"

curl -fsS -X DELETE -b "$WORK_DIR/cookies" \
    "$BASE_URL/api/repos/$REPO_ID/archives/$ARCHIVE_ENCODED" \
    | jq -e '.success == true' > /dev/null

ARCHIVES_AFTER=$(curl -fsS -b "$WORK_DIR/cookies" "$BASE_URL/api/repos/$REPO_ID/archives")
printf '%s' "$ARCHIVES_AFTER" \
    | jq -e --arg name "$ARCHIVE_NAME" 'all(.[]; .name != $name)' > /dev/null

echo "Whole-archive download and restore passed for $ARCHIVE_NAME"
