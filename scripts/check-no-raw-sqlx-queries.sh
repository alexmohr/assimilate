#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

# Fails if any Rust source file uses sqlx's runtime (non-compile-time-checked)
# query constructors. Per issue #257, the database schema is the source of
# truth for API types: every query must go through query!/query_as!/query_scalar!
# so a schema change that isn't reflected in the Rust types fails the build,
# rather than drifting silently. See AGENTS.md ("sqlx Offline Cache").
set -euo pipefail

PATTERN='sqlx::query(_as|_scalar)?\(|sqlx::query_as::<'

matches=$(grep -rEn "$PATTERN" --include='*.rs' crates/*/src || true)

if [ -n "$matches" ]; then
  echo "error: found runtime (non-compile-time-checked) sqlx query calls:" >&2
  echo "$matches" >&2
  echo >&2
  echo "Use the compile-time macros instead: sqlx::query!(...), sqlx::query_as!(...), sqlx::query_scalar!(...)." >&2
  echo "See AGENTS.md's 'sqlx Offline Cache' section for how to regenerate .sqlx/ after query changes." >&2
  exit 1
fi
