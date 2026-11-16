// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, openSettingsSection, seededIdByName, test } from './fixtures'

/**
 * A schedule takes each host's files, sets its own, or reads none (see
 * docs/scheduling.md#sources). Each test makes its own paused schedule on
 * db-server-01, so the seeded ones other specs and the screenshots read are
 * never touched.
 */
async function hostId(page: Page, hostname: string): Promise<number> {
  const agents = (await (await page.request.get('/api/agents')).json()) as {
    id: number
    hostname: string
  }[]
  const agent = agents.find((a) => a.hostname === hostname)
  expect(agent, `seed-demo.sh registers ${hostname}`).toBeDefined()
  return agent!.id
}

async function pausedSchedule(
  page: Page,
  name: string,
  extra: Record<string, unknown> = {},
  hostnames: string[] = ['db-server-01'],
): Promise<number> {
  const agentIds: number[] = []
  for (const hostname of hostnames) agentIds.push(await hostId(page, hostname))
  const created = await page.request.post('/api/schedules', {
    data: {
      name,
      agent_ids: agentIds,
      repo_id: await seededIdByName(page, '/api/repos', 'server-daily'),
      cron_expression: '0 3 * * *',
      enabled: false,
      files_mode: 'host',
      keep_daily: 7,
      ...extra,
    },
  })
  expect(created.ok()).toBe(true)
  return ((await created.json()) as { id: number }).id
}

