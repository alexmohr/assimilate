#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p docs/assets/screenshots
npx tsx e2e/seed-data.ts
cd frontend && npx playwright test ../e2e/screenshots.spec.ts --reporter=line
