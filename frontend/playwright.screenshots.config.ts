// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineConfig } from '@playwright/test'

// Captures the documentation screenshots under docs/assets/screenshots from a
// running demo environment (.devcontainer/demo). Kept out of the regular e2e
// run: the capture files use the `.screenshots.ts` suffix, which the default
// config's `*.spec.ts` match never picks up.
export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.screenshots.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  // Above the sum of the waits one test can make: agents reconnecting after
  // the seed's server restart and an archive index, each allowed 120s.
  timeout: 300_000,
  reporter: 'list',
  use: {
    baseURL: process.env.E2E_BASE_URL || 'http://localhost:8080',
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 2,
    colorScheme: 'light',
    locale: 'en-US',
    timezoneId: 'UTC',
    // Lets a machine with a preinstalled browser skip `playwright install`.
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_PATH
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH }
      : {},
  },
})
