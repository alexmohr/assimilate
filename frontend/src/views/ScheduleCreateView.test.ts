// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { apiClient } from '../api/client'
import { renderWithPlugins } from '../test-utils'
import ScheduleCreateView from './ScheduleCreateView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

const mockApiClient = apiClient as unknown as {
  get: ReturnType<typeof vi.fn>
  post: ReturnType<typeof vi.fn>
}

const AGENTS = [
  { id: 10, hostname: 'web-server-01', display_name: 'Web Server' },
  { id: 11, hostname: 'db-server-01', display_name: null },
]

const REPOS = [
  { id: 20, name: 'nas-local', ssh_user: 'borg', ssh_host: 'nas.lan', repo_path: '/srv/borg' },
  { id: 21, name: 'offsite', ssh_user: 'u1', ssh_host: 'offsite.example', repo_path: '/backups' },
]

function setup(repos: unknown[] = REPOS): void {
  mockApiClient.get.mockImplementation((url: string) => {
    if (url === '/agents') return Promise.resolve({ data: AGENTS })
    if (url === '/repos') return Promise.resolve({ data: repos })
    return Promise.resolve({ data: [] })
  })
}

async function open(): Promise<ReturnType<typeof renderWithPlugins>> {
  setup()
  const wrapper = renderWithPlugins(ScheduleCreateView)
  await flushPromises()
  return wrapper
}

function button(wrapper: ReturnType<typeof renderWithPlugins>, label: string) {
  return wrapper.findAll('button').find((b) => b.text() === label)
}

function railLabels(wrapper: ReturnType<typeof renderWithPlugins>): string[] {
  return wrapper.findAll('.wizard-step-text span:first-child').map((s) => s.text())
}

async function fillBasicsAndContinue(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
  await wrapper.find('#schedule-name').setValue('Nightly production backup')
  await button(wrapper, 'Continue')!.trigger('click')
}

async function selectFirstAgent(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
  await wrapper.find('.multi-select-trigger').trigger('click')
  await wrapper.findAll('.multi-select-item input[type="checkbox"]')[0].trigger('change')
}

describe('ScheduleCreateView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('opens on the first step with the remaining steps listed', async () => {
    const wrapper = await open()

    expect(wrapper.find('h1').text()).toBe('New Schedule')
    expect(railLabels(wrapper)).toEqual([
      'Basics',
      'Sources',
      'Targets',
      'Timing',
      'Retention',
      'Advanced',
      'Review',
    ])
    expect(wrapper.find('.wizard-step--current').text()).toContain('Basics')
  })

  it('drops Retention and Advanced for a schedule type that has no archives', async () => {
    const wrapper = await open()
    await wrapper.find('#schedule-type').setValue('check')

    expect(railLabels(wrapper)).toEqual(['Basics', 'Sources', 'Targets', 'Timing', 'Review'])
  })

  it('will not continue past a step that is still missing something', async () => {
    const wrapper = await open()

    expect(button(wrapper, 'Continue')!.attributes('disabled')).toBeDefined()
    expect(wrapper.find('.wizard-status--blocked').text()).toContain('Needs a name to continue')

    await wrapper.find('#schedule-name').setValue('Nightly production backup')
    expect(button(wrapper, 'Continue')!.attributes('disabled')).toBeUndefined()
  })

  it('needs a host before leaving the Sources step', async () => {
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)

    expect(wrapper.find('.wizard-status--blocked').text()).toContain('at least one host')
    await selectFirstAgent(wrapper)
    expect(button(wrapper, 'Continue')!.attributes('disabled')).toBeUndefined()
  })

  it('starts the target list at the first repository, marked required', async () => {
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await button(wrapper, 'Continue')!.trigger('click')

    expect(wrapper.findAll('.order-item')).toHaveLength(1)
    expect(wrapper.find('.badge').text()).toBe('Required')
    expect(button(wrapper, 'Continue')!.attributes('disabled')).toBeUndefined()
  })

  it('offers no create action before the last step', async () => {
    const wrapper = await open()
    expect(button(wrapper, 'Create schedule')).toBeUndefined()
  })

  it('creates the schedule with every target it was given', async () => {
    const wrapper = await open()
    mockApiClient.post.mockResolvedValue({ data: { id: 7 } })

    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await button(wrapper, 'Continue')!.trigger('click')
    await button(wrapper, 'Add repository')!.trigger('click')
    await button(wrapper, 'Continue')!.trigger('click')

    // Skip straight to Review through the rail.
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    expect(wrapper.text()).toContain('writes 2 copies')

    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({
        name: 'Nightly production backup',
        agent_ids: [10],
        repo_id: 20,
        repo_targets: [
          { repo_id: 20, required: true },
          { repo_id: 21, required: false },
        ],
      }),
    )
  })

  it('keeps the create action out of reach while any step is unanswered', async () => {
    const wrapper = await open()
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')

    expect(button(wrapper, 'Create schedule')!.attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('Still missing:')
    expect(wrapper.text()).toContain('a name (Basics)')
    expect(wrapper.text()).toContain('at least one host (Sources)')

    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()
    expect(mockApiClient.post).not.toHaveBeenCalled()
  })

  it('reports a failed create instead of navigating away', async () => {
    const wrapper = await open()
    mockApiClient.post.mockRejectedValue(new Error('repository unreachable'))

    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toContain('repository unreachable')
  })

  it('sends people to create a repository first when there are none', async () => {
    setup([])
    const wrapper = renderWithPlugins(ScheduleCreateView)
    await flushPromises()

    expect(wrapper.text()).toContain('No repositories yet')
    expect(wrapper.find('.wizard').exists()).toBe(false)
  })
})
