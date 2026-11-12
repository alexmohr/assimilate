// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Locator, Page } from '@playwright/test'
import { expect, loginAsAdmin, mockSystemVersion, openSettingsSection, test } from './fixtures'

/**
 * Opens the System page's Settings section card (the page opens on Version),
 * where the form with the time zone, public URL, Keep for and Timeouts
 * tables lives.
 */
async function openSystemSettings(page: Page): Promise<Locator> {
  await page.goto('/system')
  return openSettingsSection(page, 'Settings')
}

async function interceptSystemApis(page: Page): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/system/ssh-public-key',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA test-key' }),
      }),
  )
  await page.route(
    (url) => url.pathname === '/api/system/settings',
    async (route) => {
      if (route.request().method() === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            timezone: 'UTC',
            retention_days: 7,
            report_retention_days: 365,
            failed_report_retention_days: 365,
            system_event_retention_days: 90,
            notification_delivery_retention_days: 30,
            borg_query_timeout_secs: 300,
          }),
        })
      }
      return route.continue()
    },
  )
  await mockSystemVersion(page)
}

test('system settings page renders the borg timeout in the Timeouts table', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptSystemApis(page)
  const settings = await openSystemSettings(page)
  const timeouts = settings.getByRole('table', { name: 'Timeouts' })
  await expect(timeouts).toBeVisible({ timeout: 10_000 })
  await expect(timeouts.getByLabel('Borg queries')).toHaveValue('300')
  await expect(timeouts.getByLabel('Idle session')).toBeVisible()
})

test('system settings page renders report retention in the Keep for table', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptSystemApis(page)
  const settings = await openSystemSettings(page)
  const keepFor = settings.getByRole('table', { name: 'Keep for' })
  await expect(keepFor).toBeVisible({ timeout: 10_000 })
  await expect(keepFor.getByLabel('Reports', { exact: true })).toHaveValue('365')
  await expect(keepFor.getByLabel('Failed reports', { exact: true })).toHaveValue('365')
  await expect(keepFor.getByLabel('System events', { exact: true })).toHaveValue('90')
  await expect(keepFor.getByLabel('Notification deliveries', { exact: true })).toHaveValue('30')
  // The legacy catch-all retention_days has no box of its own any more.
  await expect(keepFor.getByRole('spinbutton')).toHaveCount(6)
})

test('admin can update borg timeout and save settings', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptSystemApis(page)

  let savedBody: Record<string, unknown> | null = null
  await page.route(
    (url) => url.pathname === '/api/system/settings',
    async (route) => {
      if (route.request().method() === 'PUT') {
        savedBody = (await route.request().postDataJSON()) as Record<string, unknown>
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            timezone: 'UTC',
            retention_days: 7,
            report_retention_days: 365,
            failed_report_retention_days: 365,
            system_event_retention_days: 90,
            notification_delivery_retention_days: 30,
            borg_query_timeout_secs: 600,
          }),
        })
      }
      return route.continue()
    },
  )

  const settings = await openSystemSettings(page)
  const input = page.locator('#settings-borg-timeout')
  await expect(input).toBeVisible({ timeout: 10_000 })

  await input.fill('600')
  await settings.getByRole('button', { name: 'Save', exact: true }).click()

  await expect(async () => {
    expect(savedBody).not.toBeNull()
    expect((savedBody as Record<string, unknown>).borg_query_timeout_secs).toBe(600)
  }).toPass({ timeout: 5_000 })
})

test('admin can set how long archive browse indexes are kept', async ({ page }) => {
  await loginAsAdmin(page)
  const settings = await openSystemSettings(page)
  const input = settings.getByLabel('Archive browse indexes', { exact: true })
  await expect(input).toBeVisible({ timeout: 10_000 })
  const original = await input.inputValue()

  const save = async (days: string): Promise<void> => {
    await input.fill(days)
    const saved = page.waitForResponse(
      (resp) => resp.url().endsWith('/api/system/settings') && resp.request().method() === 'PUT',
    )
    await settings.getByRole('button', { name: 'Save', exact: true }).click()
    expect((await saved).ok()).toBe(true)
  }

  await save('30')
  await page.reload()
  await openSettingsSection(page, 'Settings')
  await expect(input).toHaveValue('30', { timeout: 10_000 })

  await save(original)
})

test('admin can set the public URL used in notification links', async ({ page }) => {
  await loginAsAdmin(page)
  const settings = await openSystemSettings(page)
  const input = page.locator('#settings-public-url')
  await expect(input).toBeVisible({ timeout: 10_000 })
  const original = await input.inputValue()

  const save = async (url: string): Promise<void> => {
    await input.fill(url)
    const saved = page.waitForResponse(
      (resp) => resp.url().endsWith('/api/system/settings') && resp.request().method() === 'PUT',
    )
    await settings.getByRole('button', { name: 'Save', exact: true }).click()
    expect((await saved).ok()).toBe(true)
  }

  // The server keeps only the origin, so the trailing slash typed here is gone on reload.
  await save('https://backups.example.com/')
  await page.reload()
  await openSettingsSection(page, 'Settings')
  await expect(input).toHaveValue('https://backups.example.com', { timeout: 10_000 })

  await save(original)
})

test('admin changes the borg timeout and it persists without a restart', async ({ page }) => {
  await loginAsAdmin(page)
  const settings = await openSystemSettings(page)
  const input = page.locator('#settings-borg-timeout')
  await expect(input).toBeVisible({ timeout: 10_000 })
  const original = await input.inputValue()

  const save = async (secs: string): Promise<void> => {
    await input.fill(secs)
    const saved = page.waitForResponse(
      (resp) => resp.url().endsWith('/api/system/settings') && resp.request().method() === 'PUT',
    )
    await settings.getByRole('button', { name: 'Save', exact: true }).click()
    expect((await saved).ok()).toBe(true)
  }

  await save('900')
  await page.reload()
  await openSettingsSection(page, 'Settings')
  await expect(input).toHaveValue('900', { timeout: 10_000 })
  const stored = await page.request.get('/api/system/settings')
  expect(
    ((await stored.json()) as { borg_query_timeout_secs: number }).borg_query_timeout_secs,
  ).toBe(900)

  await save(original)
})

test('the deployment panel shows the secure cookies setting read-only', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/system')
  const deployment = await openSettingsSection(page, 'Deployment')
  // The demo leaves ASSIMILATE_SECURE_COOKIES unset, so the fail-safe default applies.
  await expect(deployment.locator('#deployment-secure-cookies')).toHaveText('On')
  await expect(deployment.getByText('ASSIMILATE_SECURE_COOKIES')).toBeVisible()
})

test('database storage has its own page under Settings', async ({ page }) => {
  await loginAsAdmin(page)
  await openSystemSettings(page)
  await expect(page.locator('#settings-public-url')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText('Total database size')).toHaveCount(0)

  // The sidebar's Settings group, not the page's Settings section card.
  const nav = page.locator('.nav')
  await nav.getByRole('button', { name: 'Settings' }).click()
  await nav.getByRole('link', { name: 'Database', exact: true }).click()
  await expect(page).toHaveURL(/\/database-storage$/)
  await expect(page.getByRole('heading', { name: 'Database Storage' })).toBeVisible()
  await expect(page.getByText('Total database size')).toBeVisible({ timeout: 10_000 })
  await expect(page.locator('.storage-name', { hasText: 'backup_reports' })).toBeVisible()
})
