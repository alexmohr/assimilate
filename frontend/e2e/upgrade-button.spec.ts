// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

interface AgentMock {
  id: number
  hostname: string
  display_name: string | null
  agent_version: string | null
  agent_git_sha: string | null
  agent_build_time: string | null
  agent_commit_count: number | null
  created_at: string
  last_seen_at: string | null
  is_connected: boolean
  is_imported: boolean
  is_hidden: boolean
  default_backup_paths: string[]
  last_ssh_user?: string | null
}

interface VersionMock {
  server_version: string
  server_git_sha: string
  build_timestamp: string
  server_commit_count: number | null
  agent_version: string | null
}

const BASE_AGENT: AgentMock = {
  id: 999,
  hostname: 'fixture-agent',
  display_name: null,
  agent_version: '0.1.0',
  agent_git_sha: null,
  agent_build_time: null,
  agent_commit_count: null,
  created_at: '2026-01-01T00:00:00Z',
  last_seen_at: '2026-01-01T00:00:00Z',
  is_connected: true,
  is_imported: false,
  is_hidden: false,
  default_backup_paths: [],
}

function makeVersion(
  server_commit_count: number | null,
  agent_version: string | null,
): VersionMock {
  return {
    server_version: '0.1.0',
    server_git_sha: '',
    build_timestamp: 'unknown',
    server_commit_count,
    agent_version,
  }
}

async function interceptAgentPage(
  page: Page,
  agent: AgentMock,
  version: VersionMock,
): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/agents',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([agent]),
      }),
  )
  await page.route(
    (url) => url.pathname === '/api/system/version',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(version),
      }),
  )
}

async function agentCard(page: Page): ReturnType<Page['locator']> {
  const card = page.locator('.card-hostname').filter({ hasText: 'fixture-agent' })
  await expect(card).toBeVisible({ timeout: 10_000 })
  return card
}

test('no upgrade button when no agent binary is available on server', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(page, BASE_AGENT, makeVersion(null, null))
  await page.goto('/agents')
  await agentCard(page)
  await expect(page.getByRole('button', { name: 'Upgrade' })).not.toBeVisible()
})

test('no upgrade button when agent version matches available binary', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: '0.1.0' },
    makeVersion(null, '0.1.0'),
  )
  await page.goto('/agents')
  await agentCard(page)
  await expect(page.getByRole('button', { name: 'Upgrade' })).not.toBeVisible()
})

test('upgrade button shown when a newer binary is available', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: '0.1.0' },
    makeVersion(null, '0.2.0'),
  )
  await page.goto('/agents')
  await agentCard(page)
  await expect(page.getByRole('button', { name: 'Upgrade' })).toBeVisible({ timeout: 5_000 })
})

test('no upgrade button when agent commit count matches server', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_commit_count: 150 },
    makeVersion(150, '0.1.0'),
  )
  await page.goto('/agents')
  await agentCard(page)
  await expect(page.getByRole('button', { name: 'Upgrade' })).not.toBeVisible()
})

test('upgrade button shown when agent commit count is behind server', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_commit_count: 100 },
    makeVersion(200, '0.1.0'),
  )
  await page.goto('/agents')
  await agentCard(page)
  await expect(page.getByRole('button', { name: 'Upgrade' })).toBeVisible({ timeout: 5_000 })
})

test('deploy button shown for agent with no version', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: null, agent_commit_count: null },
    makeVersion(null, null),
  )
  await page.goto('/agents')
  await agentCard(page)
  await expect(page.getByRole('button', { name: 'Deploy' })).toBeVisible({ timeout: 5_000 })
})

test('deploy dialog prefills the last-used SSH user and auto-loads the remote unit', async ({
  page,
}) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: '0.1.0', last_ssh_user: 'deploy-user' },
    makeVersion(null, '0.2.0'),
  )
  await page.route(
    (url) => url.pathname === '/api/agents/fixture-agent/service-unit',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          content: '[Service]\nEnvironment=BORG_AGENT_TOKEN=[REDACTED]\n',
        }),
      }),
  )
  await page.goto('/agents')
  await agentCard(page)

  await page.getByRole('button', { name: 'Upgrade' }).click()
  await expect(page.getByRole('heading', { name: /Upgrade Agent/ })).toBeVisible()

  await expect(page.getByPlaceholder('root')).toHaveValue('deploy-user')
  await expect(page.locator('textarea.service-textarea')).toHaveValue(
    /Environment=BORG_AGENT_TOKEN=\[REDACTED\]/,
  )
})
