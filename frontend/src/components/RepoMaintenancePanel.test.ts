// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('../api/appendOnly', () => ({
  keepArchive: vi.fn(),
  resumeMaintenance: vi.fn(),
  runMaintenance: vi.fn(),
  updateMaintenance: vi.fn(),
}))
// The real builder talks to the server's cron preview; the panel only needs
// its v-model.
vi.mock('./CronBuilder.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      props: { modelValue: { type: String, required: true } },
      emits: ['update:modelValue'],
      setup:
        (props, { emit }) =>
        () =>
          h('input', {
            class: 'cron-stub',
            value: props.modelValue,
            onInput: (e: Event) => emit('update:modelValue', (e.target as HTMLInputElement).value),
          }),
    }),
  }
})

import {
  keepArchive,
  resumeMaintenance,
  runMaintenance,
  updateMaintenance,
} from '../api/appendOnly'
import { useToast } from '../composables/useToast'
import { renderWithPlugins } from '../test-utils'
import { appendOnlyStatusFixture } from '../test-utils/appendOnlyFixtures'
import { dialogButton, findButton } from '../test-utils/dom'
import RepoMaintenancePanel from './RepoMaintenancePanel.vue'
import type { AppendOnlyStatusResponse } from '../types/generated'

function mount(status: AppendOnlyStatusResponse = appendOnlyStatusFixture(), running = false) {
  return renderWithPlugins(RepoMaintenancePanel, {
    props: { repoId: 7, status, maintenance: status.maintenance, running },
  })
}

const STOPPED = appendOnlyStatusFixture({
  maintenance: {
    cron_expression: '30 4 * * 0',
    check_before_prune: true,
    next_run_at: null,
    last_run_at: '2026-10-19T04:30:00Z',
    stopped: {
      since: '2026-10-19T04:31:00Z',
      vanished_archives: ['nas-2026-10-17T02:00', 'nas-2026-10-18T02:00'],
    },
  },
})

