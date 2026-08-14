// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import RepoQuotaMeter from './RepoQuotaMeter.vue'
import type { RepoQuotaSummaryResponse } from '../types/generated'

function quota(overrides: Partial<RepoQuotaSummaryResponse> = {}): RepoQuotaSummaryResponse {
  return {
    warn_bytes: 500,
    critical_bytes: 1000,
    warn_action: 'notify_only',
    critical_action: 'block_backups',
    enabled: true,
    ...overrides,
  }
}

describe('RepoQuotaMeter', () => {
  it('renders nothing when there is no quota', () => {
    const wrapper = mount(RepoQuotaMeter, { props: { quota: null, usageBytes: 100 } })
    expect(wrapper.find('.quota-meter').exists()).toBe(false)
  })

  it('renders nothing when the quota is disabled', () => {
    const wrapper = mount(RepoQuotaMeter, {
      props: { quota: quota({ enabled: false }), usageBytes: 100 },
    })
    expect(wrapper.find('.quota-meter').exists()).toBe(false)
  })

  it('renders nothing when neither threshold is set', () => {
    const wrapper = mount(RepoQuotaMeter, {
      props: { quota: quota({ warn_bytes: null, critical_bytes: null }), usageBytes: 100 },
    })
    expect(wrapper.find('.quota-meter').exists()).toBe(false)
  })

  it('shows a healthy status below the warn threshold', () => {
    const wrapper = mount(RepoQuotaMeter, { props: { quota: quota(), usageBytes: 100 } })
    expect(wrapper.find('.quota-status-ok').exists()).toBe(true)
    expect(wrapper.text()).toContain('Healthy')
    expect(wrapper.text()).toContain('100.0 B of 1000.0 B')
  })

  it('shows a warning status at or above the warn threshold', () => {
    const wrapper = mount(RepoQuotaMeter, { props: { quota: quota(), usageBytes: 600 } })
    expect(wrapper.find('.quota-status-warning').exists()).toBe(true)
    expect(wrapper.text()).toContain('Warning')
  })

  it('shows the critical action once at or above the critical threshold', () => {
    const wrapper = mount(RepoQuotaMeter, { props: { quota: quota(), usageBytes: 1200 } })
    expect(wrapper.find('.quota-status-critical').exists()).toBe(true)
    expect(wrapper.text()).toContain('Block backups')
  })

  it('clips the fill bar at 100% past the critical threshold', () => {
    const wrapper = mount(RepoQuotaMeter, { props: { quota: quota(), usageBytes: 5000 } })
    const fill = wrapper.find('.quota-fill')
    expect(fill.attributes('style')).toContain('width: 100%')
  })

  it('draws a tick at the warn threshold when it differs from the ceiling', () => {
    const wrapper = mount(RepoQuotaMeter, { props: { quota: quota(), usageBytes: 100 } })
    expect(wrapper.find('.quota-tick').exists()).toBe(true)
  })

  it('omits the tick when warn and critical thresholds coincide', () => {
    const wrapper = mount(RepoQuotaMeter, {
      props: { quota: quota({ warn_bytes: 1000 }), usageBytes: 100 },
    })
    expect(wrapper.find('.quota-tick').exists()).toBe(false)
  })

  it('falls back to warn_bytes as the ceiling when critical_bytes is unset', () => {
    const wrapper = mount(RepoQuotaMeter, {
      props: { quota: quota({ critical_bytes: null }), usageBytes: 100 },
    })
    expect(wrapper.text()).toContain('of 500.0 B')
  })
})
