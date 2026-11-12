// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, test } from './fixtures'

test.describe('Settings journey', () => {
  test('global excludes page loads with common patterns', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/excludes')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Global Excludes' })).toBeVisible()
    // The patterns read as a list, one per row; editing is behind Edit.
    const patterns = page.getByRole('list', { name: 'Global exclude patterns' })
    await expect(patterns).toBeVisible({ timeout: 15_000 })
    await expect(patterns).toContainText('node_modules')
    await expect(patterns).toContainText('__pycache__')
    await expect(page.getByRole('textbox')).toHaveCount(0)

    // The pattern reference opens from inside the card.
    await page.getByRole('button', { name: 'Pattern reference' }).click()
    await expect(page.getByRole('button', { name: 'Close pattern reference' })).toBeVisible()
  })

  test('global excludes warns about a pattern anchored at the backup root', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/excludes')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('list', { name: 'Global exclude patterns' })).toContainText(
      'node_modules',
      { timeout: 15_000 },
    )
    await page.getByRole('button', { name: 'Edit' }).click()
    const patterns = page.getByRole('textbox', { name: 'Global exclude patterns' })
    await expect(async () => {
      expect(await patterns.inputValue()).toContain('node_modules')
    }).toPass({ timeout: 15_000 })

    // The seeded patterns all begin with "/" or "*", so none is anchored.
    const warnings = page.getByRole('list', { name: 'Pattern warnings' })
    await expect(warnings).toHaveCount(0)

    await patterns.fill('.cache\n*/.cache')
    await expect(warnings).toBeVisible()
    await expect(warnings).toContainText('Line 1')
    await expect(warnings).toContainText('"*/.cache"')
    await expect(warnings.getByRole('listitem')).toHaveCount(1)

    // Not saved: the seeded patterns stay as they are.
    await page.getByRole('button', { name: 'Cancel' }).click()
    await expect(patterns).toHaveCount(0)
  })

  // The demo seeds agent tokens, not personal API tokens, so this page is
  // genuinely empty - the state has to say what a token is for and offer the
  // way to make one, not just report the absence.
  test('profile offers token creation from the empty API-token list', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/profile')
    await page.waitForLoadState('networkidle')

    const tokens = await openSettingsSection(page, 'API tokens')

    const empty = tokens.locator('.empty-state')
    await expect(empty.locator('.empty-title')).toHaveText('No API tokens yet')
    await expect(empty.locator('.empty-description')).toContainText(
      'authenticate without your password',
    )

    await empty.locator('.empty-action').click()
    await expect(page.locator('.modal-title')).toHaveText('New API token')
  })

  // Runs after the empty-list test above (CI runs one worker, a file's tests
  // in order) and removes what it creates, so that list is empty again.
  test('an API token can be given an expiry, which its row shows', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/profile')
    await page.waitForLoadState('networkidle')
    const tokens = await openSettingsSection(page, 'API tokens')

    const name = `e2e-expiring-${Date.now()}`
    await tokens.getByRole('button', { name: 'New token' }).first().click()
    const dialog = page.getByRole('dialog', { name: 'New API token' })
    await dialog.getByPlaceholder('e.g. CI pipeline').fill(name)
    // Never is the default; a preset length says when that is before it is made.
    const expiry = dialog.getByLabel('Expires')
    await expect(expiry).toHaveValue('never')
    await expect(dialog).toContainText('Works until it is deleted.')
    await expiry.selectOption('90d')
    await expect(dialog).toContainText('Stops working on')

    const [created] = await Promise.all([
      page.waitForResponse(
        (resp) => resp.url().endsWith('/api/tokens') && resp.request().method() === 'POST',
      ),
      dialog.getByRole('button', { name: 'Create' }).click(),
    ])
    expect(created.ok()).toBe(true)
    const body = (await created.json()) as { token: { id: number; expires_at: string | null } }
    try {
      expect(body.token.expires_at).not.toBeNull()
      // The created token's notice repeats when it stops working.
      const done = page.getByRole('dialog', { name: 'Token created' })
      await expect(done).toContainText('It stops working on')
      await done.getByRole('button', { name: 'Done' }).click()

      const row = tokens.locator('tbody tr').filter({ hasText: name })
      await expect(row).toBeVisible()
      const expires = row.locator('td[data-label="Expires"]')
      await expect(expires).not.toHaveText('Never')
      await expect(expires).not.toBeEmpty()
    } finally {
      const removed = await page.request.delete(`/api/tokens/${body.token.id}`)
      expect(removed.ok()).toBe(true)
    }
  })

  test('notifications page shows configured channels', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/notifications')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Notifications' })).toBeVisible()
    // Each channel is a row naming its type, and Paused when it is switched
    // off; pausing and resuming live on the channel's own page.
    const webhook = page.locator('.list-row').filter({
      has: page.locator('.list-row-name', { hasText: 'Ops Webhook' }),
    })
    const email = page.locator('.list-row').filter({
      has: page.locator('.list-row-name', { hasText: 'Admin Email' }),
    })
    await expect(webhook).toBeVisible()
    await expect(email).toBeVisible()
    await expect(webhook.locator('.list-row-sub')).toContainText('Webhook')
    await expect(email.locator('.list-row-sub')).toContainText('Email')
    // A channel that is on says nothing more; only a paused one is named.
    await expect(webhook).not.toContainText('Paused')
  })

  test('tunnels page shows configured tunnel connection details', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/tunnels')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { level: 1, name: 'Tunnels' })).toBeVisible()
    // Scoped to the tunnel's own row: the filter pills above it also say
    // "Connected", so an unscoped getByText matches both. The row itself
    // gets a generous timeout - it only renders once /api/tunnels and
    // /api/agents both return, which can run past the default 5s under CI
    // load - so the fast child checks below aren't racing page load too.
    const row = page.locator('.tunnel-row').filter({ hasText: 'media-store-01' }).first()
    await expect(row).toBeVisible({ timeout: 15_000 })
    await expect(row.locator('.tunnel-target')).toContainText('borg@127.0.0.1')
    // The demo's loopback tunnel dials a real SSH session on container
    // startup; give it its own room to reach Connected, which the row's dot
    // says (a state other than connected is named in words).
    await expect(row.locator('.status-dot')).toHaveClass(/status-dot--success/, {
      timeout: 15_000,
    })
    await expect(row.locator('.tunnel-status')).not.toContainText('Disconnected')
  })

  // A tunnel row is a link to the tunnel's own page, which holds what the
  // row's menu used to: its settings, the enabled switch, and Delete.
  test('a tunnel row opens its page with settings, the enabled switch and Delete', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/tunnels')
    const row = page.locator('.tunnel-row').filter({ hasText: 'media-store-01' }).first()
    await expect(row).toBeVisible({ timeout: 15_000 })
    await expect(row.getByRole('button')).toHaveCount(0)

    await row.click()
    await expect(page).toHaveURL(/\/tunnels\/\d+$/)
    await expect(page.locator('.detail-name')).toHaveText('media-store-01')
    await expect(page.getByLabel('SSH host')).toHaveValue('127.0.0.1')
    await expect(page.getByRole('switch', { name: 'Tunnel enabled' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    // Nothing edited yet, so there is nothing to save.
    await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled()

    await page.getByRole('button', { name: 'Delete tunnel' }).click()
    const confirm = page.getByRole('dialog', { name: 'Delete tunnel' })
    await expect(confirm).toContainText('media-store-01')
    await confirm.getByRole('button', { name: 'Cancel' }).click()
    await expect(confirm).toHaveCount(0)

    await page.getByRole('link', { name: 'Tunnels' }).first().click()
    await expect(page).toHaveURL(/\/tunnels$/)
  })

  test('settings submenu contains excludes link', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await page.getByRole('button', { name: 'Settings' }).click()
    await expect(page.getByRole('link', { name: 'Excludes' })).toBeVisible()
  })
})
