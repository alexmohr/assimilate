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
// Named, so `findComponent({ name: 'CronBuilder' })` still finds it now that
// it renders inside the wizard shell's slot rather than the view's own tree.
vi.mock('../components/CronBuilder.vue', () => ({
  default: {
    name: 'CronBuilder',
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

/** The API always sends a `power` block; the Power step reads it per host. */
const NO_WAKE = {
  wake_enabled: false,
  wake_mac_address: null,
  wake_broadcast_address: null,
  wake_timeout_seconds: 180,
  shutdown_after_backup: false,
}

/** What the Sources step's preview reads from each host's own files. */
const HOST_FILES = {
  default_backup_paths: ['/etc'],
  default_pre_backup_commands: [],
  default_post_backup_commands: [],
}

const AGENTS = [
  {
    id: 10,
    hostname: 'web-server-01',
    display_name: 'Web Server',
    power: { wake: NO_WAKE },
    ...HOST_FILES,
  },
  { id: 11, hostname: 'db-server-01', display_name: null, power: { wake: NO_WAKE }, ...HOST_FILES },
]

const REPOS = [
  {
    id: 20,
    name: 'nas-local',
    ssh_user: 'borg',
    ssh_host: 'nas.lan',
    repo_path: '/srv/borg',
    power: NO_WAKE,
  },
  {
    id: 21,
    name: 'offsite',
    ssh_user: 'u1',
    ssh_host: 'offsite.example',
    repo_path: '/backups',
    power: NO_WAKE,
  },
]

function setup(repos: unknown[] = REPOS, agents: unknown[] = AGENTS): void {
  mockApiClient.get.mockImplementation((url: string) => {
    if (url === '/agents') return Promise.resolve({ data: agents })
    if (url === '/repos') return Promise.resolve({ data: repos })
    // The Sources step counts what each host includes, to warn about none.
    if (url.endsWith('/vms')) return Promise.resolve({ data: { vms: [] } })
    if (url.endsWith('/databases')) return Promise.resolve({ data: { sources: [] } })
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

/** Clicks Continue until `label` is the current step. Walking by a fixed
    number of clicks breaks whenever a step is inserted; this does not. */
async function continueTo(
  wrapper: ReturnType<typeof renderWithPlugins>,
  label: string,
): Promise<void> {
  for (let i = 0; i < 8; i += 1) {
    if (wrapper.find('.wizard-step--current').text().includes(label)) return
    await button(wrapper, 'Continue')!.trigger('click')
  }
  throw new Error(`never reached the ${label} step`)
}

/** Picks one of the Sources step's Files choices by its label. */
async function chooseFilesMode(
  wrapper: ReturnType<typeof renderWithPlugins>,
  label: string,
): Promise<void> {
  await wrapper
    .findAll('[role="radiogroup"][aria-label="Files"] button')
    .find((b) => b.text() === label)!
    .trigger('click')
}

async function toggleNamed(
  wrapper: ReturnType<typeof renderWithPlugins>,
  label: string,
  value: boolean,
): Promise<void> {
  await wrapper
    .findAllComponents({ name: 'ToggleSwitch' })
    .find((t) => t.props('label') === label)!
    .vm.$emit('update:modelValue', value)
}

/** The Review step's value for one of its rows. */
function reviewRow(wrapper: ReturnType<typeof renderWithPlugins>, term: string): string {
  const dt = wrapper.findAll('.review-block dt').find((d) => d.text() === term)!
  return (dt.element.nextElementSibling as HTMLElement).textContent!.trim()
}

async function selectFirstAgent(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
  await wrapper.find('.multi-select-trigger').trigger('click')
  await wrapper.findAll('.multi-select-item input[type="checkbox"]')[0].trigger('change')
}

/**
 * Opens the wizard with a create request ready to succeed, names the
 * schedule, and picks the first two hosts on the Sources step.
 */
async function openOnSourcesWithTwoHosts(): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = await open()
  mockApiClient.post.mockResolvedValue({ data: { id: 9 } })
  await fillBasicsAndContinue(wrapper)
  await wrapper.find('.multi-select-trigger').trigger('click')
  const boxes = wrapper.findAll('.multi-select-item input[type="checkbox"]')
  await boxes[0].trigger('change')
  await boxes[1].trigger('change')
  return wrapper
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
      'Power',
      'Timing',
      'Retention',
      'Advanced',
      'Review',
    ])
    expect(wrapper.find('.wizard-step--current').text()).toContain('Basics')
  })

  /** Creating a schedule used to go through the Settings tab, which has always
      carried a Power section; the wizard has to offer it too or a new schedule
      cannot set its wake override until after it exists. */
  it('offers a Power step for every schedule type', async () => {
    const wrapper = await open()

    expect(railLabels(wrapper)).toContain('Power')

    await wrapper.find('#schedule-type').setValue('check')
    expect(railLabels(wrapper)).toContain('Power')
  })

  it("shows the wake control on the Power step, over this schedule's hosts", async () => {
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await continueTo(wrapper, 'Power')

    const power = wrapper.findComponent({ name: 'SchedulePowerTab' })
    expect(power.exists()).toBe(true)
    // It reports over the hosts and targets chosen in the steps before it, so
    // the read-out matches the schedule being created rather than everything.
    expect(power.props('selectedAgentIds')).toEqual([10])
    expect(power.props('selectedRepoIds')).toEqual([20])
    expect(power.props('wakeOverride')).toBe('host_default')
  })

  /** The whole point of the step: a wake override chosen during creation has
      to reach the POST, not just render. */
  it('sends a wake override chosen on the Power step with the new schedule', async () => {
    const wrapper = await open()
    mockApiClient.post.mockResolvedValue({ data: { id: 9 } })

    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await continueTo(wrapper, 'Power')
    await wrapper
      .findComponent({ name: 'SchedulePowerTab' })
      .vm.$emit('update:wakeOverride', 'enabled')

    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({ wake_override: 'enabled' }),
    )
  })

  it('drops Retention and Advanced for a schedule type that has no archives', async () => {
    const wrapper = await open()
    await wrapper.find('#schedule-type').setValue('check')

    expect(railLabels(wrapper)).toEqual([
      'Basics',
      'Sources',
      'Targets',
      'Power',
      'Timing',
      'Review',
    ])
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

  it('asks for a usable LVM size while snapshots are on', async () => {
    const wrapper = await open()
    const vm = wrapper.vm as unknown as {
      form: { fs_snapshot: string; lvm_snapshot_percent: number | string }
    }
    vm.form.fs_snapshot = 'best_effort'
    vm.form.lvm_snapshot_percent = ''
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')

    expect(wrapper.text()).toContain('an LVM snapshot size from 1 to 100 (Advanced)')

    vm.form.lvm_snapshot_percent = 20
    await flushPromises()
    expect(wrapper.text()).not.toContain('an LVM snapshot size')
  })

  // The server refuses a schedule that could never back anything up; the
  // wizard says so on the Sources step instead of failing at the end.
  it('holds the Sources step until the schedule has something to back up', async () => {
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)

    await chooseFilesMode(wrapper, 'Own files')
    expect(wrapper.text()).toContain('at least one backup path for its own files')

    await chooseFilesMode(wrapper, 'Off')
    expect(wrapper.text()).toContain('virtual machines or databases, since it reads no files')
    await toggleNamed(wrapper, 'Databases', true)
    expect(wrapper.text()).not.toContain('since it reads no files')
  })

  it('holds the Sources step while its first host lists no paths of its own', async () => {
    setup(REPOS, [{ ...AGENTS[0], default_backup_paths: [] }, AGENTS[1]])
    const wrapper = renderWithPlugins(ScheduleCreateView)
    await flushPromises()
    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)

    expect(wrapper.text()).toContain("that host's Sources list none")
    await chooseFilesMode(wrapper, 'Own files')
    expect(wrapper.text()).not.toContain("that host's Sources list none")
  })

  it('keeps the create action out of reach while any step is unanswered', async () => {
    const wrapper = await open()
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')

    expect(button(wrapper, 'Create schedule')!.attributes('disabled')).toBeDefined()
    // Named once, in the wizard's foot; the review pane does not repeat it.
    expect(wrapper.text()).toContain('Still needs a name (Basics), at least one host (Sources).')
    expect(wrapper.text().match(/a name \(Basics\)/g)).toHaveLength(1)
    expect(wrapper.find('.wizard-note--success').exists()).toBe(false)

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

    expect(wrapper.find('.multi-select-label').text()).toBe('db-server-01')
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
    await continueTo(wrapper, 'Timing')

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

    // Sources: two hosts, and the schedule's own files with their paths and
    // excludes (the excludes moved here from the Advanced step).
    await wrapper.find('.multi-select-trigger').trigger('click')
    const boxes = wrapper.findAll('.multi-select-item input[type="checkbox"]')
    await boxes[0].trigger('change')
    await boxes[1].trigger('change')
    await chooseFilesMode(wrapper, 'Own files')
    await button(wrapper, 'Edit')!.trigger('click')
    await wrapper.find('#file-sources-paths').setValue('/etc\n/srv')
    await wrapper.find('#file-sources-excludes').setValue('*.tmp')
    await button(wrapper, 'Save')!.trigger('click')
    await toggleNamed(wrapper, 'Databases', true)
    await button(wrapper, 'Continue')!.trigger('click')

    // Targets: the on-failure select the second host revealed, then Timing
    // (past Power, which needs nothing and defaults to the hosts' own setting).
    await wrapper.find('#on-failure').setValue('continue')
    await continueTo(wrapper, 'Timing')
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
        files_mode: 'own',
        run_host_commands: true,
        database_backup_enabled: true,
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
   * The bug this guards: the wizard carries per-host excludes, file-change
   * patterns and commands, but `submit()` built the payload from the form
   * alone, so a schedule created with them came out without them - silently,
   * since the request still succeeded. The overrides used to be edited on the
   * Advanced step; they now travel through the Sources step's pane.
   */
  it('sends the per-host overrides with the new schedule', async () => {
    const wrapper = await open()
    mockApiClient.post.mockResolvedValue({ data: { id: 9 } })

    await wrapper.find('#schedule-name').setValue('Per-host backup')
    await button(wrapper, 'Continue')!.trigger('click')

    await wrapper.find('.multi-select-trigger').trigger('click')
    const boxes = wrapper.findAll('.multi-select-item input[type="checkbox"]')
    await boxes[0].trigger('change')
    await boxes[1].trigger('change')

    const sources = wrapper.findComponent({ name: 'ScheduleSourcesPane' })
    await sources.vm.$emit('update:overrides', {
      ...(sources.props('overrides') as ScheduleAgentOverrides),
      usePerHostExcludes: true,
      perHostExcludes: { 10: '/var/cache', 11: '/tmp' },
      usePerAgentCmds: true,
      perAgentPreCmds: { 10: [{ command: 'systemctl stop nginx', timeout_seconds: null }] },
    })

    await continueTo(wrapper, 'Timing')
    await wrapper.findComponent({ name: 'CronBuilder' }).vm.$emit('update:modelValue', '0 4 * * *')
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
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

  it("takes each host's own files and commands unless told otherwise", async () => {
    const wrapper = await open()
    mockApiClient.post.mockResolvedValue({ data: { id: 9 } })

    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    expect(wrapper.findComponent({ name: 'ScheduleSourcesPane' }).exists()).toBe(true)
    await toggleNamed(wrapper, "Run each host's backup commands", false)

    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    expect(reviewRow(wrapper, 'Files')).toBe("Each host's files")
    expect(reviewRow(wrapper, 'VMs and databases')).toBe('Neither')
    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    const payload = mockApiClient.post.mock.calls[0][1] as Record<string, unknown>
    expect(payload).toMatchObject({
      files_mode: 'host',
      run_host_commands: false,
      // The schedule names no paths of its own: each host's are used.
      backup_sources: [],
    })
    expect(payload.backup_sources_per_agent).toBeUndefined()
  })

  it('sends paths split per host as one list per host', async () => {
    const wrapper = await openOnSourcesWithTwoHosts()
    await chooseFilesMode(wrapper, 'Own files')
    await toggleNamed(wrapper, 'Different paths per host', true)
    await button(wrapper, 'Edit')!.trigger('click')
    const fields = wrapper.findAll('textarea[placeholder="Directories to back up, one per line"]')
    expect(fields).toHaveLength(2)
    await fields[0].setValue('/home\n/etc')
    await button(wrapper, 'Save')!.trigger('click')
    await toggleNamed(wrapper, 'Virtual machines', true)

    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    expect(reviewRow(wrapper, 'Files')).toBe('Own files, paths per host')
    expect(reviewRow(wrapper, 'VMs and databases')).toBe('VMs')
    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({
        files_mode: 'own',
        vm_snapshot_enabled: true,
        backup_sources: [],
        // A host given no paths is left out rather than sent an empty list.
        backup_sources_per_agent: [{ agent_id: 10, paths: ['/home', '/etc'] }],
      }),
    )
  })

  // The pane may hand the wizard a whole new form or per-host path map rather
  // than editing the one it was given; the schedule is built from what it
  // handed back.
  it('takes the form and per-host paths the Sources pane hands back', async () => {
    const wrapper = await openOnSourcesWithTwoHosts()

    const sources = wrapper.findComponent({ name: 'ScheduleSourcesPane' })
    await sources.vm.$emit('update:form', {
      ...(sources.props('form') as ScheduleFormState),
      files_mode: 'own',
      database_backup_enabled: true,
    })
    await sources.vm.$emit('update:usePerHostPaths', true)
    await sources.vm.$emit('update:perHostSources', { 11: '/srv/data' })

    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({
        files_mode: 'own',
        database_backup_enabled: true,
        backup_sources: [],
        backup_sources_per_agent: [{ agent_id: 11, paths: ['/srv/data'] }],
      }),
    )
  })

  it('reads no files for a schedule with files off', async () => {
    const wrapper = await open()
    mockApiClient.post.mockResolvedValue({ data: { id: 9 } })

    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await chooseFilesMode(wrapper, 'Off')
    await toggleNamed(wrapper, 'Virtual machines', true)
    await toggleNamed(wrapper, 'Databases', true)

    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    expect(reviewRow(wrapper, 'Files')).toBe('Off')
    expect(reviewRow(wrapper, 'VMs and databases')).toBe('VMs and databases')
    await button(wrapper, 'Create schedule')!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({ files_mode: 'off', backup_sources: [] }),
    )
  })

  it("names the schedule's own paths on the review step", async () => {
    const wrapper = await open()
    await fillBasicsAndContinue(wrapper)
    await selectFirstAgent(wrapper)
    await chooseFilesMode(wrapper, 'Own files')

    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    expect(reviewRow(wrapper, 'Files')).toBe('Own files, no paths yet')

    await wrapper.findAll('.wizard-step')[1].trigger('click')
    await button(wrapper, 'Edit')!.trigger('click')
    await wrapper.find('#file-sources-paths').setValue('/srv\n/home')
    await button(wrapper, 'Save')!.trigger('click')
    await wrapper.findAll('.wizard-step').at(-1)!.trigger('click')
    expect(reviewRow(wrapper, 'Files')).toBe('/srv, /home')
  })

  it('offers no file, VM or database choices for a schedule type that backs nothing up', async () => {
    const wrapper = await open()
    await wrapper.find('#schedule-type').setValue('check')
    await fillBasicsAndContinue(wrapper)
    expect(wrapper.findComponent({ name: 'ScheduleSourcesPane' }).exists()).toBe(false)
  })

  it('no longer edits patterns or commands on the Advanced step', async () => {
    const wrapper = await open()
    await wrapper.findAll('.wizard-step')[6].trigger('click')
    const advanced = wrapper.findComponent({ name: 'ScheduleAdvancedTab' })
    expect(advanced.exists()).toBe(true)
    expect(advanced.find('textarea').exists()).toBe(false)
    expect(advanced.findComponent({ name: 'CommandListEditor' }).exists()).toBe(false)
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
