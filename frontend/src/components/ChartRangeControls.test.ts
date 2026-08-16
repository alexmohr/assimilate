// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import ChartRangeControls from './ChartRangeControls.vue'
import type { Repo } from '../types/repo'

const repos: Repo[] = [{ id: 1, name: 'repo-a' } as Repo, { id: 2, name: 'repo-b' } as Repo]

const options = [
  { value: 14, label: '14d' },
  { value: 30, label: '30d' },
]

describe('ChartRangeControls', () => {
  it('renders an "All Repos" option plus one per repo', () => {
    const wrapper = mount(ChartRangeControls, {
      props: { repos, options, label: 'Range', repoId: undefined, days: 30 },
    })

    const optionTexts = wrapper.findAll('option').map((o) => o.text())
    expect(optionTexts).toEqual(['All Repos', 'repo-a', 'repo-b'])
  })

  it('marks the selected day option active in the segmented control', () => {
    const wrapper = mount(ChartRangeControls, {
      props: { repos, options, label: 'Range', repoId: undefined, days: 30 },
    })

    const active = wrapper.find('.segmented-option.active')
    expect(active.text()).toBe('30d')
  })

  it('emits update:repoId when the select changes', async () => {
    const wrapper = mount(ChartRangeControls, {
      props: { repos, options, label: 'Range', repoId: undefined, days: 30 },
    })

    await wrapper.find('select').setValue('2')

    expect(wrapper.emitted('update:repoId')?.[0]).toEqual([2])
  })

  it('emits update:days when a segmented option is clicked', async () => {
    const wrapper = mount(ChartRangeControls, {
      props: { repos, options, label: 'Range', repoId: undefined, days: 30 },
    })

    await wrapper.findAll('.segmented-option')[0]!.trigger('click')

    expect(wrapper.emitted('update:days')?.[0]).toEqual([14])
  })
})
