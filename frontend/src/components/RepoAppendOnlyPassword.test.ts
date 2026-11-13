// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import RepoAppendOnlyPassword from './RepoAppendOnlyPassword.vue'

function mount(busy = false) {
  return renderWithPlugins(RepoAppendOnlyPassword, {
    props: {
      failure: { needs_password: true, message: 'The server key cannot open SFTP.' },
      login: { ssh_host: 'backup.lan', ssh_user: 'borg', sibling_repos: [] },
      busy,
    },
  })
}

describe('RepoAppendOnlyPassword', () => {
  it('explains the failure and who the password is for', () => {
    const text = mount().text()
    expect(text).toContain('The server key cannot open SFTP.')
    expect(text).toContain('borg@backup.lan')
    expect(text).toContain('used once and not stored')
  })

  it('cannot be submitted empty', () => {
    expect(mount().find('button[type="submit"]').attributes('disabled')).toBeDefined()
  })

  it('emits the password once and empties the field', async () => {
    const wrapper = mount()
    const input = wrapper.find('input[type="password"]')
    await input.setValue('s3cret')
    await wrapper.find('form').trigger('submit')

    expect(wrapper.emitted('submit')).toEqual([['s3cret']])
    expect((input.element as HTMLInputElement).value).toBe('')
  })

  it('disables the button while working', () => {
    expect(mount(true).find('button[type="submit"]').text()).toBe('Working...')
  })
})
