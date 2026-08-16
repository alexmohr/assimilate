// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import type * as VueRouter from 'vue-router'

type VueRouterModule = typeof VueRouter

const toastSuccess = vi.fn()
const toastError = vi.fn()
const routerPush = vi.fn()

vi.mock('../api/client', () => ({
  apiClient: { post: vi.fn(), delete: vi.fn() },
}))
vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))
vi.mock('vue-router', async (importOriginal) => ({
  ...(await importOriginal<VueRouterModule>()),
  useRouter: () => ({ push: routerPush }),
}))

import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import RepoDangerZone from './RepoDangerZone.vue'
import type { ActiveRepoOp, RepoWithStats } from '../types/repo'

const REPO = {
  id: 12,
  name: 'server-daily',
  repo_path: '/backup/repos/server-daily',
  ssh_host: 'backup.example.com',
  relocation_pending: false,
} as unknown as RepoWithStats

const RUNNING_OP = {
  kind: 'agent_backup',
  actor: 'web-01',
  started_at: '2026-03-01T02:00:00Z',
  queued: 0,
} as ActiveRepoOp

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoDangerZone, {
    props: { repo: REPO, currentOp: null, ...props },
  })
}

/** Every action confirms through a teleported BaseModal. */
function dialogButton(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll<HTMLButtonElement>('.modal-dialog button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!match) throw new Error(`no dialog button labelled "${label}"`)
  return match
}

function action(wrapper: ReturnType<typeof mount>, label: string) {
  const match = wrapper.findAll('button').find((b) => b.text() === label)
  if (!match) throw new Error(`no action button labelled "${label}"`)
  return match
}

/** Opens an action's confirmation dialog and clicks its confirm button. */
async function confirmVia(
  wrapper: ReturnType<typeof mount>,
  open: string,
  confirm: string,
): Promise<void> {
  await action(wrapper, open).trigger('click')
  await flushPromises()
  dialogButton(confirm).click()
  await flushPromises()
}

describe('RepoDangerZone', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    toastSuccess.mockReset()
    toastError.mockReset()
    routerPush.mockReset()
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue({ data: { message: 'ok', borg_output: '' } } as never)
    vi.mocked(apiClient.delete)
      .mockReset()
      .mockResolvedValue({} as never)
  })

  it('lists every destructive action', () => {
    const headings = mount()
      .findAll('.danger-heading')
      .map((h) => h.text())
    expect(headings).toEqual([
      'Confirm Repository Relocation',
      'Break Repository Lock',
      'Remove Repository',
      'Delete Repository',
      'Reset & Re-import',
    ])
  })

  // Every one of these destroys something. None may fire straight from the
  // row button - the confirmation dialog is the only path to the API.
  it.each([
    ['Confirm Relocation'],
    ['Break Lock'],
    ['Remove Repository'],
    ['Delete Repository'],
    ['Reset & Re-import'],
  ])('does not call the API when %s is merely clicked', async (label) => {
    const wrapper = mount()
    await action(wrapper, label).trigger('click')
    await flushPromises()
    expect(apiClient.post).not.toHaveBeenCalled()
    expect(apiClient.delete).not.toHaveBeenCalled()
  })

  // Backing out of a destructive dialog must be a genuine no-op, not just a
  // visual dismissal that still fired the request.
  it.each([
    ['Confirm Relocation'],
    ['Break Lock'],
    ['Remove Repository'],
    ['Delete Repository'],
    ['Reset & Re-import'],
  ])('does nothing when the %s dialog is cancelled', async (label) => {
    const wrapper = mount()
    await action(wrapper, label).trigger('click')
    await flushPromises()

    dialogButton('Cancel').click()
    await flushPromises()

    expect(apiClient.post).not.toHaveBeenCalled()
    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })

  describe('escape key', () => {
    it('dismisses the break-lock dialog', async () => {
      const wrapper = mount()
      await action(wrapper, 'Break Lock').trigger('click')
      await flushPromises()
      expect(document.body.querySelector('.modal-dialog')).not.toBeNull()

      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
      expect(apiClient.post).not.toHaveBeenCalled()
    })

    // Dispatched on window rather than document so only the component's own
    // useEscapeKey handler sees it: BaseModal binds Escape on document and
    // would otherwise close the dialog first, taking the window listener
    // down before the event ever reached it.
    it('closes via its own handler when that is the one that sees the key', async () => {
      const wrapper = mount()
      await action(wrapper, 'Break Lock').trigger('click')
      await flushPromises()

      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
      expect(apiClient.post).not.toHaveBeenCalled()
    })

    // Documents current behaviour, which is not what the component's own
    // useEscapeKey guard reads like: that guard skips closing while the
    // request is in flight, but BaseModal binds Escape too and emits `close`
    // unconditionally, and RepoDangerZone's `@close` handler clears the flag
    // with no guard of its own. So the dialog does close mid-request and the
    // guard is dead code. Harmless - the request still completes and its
    // result lands in state nobody is showing - but the two handlers
    // disagree, and this pins the behaviour that actually ships.
    it('closes mid-request even though the local guard tries to prevent it', async () => {
      let release: (v: unknown) => void = () => {}
      vi.mocked(apiClient.post).mockReturnValueOnce(
        new Promise((resolve) => {
          release = resolve
        }) as never,
      )
      const wrapper = mount()
      await action(wrapper, 'Break Lock').trigger('click')
      await flushPromises()
      dialogButton('Yes, Break Lock').click()
      await flushPromises()

      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
      await flushPromises()
      expect(document.body.querySelector('.modal-dialog')).toBeNull()

      release({ data: { message: 'done', borg_output: '' } })
      await flushPromises()
      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/break-lock')
    })
  })

  describe('confirm relocation', () => {
    it('confirms and asks the parent to refresh the repo row', async () => {
      const wrapper = mount()
      await confirmVia(wrapper, 'Confirm Relocation', 'Yes, Confirm Relocation')

      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/confirm-relocation')
      // relocation_pending lives on the parent's row, so the component asks
      // for a refresh rather than mutating a prop.
      expect(wrapper.emitted('changed')).toHaveLength(1)
    })

    it('shows the server message rather than a generic confirmation', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { message: 'Relocation will apply on the next run' },
      } as never)
      const wrapper = mount()
      await confirmVia(wrapper, 'Confirm Relocation', 'Yes, Confirm Relocation')
      expect(document.body.textContent).toContain('Relocation will apply on the next run')
    })

    it('reports a failure without claiming the flag was set', async () => {
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('nope'))
      const wrapper = mount()
      await confirmVia(wrapper, 'Confirm Relocation', 'Yes, Confirm Relocation')

      expect(document.body.querySelector('.form-error')).not.toBeNull()
      expect(wrapper.emitted('changed')).toBeUndefined()
    })

    it('notes when a relocation is already pending', () => {
      const wrapper = mount({ repo: { ...REPO, relocation_pending: true } })
      expect(wrapper.find('.danger-hint').text()).toContain('already pending')
    })
  })

  describe('break lock', () => {
    // Breaking a lock mid-backup corrupts the repository, so a running
    // operation has to disable the button outright, not just warn.
    it('is disabled while an operation is running, and says which', () => {
      const wrapper = mount({ currentOp: RUNNING_OP })
      const button = action(wrapper, 'Break Lock')
      expect(button.attributes('disabled')).toBeDefined()
      expect(button.attributes('title')).toBe('Agent backup in progress by web-01')
      expect(wrapper.find('.danger-hint').text()).toBe('Agent backup in progress by web-01')
    })

    it('is enabled when the repository is idle', () => {
      expect(action(mount(), 'Break Lock').attributes('disabled')).toBeUndefined()
    })

    it('breaks the lock on confirmation', async () => {
      const wrapper = mount()
      await confirmVia(wrapper, 'Break Lock', 'Yes, Break Lock')
      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/break-lock')
    })

    // borg_output is the only place that says whether a stale cache lock was
    // actually found; message alone is the same static string every time.
    it('shows borg output alongside the message', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { message: 'Lock broken', borg_output: 'Removed stale cache lock' },
      } as never)
      const wrapper = mount()
      await confirmVia(wrapper, 'Break Lock', 'Yes, Break Lock')

      expect(document.body.textContent).toContain('Lock broken')
      expect(document.body.textContent).toContain('Removed stale cache lock')
    })

    it('shows the message alone when borg said nothing further', async () => {
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: { message: 'Lock broken', borg_output: '' },
      } as never)
      const wrapper = mount()
      await confirmVia(wrapper, 'Break Lock', 'Yes, Break Lock')
      expect(document.body.textContent).toContain('Lock broken')
    })

    it('reports a failure in the dialog', async () => {
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('locked'))
      const wrapper = mount()
      await confirmVia(wrapper, 'Break Lock', 'Yes, Break Lock')
      expect(document.body.querySelector('.form-error')).not.toBeNull()
    })
  })

  describe('remove repository', () => {
    it('removes it and leaves the detail page', async () => {
      const wrapper = mount()
      await confirmVia(wrapper, 'Remove Repository', 'Remove')

      expect(apiClient.delete).toHaveBeenCalledWith('/repos/12')
      expect(routerPush).toHaveBeenCalledWith('/repos')
    })

    it('stays put and reports the error when removal fails', async () => {
      vi.mocked(apiClient.delete).mockRejectedValueOnce(new Error('in use'))
      const wrapper = mount()
      await confirmVia(wrapper, 'Remove Repository', 'Remove')

      expect(routerPush).not.toHaveBeenCalled()
      expect(wrapper.emitted('error')).toHaveLength(1)
    })
  })

  describe('delete repository', () => {
    it('names the path and host it is about to rm -rf', async () => {
      const wrapper = mount()
      await action(wrapper, 'Delete Repository').trigger('click')
      await flushPromises()

      const text = document.body.textContent ?? ''
      expect(text).toContain('/backup/repos/server-daily')
      expect(text).toContain('backup.example.com')
      expect(text).toContain('server-daily')
    })

    it('destroys the repository on confirmation and leaves the page', async () => {
      const wrapper = mount()
      await confirmVia(wrapper, 'Delete Repository', 'Destroy Forever')

      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/destroy')
      expect(routerPush).toHaveBeenCalledWith('/repos')
    })

    it('stays put and reports the error when the destroy fails', async () => {
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('ssh down'))
      const wrapper = mount()
      await confirmVia(wrapper, 'Delete Repository', 'Destroy Forever')

      expect(routerPush).not.toHaveBeenCalled()
      expect(wrapper.emitted('error')).toHaveLength(1)
    })
  })

  describe('reset and re-import', () => {
    it('resets on confirmation and reports it', async () => {
      const wrapper = mount()
      await confirmVia(wrapper, 'Reset & Re-import', 'Confirm Reset')

      expect(apiClient.post).toHaveBeenCalledWith('/repos/12/reset-and-sync?build_index=true')
      expect(toastSuccess).toHaveBeenCalled()
    })

    it('surfaces a failure as an error toast', async () => {
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('boom'))
      const wrapper = mount()
      await confirmVia(wrapper, 'Reset & Re-import', 'Confirm Reset')

      expect(toastError).toHaveBeenCalled()
      expect(toastSuccess).not.toHaveBeenCalled()
    })
  })
})
