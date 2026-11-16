// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Locator, Page, Route } from '@playwright/test'

/**
 * Provider presets for a repository host (docs/provider-guides.md). The
 * demo seed holds `offsite-hetzner`, a repository on a Hetzner Storage Box
 * host; the presets of a host that does not exist yet are exercised through
 * the import dialog, with the create call answered locally because the demo
 * cannot reach a real Hetzner, rsync.net or BorgBase host.
 */

interface PresetCase {
  label: string
  provider: string
  typedHost: string
  expectedHost: string
  expectedPort: number
  /** What the dialog says about the remote borg, or null when it runs plain `borg`. */
  remoteBorg: string | null
}

const PRESETS: PresetCase[] = [
  {
    label: 'Hetzner Storage Box',
    provider: 'hetzner',
    typedHost: 'u777777',
    expectedHost: 'u777777.your-storagebox.de',
    expectedPort: 23,
    remoteBorg: 'borg-1.4',
  },
  {
    label: 'rsync.net',
    provider: 'rsync_net',
    typedHost: 'zz999.rsync.net',
    expectedHost: 'zz999.rsync.net',
    expectedPort: 22,
    remoteBorg: 'borg14',
  },
  {
    label: 'BorgBase',
    provider: 'borgbase',
    typedHost: 'abc123.repo.borgbase.com',
    expectedHost: 'abc123.repo.borgbase.com',
    expectedPort: 22,
    remoteBorg: null,
  },
  {
    label: 'Generic SSH',
    provider: 'generic',
    typedHost: 'nas.example.org',
    expectedHost: 'nas.example.org',
    expectedPort: 22,
    remoteBorg: null,
  },
]

async function openImportDialog(page: Page): Promise<Locator> {
  await loginAsAdmin(page)
  await page.goto('/repos')
  await page.waitForLoadState('networkidle')
  await page.getByRole('button', { name: 'Import' }).click()
  const dialog = page.locator('.modal-dialog')
  await expect(dialog.getByLabel('Provider')).toBeVisible()
  return dialog
}

test.describe('Repository host providers', () => {
  for (const preset of PRESETS) {
    test(`adds a host through the ${preset.label} preset`, async ({ page }) => {
      let created: Record<string, unknown> | null = null
      await page.route('**/api/repos', async (route: Route) => {
        if (route.request().method() !== 'POST') {
          await route.fallback()
          return
        }
        created = route.request().postDataJSON() as Record<string, unknown>
        await route.fulfill({ status: 502, json: { error: 'host not reachable from the demo' } })
      })

      const dialog = await openImportDialog(page)
      await dialog.getByLabel('Provider').selectOption({ label: preset.label })

      const host = dialog.getByPlaceholder('backup.example.com')
      await host.fill(preset.typedHost)
      await host.blur()
      await expect(host).toHaveValue(preset.expectedHost)

      const port = dialog.getByRole('spinbutton')
      await expect(port).toHaveValue(String(preset.expectedPort))
      if (preset.provider === 'generic') {
        await expect(port).toBeEnabled()
      } else {
        await expect(port).toBeDisabled()
      }
      if (preset.remoteBorg !== null) {
        await expect(dialog).toContainText(preset.remoteBorg)
      }

      await dialog.getByPlaceholder('e.g. inhouse-backups').fill(`preset-${preset.provider}`)
      await dialog.getByPlaceholder('/backup/repos/myhost').fill('./backups/e2e')
      await dialog.getByPlaceholder('Repository encryption passphrase').fill('secret')
      await dialog.getByRole('button', { name: 'Import Repo' }).click()

      await expect(dialog.locator('.form-error')).toContainText('not reachable')
      expect(created).toMatchObject({
        ssh_host: preset.expectedHost,
        ssh_port: preset.expectedPort,
      })
      if (preset.provider === 'generic') {
        expect(created).not.toHaveProperty('provider')
      } else {
        expect(created).toMatchObject({ provider: preset.provider })
      }
    })
  }

  test('the seeded Hetzner host keeps its provider and locks the dialog to it', async ({
    page,
  }) => {
    const dialog = await openImportDialog(page)
    await dialog.getByPlaceholder('backup.example.com').fill('u123456.your-storagebox.de')

    const provider = dialog.getByLabel('Provider')
    await expect(provider).toBeDisabled()
    await expect(provider.locator('option:checked')).toHaveText('Hetzner Storage Box')
    await expect(dialog.getByRole('spinbutton')).toHaveValue('23')
  })

  test('the host page names the provider', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await page.getByRole('link', { name: 'u123456.your-storagebox.de', exact: true }).click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repo-hosts\/\d+/)
    await expect(page.getByTestId('host-provider')).toHaveText('Hetzner Storage Box')
  })
})
