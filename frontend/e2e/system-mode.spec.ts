// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from './fixtures'
import type { SystemModeResponse } from '../src/types/generated/SystemModeResponse'

test('deployment mode is readable before login and defaults to server', async ({ page }) => {
  const resp = await page.request.get('/api/system/mode')
  expect(resp.status()).toBe(200)
  const body = (await resp.json()) as SystemModeResponse
  expect(body.mode).toBe('server')
})
