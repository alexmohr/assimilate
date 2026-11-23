// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

/** Opens the Add agent dialog on its Install command tab. */
async function openInstallTab(page: Page): Promise<void> {
  await page.goto('/agents')
  await page.waitForLoadState('networkidle')
  await page.getByRole('button', { name: 'New', exact: true }).click()
  await page.getByRole('tab', { name: 'Install command' }).click()
}

test.describe('Install command', () => {
  test('creates a single-use token and shows the install command for each platform', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await openInstallTab(page)

    await page.getByRole('button', { name: 'Generate command' }).click()

    const command = page.getByTestId('install-command')
    await expect(command).toContainText('/install.sh')
    await expect(command).toContainText('--token enr_')
    await expect(command).toContainText('--server-key')
    await expect(page.getByTestId('install-expiry')).toContainText('Expires in')

    await page.getByRole('radio', { name: 'Windows' }).click()
    await expect(command).toContainText('/install.ps1')
    await expect(command).toContainText('-Token enr_')

    await page.getByRole('radio', { name: 'macOS' }).click()
    await expect(command).toContainText('| sudo sh')

    // The manual route, with where each platform starts the agent at boot.
    await page.getByRole('button', { name: 'Install manually' }).click()
    await expect(page.getByText('com.assimilate.agent.plist')).toBeVisible()
    await page.getByRole('radio', { name: 'Linux' }).click()
    await expect(page.getByText('systemctl enable --now assimilate-agent')).toBeVisible()

    await page.getByRole('button', { name: 'Done' }).click()
    await expect(page.getByRole('dialog')).toBeHidden()
  })

  test('revoking the token closes the dialog and lists the token as revoked', async ({ page }) => {
    await loginAsAdmin(page)
    await openInstallTab(page)
    await page.getByLabel('Restrict to hostname').fill('e2e-revoked-host')
    await page.getByRole('button', { name: 'Generate command' }).click()
    await expect(page.getByTestId('install-command')).toBeVisible()

    await page.getByRole('button', { name: 'Revoke token' }).click()
    await expect(page.getByRole('dialog')).toBeHidden()

    await openInstallTab(page)
    await page.getByRole('button', { name: /Enrollment tokens/ }).click()
    const row = page
      .getByTestId('enrollment-token-row')
      .filter({ hasText: 'e2e-revoked-host' })
      .first()
    await expect(row).toContainText('Revoked')
    await expect(row.getByRole('button', { name: 'Revoke' })).toHaveCount(0)
  })

  test('a pending token can be revoked from the list', async ({ page }) => {
    await loginAsAdmin(page)
    await openInstallTab(page)
    await page.getByLabel('Restrict to hostname').fill('e2e-listed-host')
    await page.getByRole('button', { name: 'Generate command' }).click()
    await expect(page.getByTestId('install-command')).toBeVisible()

    await page.getByRole('button', { name: /Enrollment tokens/ }).click()
    const row = page
      .getByTestId('enrollment-token-row')
      .filter({ hasText: 'e2e-listed-host' })
      .first()
    await expect(row).toContainText('Pending')
    await row.getByRole('button', { name: 'Revoke' }).click()
    await expect(row).toContainText('Revoked')
  })

  test('the install scripts and the agent download are public', async ({ request }) => {
    const sh = await request.get('/install.sh')
    expect(sh.ok()).toBe(true)
    expect(await sh.text()).toContain('--server-key')

    const ps1 = await request.get('/install.ps1')
    expect(ps1.ok()).toBe(true)
    expect(await ps1.text()).toContain('-ServerKey')
  })

  test('the agent download answers without a login', async ({ request }) => {
    // A server image carries the Linux agent; one run without it says so.
    const download = await request.get('/download/agent?os=linux&arch=x86_64')
    const checksum = await request.get('/download/agent.sha256?os=linux&arch=x86_64')
    expect([200, 404]).toContain(download.status())
    expect(checksum.status()).toBe(download.status())
    if (checksum.ok()) expect(await checksum.text()).toMatch(/^[0-9a-f]{64} {2}assimilate-agent\n$/)

    const unknown = await request.get('/download/agent?os=linux&arch=riscv64')
    expect(unknown.status()).toBe(400)
  })

  test('the enrollment API needs an admin', async ({ request }) => {
    const response = await request.get('/api/enrollment-tokens')
    expect(response.status()).toBe(401)
  })
})
