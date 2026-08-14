// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import RepoQuotaSlice from './RepoQuotaSlice.vue'
import type { RepoQuotaSummaryResponse } from '../types/generated'

function quota(overrides: Partial<RepoQuotaSummaryResponse> = {}): RepoQuotaSummaryResponse {
  return {
    warn_bytes: 150,
    critical_bytes: 250,
    warn_action: 'notify_only',
    critical_action: 'block_backups',
    enabled: true,
    ...overrides,
  }
}

describe('RepoQuotaSlice', () => {
  it('shows a neutral "of box" chip when the repo has no own quota', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: {
        quota: null,
        usageBytes: 300,
        offsetBytes: 0,
        boxMaxBytes: 1000,
        colorStep: 0,
      },
    })
    expect(wrapper.find('.slice-chip-neutral').text()).toBe('30% of box')
    expect(wrapper.find('.own-bracket').exists()).toBe(false)
  })

  it('shows a "—" chip when there is no box scale and no own quota', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: { quota: null, usageBytes: 300, offsetBytes: 0, boxMaxBytes: 0, colorStep: 0 },
    })
    expect(wrapper.find('.slice-chip').text()).toBe('—')
  })

  it('draws a bracket and a colored "of own" chip when the repo has its own quota', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: {
        quota: quota(),
        usageBytes: 100,
        offsetBytes: 100,
        boxMaxBytes: 1000,
        colorStep: 0,
      },
    })
    const bracket = wrapper.find('.own-bracket')
    expect(bracket.exists()).toBe(true)
    expect(bracket.classes()).not.toContain('own-bracket-overcommit')
    expect(wrapper.find('.slice-chip-ok').text()).toBe('40% of own') // 100/250
    expect(wrapper.text()).toContain('headroom')
  })

  it('hatches the over-limit portion and reports the overage when usage exceeds the own ceiling', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: {
        quota: quota(),
        usageBytes: 300,
        offsetBytes: 0,
        boxMaxBytes: 1000,
        colorStep: 0,
      },
    })
    expect(wrapper.find('.slice-fill-over').exists()).toBe(true)
    expect(wrapper.find('.slice-chip-critical').exists()).toBe(true)
    expect(wrapper.text()).toContain('over by 50.0 B')
    expect(wrapper.text()).toContain('Block backups')
  })

  it('marks the bracket as overcommitted when the own ceiling runs past the box edge', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: {
        quota: quota({ warn_bytes: 400, critical_bytes: 500 }),
        usageBytes: 250,
        offsetBytes: 700,
        boxMaxBytes: 1000,
        colorStep: 0,
      },
    })
    expect(wrapper.find('.own-bracket-overcommit').exists()).toBe(true)
  })

  it('cycles the fill tone by colorStep', () => {
    const even = mount(RepoQuotaSlice, {
      props: { quota: null, usageBytes: 100, offsetBytes: 0, boxMaxBytes: 1000, colorStep: 0 },
    })
    const odd = mount(RepoQuotaSlice, {
      props: { quota: null, usageBytes: 100, offsetBytes: 0, boxMaxBytes: 1000, colorStep: 1 },
    })
    expect(even.find('.slice-fill').classes()).toContain('slice-fill-step-0')
    expect(odd.find('.slice-fill').classes()).toContain('slice-fill-step-1')
  })

  it('notes when the host has no quota configured at all', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: { quota: null, usageBytes: 100, offsetBytes: 0, boxMaxBytes: 0, colorStep: 0 },
    })
    expect(wrapper.text()).toContain('no host quota')
  })
})
