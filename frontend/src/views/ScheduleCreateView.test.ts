// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { apiClient } from '../api/client'
import type * as VueRouter from 'vue-router'
import { renderWithPlugins } from '../test-utils'
import ScheduleCreateView from './ScheduleCreateView.vue'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

// `renderWithPlugins` pushes its route after mounting, so the view's own
// onMounted read of `route.query` lands before the navigation settles. Stubbing
// useRoute is the only way to exercise the ?agent_id= preselect.
const routeQuery: Record<string, string> = {}
const push = vi.fn()
vi.mock('vue-router', async () => {
  const actual = await vi.importActual<typeof VueRouter>('vue-router')
  return {
    ...actual,
    useRoute: () => ({ query: routeQuery }),
    // Stubbed as well, so the tests can assert where the view sends people
    // rather than what the harness's own memory router happens to render.
    useRouter: () => ({ push }),
  }
})

// The real cron builder is a heavy mount, and the wizard only needs its
// v-model here - the builder has its own tests. Stubbed for the same reason
// ScheduleDetailView.test.ts stubs it: without this, reaching the Timing step
// under coverage instrumentation runs past the default test timeout.
vi.mock('../components/CronBuilder.vue', () => ({
  default: {
    props: ['modelValue'],
    emits: ['update:modelValue'],
    template:
      '<input class="cron-builder-stub" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
  },
}))

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
    for (const key of Object.keys(routeQuery)) delete routeQuery[key]
    push.mockReset()
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
    expect(push).toHaveBeenCalledWith('/schedules/7')
  })

  /** The step rail stays clickable during the create request, so the Targets
      step is reachable while it is in flight - and must not be editable there. */
  it('locks the target editor while the create request is in flight', async () => {
    const wrapper = await open()
    // A create that never settles, so the wizard stays mid-submit.
    mockApiClient.post.mockReturnValue(new Promise(() => {}))

    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await button(wrapper, 'Continue')!.trigger('click')
    await button(wrapper, 'Add repository')!.trigger('click')
    await button(wrapper, 'Continue')!.trigger('click')
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    await button(wrapper, 'Create schedule')!.trigger('click')

    // Back to Targets through the rail, mid-request.
    await wrapper.findAll('.wizard-step')[2].trigger('click')

    expect(
      wrapper.find('select[aria-label="Repository for target 1"]').attributes('disabled'),
    ).toBeDefined()
    // Both repositories are targets by now, so the add button is matched by
    // class rather than label - it reads "Every repository is already a target".
    expect(wrapper.find('.repo-target-add').attributes('disabled')).toBeDefined()
    expect(
      wrapper.find('button[aria-label="Remove repository"]').attributes('disabled'),
    ).toBeDefined()
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

  it('preselects the agent named in the query string', async () => {
    routeQuery.agent_id = '11'
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)

    expect(wrapper.find('.multi-select-label').text()).toBe('1 agent selected')
  })

  it('ignores an agent_id that names no agent', async () => {
    routeQuery.agent_id = '999'
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)

    expect(wrapper.find('.multi-select-label').text()).toBe('Select agents...')
  })

  it('will not leave the Targets step with nothing required', async () => {
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await button(wrapper, 'Continue')!.trigger('click')

    await wrapper
      .findAllComponents({ name: 'ToggleSwitch' })[0]
      .vm.$emit('update:modelValue', false)
    expect(wrapper.find('.wizard-status--blocked').text()).toContain(
      'at least one required repository',
    )
    expect(button(wrapper, 'Continue')!.attributes('disabled')).toBeDefined()
  })

  it('will not leave the Timing step with an unusable cron expression', async () => {
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await button(wrapper, 'Continue')!.trigger('click')
    await button(wrapper, 'Continue')!.trigger('click')

    await wrapper.findComponent({ name: 'CronBuilder' }).vm.$emit('update:modelValue', '0 2 *')
    expect(wrapper.find('.wizard-status--blocked').text()).toContain('a five-field cron expression')

    await wrapper.findComponent({ name: 'CronBuilder' }).vm.$emit('update:modelValue', '0 5 * * 1')
    expect(button(wrapper, 'Continue')!.attributes('disabled')).toBeUndefined()
  })

  it('steps back the way it came', async () => {
    const wrapper = await open()
    expect(button(wrapper, 'Back')!.attributes('disabled')).toBeDefined()

    await fillBasicsAndContinue(wrapper)
    expect(wrapper.find('.wizard-step--current').text()).toContain('Sources')

    await button(wrapper, 'Back')!.trigger('click')
    expect(wrapper.find('.wizard-step--current').text()).toContain('Basics')
  })

  it('sends each Review block back to the step that owns it', async () => {
    const wrapper = await open()
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')

    const edits = wrapper.findAll('.review-block .btn')
    expect(edits).toHaveLength(5)

    const expected = ['Basics', 'Timing', 'Sources', 'Targets', 'Retention']
    for (const [index, step] of expected.entries()) {
      await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
      await wrapper.findAll('.review-block .btn')[index].trigger('click')
      expect(wrapper.find('.wizard-step--current').text()).toContain(step)
    }
  })

  it('writes every field on the way through into the create payload', async () => {
    const wrapper = await open()
    mockApiClient.post.mockResolvedValue({ data: { id: 9 } })

    // Basics: name, and the enabled toggle off so the schedule is created paused.
    await wrapper.find('#schedule-name').setValue('Nightly production backup')
    await wrapper.findComponent({ name: 'ToggleSwitch' }).vm.$emit('update:modelValue', false)
    await button(wrapper, 'Continue')!.trigger('click')

    // Sources: two hosts, paths.
    await wrapper.find('.multi-select-trigger').trigger('click')
    const boxes = wrapper.findAll('.multi-select-item input[type="checkbox"]')
    await boxes[0].trigger('change')
    await boxes[1].trigger('change')
    await wrapper.find('#backup-paths').setValue('/etc\n/srv')
    await button(wrapper, 'Continue')!.trigger('click')

    // Targets: the on-failure select the second host revealed, then Timing.
    await wrapper.find('#on-failure').setValue('continue')
    await button(wrapper, 'Continue')!.trigger('click')
    await wrapper.findComponent({ name: 'CronBuilder' }).vm.$emit('update:modelValue', '0 4 * * *')
    await wrapper.find('#missed-threshold').setValue('5')
    await button(wrapper, 'Continue')!.trigger('click')

    // Retention.
    const keeps = wrapper.findAll('.retention-grid input')
    expect(keeps).toHaveLength(5)
    await keeps[0].setValue('12')
    await keeps[1].setValue('14')
    await keeps[2].setValue('8')
    await keeps[3].setValue('24')
    await keeps[4].setValue('3')
    await button(wrapper, 'Continue')!.trigger('click')

    // Advanced hands its edits back through the same form model, so an edit
    // made there has to reach the payload like any other field.
    const advanced = wrapper.findComponent({ name: 'ScheduleAdvancedTab' })
    expect(advanced.exists()).toBe(true)
    await advanced.vm.$emit('update:form', {
      ...(advanced.props('form') as ScheduleFormState),
      rate_limit_kbps: 2048,
      exclude_patterns: '*.tmp',
    })
    await button(wrapper, 'Continue')!.trigger('click')

    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({
        name: 'Nightly production backup',
        enabled: false,
        agent_ids: [10, 11],
        on_failure: 'continue',
        backup_sources: ['/etc', '/srv'],
        cron_expression: '0 4 * * *',
        missed_backup_threshold: 5,
        keep_hourly: 12,
        keep_daily: 14,
        keep_weekly: 8,
        keep_monthly: 24,
        keep_yearly: 3,
        rate_limit_kbps: 2048,
        exclude_patterns_raw: '*.tmp',
      }),
    )
  })

  /**
   * The bug this guards: the Advanced step offers per-host excludes,
   * file-change patterns and commands, but `submit()` built the payload from
   * the form alone, so a schedule created with them came out without them -
   * silently, since the request still succeeded.
   */
  it('sends the Advanced step per-host overrides with the new schedule', async () => {
    const wrapper = await open()
    mockApiClient.post.mockResolvedValue({ data: { id: 9 } })

    await wrapper.find('#schedule-name').setValue('Per-host backup')
    await button(wrapper, 'Continue')!.trigger('click')

    await wrapper.find('.multi-select-trigger').trigger('click')
    const boxes = wrapper.findAll('.multi-select-item input[type="checkbox"]')
    await boxes[0].trigger('change')
    await boxes[1].trigger('change')
    await button(wrapper, 'Continue')!.trigger('click')

    await button(wrapper, 'Continue')!.trigger('click')
    await wrapper.findComponent({ name: 'CronBuilder' }).vm.$emit('update:modelValue', '0 4 * * *')
    await button(wrapper, 'Continue')!.trigger('click')
    await button(wrapper, 'Continue')!.trigger('click')

    const advanced = wrapper.findComponent({ name: 'ScheduleAdvancedTab' })
    await advanced.vm.$emit('update:overrides', {
      ...(advanced.props('overrides') as ScheduleAgentOverrides),
      usePerHostExcludes: true,
      perHostExcludes: { 10: '/var/cache', 11: '/tmp' },
      usePerAgentCmds: true,
      perAgentPreCmds: { 10: [{ command: 'systemctl stop nginx', timeout_seconds: null }] },
    })
    await button(wrapper, 'Continue')!.trigger('click')

    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({
        // Each override replaces its schedule-wide counterpart.
        exclude_patterns_raw: '',
        exclude_patterns_per_agent: [
          { agent_id: 10, raw_text: '/var/cache' },
          { agent_id: 11, raw_text: '/tmp' },
        ],
        pre_backup_commands: [],
        post_backup_commands: [],
        commands_per_agent: [
          {
            agent_id: 10,
            pre_backup_commands: [{ command: 'systemctl stop nginx', timeout_seconds: null }],
            post_backup_commands: [],
          },
          { agent_id: 11, pre_backup_commands: [], post_backup_commands: [] },
        ],
      }),
    )
    // Untouched overrides stay off rather than sending empty per-agent lists.
    const payload = mockApiClient.post.mock.calls[0][1] as Record<string, unknown>
    expect(payload.file_change_patterns_per_agent).toBeUndefined()
  })

  /**
   * The bug this guards: the on-failure control was shown only for more than
   * one host, but `on_failure` also decides what a failing *required target*
   * does - so one host writing to two repositories, the case this feature
   * exists for, could never reach the setting during creation.
   */
  it('offers the on-failure choice to a single host with two targets', async () => {
    const wrapper = await open()

    await wrapper.find('#schedule-name').setValue('Dual target')
    await button(wrapper, 'Continue')!.trigger('click')

    await wrapper.find('.multi-select-trigger').trigger('click')
    await wrapper.findAll('.multi-select-item input[type="checkbox"]')[0].trigger('change')
    await button(wrapper, 'Continue')!.trigger('click')

    // One host, one target: nothing a failure could carry on to.
    expect(wrapper.find('#on-failure').exists()).toBe(false)

    await button(wrapper, 'Add repository')!.trigger('click')
    expect(wrapper.findAll('.order-item')).toHaveLength(2)
    expect(wrapper.find('#on-failure').exists()).toBe(true)
  })

  it('leaves for the schedules list on cancel', async () => {
    const wrapper = await open()
    await button(wrapper, 'Cancel')!.trigger('click')
    await flushPromises()

    expect(push).toHaveBeenCalledWith('/schedules')
  })

  it('offers a way to create the missing repository', async () => {
    setup([])
    const wrapper = renderWithPlugins(ScheduleCreateView)
    await flushPromises()

    await button(wrapper, 'New repository')!.trigger('click')
    await flushPromises()

    expect(push).toHaveBeenCalledWith('/repos')
  })
})
