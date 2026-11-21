// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import EnrollmentTokenList from './EnrollmentTokenList.vue'

describe('EnrollmentTokenList', () => {
  it('counts the pending tokens and offers Revoke only on those', async () => {
    const wrapper = renderWithPlugins(EnrollmentTokenList)
    await wrapper.get('button').trigger('click')

    expect(wrapper.text()).toContain('Pending tokens')
    expect(wrapper.findAll('.token-row')).toHaveLength(3)
    expect(wrapper.findAll('.token-row button')).toHaveLength(1)
    expect(wrapper.text()).toContain('Used')
    expect(wrapper.text()).toContain('Expired')
  })
})
