// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import CronBuilder from './CronBuilder.vue'
import { logger } from '../utils/logger'

// A timezone the browser's Intl database doesn't know, as a stale or
// hand-edited server setting could produce.
vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: () => 'Not/A/Zone',
}))

describe('CronBuilder next-run preview', () => {
  it('shows no next runs, rather than failing, when the timezone is unknown', () => {
    const debug = vi.spyOn(logger, 'debug').mockImplementation(() => {})
    const wrapper = mount(CronBuilder, { props: { modelValue: '0 2 * * *' } })

    expect(wrapper.find('.cron-error').exists()).toBe(false)
    expect(wrapper.find('.next-runs').exists()).toBe(false)
    expect(debug).toHaveBeenCalledWith('next run preview failed', expect.any(RangeError))
  })
})
