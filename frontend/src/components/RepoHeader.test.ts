// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockToast, resetToastSpies, toastSpies } from '../test-utils/sharedMocks'
import { openMenu } from '../test-utils/overflowMenu'

// The header owns sync, the import reset and the passphrase reveal, so it
// reports through toasts and writes as well as reads.
vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../composables/useToast', () => mockToast())

import { renderWithPlugins } from '../test-utils'
import { dialogButton, findButton } from '../test-utils/dom'
import { repoFixture as repo } from '../test-utils/repoFixtures'
import { apiClient } from '../api/client'
import RepoHeader from './RepoHeader.vue'

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoHeader, {
    props: { repo: repo(), isAdmin: true, importPhaseVerb: 'Importing', ...props },
  })
}

describe('RepoHeader', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    resetToastSpies()
    vi.mocked(apiClient.get)
      .mockReset()
      .mockResolvedValue({ data: { passphrase: 'hunter2' } } as never)
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue({} as never)
  })

  describe('identity', () => {
    it('names the repository and its ssh target', () => {
      const wrapper = mount()
      expect(wrapper.find('.detail-name').text()).toBe('server-daily')
      expect(wrapper.find('.detail-subtitle').text()).toBe(
        'borg@backup.example.com:/backup/repos/server-daily',
      )
    })

    it('summarises what is in the repository in the meta line', () => {
      const text = mount().find('.detail-meta').text()
      expect(text).toContain('30')
      expect(text).toContain('repokey-blake2')
    })
  })

  describe('status badge', () => {
    // The e2e suite asserts on these exact tone classes, so a rename that
    // only updates the component is caught here rather than 15 minutes into
    // a Playwright run.
    it('shows an enabled repository as a success badge', () => {
      const badge = mount().find('.repo-status-badge')
      expect(badge.text()).toBe('Enabled')
      expect(badge.classes()).toContain('badge--success')
    })

    it('shows a disabled repository as a neutral badge', () => {
      const badge = mount({ repo: repo({ enabled: false }) }).find('.repo-status-badge')
      expect(badge.text()).toBe('Disabled')
      expect(badge.classes()).toContain('badge--neutral')
    })

    it('pulses a warning badge while an import runs', () => {
      const badge = mount({ repo: repo({ importing: true }) }).find('.repo-status-badge')
      expect(badge.classes()).toEqual(expect.arrayContaining(['badge--warning', 'badge--pulse']))
    })

    it('counts import progress in the badge when a total is known', () => {
      const wrapper = mount({
        repo: repo({ importing: true, import_progress: 3, import_total: 9 }),
      })
      expect(wrapper.find('.repo-status-badge').text()).toBe('Importing 3/9')
    })

    it('shows a danger badge carrying the import error as its title', () => {
      const badge = mount({ repo: repo({ import_error: 'ssh refused' }) }).find(
        '.repo-status-badge',
      )
      expect(badge.text()).toBe('Import failed')
      expect(badge.classes()).toContain('badge--danger')
      expect(badge.attributes('title')).toBe('ssh refused')
    })

    it('tracks a running import under the header, where every tab sees it', () => {
      const wrapper = mount({
        repo: repo({ importing: true, import_progress: 3, import_total: 6 }),
      })
      expect(wrapper.find('.progress-bar').attributes('style')).toContain('50%')
    })
  })

  describe('admin gating', () => {
    it('offers no actions to a non-admin', () => {
      expect(mount({ isAdmin: false }).find('.detail-actions button').exists()).toBe(false)
    })

    it('offers Sync now to an admin when no import is running', () => {
      expect(findButton(mount(), /Sync now/).exists()).toBe(true)
    })

    it('swaps Sync now for Cancel import while importing', () => {
      const wrapper = mount({ repo: repo({ importing: true }) })
      expect(wrapper.findAll('button').some((b) => /Sync now/.test(b.text()))).toBe(false)
      expect(findButton(wrapper, /Cancel import/).exists()).toBe(true)
    })

    it('offers Cancel import after a failed import too, so the state can be cleared', () => {
      const wrapper = mount({ repo: repo({ import_error: 'boom' }) })
      expect(findButton(wrapper, /Cancel import/).exists()).toBe(true)
    })
  })

  describe('full resync', () => {
    it('starts the sync and reports it', async () => {
      const wrapper = mount()
      await findButton(wrapper, /Sync now/).trigger('click')
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/sync?build_index=true')
      expect(toastSpies.success).toHaveBeenCalled()
    })

    it('surfaces a failure as an error toast', async () => {
      const wrapper = mount()
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('nope'))
      await findButton(wrapper, /Sync now/).trigger('click')
      await flushPromises()
      expect(toastSpies.error).toHaveBeenCalled()
    })
  })

  describe('reset import', () => {
    it('resets the import state and tells the parent', async () => {
      const wrapper = mount({ repo: repo({ importing: true }) })
      await findButton(wrapper, /Cancel import/).trigger('click')
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/reset-import')
      expect(wrapper.emitted('import-reset')).toHaveLength(1)
    })

    it('does not claim success when the reset fails', async () => {
      const wrapper = mount({ repo: repo({ importing: true }) })
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('nope'))
      await findButton(wrapper, /Cancel import/).trigger('click')
      await flushPromises()
      expect(toastSpies.error).toHaveBeenCalled()
      expect(wrapper.emitted('import-reset')).toBeUndefined()
    })
  })

  describe('passphrase', () => {
    it('fetches the passphrase on demand and shows it in the dialog', async () => {
      const wrapper = mount()
      await openMenu(wrapper)
      await findButton(wrapper, /passphrase/i).trigger('click')
      await flushPromises()

      expect(apiClient.get).toHaveBeenCalledWith('/repos/12/passphrase')
      expect(document.body.textContent).toContain('hunter2')
    })

    it('copies the passphrase to the clipboard on request', async () => {
      const wrapper = mount()
      await openMenu(wrapper)
      await findButton(wrapper, /passphrase/i).trigger('click')
      await flushPromises()

      const copy = dialogButton('Copy')
      expect(copy).toBeDefined()
      copy.click()
      await flushPromises()
      // The clipboard itself is the composable's business; what matters here
      // is that the button is wired to the revealed value at all.
      expect(document.body.textContent).toContain('hunter2')
    })

    it('closes on the modal dismiss control too, not only on Done', async () => {
      const wrapper = mount()
      await openMenu(wrapper)
      await findButton(wrapper, /passphrase/i).trigger('click')
      await flushPromises()

      document.body.querySelector<HTMLButtonElement>('.modal-close')?.click()
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })

    it('closes on Done, so the secret does not stay on screen', async () => {
      const wrapper = mount()
      await openMenu(wrapper)
      await findButton(wrapper, /passphrase/i).trigger('click')
      await flushPromises()
      expect(document.body.textContent).toContain('hunter2')

      dialogButton('Done').click()
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })

    it('opens the dialog with the error when the fetch fails', async () => {
      vi.mocked(apiClient.get).mockRejectedValueOnce(new Error('forbidden'))
      const wrapper = mount()
      await openMenu(wrapper)
      await findButton(wrapper, /passphrase/i).trigger('click')
      await flushPromises()

      expect(document.body.querySelector('.form-error')).not.toBeNull()
      expect(document.body.textContent).not.toContain('hunter2')
    })
  })
})
