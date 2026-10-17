// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test('admin enables metrics, creates a monitoring token and scrapes /metrics with it', async ({
  page,
  request,
}) => {
  await loginAsAdmin(page)

  // Enable the endpoint on the System page.
  await page.goto('/system')
  const metricsPanel = page.locator('.panel', {
    has: page.getByRole('heading', { name: 'Prometheus metrics' }),
  })
  const enabled = metricsPanel.locator('#metrics-enabled')
  await expect(enabled).toBeVisible({ timeout: 10_000 })
  await enabled.check()
  await Promise.all([
    page.waitForResponse(
      (resp) =>
        resp.url().endsWith('/api/system/metrics') &&
        resp.request().method() === 'PUT' &&
        resp.status() === 200,
    ),
    metricsPanel.getByRole('button', { name: 'Update metrics' }).click(),
  ])
  await expect(metricsPanel.locator('.save-success')).toBeVisible()

  // Create a monitoring token on the API tokens page.
  await page.goto('/tokens')
  await page.getByRole('button', { name: 'New' }).first().click()
  const name = `e2e-prometheus-${Date.now()}`
  await page.locator('#token-name').fill(name)
  await page.locator('#token-scope').selectOption('monitoring')
  const [created] = await Promise.all([
    page.waitForResponse(
      (resp) =>
        resp.url().endsWith('/api/tokens') &&
        resp.request().method() === 'POST' &&
        resp.status() === 200,
    ),
    page.getByRole('button', { name: 'Create' }).click(),
  ])
  const { token: createdToken } = (await created.json()) as { token: { id: number } }
  const token = (await page.locator('.token-text').textContent())?.trim() ?? ''
  expect(token).toMatch(/^[0-9a-f]{64}$/)
  await page.getByRole('button', { name: 'Done' }).click()
  await expect(page.locator('tr', { hasText: name }).locator('.badge')).toHaveText('Monitoring')

  // `request` carries no session cookie: only the token authenticates it.
  const scrape = await request.get('/metrics', {
    headers: { Authorization: `Bearer ${token}` },
  })
  expect(scrape.status()).toBe(200)
  expect(scrape.headers()['content-type']).toContain('text/plain; version=0.0.4')
  const body = await scrape.text()
  expect(body).toContain('# TYPE assimilate_agent_connected gauge')
  expect(body).toContain('assimilate_agent_connected{host="web-server-01"}')
  expect(body).toContain('assimilate_schedule_enabled{schedule_id=')

  // Without the token the scrape is refused, and the token reads nothing else.
  expect((await request.get('/metrics')).status()).toBe(401)
  const api = await request.get('/api/agents', {
    headers: { Authorization: `Bearer ${token}` },
  })
  expect(api.status()).toBe(401)

  // Leave the demo's token list as the seed made it.
  const removed = await page.request.delete(`/api/tokens/${createdToken.id}`)
  expect(removed.status()).toBe(200)
})
