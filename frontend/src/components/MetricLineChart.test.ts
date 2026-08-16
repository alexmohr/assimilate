// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import MetricLineChart from './MetricLineChart.vue'

vi.mock('vue-chartjs', () => ({
  Line: { template: '<canvas />', props: ['data', 'options'] },
}))

describe('MetricLineChart', () => {
  it('renders the label and the underlying chart canvas', () => {
    const data = { labels: ['a'], datasets: [] }
    const options = { responsive: true }
    const wrapper = mount(MetricLineChart, {
      props: { label: 'Deduplicated', data, options },
    })

    expect(wrapper.find('.metric-label').text()).toBe('Deduplicated')
    expect(wrapper.find('canvas').exists()).toBe(true)
  })
})
