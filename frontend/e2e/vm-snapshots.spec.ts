// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * Staging a host's libvirt domains before a backup. db-server-01 is the
 * demo's virtualization host: staging is on, the domains a scan reported are
 * seeded, and its hourly schedule opts in.
 */
test.describe('Virtual machine staging', () => {
  test('the host settings and its domains are listed', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=vms')
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('/srv/vm-staging')
    await expect(pane).toContainText('7 increments')

    // One row per domain, with the mode the agent decided and what it staged.
    await expect(pane.locator('tbody tr')).toHaveCount(5)
    await expect(pane).toContainText('web01')
    await expect(pane).toContainText('Incremental')
    await expect(pane).toContainText('full + 4 increments')
    await expect(pane).toContainText('Excluded')
  })

  test('the host settings can be edited', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=vms')
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await pane.getByRole('button', { name: 'Edit' }).click()
    await expect(page.locator('#vm-staging-dir')).toHaveValue('/srv/vm-staging')

    await page.locator('#vm-full-interval').fill('14')
    await pane.getByRole('button', { name: 'Save' }).click()

    await expect(pane.getByRole('button', { name: 'Edit' })).toBeVisible()
    await expect(pane).toContainText('14 increments')

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.settings-pane')).toContainText('14 increments')
  })

  test('a per-domain limit is saved on its own', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=vms')
    await page.waitForLoadState('networkidle')

    const row = page.locator('tbody tr', { hasText: 'web01' })
    await expect(row).toContainText('Host default')

    await row.locator('input.vm-limit').fill('300')
    await row.locator('input.vm-limit').blur()
    await expect(row).toContainText('Overridden')

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('tbody tr', { hasText: 'web01' })).toContainText('Overridden')
  })

  /**
   * The host is shared demo state, so the mode is put back through the API
   * rather than the UI: a failed assertion skips the rest of the test body,
   * and a half-switched host would then break every spec that follows.
   */
  test.describe('selection mode', () => {
    test.afterEach(async ({ page }) => {
      await page.request.put('/api/agents/db-server-01/vm-snapshot', {
        data: {
          enabled: true,
          selection: 'all',
          staging_dir: '/srv/vm-staging',
          full_interval: 7,
          timeout_seconds: 1800,
          default_limit_bytes: 214748364800,
        },
      })
      // The limit this test sets is demo state too, so it goes back with the
      // mode. Sending no `included` is the behaviour under test: it must
      // leave mail01 undecided rather than deciding for it.
      await page.request.put('/api/agents/db-server-01/vms/mail01', {
        data: { limit_bytes: null },
      })
    })

    test('only-selected drops the domains nobody picked', async ({ page }) => {
      await loginAsAdmin(page)
      await page.goto('/agents/db-server-01?tab=settings&section=vms')
      await page.waitForLoadState('networkidle')

      const pane = page.locator('.settings-pane')
      await expect(pane).toContainText('Every domain except the ones excluded below')

      // mail01 carries no decision either way. Giving it a limit first is the
      // regression this pins: a budget is a cap, not consent, so it must not
      // survive the switch as an opt-in.
      const undecided = page.locator('tbody tr', { hasText: 'mail01' })
      await undecided.locator('input.vm-limit').fill('64')
      await undecided.locator('input.vm-limit').blur()
      await expect(undecided).toContainText('Overridden')
      await expect(undecided).toContainText('Offline copy')

      await pane.getByRole('button', { name: 'Edit' }).click()
      await pane.getByRole('radio', { name: 'Only selected' }).click()
      await pane.getByRole('button', { name: 'Save' }).click()
      await expect(pane.getByRole('button', { name: 'Edit' })).toBeVisible()

      await expect(pane).toContainText('Only the domains selected below')
      await expect(undecided, 'setting a limit must not opt an undecided domain in').toContainText(
        'Excluded',
      )
      await expect(
        page.locator('tbody tr', { hasText: 'db01' }),
        'a domain the operator selected is unaffected by the mode',
      ).toContainText('Incremental')

      await page.reload()
      await page.waitForLoadState('networkidle')
      await expect(page.locator('tbody tr', { hasText: 'mail01' })).toContainText('Excluded')
    })
  })

  test('the restore wizard walks both stages', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=vms')
    await page.waitForLoadState('networkidle')

    await page
      .locator('tbody tr', { hasText: 'web01' })
      .getByRole('button', { name: 'Restore' })
      .click()

    const dialog = page.getByRole('dialog')
    await expect(dialog).toContainText('Restore virtual machine')
    // Stage one is borg: the archives this host produced are the points in time.
    await expect(dialog.locator('input[name="vm-restore-archive"]').first()).toBeVisible()
    await dialog.locator('input[name="vm-restore-archive"]').first().check()

    await dialog.getByRole('button', { name: 'Next' }).click()
    // The wizard says exactly where the files land, so stage two is not a guess.
    await expect(dialog).toContainText('/srv/vm-staging/web01')

    await dialog.getByRole('button', { name: 'Next' }).click()
    await expect(page.locator('#vm-restore-name')).toHaveValue('web01-restored')
    await expect(dialog).toContainText('Define the domain, leave it shut off')
  })

  test('a schedule opts in from its advanced settings', async ({ page }) => {
    await loginAsAdmin(page)

    // The schedule is found by the flag rather than by clicking a card: the
    // list shows a schedule's name and repository, never its agent's hostname,
    // and the demo gives db-server-01 several schedules of which only the
    // hourly one opts in.
    const schedules = await (await page.request.get('/api/schedules')).json()
    const optedIn = (schedules as { id: number; vm_snapshot_enabled: boolean }[]).find(
      (s) => s.vm_snapshot_enabled,
    )
    expect(optedIn, 'seed-demo.sh seeds one schedule with vm_snapshot_enabled').toBeDefined()

    await page.goto(`/schedules/${optedIn!.id}?tab=settings&section=advanced`)
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    const optIn = pane.locator('.field-inline', { hasText: 'Stage virtual machines' })
    await expect(optIn).toBeVisible()
    await expect(optIn.locator('[role="switch"]')).toHaveAttribute('aria-checked', 'true')
  })
})
