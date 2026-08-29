// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import { clickMenuItemStartingWith, menuLabels, openMenu } from '../test-utils/overflowMenu'
import ScheduleHeader from './ScheduleHeader.vue'
import type { ScheduleRow } from '../types/schedule'

const SCHEDULE = {
  id: 1,
  name: 'Nightly production backup',
  schedule_type: 'backup',
  enabled: true,
} as unknown as ScheduleRow

function mount(
  scheduleOverrides: Record<string, unknown> = {},
  props: Record<string, unknown> = {},
) {
  return renderWithPlugins(ScheduleHeader, {
    props: {
      schedule: { ...SCHEDULE, ...scheduleOverrides },
      typeLabel: 'Backup',
      cronSummary: 'Daily at 02:00',
      backupRunning: false,
      runNowLoading: false,
      cancelLoading: false,
      overdueCount: 0,
      failedReportCount: 0,
      ...props,
    },
  })
}

describe('ScheduleHeader', () => {
  it('falls back to the type label when the schedule has no name', () => {
    const wrapper = mount({ name: null })
    expect(wrapper.find('.detail-name').text()).toBe('Backup')
  })

  it('uses the schedule name when set', () => {
    const wrapper = mount()
    expect(wrapper.find('.detail-name').text()).toBe('Nightly production backup')
  })

  it('shows Enabled or Disabled', () => {
    expect(mount({ enabled: true }).find('.badge--success').text()).toContain('Enabled')
    expect(mount({ enabled: false }).find('.badge--neutral').text()).toContain('Disabled')
  })

  it('shows a Running badge only while a backup is in flight', () => {
    expect(mount({}, { backupRunning: true }).find('.badge--accent').text()).toContain('Running')
    expect(mount({}, { backupRunning: false }).find('.badge--accent').exists()).toBe(false)
  })

  it('shows an overdue-target count only when targets are overdue', () => {
    const wrapper = mount({}, { overdueCount: 2 })
    expect(wrapper.find('.badge--warning').text()).toBe('2 targets overdue')
    expect(mount({}, { overdueCount: 0 }).find('.badge--warning').exists()).toBe(false)
  })

  it('singularizes a single overdue target', () => {
    expect(mount({}, { overdueCount: 1 }).find('.badge--warning').text()).toBe('1 target overdue')
  })

  it('shows Run now when nothing is running and emits runNow', async () => {
    const wrapper = mount()
    const btn = wrapper.findAll('button').find((b) => b.text() === 'Run now')
    expect(btn).toBeTruthy()
    await btn!.trigger('click')
    expect(wrapper.emitted('runNow')).toHaveLength(1)
  })

  it('shows Cancel backup while running and emits cancelBackup', async () => {
    const wrapper = mount({}, { backupRunning: true })
    const btn = wrapper.findAll('button').find((b) => b.text() === 'Cancel backup')
    expect(btn).toBeTruthy()
    await btn!.trigger('click')
    expect(wrapper.emitted('cancelBackup')).toHaveLength(1)
  })

  it('hides Logs and Delete until the overflow menu is opened', async () => {
    const wrapper = mount()
    expect(wrapper.findAll('.overflow-menu-item')).toHaveLength(0)

    await openMenu(wrapper)

    expect(menuLabels(wrapper)).toEqual(['Logs', 'Delete schedule'])
  })

  it.each([
    ['Logs', 'logs'],
    ['Delete schedule', 'delete'],
  ])('emits %s from the menu and closes it', async (label, event) => {
    const wrapper = mount()
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === label)!
      .trigger('click')

    expect(wrapper.emitted(event)).toHaveLength(1)
    expect(wrapper.findAll('.overflow-menu-item')).toHaveLength(0)
  })

  // A failed run has no archive behind it, so clearing it out is safe - and,
  // like Delete schedule in the same menu, the server (not this component)
  // is the source of truth for who may do it: absent entirely rather than
  // disabled when there is nothing to clear.
  describe('clean up failed backups', () => {
    it('is omitted when there are no failed reports', async () => {
      const wrapper = mount({}, { failedReportCount: 0 })
      await openMenu(wrapper)
      expect(menuLabels(wrapper).some((l) => l.startsWith('Clean up failed'))).toBe(false)
    })

    it('shows the failed count', async () => {
      const wrapper = mount({}, { failedReportCount: 4 })
      await openMenu(wrapper)
      expect(menuLabels(wrapper)).toEqual([
        'Logs',
        'Clean up failed backups (4)',
        'Delete schedule',
      ])
    })

    it('emits cleanFailedReports from the menu and closes it', async () => {
      const wrapper = mount({}, { failedReportCount: 4 })
      await openMenu(wrapper)
      await clickMenuItemStartingWith(wrapper, 'Clean up failed')

      expect(wrapper.emitted('cleanFailedReports')).toHaveLength(1)
      expect(wrapper.findAll('.overflow-menu-item')).toHaveLength(0)
    })
  })
})