describe('RepoMaintenancePanel', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    vi.mocked(keepArchive).mockReset().mockResolvedValue(undefined)
    vi.mocked(runMaintenance).mockReset().mockResolvedValue(undefined)
    vi.mocked(resumeMaintenance).mockReset()
    vi.mocked(updateMaintenance).mockReset()
  })

  it('shows the schedule and the last run', () => {
    const wrapper = mount()
    expect((wrapper.find('.cron-stub').element as HTMLInputElement).value).toBe('30 4 * * 0')
    expect(wrapper.text()).toContain('Next run')
    expect(wrapper.text()).toContain('Last run')
    expect(wrapper.find('.callout--warning').exists()).toBe(false)
  })

  it('starts a run in the background', async () => {
    const wrapper = mount()
    await findButton(wrapper, /Run maintenance now/).trigger('click')
    await flushPromises()

    expect(runMaintenance).toHaveBeenCalledWith(7)
    expect(wrapper.emitted('changed')).toHaveLength(1)
  })

  it('cannot start a second run while one is in progress', () => {
    const button = findButton(mount(appendOnlyStatusFixture(), true), /Running/)
    expect(button.attributes('disabled')).toBeDefined()
  })

  it('offers saving only after the schedule changed, and sends both fields', async () => {
    const next = appendOnlyStatusFixture()
    vi.mocked(updateMaintenance).mockResolvedValue(next)
    const wrapper = mount()
    expect(wrapper.text()).not.toContain('Save schedule')

    await wrapper.find('.cron-stub').setValue('0 3 * * *')
    await wrapper.find('[role="switch"]').trigger('click')
    await findButton(wrapper, /Save schedule/).trigger('click')
    await flushPromises()

    expect(updateMaintenance).toHaveBeenCalledWith(7, {
      cron_expression: '0 3 * * *',
      check_before_prune: false,
    })
    expect(wrapper.emitted('status')).toEqual([[next]])
  })

  it('shows the server error for a bad cron expression', async () => {
    vi.mocked(updateMaintenance).mockRejectedValue(new Error('invalid cron expression'))
    const wrapper = mount()
    await wrapper.find('.cron-stub').setValue('nonsense')
    await findButton(wrapper, /Save schedule/).trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toContain('invalid cron expression')
  })

  it('puts the schedule back on cancel', async () => {
    const wrapper = mount()
    await wrapper.find('.cron-stub').setValue('0 3 * * *')
    await findButton(wrapper, /Cancel/).trigger('click')
    expect((wrapper.find('.cron-stub').element as HTMLInputElement).value).toBe('30 4 * * 0')
  })

  describe('when maintenance stopped', () => {
    it('names the archives that vanished and blocks a manual run', () => {
      const wrapper = mount(STOPPED)
      const callout = wrapper.find('.callout--warning')
      expect(callout.text()).toContain('Maintenance stopped')
      expect(callout.text()).toContain('nas-2026-10-17T02:00')
      expect(callout.text()).toContain('nas-2026-10-18T02:00')
      expect(callout.text()).toContain('Nothing was written')
      expect(findButton(wrapper, /Run maintenance now/).attributes('disabled')).toBeDefined()
    })

    it('explains the consequence before resuming', async () => {
      const wrapper = mount(STOPPED)
      await findButton(wrapper, /Accept and resume/).trigger('click')

      expect(document.body.textContent).toContain('only really gone once maintenance next writes')
      expect(document.body.textContent).toContain('roll the repository back on the host first')
      expect(resumeMaintenance).not.toHaveBeenCalled()
    })

    it('resumes once confirmed', async () => {
      const resumed = appendOnlyStatusFixture()
      vi.mocked(resumeMaintenance).mockResolvedValue(resumed)
      const wrapper = mount(STOPPED)
      await findButton(wrapper, /Accept and resume/).trigger('click')
      dialogButton('Accept and resume').click()
      await flushPromises()

      expect(resumeMaintenance).toHaveBeenCalledWith(7)
      expect(wrapper.emitted('status')).toEqual([[resumed]])
    })

    it('shows why resuming failed and stays open', async () => {
      vi.mocked(resumeMaintenance).mockRejectedValue(new Error('not stopped'))
      const wrapper = mount(STOPPED)
      await findButton(wrapper, /Accept and resume/).trigger('click')
      dialogButton('Accept and resume').click()
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog .form-error')?.textContent).toContain(
        'not stopped',
      )
    })
  })

  it('keeps a queued archive through the pending deletions table', async () => {
    const status = appendOnlyStatusFixture({
      pending_deletions: [
        {
          archive_name: 'photos-import-test',
          reason: 'manual',
          requested_by: 'admin',
          requested_at: '2026-10-25T10:00:00Z',
        },
      ],
    })
    const wrapper = mount(status)
    await wrapper.find('tbody button').trigger('click')
    await flushPromises()

    expect(keepArchive).toHaveBeenCalledWith(7, 'photos-import-test')
    expect(wrapper.emitted('changed')).toHaveLength(1)
  })

  function toastMessages(): string[] {
    return useToast().toasts.value.map((t) => t.message)
  }

  it('reports a run that could not be started', async () => {
    useToast().toasts.value = []
    vi.mocked(runMaintenance).mockRejectedValue(new Error('already running'))
    const wrapper = mount()
    await findButton(wrapper, /Run maintenance now/).trigger('click')
    await flushPromises()

    expect(toastMessages()).toContain('already running')
    expect(wrapper.emitted('changed')).toBeUndefined()
  })

  it('reports an archive that could not be kept', async () => {
    useToast().toasts.value = []
    vi.mocked(keepArchive).mockRejectedValue(new Error('gone already'))
    const wrapper = mount(
      appendOnlyStatusFixture({
        pending_deletions: [
          {
            archive_name: 'photos-import-test',
            reason: 'manual',
            requested_by: 'admin',
            requested_at: '2026-10-25T10:00:00Z',
          },
        ],
      }),
    )
    await wrapper.find('tbody button').trigger('click')
    await flushPromises()

    expect(toastMessages()).toContain('gone already')
    expect(wrapper.emitted('changed')).toBeUndefined()
  })

  it('follows a schedule change from the server when the form is untouched', async () => {
    const wrapper = mount()
    const changed = appendOnlyStatusFixture()
    await wrapper.setProps({
      maintenance: { ...changed.maintenance!, cron_expression: '0 5 * * 1' },
    })
    expect((wrapper.find('.cron-stub').element as HTMLInputElement).value).toBe('0 5 * * 1')
  })

  it('keeps unsaved edits when the server schedule changes underneath', async () => {
    const wrapper = mount()
    await wrapper.find('.cron-stub').setValue('0 3 * * *')
    await wrapper.setProps({
      maintenance: { ...appendOnlyStatusFixture().maintenance!, cron_expression: '0 5 * * 1' },
    })
    expect((wrapper.find('.cron-stub').element as HTMLInputElement).value).toBe('0 3 * * *')
  })

  it('explains what the schedule covers behind its help hint', async () => {
    const wrapper = mount()
    await wrapper.find('[aria-label="Help: maintenance schedule"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('prunes, compacts')
  })

  it('closes the resume dialog without resuming on cancel', async () => {
    const wrapper = mount(STOPPED)
    await findButton(wrapper, /Accept and resume/).trigger('click')
    dialogButton('Cancel').click()
    await flushPromises()

    expect(resumeMaintenance).not.toHaveBeenCalled()
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })
})
