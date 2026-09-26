// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockToast, resetToastSpies, toastSpies } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../composables/useToast', () => mockToast())

import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
import { dialogButton, findButton } from '../test-utils/dom'
import { apiClient } from '../api/client'
import RepoHostConnectionCard from './RepoHostConnectionCard.vue'
import type { RepoHost } from '../api/repoHosts'

const PINNED = 'ssh-ed25519 AAAAPINNED'
const CHANGED = 'ssh-ed25519 AAAACHANGED'

function host(overrides: Partial<RepoHost> = {}): RepoHost {
  return {
    id: 5,
    ssh_host: 'nas.lan',
    ssh_port: 22,
    ssh_host_key: PINNED,
    intermittent: false,
    power: {
      wake_enabled: false,
      wake_mac_address: null,
      wake_broadcast_address: null,
      wake_timeout_seconds: 180,
      shutdown_after_backup: false,
    },
    repositories: [
      { id: 1, name: 'a', ssh_user: 'borg', repo_path: '/a', enabled: true },
      { id: 2, name: 'b', ssh_user: 'borg', repo_path: '/b', enabled: true },
    ],
    ...overrides,
  }
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoHostConnectionCard, {
    props: { host: host(), canEdit: true, ...props },
  })
}

/** Scans answer with `key` until told otherwise. */
function scanReturns(key: string): void {
  vi.mocked(apiClient.post).mockImplementation(
    (url: string) =>
      Promise.resolve({
        data: url.endsWith('/scan') ? { ssh_host_key: key } : {},
      }) as never,
  )
}

describe('RepoHostConnectionCard', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    resetToastSpies()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.post).mockReset()
    scanReturns(PINNED)
  })

  it('shows where the host is reached and the key it is pinned to', () => {
    const text = mount().text()
    expect(text).toContain('nas.lan')
    expect(text).toContain('22')
    expect(text).toContain('ssh-ed25519')
    expect(text).toContain('AAAAPINNED')
  })

  it('says so when no key is pinned yet', () => {
    expect(mount({ host: host({ ssh_host_key: null }) }).text()).toContain('Not pinned yet')
  })

  it('saves a new address for every repository on the host', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({
      data: host({ ssh_host: 'nas.example.com', ssh_port: 2222 }),
    } as never)
    const wrapper = mount()
    await startEditingSection(wrapper)
    expect(wrapper.text()).toContain('all 2 repositories')

    await wrapper.find('#repo-host-hostname').setValue('  nas.example.com  ')
    await wrapper.find('#repo-host-port').setValue('2222')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/repo-hosts/5', {
      ssh_host: 'nas.example.com',
      ssh_port: 2222,
    })
    expect(wrapper.emitted('saved')?.[0]?.[0]).toMatchObject({ ssh_host: 'nas.example.com' })
  })

  it('stays in edit mode and shows why when the address is refused', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('there is already a host named nas'))
    const wrapper = mount()
    await expectSaveErrorKeepsEditing(wrapper, 'already a host', '#repo-host-hostname')
  })

  it('hides every control from a viewer who cannot edit', async () => {
    const wrapper = mount({ canEdit: false })
    await flushPromises()
    const labels = wrapper.findAll('button').map((b) => b.text().trim())
    expect(labels).not.toContain('Edit')
    expect(labels).not.toContain('Scan key')
    expect(apiClient.post).not.toHaveBeenCalled()
  })

  describe('host key', () => {
    it('checks the key on mount and stays quiet when it matches', async () => {
      const wrapper = mount()
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repo-hosts/5/ssh-host-key/scan')
      expect(wrapper.text()).not.toContain('different key')
    })

    // A changed key is the signature of a reinstall or of a man-in-the-middle:
    // it is flagged, and only accepted after someone has looked at it.
    it('flags a changed key and accepts it on confirmation', async () => {
      scanReturns(CHANGED)
      const wrapper = mount()
      await flushPromises()
      expect(wrapper.text()).toContain('different key than the one pinned for it')

      await findButton(wrapper, /^Review key$/).trigger('click')
      await flushPromises()
      expect(document.body.textContent).toContain(CHANGED)
      dialogButton('Accept key').click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/repo-hosts/5/ssh-host-key', {
        ssh_host_key: CHANGED,
      })
      expect(wrapper.emitted('saved')?.[0]?.[0]).toMatchObject({ ssh_host_key: CHANGED })
      expect(toastSpies.success).toHaveBeenCalled()
      expect(wrapper.text()).not.toContain('different key than the one pinned for it')
    })

    // Declining is the safe default: cancelling leaves the pinned key alone.
    it('records nothing when the operator cancels instead of accepting', async () => {
      scanReturns(CHANGED)
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /^Review key$/).trigger('click')
      await flushPromises()
      vi.mocked(apiClient.post).mockClear()

      dialogButton('Cancel').click()
      await flushPromises()

      expect(apiClient.post).not.toHaveBeenCalled()
      expect(wrapper.emitted('saved')).toBeUndefined()
      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })

    it('keeps the dialog open with the error when accepting fails', async () => {
      scanReturns(CHANGED)
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /^Review key$/).trigger('click')
      await flushPromises()
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('denied'))

      dialogButton('Accept key').click()
      await flushPromises()

      expect(document.body.querySelector('.form-error')?.textContent).toContain('denied')
      expect(wrapper.emitted('saved')).toBeUndefined()
    })

    // The host may simply be asleep; that says nothing about its key.
    it('treats a failed check as "no mismatch known" rather than an alarm', async () => {
      vi.mocked(apiClient.post).mockRejectedValue(new Error('unreachable'))
      const wrapper = mount()
      await flushPromises()
      expect(wrapper.text()).not.toContain('different key')
    })

    it('confirms on a manual scan when the host still presents the pinned key', async () => {
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /^Scan key$/).trigger('click')
      await flushPromises()
      expect(toastSpies.success).toHaveBeenCalledWith(
        'The host presents the key already pinned for it',
      )
      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })

    it('offers a manually scanned key for a host with none pinned', async () => {
      const wrapper = mount({ host: host({ ssh_host_key: null }) })
      await flushPromises()
      await findButton(wrapper, /^Scan key$/).trigger('click')
      await flushPromises()
      expect(document.body.textContent).toContain(PINNED)
      expect(document.body.textContent).toContain('presents this key')
    })

    it('reports a failed manual scan', async () => {
      const wrapper = mount()
      await flushPromises()
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('connection refused'))
      await findButton(wrapper, /^Scan key$/).trigger('click')
      await flushPromises()
      expect(toastSpies.error).toHaveBeenCalledWith('connection refused')
    })
  })
})
