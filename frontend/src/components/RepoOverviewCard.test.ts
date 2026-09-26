// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockToast, resetToastSpies } from '../test-utils/sharedMocks'
import type { ActiveRepoOp } from '../types/repo'

// The card owns the edit form and the host-key check it runs on mount.
vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../composables/useToast', () => mockToast())

import { renderWithPlugins } from '../test-utils'
import { findButton } from '../test-utils/dom'
import { repoFixture as repo } from '../test-utils/repoFixtures'
import { apiClient } from '../api/client'
import RepoOverviewCard from './RepoOverviewCard.vue'

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoOverviewCard, {
    props: {
      repo: repo(),
      isAdmin: true,
      currentOp: null,
      ...props,
    },
  })
}

describe('RepoOverviewCard', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    resetToastSpies()
    vi.mocked(apiClient.get)
      .mockReset()
      .mockImplementation(
        (url: string) =>
          Promise.resolve({
            data:
              url === '/repo-hosts'
                ? [
                    { id: 5, ssh_host: 'backup.example.com', ssh_port: 22 },
                    { id: 8, ssh_host: 'nas.lan', ssh_port: 2222 },
                  ]
                : { passphrase: 'hunter2' },
          }) as never,
      )
    vi.mocked(apiClient.put)
      .mockReset()
      .mockResolvedValue({} as never)
    // The card scans its host's key on mount; default to "unchanged".
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue({ data: { ssh_host_key: repo().ssh_host_key } } as never)
  })

  describe('info grid', () => {
    it('renders the connection details', () => {
      const text = mount().text()
      expect(text).toContain('server-daily')
      expect(text).toContain('backup.example.com:22')
      expect(text).toContain('/backup/repos/server-daily')
      expect(mount().find('a[href="/repo-hosts/5"]').exists()).toBe(true)
    })

    it('names the last operation rather than echoing its wire value', () => {
      expect(mount().text()).toContain('Agent backup')
    })

    it.each([
      ['server_sync', 'Server sync'],
      ['break_lock', 'Break lock'],
      ['delete_archive', 'Delete archive'],
      ['agent_check', 'Integrity check'],
      ['agent_verify', 'Verify'],
      ['compact_repo', 'Compact repository'],
    ])('labels a %s last operation', (kind, label) => {
      expect(mount({ repo: repo({ last_op_kind: kind }) }).text()).toContain(label)
    })

    it('passes an unrecognized operation kind through rather than blanking it', () => {
      expect(mount({ repo: repo({ last_op_kind: 'future_op' }) }).text()).toContain('future_op')
    })

    it('reads Never when no operation has run', () => {
      const wrapper = mount({ repo: repo({ last_op_kind: null, last_op_at: null }) })
      expect(wrapper.text()).toContain('Never')
    })

    it('describes the running operation when one is active', () => {
      const op = {
        kind: 'server_sync',
        actor: 'web-01',
        started_at: '2026-03-01T02:00:00Z',
        queued: 2,
      } as ActiveRepoOp
      const wrapper = mount({ currentOp: op })
      expect(wrapper.find('.current-op-running').text()).toBe('Server sync in progress (+2 queued)')
    })

    it('discloses the connection details explanation on demand', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: connection details"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toBe(
        'Where this repository lives and how borg writes to it.',
      )
    })

    it('explains what the repository host is on demand', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: the repository host"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain(
        'set on the host, once for every repository on it',
      )
    })
  })

  describe('edit mode', () => {
    async function startEditing() {
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /^Edit$/).trigger('click')
      return wrapper
    }

    it('prefills the form from the repository', async () => {
      const wrapper = await startEditing()
      const inputs = wrapper.findAll('.edit-form input')
      expect((inputs[0].element as HTMLInputElement).value).toBe('server-daily')
    })

    // The wire value carries borg's level suffix ("zstd,6"); the select only
    // has the bare algorithms, so an unnormalized value would render blank.
    it('strips the compression level so the select can match it', async () => {
      const wrapper = await startEditing()
      const select = wrapper.find('select[aria-label="Compression"]')
      expect((select.element as HTMLSelectElement).value).toBe('zstd')
    })

    it('falls back to lz4 for a compression the select does not offer', async () => {
      const wrapper = mount({ repo: repo({ compression: 'brotli' }) })
      await flushPromises()
      await findButton(wrapper, /^Edit$/).trigger('click')
      expect(
        (wrapper.find('select[aria-label="Compression"]').element as HTMLSelectElement).value,
      ).toBe('lz4')
    })

    it('leaves edit mode without saving on cancel', async () => {
      const wrapper = await startEditing()
      await findButton(wrapper, /^Cancel$/).trigger('click')
      expect(wrapper.find('.edit-form').exists()).toBe(false)
      expect(apiClient.put).not.toHaveBeenCalled()
    })

    it('tests the connection before saving, and saves when it succeeds', async () => {
      const wrapper = await startEditing()
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_ok: true, borg_installed: true },
      } as never)

      await findButton(wrapper, /^Save/).trigger('click')
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/ssh/test-connection', {
        ssh_host: 'backup.example.com',
        ssh_user: 'borg',
        ssh_port: 22,
      })
      expect(apiClient.put).toHaveBeenCalled()
      expect(wrapper.emitted('saved')).toHaveLength(1)
    })

    // Drives every editable field, so a v-model wired to the wrong form key
    // is caught here rather than silently saving the old value.
    it('sends every edited field, trimming the ones that are pasted', async () => {
      const wrapper = await startEditing()

      await wrapper.find('input[placeholder="e.g. Web Server Backup"]').setValue('  renamed  ')
      await wrapper.find('#repo-host').setValue('new')
      await wrapper.find('#repo-ssh-host').setValue('  new.example.com  ')
      await wrapper.find('.edit-form input[type="number"]').setValue('2222')
      await wrapper.find('#repo-ssh-user').setValue('  operator  ')
      await wrapper.find('#repo-path').setValue('  /srv/borg  ')

      await wrapper.find('select[aria-label="Compression"]').setValue('zlib')
      await wrapper.find('select[aria-label="Encryption"]').setValue('keyfile')

      const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })
      await toggles[0].vm.$emit('update:modelValue', false)

      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_ok: true, borg_installed: true },
      } as never)
      await findButton(wrapper, /^Save/).trigger('click')
      await flushPromises()

      expect(apiClient.put).toHaveBeenCalledWith('/repos/12', {
        name: 'renamed',
        repo_path: '/srv/borg',
        ssh_user: 'operator',
        ssh_host: 'new.example.com',
        ssh_port: 2222,
        compression: 'zlib',
        encryption: 'keyfile',
        enabled: false,
        sync_schedule: null,
      })
    })

    it('tests the connection against the edited host, not the saved one', async () => {
      const wrapper = await startEditing()
      await wrapper.find('#repo-host').setValue('new')
      await wrapper.find('#repo-ssh-host').setValue('moved.example.com')

      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_ok: true, borg_installed: true },
      } as never)
      await findButton(wrapper, /^Save/).trigger('click')
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/ssh/test-connection', {
        ssh_host: 'moved.example.com',
        ssh_user: 'borg',
        ssh_port: 22,
      })
    })

    // Picking a known host takes its hostname and its port: a host has one
    // port, and the server refuses a repository that names another.
    it('moves the repository to a known host, port included', async () => {
      const wrapper = await startEditing()
      await flushPromises()
      expect((wrapper.find('#repo-host').element as HTMLSelectElement).value).toBe('5')
      expect(wrapper.find('#repo-ssh-host').exists()).toBe(false)

      await wrapper.find('#repo-host').setValue('8')
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_ok: true, borg_installed: true },
      } as never)
      await findButton(wrapper, /^Save/).trigger('click')
      await flushPromises()

      expect(apiClient.put).toHaveBeenCalledWith(
        '/repos/12',
        expect.objectContaining({ ssh_host: 'nas.lan', ssh_port: 2222 }),
      )
    })

    it('explains what moving to another host takes with it', async () => {
      const wrapper = await startEditing()
      await wrapper.find('[aria-label="Help: moving to another host"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain("takes that host's port")
    })

    // The host list is best-effort: without it the repository's own host and a
    // new one can still be chosen.
    it('still offers its own host and a new one when the host list fails to load', async () => {
      vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
      const wrapper = await startEditing()
      await flushPromises()

      const options = wrapper.findAll('#repo-host option').map((o) => o.text())
      expect(options).toEqual(['backup.example.com:22', 'Add a new host...'])
    })

    it('saves the cron expression once disk sync is enabled', async () => {
      const wrapper = await startEditing()
      const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })
      await toggles[toggles.length - 1].vm.$emit('update:modelValue', true)
      await flushPromises()

      await wrapper
        .findComponent({ name: 'CronBuilder' })
        .vm.$emit('update:modelValue', '30 3 * * *')

      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_ok: true, borg_installed: true },
      } as never)
      await findButton(wrapper, /^Save/).trigger('click')
      await flushPromises()

      expect(apiClient.put).toHaveBeenCalledWith(
        '/repos/12',
        expect.objectContaining({ sync_schedule: '30 3 * * *' }),
      )
    })

    it('clears the schedule again when disk sync is turned back off', async () => {
      const wrapper = mount({ repo: repo({ sync_schedule: '0 0,12 * * *' }) })
      await flushPromises()
      await findButton(wrapper, /^Edit$/).trigger('click')

      const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })
      await toggles[toggles.length - 1].vm.$emit('update:modelValue', false)
      await flushPromises()

      expect(wrapper.findComponent({ name: 'CronBuilder' }).exists()).toBe(false)
    })

    // Saving an unreachable host would leave a repo that silently fails every
    // backup, so the connection test is a gate and not just a warning.
    it('refuses to save when the host is unreachable, and says why', async () => {
      const wrapper = await startEditing()
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_ok: false, borg_installed: false, error: 'connection refused' },
      } as never)

      await findButton(wrapper, /^Save/).trigger('click')
      await flushPromises()

      expect(apiClient.put).not.toHaveBeenCalled()
      expect(wrapper.find('.form-error').text()).toContain('connection refused')
      expect(wrapper.emitted('saved')).toBeUndefined()
    })

    it('reports a save failure in the form rather than closing it', async () => {
      const wrapper = await startEditing()
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_ok: true, borg_installed: true },
      } as never)
      vi.mocked(apiClient.put).mockRejectedValueOnce(new Error('boom'))

      await findButton(wrapper, /^Save/).trigger('click')
      await flushPromises()

      expect(wrapper.find('.form-error').exists()).toBe(true)
      expect(wrapper.find('.edit-form').exists()).toBe(true)
    })

    it('reveals the cron field only once disk sync is turned on', async () => {
      const wrapper = await startEditing()
      expect(wrapper.findAll('.input.mono').some((i) => i.attributes('placeholder'))).toBe(false)

      const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })
      await toggles[toggles.length - 1].vm.$emit('update:modelValue', true)
      await flushPromises()

      expect(wrapper.text()).toContain('Sync schedule')
    })
  })

  // The key is pinned on the repository host now, and accepted there for
  // every repository on it. The card still checks it on mount, so a changed
  // key surfaces here rather than only as the next failed backup.
  describe('ssh host key', () => {
    it("scans its host's key on mount and stays quiet when it matches", async () => {
      const wrapper = mount()
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repo-hosts/5/ssh-host-key/scan')
      expect(wrapper.text()).not.toContain('different SSH host key')
    })

    it('re-scans when the card switches to a repository on another host', async () => {
      const wrapper = mount()
      await flushPromises()
      vi.mocked(apiClient.post).mockClear()

      await wrapper.setProps({ repo: repo({ id: 99, repo_host: { id: 9, intermittent: false } }) })
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/repo-hosts/9/ssh-host-key/scan')
    })

    it("re-scans when its host's address changes, and not for an unchanged refresh", async () => {
      const wrapper = mount()
      await flushPromises()
      vi.mocked(apiClient.post).mockClear()

      await wrapper.setProps({ repo: repo() })
      await flushPromises()
      expect(apiClient.post).not.toHaveBeenCalled()

      await wrapper.setProps({ repo: repo({ ssh_host: 'backup.example.org' }) })
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repo-hosts/5/ssh-host-key/scan')
    })

    // A changed host key is the signature of a man-in-the-middle, so it has
    // to surface in the UI rather than only failing the next backup.
    it('flags a changed host key and points at the host to review it', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_host_key: 'ssh-ed25519 AAAADIFFERENT' },
      } as never)
      const wrapper = mount()
      await flushPromises()
      expect(wrapper.text()).toContain('different SSH host key')
      expect(wrapper.find('a[href="/repo-hosts/5?section=connection"]').exists()).toBe(true)
    })

    // A scan failure is logged, not surfaced: the host being briefly
    // unreachable is not evidence that its key changed.
    it('treats a failed scan as "no mismatch known" rather than an alarm', async () => {
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('unreachable'))
      const wrapper = mount()
      await flushPromises()
      expect(wrapper.text()).not.toContain('different SSH host key')
    })

    it('does not scan for a viewer who could not act on it', async () => {
      mount({ isAdmin: false })
      await flushPromises()
      expect(apiClient.post).not.toHaveBeenCalled()
    })
  })
})
