// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * Test the configuration export/import feature end-to-end via the API.
 *
 * Verifies that the JSON format produced by export can be re-imported
 * without errors, and that the JSON structure matches what the backend
 * structs define (not the stale shared/responses.rs types which have
 * COMPLETELY DIFFERENT field names like `target_hostnames` instead of
 * `targets` and `pre_backup_commands` as a string instead of an array).
 */
test('config export produces JSON with targets array, not target_hostnames', async ({ page }) => {
  await loginAsAdmin(page)

  const exportRes = await page.request.get('/api/config/export')
  expect(exportRes.ok()).toBeTruthy()
  const exportData = (await exportRes.json()) as Record<string, unknown>

  expect(exportData).toHaveProperty('version', 1)
  expect(exportData).toHaveProperty('exported_at')
  expect(exportData).toHaveProperty('hosts')
  expect(exportData).toHaveProperty('schedules')

  const hosts = exportData.hosts as Array<Record<string, unknown>>
  for (const host of hosts) {
    expect(host).toHaveProperty('hostname')
    expect(host).toHaveProperty('display_name')
    expect(host).toHaveProperty('default_backup_paths')
    expect(host).toHaveProperty('default_exclude_patterns')
    expect(host).toHaveProperty('default_pre_backup_commands')
    expect(host).toHaveProperty('default_post_backup_commands')
    expect(host).toHaveProperty('default_file_change_patterns_raw')
    expect(host).toHaveProperty('hostname_patterns')

    // Stale HostExportResponse was missing default_file_change_patterns_raw
    // and had a non-existent tags field.
    expect(host).not.toHaveProperty('tags')
  }

  const schedules = exportData.schedules as Array<Record<string, unknown>>
  for (const sched of schedules) {
    expect(sched).toHaveProperty('name')
    expect(sched).toHaveProperty('schedule_type')
    expect(sched).toHaveProperty('cron_expression')
    expect(sched).toHaveProperty('enabled')
    expect(sched).toHaveProperty('canary_enabled')
    expect(sched).toHaveProperty('execution_mode')
    expect(sched).toHaveProperty('on_failure')
    expect(sched).toHaveProperty('exclude_patterns_raw')
    expect(sched).toHaveProperty('file_change_patterns_raw')
    expect(sched).toHaveProperty('ignore_global_excludes')
    expect(sched).toHaveProperty('keep_hourly')
    expect(sched).toHaveProperty('keep_daily')
    expect(sched).toHaveProperty('keep_weekly')
    expect(sched).toHaveProperty('keep_monthly')
    expect(sched).toHaveProperty('keep_yearly')
    expect(sched).toHaveProperty('compact_enabled')
    expect(sched).toHaveProperty('rate_limit_kbps')
    expect(sched).toHaveProperty('repo_name')
    expect(sched).toHaveProperty('backup_sources')
    expect(sched).toHaveProperty('targets')
    expect(sched).toHaveProperty('pre_backup_commands')
    expect(sched).toHaveProperty('post_backup_commands')

    // Stale ScheduleExportResponse had target_hostnames: string[] instead
    // of targets array with hostname/order/sources/patterns.
    const targets = sched.targets as Array<Record<string, unknown>>
    for (const target of targets) {
      expect(target).toHaveProperty('hostname')
      expect(target).toHaveProperty('execution_order')
      expect(target).toHaveProperty('backup_sources')
      expect(target).toHaveProperty('exclude_patterns')
      expect(target).toHaveProperty('file_change_patterns')
    }

    // Stale ScheduleExportResponse had pre/post_backup_commands as string;
    // the real API uses arrays so import deserialization doesn't fail.
    expect(Array.isArray(sched.pre_backup_commands)).toBeTruthy()
    expect(Array.isArray(sched.post_backup_commands)).toBeTruthy()

    if (sched.repo_name !== null) {
      expect(typeof sched.repo_name).toBe('string')
    }

    // Stale types included target_hostnames and tags which don't appear
    // in the actual export; if present they'd confuse importers.
    expect(sched).not.toHaveProperty('target_hostnames')
    expect(sched).not.toHaveProperty('tags')
  }
})

test('config export JSON can be round-tripped through import', async ({ page }) => {
  await loginAsAdmin(page)

  const exportRes = await page.request.get('/api/config/export')
  expect(exportRes.ok()).toBeTruthy()
  const exportData = await exportRes.json()

  const importRes = await page.request.post('/api/config/import', {
    data: exportData,
  })
  expect(importRes.ok()).toBeTruthy()

  const importResult = (await importRes.json()) as {
    hosts_created: number
    hosts_updated: number
    schedules_created: number
    warnings: string[]
  }

  // Round-trip must not produce errors or warnings; updating existing
  // hosts/schedules must succeed without creating duplicates.
  expect(importResult.warnings).toHaveLength(0)
  expect(importResult.hosts_created).toBe(0)
  expect(importResult.hosts_updated).toBeGreaterThan(0)
  expect(importResult.schedules_created).toBeGreaterThan(0)
})

test('import with stale shared types format (target_hostnames) would fail', async ({ page }) => {
  await loginAsAdmin(page)

  // This JSON uses the stale ScheduleExportResponse format that had
  // target_hostnames (wrong), pre_backup_commands as string (wrong type),
  // and tags (non-existent field). All of these structural issues would
  // cause a 400 Bad Request from serde deserialization.
  const stalePayload = {
    version: 1,
    exported_at: '2026-07-05T12:00:00Z',
    hosts: [],
    schedules: [
      {
        name: 'stale-format-schedule',
        schedule_type: 'backup',
        cron_expression: '0 2 * * *',
        enabled: true,
        canary_enabled: false,
        execution_mode: 'sequential',
        on_failure: 'stop',
        exclude_patterns_raw: '',
        file_change_patterns_raw: '',
        ignore_global_excludes: false,
        keep_hourly: 24,
        keep_daily: 7,
        keep_weekly: 4,
        keep_monthly: 6,
        keep_yearly: 0,
        compact_enabled: true,
        rate_limit_kbps: null,
        pre_backup_commands: '[]',
        post_backup_commands: '[]',
        backup_sources: [],
        target_hostnames: ['web-server-01'],
        repo_name: 'server-daily',
        tags: ['production'],
      },
    ],
  }

  const importRes = await page.request.post('/api/config/import', {
    data: stalePayload,
  })

  // Serde may report any of the structural issues first (missing targets,
  // invalid type for pre_backup_commands since it's a string not array).
  expect(importRes.ok()).toBeFalsy()
  expect(importRes.status()).toBe(400)

  const body = (await importRes.json()) as { error: string }
  // Verify a serde deserialization error was returned (not a 500 or 404).
  expect(body.error).toMatch(/targets|missing field|invalid type|expected/)
})
