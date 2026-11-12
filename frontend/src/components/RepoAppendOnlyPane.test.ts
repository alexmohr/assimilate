// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('../api/appendOnly', () => ({
  checkAppendOnly: vi.fn(),
  getAppendOnlyStatus: vi.fn(),
  keepArchive: vi.fn(),
  reapplyAppendOnly: vi.fn(),
  resumeMaintenance: vi.fn(),
  runMaintenance: vi.fn(),
  setAppendOnly: vi.fn(),
  updateMaintenance: vi.fn(),
}))
vi.mock('../utils/logger', () => ({ logger: { error: vi.fn() } }))
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))
vi.mock('./CronBuilder.vue', () => ({ default: { template: '<div class="cron-stub" />' } }))

import {
  checkAppendOnly,
  getAppendOnlyStatus,
  reapplyAppendOnly,
  setAppendOnly,
} from '../api/appendOnly'
import { logger } from '../utils/logger'
import { renderWithPlugins } from '../test-utils'
import { runMaintenance, updateMaintenance } from '../api/appendOnly'
import { appendOnlyStatusFixture } from '../test-utils/appendOnlyFixtures'
import { repoFixture } from '../test-utils/repoFixtures'
import { findButton } from '../test-utils/dom'
import { pushWs, resetWsMock } from '../test-utils/wsMock'
import RepoAppendOnlyPane from './RepoAppendOnlyPane.vue'
import type { ActiveRepoOp } from '../types/repo'
import type { AppendOnlyStatusResponse } from '../types/generated'

const OFF = appendOnlyStatusFixture({
  enabled: false,
  installed_at: null,
  check: null,
  maintenance_key: null,
  maintenance: null,
})

async function mount(status: AppendOnlyStatusResponse, currentOp: ActiveRepoOp | null = null) {
  vi.mocked(getAppendOnlyStatus).mockResolvedValue(status)
  const wrapper = renderWithPlugins(RepoAppendOnlyPane, {
    props: { repo: repoFixture({ id: 12 }), currentOp },
  })
  await flushPromises()
  return wrapper
}

function toggle(wrapper: Awaited<ReturnType<typeof mount>>) {
  return wrapper.find('[role="switch"][aria-label="Append-only"]')
}

