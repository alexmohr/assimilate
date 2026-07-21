// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

const HOSTNAME = 'e2e-fcp-host'

function baseAgent(defaultFileChangePatternsRaw: string): Record<string, unknown> {
  return {
    id: 9001,
    hostname: HOSTNAME,
    display_name: 'E2E FCP Host',
    agent_version: '1.0.0',
    agent_git_sha: null,
    agent_build_time: null,
    agent_commit_count: null,
    created_at: '2026-01-01T00:00:00Z',
    last_seen_at: '2026-01-01T00:00:00Z',
    default_backup_paths: [],
    default_exclude_patterns: [],
    default_pre_backup_commands: '[]',
    default_post_backup_commands: '[]',
    default_file_change_patterns_raw: defaultFileChangePatternsRaw,
    is_connected: true,
    is_imported: false,
    is_hidden: false,
    supports_restart: false,
    owner_id: null,
    visibility: 'public',
    restart_unavailable_reason: null,
  }
}

async function interceptHostApis(page: Page, agent: Record<string, unknown>): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/agents',
    async (route) => {
      if (route.request().method() === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([agent]),
        })
      }
      return route.continue()
    },
  )
  await page.route(
    (url) =>
      url.pathname === `/api/agents/${HOSTNAME}/repos` ||
      url.pathname === `/api/agents/${HOSTNAME}/reports` ||
      url.pathname === `/api/agents/${HOSTNAME}/tags` ||
      url.pathname === `/api/agents/${HOSTNAME}/hostname-patterns`,
    async (route) => route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route(
    (url) => url.pathname === '/api/schedules',
    async (route) => route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route(
    (url) => url.pathname === '/api/system/version',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          server_version: '0.1.0',
          server_git_sha: '',
          build_timestamp: 'unknown',
          server_commit_count: null,
          agent_version: null,
        }),
      }),
  )
}

test('admin can view and edit default file change patterns on a host', async ({ page }) => {
  await loginAsAdmin(page)
  const agent = baseAgent('*/tmp/session-*.lock* ignore')
  await interceptHostApis(page, agent)

  await page.goto(`/agents/${HOSTNAME}`)

  const card = page.locator('.info-card').filter({ hasText: 'Default File Change Patterns' })
  await expect(card).toBeVisible({ timeout: 10_000 })
  await expect(card).toContainText('*/tmp/session-*.lock*')
  await expect(card).toContainText('ignore')

  let savedBody: Record<string, unknown> | null = null
  await page.route(
    (url) => url.pathname === `/api/agents/${HOSTNAME}`,
    async (route) => {
      if (route.request().method() === 'PUT') {
        savedBody = (await route.request().postDataJSON()) as Record<string, unknown>
        agent.default_file_change_patterns_raw = savedBody.default_file_change_patterns_raw
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(agent),
        })
      }
      return route.continue()
    },
  )

  await card.getByRole('button', { name: 'Edit' }).click()
  await card.getByRole('button', { name: '+ Add pattern' }).click()
  await card.locator('input[type="text"]').last().fill('*/var/log/app.log*')
  await card.locator('select').last().selectOption('fatal')
  await card.getByRole('button', { name: 'Save' }).click()

  await expect(async () => {
    expect(savedBody).not.toBeNull()
    expect((savedBody as Record<string, unknown>).default_file_change_patterns_raw).toContain(
      '*/var/log/app.log* fatal',
    )
  }).toPass({ timeout: 5_000 })

  await expect(card).toContainText('*/var/log/app.log*')
  await expect(card).toContainText('fatal')
})
