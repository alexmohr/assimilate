// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, seededIdByName, test } from './fixtures'

/**
 * Where a channel's notifications link to, set per channel in its Link
 * section. The seed gives Ops Webhook a link of its own,
 * https://status.example.com/backups, and leaves Admin Email on the automatic
 * one (see `.devcontainer/demo/seed-demo.sh`). Nothing is saved here: changing
 * a link is covered by notification-channel-wizard.spec.ts on a channel of
 * its own.
 */
test.describe('Notification channel link', () => {
  test('a channel with its own address names it, and edits it in place', async ({ page }) => {
    await loginAsAdmin(page)
    const id = await seededIdByName(page, '/api/notifications/channels', 'Ops Webhook')
    await page.goto(`/notifications/channels/${id}?section=link`)

    const link = await openSettingsSection(page, 'Link')
    await expect(link.locator('.info-grid')).toContainText('Links to')
    await expect(link.locator('.info-grid dd')).toHaveText('https://status.example.com/backups')

    await link.getByRole('button', { name: 'Edit' }).click()
    const picker = link.getByRole('radiogroup', { name: 'Where notifications link to' })
    await expect(picker.getByRole('radio', { name: /Your own address/ })).toBeChecked()
    await expect(picker.getByLabel('Address', { exact: true })).toHaveValue(
      'https://status.example.com/backups',
    )
    await link.getByRole('button', { name: 'Cancel' }).click()
    await expect(link.locator('.info-grid dd')).toHaveText('https://status.example.com/backups')
  })

  test('a channel left on the automatic link opens where the event happened', async ({ page }) => {
    await loginAsAdmin(page)
    const id = await seededIdByName(page, '/api/notifications/channels', 'Admin Email')
    await page.goto(`/notifications/channels/${id}?section=link`)

    const link = await openSettingsSection(page, 'Link')
    await expect(link.locator('.info-grid dd')).toHaveText('Where the event happened')

    await link.getByRole('button', { name: 'Edit' }).click()
    await expect(
      link
        .getByRole('radiogroup', { name: 'Where notifications link to' })
        .getByRole('radio', { name: /Where the event happened/ }),
    ).toBeChecked()
    await expect(link.getByLabel('Address', { exact: true })).toHaveCount(0)
    await link.getByRole('button', { name: 'Cancel' }).click()
  })

  test('the API returns each channel link', async ({ page }) => {
    await loginAsAdmin(page)
    const response = await page.request.get('/api/notifications/channels')
    expect(response.ok()).toBe(true)
    const channels = (await response.json()) as {
      name: string
      link: { target: string; url?: string | null }
    }[]
    expect(channels.find((c) => c.name === 'Ops Webhook')?.link).toMatchObject({
      target: 'custom',
      url: 'https://status.example.com/backups',
    })
    expect(channels.find((c) => c.name === 'Admin Email')?.link).toMatchObject({
      target: 'automatic',
    })
  })
})
