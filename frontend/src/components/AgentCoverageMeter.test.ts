// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import AgentCoverageMeter from './AgentCoverageMeter.vue'

describe('AgentCoverageMeter', () => {
  it('shows a "No backups yet" status when a cadence is configured but nothing has run yet', () => {
    const wrapper = mount(AgentCoverageMeter, {
      props: { lastBackupAt: null, cadenceSecs: 3600 },
    })
    expect(wrapper.text()).toContain('No backups yet')
    // Regression: this used to fall into the same 'unknown' bucket as "no
    // cadence configured" and show a misleading "No cadence" status badge
    // even though a real cadence (3600) was passed in.
    expect(wrapper.find('.coverage-status-no-data').text()).toBe('No backups yet')
    expect(wrapper.find('.coverage-status-no-cadence').exists()).toBe(false)
  })

  it('shows "No cadence" when the agent has no schedule to derive one from', () => {
    const lastBackupAt = new Date(Date.now() - 60_000).toISOString()
    const wrapper = mount(AgentCoverageMeter, {
      props: { lastBackupAt, cadenceSecs: null },
    })
    expect(wrapper.find('.coverage-status-no-cadence').text()).toBe('No cadence')
    expect(wrapper.text()).toContain('since last backup')
  })

  it('is on time well within cadence', () => {
    const lastBackupAt = new Date(Date.now() - 3600_000).toISOString() // 1h ago
    const wrapper = mount(AgentCoverageMeter, {
      props: { lastBackupAt, cadenceSecs: 8 * 3600 }, // every 8h
    })
    expect(wrapper.find('.coverage-status-ok').exists()).toBe(true)
    expect(wrapper.text()).toContain('On time')
  })

  it('is due soon once elapsed time reaches the cadence', () => {
    const lastBackupAt = new Date(Date.now() - 9 * 3600_000).toISOString()
    const wrapper = mount(AgentCoverageMeter, {
      props: { lastBackupAt, cadenceSecs: 8 * 3600 },
    })
    expect(wrapper.find('.coverage-status-warning').exists()).toBe(true)
    expect(wrapper.text()).toContain('Due soon')
  })

  it('is overdue at twice the cadence', () => {
    const lastBackupAt = new Date(Date.now() - 17 * 3600_000).toISOString()
    const wrapper = mount(AgentCoverageMeter, {
      props: { lastBackupAt, cadenceSecs: 8 * 3600 },
    })
    expect(wrapper.find('.coverage-status-critical').exists()).toBe(true)
    expect(wrapper.text()).toContain('Overdue')
  })

  it('clips the fill bar at 100% once far overdue', () => {
    const lastBackupAt = new Date(Date.now() - 100 * 3600_000).toISOString()
    const wrapper = mount(AgentCoverageMeter, {
      props: { lastBackupAt, cadenceSecs: 3600 },
    })
    expect(wrapper.find('.coverage-fill').attributes('style')).toContain('width: 100%')
  })
})
