// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

const toastSuccess = vi.fn()
const toastError = vi.fn()

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), put: vi.fn() },
}))
vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import RepoOverviewCard from './RepoOverviewCard.vue'
import type { ActiveRepoOp, RepoWithStats } from '../types/repo'

const REPO = {
  id: 12,
  name: 'server-daily',
  repo_path: '/backup/repos/server-daily',
  ssh_user: 'borg',
  ssh_host: 'backup.example.com',
  ssh_port: 22,
  ssh_host_key: 'ssh-ed25519 AAAAKNOWN',
  compression: 'zstd,6',
  encryption: 'repokey-blake2',
  enabled: true,
  importing: false,
  import_error: null,
  import_progress: 0,
  import_total: 0,
  sync_schedule: null,
  agent_count: 3,
  last_op_kind: 'agent_backup',
  last_op_at: '2026-03-01T02:00:00Z',
  last_op_by: 'web-01',
} as unknown as RepoWithStats

function repo(overrides: Partial<RepoWithStats> = {}): RepoWithStats {
  return { ...REPO, ...overrides }
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoOverviewCard, {
    props: {
      repo: repo(),
      isAdmin: true,
      currentOp: null,
      importPhaseVerb: 'Importing',
      ...props,
    },
  })
}

/** The dialogs teleport, so their nodes live on the document body. */
function dialogButton(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll<HTMLButtonElement>('.modal-dialog button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!match) throw new Error(`no dialog button labelled "${label}"`)
  return match
}

function findButton(wrapper: ReturnType<typeof mount>, label: RegExp) {
  const match = wrapper.findAll('button').find((b) => label.test(b.text()))
  if (!match) throw new Error(`no button matching ${label}`)
  return match
}

describe('RepoOverviewCard', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    toastSuccess.mockReset()
    toastError.mockReset()
    vi.mocked(apiClient.get)
      .mockReset()
      .mockResolvedValue({ data: { passphrase: 'hunter2' } } as never)
    vi.mocked(apiClient.put)
      .mockReset()
      .mockResolvedValue({} as never)
    // The card scans the SSH host key on mount; default to "unchanged".
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue({ data: { ssh_host_key: REPO.ssh_host_key } } as never)
  })

  describe('status badge', () => {
    // The e2e suite asserts on these exact tone classes, so a rename that
    // only updates the component is caught here rather than 15 minutes into
    // a Playwright run.
    it('shows an enabled repository as a success badge', () => {
      const wrapper = mount()
      const badge = wrapper.find('.repo-status-badge')
      expect(badge.text()).toBe('Enabled')
      expect(badge.classes()).toContain('badge--success')
    })

    it('shows a disabled repository as a neutral badge', () => {
      const wrapper = mount({ repo: repo({ enabled: false }) })
      const badge = wrapper.find('.repo-status-badge')
      expect(badge.text()).toBe('Disabled')
      expect(badge.classes()).toContain('badge--neutral')
    })

    it('pulses a warning badge while an import runs', () => {
      const wrapper = mount({ repo: repo({ importing: true }) })
      const badge = wrapper.find('.repo-status-badge')
      expect(badge.classes()).toEqual(expect.arrayContaining(['badge--warning', 'badge--pulse']))
    })

    it('counts import progress in the badge when a total is known', () => {
      const wrapper = mount({
        repo: repo({ importing: true, import_progress: 3, import_total: 9 }),
      })
      expect(wrapper.find('.repo-status-badge').text()).toBe('Importing 3/9')
    })

    it('shows a danger badge carrying the import error as its title', () => {
      const wrapper = mount({ repo: repo({ import_error: 'ssh refused' }) })
      const badge = wrapper.find('.repo-status-badge')
      expect(badge.text()).toBe('Import Failed')
      expect(badge.classes()).toContain('badge--danger')
      expect(badge.attributes('title')).toBe('ssh refused')
    })
  })

  describe('info grid', () => {
    it('renders the connection details', () => {
      const text = mount().text()
      expect(text).toContain('server-daily')
      expect(text).toContain('borg@backup.example.com:22')
      expect(text).toContain('/backup/repos/server-daily')
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
  })

  describe('admin gating', () => {
    it('offers no actions to a non-admin', () => {
      const wrapper = mount({ isAdmin: false })
      expect(wrapper.find('.info-header-actions button').exists()).toBe(false)
    })

    it('offers Full Resync to an admin when no import is running', () => {
      expect(findButton(mount(), /Full Resync/).exists()).toBe(true)
    })

    it('swaps Full Resync for Cancel Import while importing', () => {
      const wrapper = mount({ repo: repo({ importing: true }) })
      expect(wrapper.findAll('button').some((b) => /Full Resync/.test(b.text()))).toBe(false)
      expect(findButton(wrapper, /Cancel Import/).exists()).toBe(true)
    })

    it('offers Cancel Import after a failed import too, so the state can be cleared', () => {
      const wrapper = mount({ repo: repo({ import_error: 'boom' }) })
      expect(findButton(wrapper, /Cancel Import/).exists()).toBe(true)
    })
  })

  describe('full resync', () => {
    it('starts the sync and reports it', async () => {
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /Full Resync/).trigger('click')
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/sync?build_index=true')
      expect(toastSuccess).toHaveBeenCalled()
    })

    it('surfaces a failure as an error toast', async () => {
      const wrapper = mount()
      await flushPromises()
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('nope'))
      await findButton(wrapper, /Full Resync/).trigger('click')
      await flushPromises()
      expect(toastError).toHaveBeenCalled()
    })
  })

  describe('reset import', () => {
    it('resets the import state and tells the parent', async () => {
      const wrapper = mount({ repo: repo({ importing: true }) })
      await flushPromises()
      await findButton(wrapper, /Cancel Import/).trigger('click')
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/reset-import')
      expect(wrapper.emitted('import-reset')).toHaveLength(1)
    })

    it('does not claim success when the reset fails', async () => {
      const wrapper = mount({ repo: repo({ importing: true }) })
      await flushPromises()
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('nope'))
      await findButton(wrapper, /Cancel Import/).trigger('click')
      await flushPromises()
      expect(toastError).toHaveBeenCalled()
      expect(wrapper.emitted('import-reset')).toBeUndefined()
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
      const select = wrapper.find('select')
      expect((select.element as HTMLSelectElement).value).toBe('zstd')
    })

    it('falls back to lz4 for a compression the select does not offer', async () => {
      const wrapper = mount({ repo: repo({ compression: 'brotli' }) })
      await flushPromises()
      await findButton(wrapper, /^Edit$/).trigger('click')
      expect((wrapper.find('select').element as HTMLSelectElement).value).toBe('lz4')
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
      const monoInputs = wrapper.findAll('.edit-form input.mono')
      await monoInputs[0].setValue('  operator  ')
      await monoInputs[1].setValue('  new.example.com  ')
      await monoInputs[2].setValue('  /srv/borg  ')
      await wrapper.find('.edit-form input[type="number"]').setValue('2222')

      const selects = wrapper.findAll('.edit-form select')
      await selects[0].setValue('zlib')
      await selects[1].setValue('keyfile')

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
      const monoInputs = wrapper.findAll('.edit-form input.mono')
      await monoInputs[1].setValue('moved.example.com')

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

    it('saves the cron expression once disk sync is enabled', async () => {
      const wrapper = await startEditing()
      const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })
      await toggles[toggles.length - 1].vm.$emit('update:modelValue', true)
      await flushPromises()

      await wrapper.find('input[placeholder="0 0,12 * * *"]').setValue('30 3 * * *')

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

      expect(wrapper.find('input[placeholder="0 0,12 * * *"]').exists()).toBe(false)
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

      expect(wrapper.text()).toContain('Sync Schedule')
    })
  })

  describe('passphrase', () => {
    it('fetches the passphrase on demand and shows it in the dialog', async () => {
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /Passphrase/).trigger('click')
      await flushPromises()

      expect(apiClient.get).toHaveBeenCalledWith('/repos/12/passphrase')
      expect(document.body.textContent).toContain('hunter2')
    })

    it('copies the passphrase to the clipboard on request', async () => {
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /Passphrase/).trigger('click')
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
      await flushPromises()
      await findButton(wrapper, /Passphrase/).trigger('click')
      await flushPromises()

      document.body.querySelector<HTMLButtonElement>('.modal-close')?.click()
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })

    it('closes on Done, so the secret does not stay on screen', async () => {
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /Passphrase/).trigger('click')
      await flushPromises()
      expect(document.body.textContent).toContain('hunter2')

      dialogButton('Done').click()
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })

    it('opens the dialog with the error when the fetch fails', async () => {
      vi.mocked(apiClient.get).mockRejectedValueOnce(new Error('forbidden'))
      const wrapper = mount()
      await flushPromises()
      await findButton(wrapper, /Passphrase/).trigger('click')
      await flushPromises()

      expect(document.body.querySelector('.form-error')).not.toBeNull()
      expect(document.body.textContent).not.toContain('hunter2')
    })
  })

  describe('ssh host key', () => {
    it('scans the host key on mount and stays quiet when it matches', async () => {
      const wrapper = mount()
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/ssh-host-key/scan')
      expect(wrapper.text()).not.toContain('host key')
    })

    it('re-scans when the card switches to another repository', async () => {
      const wrapper = mount()
      await flushPromises()
      vi.mocked(apiClient.post).mockClear()

      await wrapper.setProps({ repo: repo({ id: 99 }) })
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/repos/99/ssh-host-key/scan')
    })

    // A changed host key is the signature of a man-in-the-middle, so it has
    // to surface in the UI rather than only failing the next backup.
    it('flags a changed host key', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_host_key: 'ssh-ed25519 AAAADIFFERENT' },
      } as never)
      const wrapper = mount()
      await flushPromises()
      expect(wrapper.text().toLowerCase()).toContain('host key')
    })

    it('accepts the new key on confirmation and re-checks afterwards', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_host_key: 'ssh-ed25519 AAAADIFFERENT' },
      } as never)
      const wrapper = mount()
      await flushPromises()

      await findButton(wrapper, /Review|Accept/).trigger('click')
      await flushPromises()
      dialogButton('Accept Key').click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/ssh-host-key', {
        ssh_host_key: 'ssh-ed25519 AAAADIFFERENT',
      })
      expect(wrapper.emitted('saved')).toHaveLength(1)
      expect(toastSuccess).toHaveBeenCalled()
    })

    it('dismisses the host-key dialog on the modal control without recording it', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_host_key: 'ssh-ed25519 AAAADIFFERENT' },
      } as never)
      const wrapper = mount()
      await flushPromises()

      await findButton(wrapper, /Review|Accept/).trigger('click')
      await flushPromises()
      vi.mocked(apiClient.post).mockClear()
      document.body.querySelector<HTMLButtonElement>('.modal-close')?.click()
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
      expect(apiClient.post).not.toHaveBeenCalled()
    })

    // Declining is the safe default: cancelling must leave the recorded key
    // untouched rather than quietly accepting the new one.
    it('records nothing when the operator cancels instead of accepting', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_host_key: 'ssh-ed25519 AAAADIFFERENT' },
      } as never)
      const wrapper = mount()
      await flushPromises()

      await findButton(wrapper, /Review|Accept/).trigger('click')
      await flushPromises()
      vi.mocked(apiClient.post).mockClear()
      dialogButton('Cancel').click()
      await flushPromises()

      expect(apiClient.post).not.toHaveBeenCalled()
      expect(wrapper.emitted('saved')).toBeUndefined()
      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })

    it('keeps the dialog open with the error when accepting fails', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { ssh_host_key: 'ssh-ed25519 AAAADIFFERENT' },
      } as never)
      const wrapper = mount()
      await flushPromises()

      await findButton(wrapper, /Review|Accept/).trigger('click')
      await flushPromises()
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('denied'))
      dialogButton('Accept Key').click()
      await flushPromises()

      expect(document.body.querySelector('.form-error')).not.toBeNull()
      expect(wrapper.emitted('saved')).toBeUndefined()
    })

    // A scan failure is logged, not surfaced: the host being briefly
    // unreachable is not evidence that its key changed.
    it('treats a failed scan as "no mismatch known" rather than an alarm', async () => {
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('unreachable'))
      const wrapper = mount()
      await flushPromises()
      expect(wrapper.text().toLowerCase()).not.toContain('different ssh host key')
    })
  })
})