test.describe('Schedule sources', () => {
  test('a schedule takes each host files, then sets only /home of its own', async ({ page }) => {
    await loginAsAdmin(page)
    const name = `e2e own files ${Date.now()}`
    const id = await pausedSchedule(page, name)
    try {
      await page.goto(`/schedules/${id}?tab=settings&section=sources`)
      const sources = await openSettingsSection(page, 'Sources')

      // Each host's files: the table shows db-server-01's own paths.
      const files = sources.getByRole('radiogroup', { name: 'Files' })
      await expect(files.getByRole('radio', { name: "Each host's files" })).toHaveAttribute(
        'aria-checked',
        'true',
      )
      const table = sources.locator('.data-table')
      await expect(table).toContainText('/var/lib/postgresql')

      // Own files: the same form as the host's, saved from the section.
      await files.getByRole('radio', { name: 'Own files' }).click()
      const own = sources.locator('.own-files')
      await own.getByRole('button', { name: 'Edit' }).click()
      await own.locator('#file-sources-paths').fill('/home')
      await own.locator('#file-sources-excludes').fill('*/.cache')
      const [saved] = await Promise.all([
        page.waitForResponse(
          (resp) =>
            resp.url().endsWith(`/api/schedules/${id}`) && resp.request().method() === 'PUT',
        ),
        own.getByRole('button', { name: 'Save' }).click(),
      ])
      expect(saved.ok()).toBe(true)
      expect(saved.request().postDataJSON()).toMatchObject({
        files_mode: 'own',
        backup_sources: ['/home'],
        exclude_patterns_raw: '*/.cache',
      })
      await expect(own.locator('.path-item').first()).toHaveText('/home')
      await expect(table).toContainText("replaces the host's 2")

      // The host's Used by names the schedule with the paths it took.
      await page.goto('/agents/db-server-01?tab=sources&section=used-by')
      const usedBy = await openSettingsSection(page, 'Used by')
      const row = usedBy.locator('tr', { has: page.getByRole('link', { name }) })
      await expect(row).toContainText('/home')
    } finally {
      expect((await page.request.delete(`/api/schedules/${id}`)).ok()).toBe(true)
    }
  })

  test('a databases-only schedule reads no files', async ({ page }) => {
    await loginAsAdmin(page)
    const id = await pausedSchedule(page, `e2e files off ${Date.now()}`)
    try {
      await page.goto(`/schedules/${id}?tab=settings&section=sources`)
      const sources = await openSettingsSection(page, 'Sources')

      await sources
        .getByRole('radiogroup', { name: 'Files' })
        .getByRole('radio', { name: 'Off' })
        .click()
      await expect(
        sources.getByRole('switch', { name: "Run each host's backup commands" }),
      ).toHaveCount(0)
      await sources.getByRole('switch', { name: 'Databases' }).click()
      await expect(sources.locator('.data-table')).toContainText('database sources')

      const [saved] = await Promise.all([
        page.waitForResponse(
          (resp) =>
            resp.url().endsWith(`/api/schedules/${id}`) && resp.request().method() === 'PUT',
        ),
        page.getByRole('button', { name: 'Save changes' }).click(),
      ])
      expect(saved.ok()).toBe(true)
      expect((await saved.json()) as Record<string, unknown>).toMatchObject({
        files_mode: 'off',
        database_backup_enabled: true,
      })
    } finally {
      expect((await page.request.delete(`/api/schedules/${id}`)).ok()).toBe(true)
    }
  })

  test('own files can give each host its own paths', async ({ page }) => {
    await loginAsAdmin(page)
    const db = await hostId(page, 'db-server-01')
    const web = await hostId(page, 'web-server-01')
    const id = await pausedSchedule(
      page,
      `e2e per-host files ${Date.now()}`,
      { files_mode: 'own', backup_sources: ['/srv'] },
      ['db-server-01', 'web-server-01'],
    )
    try {
      await page.goto(`/schedules/${id}?tab=settings&section=sources`)
      const sources = await openSettingsSection(page, 'Sources')
      await sources.getByRole('switch', { name: 'Different paths per host' }).click()

      const own = sources.locator('.own-files')
      await own.getByRole('button', { name: 'Edit' }).click()
      const perHost = own.getByPlaceholder('Directories to back up, one per line')
      await expect(perHost).toHaveCount(2)
      await perHost.nth(0).fill('/var/lib/postgresql')
      await perHost.nth(1).fill('/var/www')
      const [saved] = await Promise.all([
        page.waitForResponse(
          (resp) =>
            resp.url().endsWith(`/api/schedules/${id}`) && resp.request().method() === 'PUT',
        ),
        own.getByRole('button', { name: 'Save' }).click(),
      ])
      expect(saved.ok()).toBe(true)
      const body = saved.request().postDataJSON() as {
        backup_sources: string[]
        backup_sources_per_agent: { agent_id: number; paths: string[] }[]
      }
      expect(body.backup_sources).toEqual([])
      expect(
        Object.fromEntries(body.backup_sources_per_agent.map((e) => [e.agent_id, e.paths])),
      ).toEqual({ [db]: ['/var/lib/postgresql'], [web]: ['/var/www'] })

      // Each host's row of the table reads its own paths.
      const table = sources.locator('.data-table')
      await expect(table.locator('tr', { hasText: 'db-server-01' })).toContainText(
        '/var/lib/postgresql',
      )
      await expect(
        table.locator('tr', { hasText: /web-server-01|Production Web Server/ }),
      ).toContainText('/var/www')
    } finally {
      expect((await page.request.delete(`/api/schedules/${id}`)).ok()).toBe(true)
    }
  })

  // db-server-01 has domains set up (seed-demo.sh); web-server-01 has none.
  test("a schedule taking VMs names the host that has none, on the schedule's page", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    const id = await pausedSchedule(
      page,
      `e2e vms missing ${Date.now()}`,
      { vm_snapshot_enabled: true },
      ['db-server-01', 'web-server-01'],
    )
    try {
      await page.goto(`/schedules/${id}`)
      const warning = page.getByTestId('source-not-set-up-vms')
      await expect(warning).toBeVisible({ timeout: 15_000 })
      await expect(warning).toContainText('No virtual machines will be backed up on 1 host')
      const hosts = warning.getByRole('link')
      await expect(hosts).toHaveCount(1)
      await expect(hosts).toHaveAttribute(
        'href',
        /^\/agents\/web-server-01\?tab=sources&section=vms/,
      )
      await expect(page.getByTestId('source-not-set-up-databases')).toHaveCount(0)
    } finally {
      expect((await page.request.delete(`/api/schedules/${id}`)).ok()).toBe(true)
    }
  })
})
