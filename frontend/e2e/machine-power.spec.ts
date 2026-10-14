// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, seededIdByName, test } from './fixtures'
import type { Locator, Page } from '@playwright/test'

/**
 * One machine, one Power & dependencies pane, wherever a role of it is shown -
 * and a schedule's own choice per machine, including waiting for a machine
 * without waking it.
 *
 * The demo's NAS is the machine behind the "localhost" repository host: the
 * nas-media dependency is on it too, it requires files-01 of every backup
 * written to it, and "Web server dual-target" waits for it rather than waking
 * it (see `.devcontainer/demo/seed-demo.sh`).
 */
async function openSchedulePower(page: Page, name: string): Promise<Locator> {
  const id = await seededIdByName(page, '/api/schedules', name)
  await page.goto(`/schedules/${id}?tab=settings&section=power`)
  await page.waitForLoadState('networkidle')
  return page.locator('.settings-pane .pane-section', { hasText: 'Per machine' })
}

test.describe('Machines', () => {
  test('a schedule waits for the NAS without waking it, and can wait for another', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    const card = await openSchedulePower(page, 'Web server dual-target')

    // The seeded choice: the NAS behind "localhost" is waited for, not woken.
    const nasEffect = effectOf(card, 'localhost')
    await expect(nasEffect).toContainText('Waits')
    await expect(nasEffect).toContainText('Wait for it')

    // An agent cannot be waited for: it has nothing to run the backup.
    await card.getByRole('button', { name: 'Edit' }).click()
    const agentChoice = card.getByRole('radiogroup', { name: /web-server-01/ })
    await expect(agentChoice.getByRole('radio')).toHaveText(['Same as above', 'Wake', "Don't wake"])

    // Wait for the "demo" host as well, then put it back.
    const demoChoice = card.getByRole('radiogroup', { name: /about demo$/ })
    await demoChoice.getByRole('radio', { name: 'Wait for it' }).click()
    await card.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(card.getByRole('button', { name: 'Edit' })).toBeVisible()
    await expect(effectOf(card, 'demo')).toContainText('Wait for it')

    await page.reload()
    await page.waitForLoadState('networkidle')
    const reloaded = page.locator('.settings-pane .pane-section', { hasText: 'Per machine' })
    await expect(effectOf(reloaded, 'demo')).toContainText('Wait for it')

    await reloaded.getByRole('button', { name: 'Edit' }).click()
    await reloaded
      .getByRole('radiogroup', { name: /about demo$/ })
      .getByRole('radio', { name: 'Same as above' })
      .click()
    await reloaded.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(effectOf(reloaded, 'demo')).not.toContainText('Wait for it')
  })

  test("the NAS's pane names its roles, the schedule that waits for it, and what it requires", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    const hosts = (await (await page.request.get('/api/repo-hosts')).json()) as {
      id: number
      ssh_host: string
    }[]
    const nas = hosts.find((h) => h.ssh_host === 'localhost')
    expect(nas).toBeDefined()
    await page.goto(`/repo-hosts/${nas!.id}?section=power`)
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.repo-host-detail .settings-pane')
    const roles = pane.locator('.machine-roles')
    await expect(roles).toContainText('repository host')
    await expect(roles.getByRole('link', { name: 'nas-media' })).toBeVisible()
    await expect(pane).toContainText('9C:B6:D0:1A:44:7F')

    const exception = pane.locator('.override-link', { hasText: 'Web server dual-target' })
    await expect(exception).toContainText('waits for it without waking it')

    const required = pane.locator('.pane-section', { hasText: 'Required dependencies' })
    await expect(required).toContainText('files-01')

    // The same machine, from the dependency's side.
    await roles.getByRole('link', { name: 'nas-media' }).click()
    await page.waitForURL(/\/dependency-hosts\/\d+\?section=power/)
    await page.waitForLoadState('networkidle')
    const dependencyPane = page.locator('.dependency-host-detail .settings-pane')
    await expect(dependencyPane).toContainText('Same as repository host')
    await expect(dependencyPane).toContainText('9C:B6:D0:1A:44:7F')
  })

  test('a schedule inherits what its backup server requires', async ({ page }) => {
    await loginAsAdmin(page)
    await openSchedulePower(page, 'Web server dual-target')
    const dependencies = page.locator('.settings-pane .pane-section', {
      hasText: 'Required dependencies',
    })
    await expect(dependencies.locator('.dependency-chip--inherited')).toContainText(
      'from backup server localhost',
    )
  })
})

/**
 * The read-out of the machine named `label` in a card: the `dd` after the
 * `dt` naming it. A locator, so it waits for the card to load.
 */
function effectOf(card: Locator, label: string): Locator {
  return card.locator(`dt:has(.mono:text-is("${label}")) + dd`)
}
