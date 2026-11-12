// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import { appendOnlyStatusFixture } from '../test-utils/appendOnlyFixtures'
import RepoAppendOnlyCheck from './RepoAppendOnlyCheck.vue'
import type { AppendOnlyCheckResponse } from '../types/generated'

function mount(check: AppendOnlyCheckResponse | null, busy = false) {
  return renderWithPlugins(RepoAppendOnlyCheck, { props: { check, busy } })
}

describe('RepoAppendOnlyCheck', () => {
  it('says nothing was checked yet', () => {
    expect(mount(null).text()).toContain('Not checked yet.')
  })

  it('lists the three results, all passing', () => {
    const wrapper = mount(appendOnlyStatusFixture().check)
    expect(wrapper.findAll('.check-ok')).toHaveLength(3)
    expect(wrapper.findAll('.check-bad')).toHaveLength(0)
  })

  it('marks a failing check and shows its error', () => {
    const check = {
      ...appendOnlyStatusFixture().check!,
      maintenance_key_ok: false,
      restriction_in_place: false,
      error: 'Permission denied',
    }
    const wrapper = mount(check)
    expect(wrapper.findAll('.check-bad')).toHaveLength(2)
    expect(wrapper.find('.form-error').text()).toBe('Permission denied')
  })

  it('does not claim to prove that borg refuses deletion', () => {
    expect(mount(null).text()).toContain('does not prove borg refuses deletions')
  })

  it('asks for a new check', async () => {
    const wrapper = mount(null)
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('check')).toHaveLength(1)
  })

  it('disables the button while checking', () => {
    const button = mount(null, true).find('button')
    expect(button.text()).toBe('Checking...')
    expect(button.attributes('disabled')).toBeDefined()
  })
})
