// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineConfig, devices } from '@playwright/test'

const usePreviewServer = process.env.PLAYWRIGHT_WEB_SERVER === 'preview'
const baseURL = process.env.E2E_BASE_URL || 'http://localhost:8080'

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? [['html'], ['json', { outputFile: 'test-results.json' }]] : 'html',
  timeout: process.env.CI ? 300_000 : 60_000,
  use: {
    baseURL,
    trace: 'on-first-retry',
    navigationTimeout: process.env.CI ? 120_000 : 30_000,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer:
    process.env.CI || !usePreviewServer
      ? undefined
      : {
          command: 'npm run preview',
          url: 'http://localhost:4173',
          reuseExistingServer: !process.env.CI,
        },
})
