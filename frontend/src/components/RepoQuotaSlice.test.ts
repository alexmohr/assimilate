// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import RepoQuotaSlice from './RepoQuotaSlice.vue'

describe('RepoQuotaSlice', () => {
  it('shows a neutral "of box" chip positioned and sized by the box scale', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: { usageBytes: 300, offsetBytes: 0, boxMaxBytes: 1000, colorStep: 0 },
    })
    expect(wrapper.find('.slice-chip-neutral').text()).toBe('30% of box')
    const fill = wrapper.find('.slice-fill')
    expect(fill.attributes('style')).toContain('left: 0%')
    expect(fill.attributes('style')).toContain('width: 30%')
  })

  it('offsets the fill by the cumulative bytes of repos before it', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: { usageBytes: 100, offsetBytes: 200, boxMaxBytes: 1000, colorStep: 0 },
    })
    const fill = wrapper.find('.slice-fill')
    expect(fill.attributes('style')).toContain('left: 20%')
    expect(fill.attributes('style')).toContain('width: 10%')
  })

  it('shows a "—" chip when there is no box scale', () => {
    const wrapper = mount(RepoQuotaSlice, {
      props: { usageBytes: 300, offsetBytes: 0, boxMaxBytes: 0, colorStep: 0 },
    })
    expect(wrapper.find('.slice-chip').text()).toBe('—')
  })

  it('cycles the fill tone by colorStep', () => {
    const even = mount(RepoQuotaSlice, {
      props: { usageBytes: 100, offsetBytes: 0, boxMaxBytes: 1000, colorStep: 0 },
    })
    const odd = mount(RepoQuotaSlice, {
      props: { usageBytes: 100, offsetBytes: 0, boxMaxBytes: 1000, colorStep: 1 },
    })
    expect(even.find('.slice-fill').classes()).toContain('slice-fill-step-0')
    expect(odd.find('.slice-fill').classes()).toContain('slice-fill-step-1')
  })
})