describe('RepoAppendOnlyPane', () => {
  beforeEach(() => {
    resetWsMock()
    vi.mocked(getAppendOnlyStatus).mockReset()
    vi.mocked(setAppendOnly).mockReset()
    vi.mocked(reapplyAppendOnly).mockReset()
    vi.mocked(checkAppendOnly).mockReset()
  })

  it('shows a load failure instead of the pane', async () => {
    vi.mocked(getAppendOnlyStatus).mockRejectedValue(new Error('no way'))
    const wrapper = renderWithPlugins(RepoAppendOnlyPane, {
      props: { repo: repoFixture({ id: 12 }), currentOp: null },
    })
    await flushPromises()

    expect(wrapper.find('.error-banner').text()).toContain('no way')
    expect(wrapper.find('[role="switch"]').exists()).toBe(false)
  })

  it('shows only the switch while append-only is off', async () => {
    const wrapper = await mount(OFF)

    expect(toggle(wrapper).attributes('aria-checked')).toBe('false')
    expect(wrapper.text()).not.toContain('Connection check')
    expect(wrapper.text()).not.toContain('Run maintenance now')
  })

  it('names the repositories that share the login', async () => {
    const wrapper = await mount(appendOnlyStatusFixture())
    const callout = wrapper.find('.callout--info')

    expect(callout.text()).toContain('borg@backup.lan')
    expect(callout.text()).toContain('laptops')
    expect(callout.text()).toContain('photos')
    expect(callout.text()).toContain('its own SSH user')
  })

  it('turns append-only on and reports the change', async () => {
    const on = appendOnlyStatusFixture()
    vi.mocked(setAppendOnly).mockResolvedValue(on)
    const wrapper = await mount(OFF)

    await toggle(wrapper).trigger('click')
    await flushPromises()

    expect(setAppendOnly).toHaveBeenCalledWith(12, { enabled: true, password: null })
    expect(wrapper.emitted('changed')).toHaveLength(1)
    expect(toggle(wrapper).attributes('aria-checked')).toBe('true')
    expect(wrapper.text()).toContain('Connection check')
  })

  it('does not let a reload that started before a toggle undo it', async () => {
    const on = appendOnlyStatusFixture()
    vi.mocked(setAppendOnly).mockResolvedValue(on)
    const wrapper = await mount(OFF)

    // A reload begins while the toggle is in flight and answers last, with
    // the state from before the toggle.
    let answerReload: (status: AppendOnlyStatusResponse) => void = () => {}
    vi.mocked(getAppendOnlyStatus).mockReturnValueOnce(
      new Promise<AppendOnlyStatusResponse>((resolve) => {
        answerReload = resolve
      }),
    )
    pushWs('DataChanged', {})
    await toggle(wrapper).trigger('click')
    await flushPromises()
    answerReload(OFF)
    await flushPromises()

    expect(toggle(wrapper).attributes('aria-checked')).toBe('true')
  })

  it('turns append-only off', async () => {
    vi.mocked(setAppendOnly).mockResolvedValue(OFF)
    const wrapper = await mount(appendOnlyStatusFixture())

    await toggle(wrapper).trigger('click')
    await flushPromises()

    expect(setAppendOnly).toHaveBeenCalledWith(12, { enabled: false, password: null })
    expect(toggle(wrapper).attributes('aria-checked')).toBe('false')
  })

  it('shows the message when turning it off is refused', async () => {
    vi.mocked(setAppendOnly).mockRejectedValue(new Error('2 deletions are queued'))
    const wrapper = await mount(appendOnlyStatusFixture())

    await toggle(wrapper).trigger('click')
    await flushPromises()

    expect(wrapper.find('[role="alert"]').text()).toContain('2 deletions are queued')
    expect(toggle(wrapper).attributes('aria-checked')).toBe('true')
  })

  describe('when the server key cannot edit the host', () => {
    const NEEDS_PASSWORD = appendOnlyStatusFixture({
      ...OFF,
      setup_failure: { needs_password: true, message: 'SFTP is not available for this key.' },
    })

    it('asks for the SSH password only then', async () => {
      expect((await mount(OFF)).find('input[type="password"]').exists()).toBe(false)

      const wrapper = await mount(NEEDS_PASSWORD)
      expect(wrapper.find('input[type="password"]').exists()).toBe(true)
      expect(wrapper.text()).toContain('SFTP is not available for this key.')
    })

    it('retries turning it on with the password', async () => {
      vi.mocked(setAppendOnly).mockResolvedValueOnce(NEEDS_PASSWORD)
      const wrapper = await mount(OFF)
      await toggle(wrapper).trigger('click')
      await flushPromises()
      expect(setAppendOnly).toHaveBeenLastCalledWith(12, { enabled: true, password: null })

      vi.mocked(setAppendOnly).mockResolvedValueOnce(appendOnlyStatusFixture())
      await wrapper.find('input[type="password"]').setValue('hunter2')
      await wrapper.find('form').trigger('submit')
      await flushPromises()

      expect(setAppendOnly).toHaveBeenLastCalledWith(12, { enabled: true, password: 'hunter2' })
      expect(wrapper.find('input[type="password"]').exists()).toBe(false)
    })

    it('retries a reapply with the password', async () => {
      vi.mocked(reapplyAppendOnly).mockResolvedValueOnce({
        ...appendOnlyStatusFixture(),
        setup_failure: { needs_password: true, message: 'Locked out.' },
      })
      const wrapper = await mount(appendOnlyStatusFixture())
      await findButton(wrapper, /^Reapply$/).trigger('click')
      await flushPromises()
      expect(reapplyAppendOnly).toHaveBeenLastCalledWith(12, { password: null })

      vi.mocked(reapplyAppendOnly).mockResolvedValueOnce(appendOnlyStatusFixture())
      await wrapper.find('input[type="password"]').setValue('pw')
      await wrapper.find('form').trigger('submit')
      await flushPromises()

      expect(reapplyAppendOnly).toHaveBeenLastCalledWith(12, { password: 'pw' })
    })
  })

  it('does not ask the parent to refetch after a setup that failed', async () => {
    vi.mocked(setAppendOnly).mockResolvedValueOnce(
      appendOnlyStatusFixture({
        ...OFF,
        setup_failure: { needs_password: true, message: 'SFTP is not available for this key.' },
      }),
    )
    const wrapper = await mount(OFF)

    await toggle(wrapper).trigger('click')
    await flushPromises()

    expect(setAppendOnly).toHaveBeenCalledTimes(1)
    expect(wrapper.emitted('changed')).toBeUndefined()
  })

  it('shows a failure that no password would fix as an error', async () => {
    const wrapper = await mount(
      appendOnlyStatusFixture({
        setup_failure: { needs_password: false, message: 'The host refused the key.' },
      }),
    )

    expect(wrapper.find('input[type="password"]').exists()).toBe(false)
    expect(wrapper.find('.error-banner').text()).toContain('The host refused the key.')
  })

  it('runs the connection check on request', async () => {
    const checked = appendOnlyStatusFixture({
      check: { ...appendOnlyStatusFixture().check!, backup_key_ok: false, error: 'refused' },
    })
    vi.mocked(checkAppendOnly).mockResolvedValue(checked)
    const wrapper = await mount(appendOnlyStatusFixture())

    await findButton(wrapper, /Check now/).trigger('click')
    await flushPromises()

    expect(checkAppendOnly).toHaveBeenCalledWith(12)
    expect(wrapper.find('.check-bad').exists()).toBe(true)
    expect(wrapper.text()).toContain('refused')
  })

  it('shows the stopped state from maintenance', async () => {
    const wrapper = await mount(
      appendOnlyStatusFixture({
        maintenance: {
          cron_expression: '30 4 * * 0',
          check_before_prune: true,
          next_run_at: null,
          last_run_at: null,
          stopped: { since: '2026-10-19T04:31:00Z', vanished_archives: ['gone-1'] },
        },
      }),
    )

    expect(wrapper.find('.callout--warning').text()).toContain('gone-1')
  })

  it('shows a run in progress', async () => {
    const wrapper = await mount(appendOnlyStatusFixture(), {
      kind: 'maintenance',
      actor: 'server',
      started_at: '2026-11-02T04:30:00Z',
      queued: 0,
    } as ActiveRepoOp)

    expect(findButton(wrapper, /Running/).attributes('disabled')).toBeDefined()
  })

  it('refreshes when the server reports a change to this repository', async () => {
    const wrapper = await mount(appendOnlyStatusFixture())
    expect(getAppendOnlyStatus).toHaveBeenCalledTimes(1)

    pushWs('RepoOpChanged', { repo_id: 99, op: null })
    expect(getAppendOnlyStatus).toHaveBeenCalledTimes(1)

    vi.mocked(getAppendOnlyStatus).mockResolvedValue(OFF)
    pushWs('RepoOpChanged', { repo_id: 12, op: null })
    await flushPromises()
    expect(getAppendOnlyStatus).toHaveBeenCalledTimes(2)
    expect(toggle(wrapper).attributes('aria-checked')).toBe('false')

    pushWs('DataChanged', {})
    expect(getAppendOnlyStatus).toHaveBeenCalledTimes(3)
  })

  it('shows why the connection check could not run', async () => {
    vi.mocked(checkAppendOnly).mockRejectedValue(new Error('server unreachable'))
    const wrapper = await mount(appendOnlyStatusFixture())

    await findButton(wrapper, /Check now/).trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toContain('server unreachable')
  })

  it('keeps what it shows when a background refresh fails', async () => {
    const wrapper = await mount(appendOnlyStatusFixture())
    vi.mocked(getAppendOnlyStatus).mockRejectedValue(new Error('blip'))

    pushWs('DataChanged', {})
    await flushPromises()

    expect(logger.error).toHaveBeenCalled()
    expect(wrapper.find('.error-banner').exists()).toBe(false)
    expect(toggle(wrapper).attributes('aria-checked')).toBe('true')
  })

  it('loads the status of the next repository when the view switches', async () => {
    const wrapper = await mount(appendOnlyStatusFixture())
    await wrapper.setProps({ repo: repoFixture({ id: 13 }) })
    await flushPromises()

    expect(getAppendOnlyStatus).toHaveBeenLastCalledWith(13)
  })

  it('explains the quota effect behind its help hint', async () => {
    const wrapper = await mount(appendOnlyStatusFixture())
    await wrapper.find('[aria-label="Help: what append-only changes"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('queued deletions')
  })

  it('passes on what the maintenance panel changes', async () => {
    vi.mocked(runMaintenance).mockResolvedValue(undefined)
    const saved = appendOnlyStatusFixture({
      maintenance: { ...appendOnlyStatusFixture().maintenance!, cron_expression: '0 3 * * *' },
    })
    vi.mocked(updateMaintenance).mockResolvedValue(saved)
    const wrapper = await mount(appendOnlyStatusFixture())

    await findButton(wrapper, /Run maintenance now/).trigger('click')
    await flushPromises()
    expect(wrapper.emitted('changed')).toHaveLength(1)

    wrapper.findComponent({ name: 'RepoMaintenancePanel' }).vm.$emit('status', saved)
    await flushPromises()
    expect((wrapper.find('.cron-stub').element as HTMLElement).tagName).toBe('DIV')
    expect(wrapper.findComponent({ name: 'RepoMaintenancePanel' }).props('maintenance')).toEqual(
      saved.maintenance,
    )
  })
})
