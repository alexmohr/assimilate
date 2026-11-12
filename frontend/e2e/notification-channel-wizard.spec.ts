// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, test } from './fixtures'

/**
 * A new notification channel is made on a page of its own, a wizard of five
 * steps (delivery, events, scope, link, review), which replaced the channel
 * dialog. The channel is a webhook, so nothing has to answer while the test
 * runs, and the test removes it again.
 */
test.describe('New notification channel wizard', () => {
  test('creates a webhook channel with its events and its own link', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/notifications')
    await page.getByRole('button', { name: 'New', exact: true }).click()
    await expect(page).toHaveURL(/\/notifications\/new$/)
    await expect(page.getByRole('navigation', { name: 'Channel steps' })).toBeVisible()

    const foot = page.locator('.wizard-foot')
    const next = foot.getByRole('button', { name: 'Continue' })
    const current = page.locator('.wizard-step--current')

    // Delivery: the type, a name and where to; each step says what it needs.
    await expect(next).toBeDisabled()
    await page.locator('#channel-type').selectOption('webhook')
    const name = `e2e wizard webhook ${Date.now()}`
    await page.locator('#channel-name').fill(name)
    await expect(foot).toContainText('Needs a webhook URL to continue.')
    await page
      .getByPlaceholder('https://hooks.example.com/notify')
      .fill('https://hooks.example.com/e2e-wizard')
    await expect(next).toBeEnabled()
    await next.click()

    // Events: switched on one by one, nothing sent yet.
    await expect(current).toContainText('Events')
    const failed = page
      .locator('.event-item')
      .filter({ has: page.getByText('Backup failed', { exact: true }) })
    await failed.getByRole('switch').click()
    await expect(failed.getByRole('switch')).toHaveAttribute('aria-checked', 'true')
    await next.click()

    // Scope: everything, by leaving it unticked.
    await expect(current).toContainText('Scope')
    await next.click()

    // Link: where a notification leads. An own address must be a web address.
    await expect(current).toContainText('Link')
    const picker = page.getByRole('radiogroup', { name: 'Where notifications link to' })
    await expect(picker.getByRole('radio', { name: /Where the event happened/ })).toBeChecked()
    await picker.getByRole('radio', { name: /Your own address/ }).check()
    const address = page.getByLabel('Address', { exact: true })
    await address.fill('status.example.com')
    await expect(next).toBeDisabled()
    await expect(foot).toContainText('an address starting with http:// or https://')
    await address.fill('https://status.example.com/e2e-wizard')
    await expect(next).toBeEnabled()
    await next.click()

    // Review: every block, and the link chosen.
    await expect(current).toContainText('Review')
    const review = page.locator('.review-grid')
    await expect(review).toContainText(name)
    await expect(review).toContainText('Webhook')
    await expect(review).toContainText('https://hooks.example.com/e2e-wizard')
    await expect(review).toContainText('Backup failed')
    await expect(review).toContainText('https://status.example.com/e2e-wizard')

    const [created] = await Promise.all([
      page.waitForResponse(
        (resp) =>
          resp.url().endsWith('/api/notifications/channels') && resp.request().method() === 'POST',
      ),
      foot.getByRole('button', { name: 'Create channel', exact: true }).click(),
    ])
    expect(created.ok()).toBe(true)
    const { id } = (await created.json()) as { id: number }
    try {
      // The new channel opens on its own page, with what the wizard set.
      await expect(page).toHaveURL(new RegExp(`/notifications/channels/${id}`))
      await expect(page.locator('.detail-name')).toHaveText(name)

      const link = await openSettingsSection(page, 'Link')
      await expect(link).toContainText('https://status.example.com/e2e-wizard')

      // The link is changed on the channel's page as well.
      await link.getByRole('button', { name: 'Edit' }).click()
      await link.getByRole('radio', { name: /^Dashboard/ }).check()
      await link.getByRole('button', { name: 'Save', exact: true }).click()
      await expect(link.getByRole('button', { name: 'Edit' })).toBeVisible()
      await expect(link.locator('.info-grid dd')).toHaveText('Dashboard')

      const events = await openSettingsSection(page, 'Events')
      await expect(
        events
          .locator('.event-item')
          .filter({ has: page.getByText('Backup failed', { exact: true }) })
          .getByRole('switch'),
      ).toHaveAttribute('aria-checked', 'true')
    } finally {
      const removed = await page.request.delete(`/api/notifications/channels/${id}`)
      expect(removed.ok()).toBe(true)
    }
  })
})
