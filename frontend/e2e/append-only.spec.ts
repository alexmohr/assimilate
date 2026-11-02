// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, seededIdByName, test } from './fixtures'

/**
 * Append-only repositories (docs/append-only.md). The demo seeds
 * `vault-offsite` as the only repository of its SSH login (`borg-ao`), turned
 * on through the API the way the settings toggle does it, with one archive
 * deletion queued and a short maintenance history (`seed-demo.sh`).
 */
test.describe('Append-only repositories', () => {
  test('the settings section shows the keys, the check and the maintenance state', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    const vault = await seededIdByName(page, '/api/repos', 'vault-offsite')
    await page.goto(`/repos/${vault}?tab=settings&section=append-only`)
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await expect(page.locator('.settings-nav-item[aria-current="true"]')).toHaveText('Append-only')
    await expect(pane.getByRole('switch', { name: 'Append-only' })).toBeChecked()
    await expect(pane).toContainText('borg-ao@127.0.0.1')

    // The server set both keys up on the host when the seed turned it on.
    await expect(pane.getByRole('heading', { name: 'Set up by Assimilate' })).toBeVisible()
    await expect(pane.getByRole('heading', { name: 'Connection check' })).toBeVisible()

    // Maintenance: scheduled, one queued deletion, and the seeded history.
    await expect(pane.getByRole('heading', { name: 'Pending deletions' })).toBeVisible()
    await expect(pane.getByRole('heading', { name: 'Recent runs' })).toBeVisible()
    await expect(pane).toContainText('Check failed')
    await expect(pane.getByRole('button', { name: 'Run maintenance now' })).toBeEnabled()
  })

  test('the repository is marked as append-only in the list and its header', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.entity-card').filter({ hasText: 'vault-offsite' })
    await expect(card).toContainText('Append-only')

    // A repository on a login that is not append-only carries no badge.
    const plain = page.locator('.entity-card').filter({ hasText: 'server-daily' })
    await expect(plain).not.toContainText('Append-only')
  })

  test('a queued deletion stays in the archive list until maintenance and can be kept', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    const vault = await seededIdByName(page, '/api/repos', 'vault-offsite')
    await page.goto(`/repos/${vault}?tab=settings&section=append-only`)
    await page.waitForLoadState('networkidle')

    const pending = page.locator('.panel').filter({ hasText: 'Pending deletions' })
    await expect(pending.getByRole('button', { name: /^Keep / })).toHaveCount(1)
    await pending.getByRole('button', { name: /^Keep / }).click()
    await expect(pending.getByRole('button', { name: /^Keep / })).toHaveCount(0)

    // Queue it again from the API, as the delete button does, so the seed's
    // state is back for the other tests.
    const archives = (await (await page.request.get(`/api/repos/${vault}/archives`)).json()) as {
      name: string
    }[]
    expect(archives.length).toBeGreaterThan(0)
    const oldest = archives.map((a) => a.name).sort()[0]
    const res = await page.request.delete(
      `/api/repos/${vault}/archives/${encodeURIComponent(oldest)}`,
    )
    expect(res.status()).toBe(202)
    await page.reload()
    await expect(
      page
        .locator('.panel')
        .filter({ hasText: 'Pending deletions' })
        .getByRole('button', { name: /^Keep / }),
    ).toHaveCount(1)
  })

  test('the maintenance schedule is validated and saved', async ({ page }) => {
    await loginAsAdmin(page)
    const vault = await seededIdByName(page, '/api/repos', 'vault-offsite')

    const bad = await page.request.put(`/api/repos/${vault}/append-only/maintenance`, {
      data: { cron_expression: 'not a cron', check_before_prune: true },
    })
    expect(bad.status()).toBe(400)

    const saved = await page.request.put(`/api/repos/${vault}/append-only/maintenance`, {
      data: { cron_expression: '30 4 * * 0', check_before_prune: true },
    })
    expect(saved.status()).toBe(200)
    const body = (await saved.json()) as {
      enabled: boolean
      maintenance: { cron_expression: string }
    }
    expect(body.enabled).toBe(true)
    expect(body.maintenance.cron_expression).toBe('30 4 * * 0')
  })

  test('a repository that is not append-only offers to turn it on', async ({ page }) => {
    await loginAsAdmin(page)
    const daily = await seededIdByName(page, '/api/repos', 'server-daily')
    await page.goto(`/repos/${daily}?tab=settings&section=append-only`)
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await expect(pane.getByRole('switch', { name: 'Append-only' })).not.toBeChecked()
    // Nothing of the maintenance section exists until it is on.
    await expect(pane.getByRole('heading', { name: 'Recent runs' })).toHaveCount(0)
  })
})
