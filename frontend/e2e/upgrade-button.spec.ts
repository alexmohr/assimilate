// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, loginAsOperator, test } from './fixtures'

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

/**
 * Opens the fixture agent's own page. Deploy and Upgrade used to sit in the
 * agents list's row menu; that menu is gone, and the agent page's header has
 * always offered them as its accented action.
 */
async function openAgentPage(page: Page): Promise<void> {
  await page.goto('/agents/fixture-agent')
  await expect(page.locator('.detail-name')).toHaveText('fixture-agent', { timeout: 10_000 })
}

/** The header's Deploy agent / Upgrade agent button, once it is showing. */
async function deployAction(
  page: Page,
  name: 'Deploy' | 'Upgrade',
): Promise<ReturnType<Page['locator']>> {
  const button = page
    .locator('.detail-actions')
    .getByRole('button', { name: `${name} agent`, exact: true })
  await expect(button).toBeVisible({ timeout: 5_000 })
  return button
}

async function expectNoDeployAction(page: Page): Promise<void> {
  await expect(
    page.locator('.detail-actions').getByRole('button', { name: /^(Deploy|Upgrade) agent$/ }),
  ).toHaveCount(0)
}

test('no upgrade button when no agent binary is available on server', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(page, BASE_AGENT, makeVersion(null, null))
  await openAgentPage(page)
  await expectNoDeployAction(page)
})

test('no upgrade button when agent version matches available binary', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: '0.1.0' },
    makeVersion(null, '0.1.0'),
  )
  await openAgentPage(page)
  await expectNoDeployAction(page)
})

test('upgrade button shown when a newer binary is available', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: '0.1.0' },
    makeVersion(null, '0.2.0'),
  )
  await openAgentPage(page)
  await deployAction(page, 'Upgrade')
})

test('no upgrade button when agent commit count matches server', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_commit_count: 150 },
    makeVersion(150, '0.1.0'),
  )
  await openAgentPage(page)
  await expectNoDeployAction(page)
})

test('upgrade button shown when agent commit count is behind server', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_commit_count: 100 },
    makeVersion(200, '0.1.0'),
  )
  await openAgentPage(page)
  await deployAction(page, 'Upgrade')
})

test('deploy button shown for agent with no version', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: null, agent_commit_count: null },
    makeVersion(null, null),
  )
  await openAgentPage(page)
  await deployAction(page, 'Deploy')
})

test('no deploy/upgrade button for a role without can_upgrade_agent', async ({ page }) => {
  await loginAsOperator(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: '0.1.0' },
    makeVersion(null, '0.2.0'),
  )
  await openAgentPage(page)
  await expectNoDeployAction(page)
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
  await openAgentPage(page)

  await (await deployAction(page, 'Upgrade')).click()
  await expect(page.getByRole('heading', { name: /Upgrade Agent/ })).toBeVisible()

  // The version transition is the point of the dialog, so it leads the body.
  const hero = page.locator('.upgrade-hero')
  await expect(hero).toContainText('0.1.0')
  await expect(hero).toContainText('0.2.0')
  await expect(page.getByRole('button', { name: /^Upgrade to 0\.2\.0$/ })).toBeVisible()

  await expect(page.getByPlaceholder('root')).toHaveValue('deploy-user')

  // The service unit editor is the largest control in the dialog and is
  // rarely hand-edited, so it stays collapsed until asked for.
  const unitToggle = page.getByRole('button', { name: /Systemd service unit/ })
  await expect(unitToggle).toBeVisible()
  await expect(page.locator('textarea.service-textarea')).toBeHidden()

  await unitToggle.click()
  await expect(page.locator('textarea.service-textarea')).toHaveValue(
    /Environment=BORG_AGENT_TOKEN=\[REDACTED\]/,
  )
})

// `systemctl show` gives every user on the host a unit's environment, so the
// unit a first deploy installs loads the token from a root-only file instead.
test('deploy dialog defaults to a unit that loads the token from its file', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: null, agent_commit_count: null },
    makeVersion(null, null),
  )
  await page.route(
    (url) => url.pathname === '/api/agents/fixture-agent/service-unit',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ content: null }),
      }),
  )
  await openAgentPage(page)

  await (await deployAction(page, 'Deploy')).click()

  // A first deploy opens the unit editor, since there is no unit to keep.
  const unit = page.locator('textarea.service-textarea')
  await expect(unit).toHaveValue(/EnvironmentFile=\/etc\/assimilate-agent\.env/)
  await expect(unit).not.toHaveValue(/BORG_AGENT_TOKEN/)
})

/** A deploy's five steps, the first ones as given and the rest pending. */
function deploySteps(
  statuses: string[],
): { step: string; status: string; duration_ms: number | null }[] {
  return ['connect', 'upload', 'install', 'restart', 'reconnect'].map((step, i) => ({
    step,
    status: statuses[i] ?? 'pending',
    duration_ms: statuses[i] === 'done' ? 400 : null,
  }))
}

// The upgrade runs on the server, so a proxy cutting the request that started
// it cannot fail it: the request answers at once and the dialog follows the
// deploy's steps until the agent is back.
test('an upgrade runs on the server while the dialog follows its steps', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptAgentPage(
    page,
    { ...BASE_AGENT, agent_version: '0.1.0' },
    makeVersion(null, '0.2.0'),
  )
  await page.route(
    (url) => url.pathname === '/api/agents/fixture-agent/service-unit',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ content: null }),
      }),
  )
  const base = { skipped: false, token: null, available_version: null, error: null }
  let reads = 0
  let dismissed = false
  await page.route(
    (url) => url.pathname === '/api/agents/fixture-agent/deploy',
    async (route) => {
      const method = route.request().method()
      if (method === 'POST') {
        return route.fulfill({
          status: 202,
          contentType: 'application/json',
          body: JSON.stringify({ ...base, state: 'running', steps: deploySteps(['running']) }),
        })
      }
      if (method === 'DELETE') {
        dismissed = true
        return route.fulfill({ status: 204 })
      }
      // The first read is the dialog looking for a deploy already under way.
      reads += 1
      if (reads === 1)
        return route.fulfill({ status: 404, contentType: 'application/json', body: '{}' })
      const body =
        reads === 2
          ? { ...base, state: 'running', steps: deploySteps(['done', 'done', 'running']) }
          : {
              ...base,
              state: 'succeeded',
              steps: deploySteps(['done', 'done', 'done', 'done', 'done']),
              token: 'tok-e2e-0123',
              available_version: '0.2.0',
            }
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(body),
      })
    },
  )
  await openAgentPage(page)

  await (await deployAction(page, 'Upgrade')).click()
  await page.getByPlaceholder('e.g. 192.168.1.10').fill('10.0.0.5')
  await page.getByRole('button', { name: /^Upgrade to 0\.2\.0$/ }).click()

  // The form gives way to the steps right away.
  await expect(page.locator('.deploy-step')).toHaveCount(5)
  await expect(page.locator('.upgrade-hero')).toContainText('Upgrading to')
  await expect(page.getByText('You can close this window and it will carry on.')).toBeVisible()

  await expect(page.locator('.token-text')).toHaveText('tok-e2e-0123', { timeout: 15_000 })
  await expect(page.getByText('Upgraded to 0.2.0.')).toBeVisible()

  await page.getByRole('button', { name: 'Done' }).click()
  await expect(page.getByRole('heading', { name: /Upgrade Agent/ })).toBeHidden()
  expect(dismissed).toBe(true)
})
