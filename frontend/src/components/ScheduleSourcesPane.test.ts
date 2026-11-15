// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { getAgentDatabases } from '../api/databases'
import { getAgentVms } from '../api/vms'
import ScheduleSourcesPane from './ScheduleSourcesPane.vue'
import type { AgentRow } from '../types/agent'
import type { AgentDatabasesResponse, AgentVmSnapshotResponse } from '../types/generated'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

vi.mock('../api/vms', () => ({ getAgentVms: vi.fn() }))
vi.mock('../api/databases', () => ({ getAgentDatabases: vi.fn() }))

function agent(id: number, overrides: Partial<AgentRow> = {}): AgentRow {
  return {
    id,
    hostname: `host-0${id}`,
    display_name: null,
    domain: null,
    is_imported: false,
    is_connected: true,
    default_backup_paths: ['/etc', '/home'],
    default_pre_backup_commands: [{ command: 'systemctl stop app', timeout_seconds: null }],
    default_post_backup_commands: [],
    ...overrides,
  } as unknown as AgentRow
}

const AGENTS = [agent(1), agent(2)]

function form(overrides: Partial<ScheduleFormState> = {}): ScheduleFormState {
  return {
    ...DEFAULT_SCHEDULE_FORM_STATE,
    name: 'nightly',
    exclude_patterns: '*.cache',
    backup_sources: '/srv',
    ...overrides,
  }
}

function agentOverrides(o: Partial<ScheduleAgentOverrides> = {}): ScheduleAgentOverrides {
  return {
    usePerHostExcludes: false,
    perHostExcludes: {},
    usePerHostIncludes: false,
    perHostIncludes: {},
    usePerHostFileChangePatterns: false,
    perHostFileChangePatterns: {},
    usePerAgentCmds: false,
    perAgentPreCmds: {},
    perAgentPostCmds: {},
    ...o,
  }
}

/** How many VMs and database sources each host includes, by agent id. */
function hostSources(byId: Record<number, { vms: number; databases: number }>): void {
  vi.mocked(getAgentVms).mockImplementation((hostname) => {
    const id = Number(hostname.slice(-1))
    const count = byId[id]?.vms ?? 0
    return Promise.resolve({
      vms: Array.from({ length: count }, () => ({ included: true })),
    } as unknown as AgentVmSnapshotResponse)
  })
  vi.mocked(getAgentDatabases).mockImplementation((hostname) => {
    const id = Number(hostname.slice(-1))
    const count = byId[id]?.databases ?? 0
    return Promise.resolve({
      sources: [
        ...Array.from({ length: count }, () => ({ included: true })),
        // One the host found but did not include: never counted.
        { included: false },
      ],
    } as unknown as AgentDatabasesResponse)
  })
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleSourcesPane, {
    props: {
      agentIds: [1, 2],
      agentLabel: (id: number) => `host-0${id}`,
      agents: AGENTS,
      form: form(),
      usePerHostPaths: false,
      perHostSources: {},
      overrides: agentOverrides(),
      'onUpdate:form': () => {},
      'onUpdate:usePerHostPaths': () => {},
      'onUpdate:perHostSources': () => {},
      'onUpdate:overrides': () => {},
      ...props,
    },
  })
}

function toggle(wrapper: ReturnType<typeof mount>, label: string) {
  return wrapper.findAllComponents({ name: 'ToggleSwitch' }).find((t) => t.props('label') === label)
}

function button(wrapper: ReturnType<typeof mount>, label: string) {
  return wrapper.findAll('button').find((b) => b.text() === label)
}

/** The rows of the "What each host will back up" table, as cell texts. */
function previewRows(wrapper: ReturnType<typeof mount>): string[][] {
  return wrapper
    .findAll('tbody tr')
    .map((row) => row.findAll('td').map((cell) => cell.text().replace(/\s+/g, ' ')))
}

describe('ScheduleSourcesPane', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    hostSources({ 1: { vms: 2, databases: 2 }, 2: { vms: 3, databases: 3 } })
  })

  describe('files', () => {
    it("offers each host's files, the schedule's own, or none", () => {
      const options = mount().findAll('[role="radiogroup"][aria-label="Files"] button')
      expect(options.map((o) => o.text())).toEqual(["Each host's files", 'Own files', 'Off'])
    })

    it('writes the chosen files mode back through the form model', async () => {
      const state = form()
      const wrapper = mount({ form: state })
      await wrapper.findAll('[role="radiogroup"][aria-label="Files"] button')[1].trigger('click')
      expect(state.files_mode).toBe('own')
    })

    it('explains the three files modes on request', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: where the files come from"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain(
        'reads no files, for a schedule that only dumps databases or snapshots VMs',
      )
    })

    it("offers to skip the hosts' commands only while taking each host's files", () => {
      expect(toggle(mount(), "Run each host's backup commands")).toBeDefined()
      for (const files_mode of ['own', 'off'] as const) {
        expect(
          toggle(mount({ form: form({ files_mode }) }), "Run each host's backup commands"),
        ).toBeUndefined()
      }
    })

    it('writes the host-commands switch onto form.run_host_commands', async () => {
      const state = form()
      const wrapper = mount({ form: state })
      await toggle(wrapper, "Run each host's backup commands")!.vm.$emit('update:modelValue', false)
      expect(state.run_host_commands).toBe(false)
    })

    it('shows no own-files section unless the schedule sets its own files', () => {
      expect(mount().findComponent({ name: 'FileSourcesSection' }).exists()).toBe(false)
      expect(
        mount({ form: form({ files_mode: 'off' }) })
          .findComponent({ name: 'FileSourcesSection' })
          .exists(),
      ).toBe(false)
    })
  })

  describe('own files', () => {
    const own = (overrides: Partial<ScheduleFormState> = {}) =>
      form({ files_mode: 'own', ...overrides })

    it('renders the schedule-level values it was given', () => {
      const wrapper = mount({ form: own() })
      const view = wrapper.findComponent({ name: 'FileSourcesView' })
      expect(view.props('paths')).toEqual(['/srv'])
      expect(view.props('excludes')).toEqual(['*.cache'])
    })

    // The editor is a whole form of its own, so it takes the pane's full
    // width beneath the rows rather than the narrow control track of one.
    it('nests the editor under the rows rather than in a row control', () => {
      const section = mount({ form: own() }).findComponent({ name: 'FileSourcesSection' })
      expect(section.element.closest('.pane-nest')).not.toBeNull()
      expect(section.element.closest('.pane-row-control')).toBeNull()
    })

    it('starts the edit form from the saved values', async () => {
      const wrapper = mount({ form: own() })
      await button(wrapper, 'Edit')!.trigger('click')
      expect((wrapper.find('#file-sources-paths').element as HTMLTextAreaElement).value).toBe(
        '/srv',
      )
      expect((wrapper.find('#file-sources-excludes').element as HTMLTextAreaElement).value).toBe(
        '*.cache',
      )
    })

    it('writes every edited field back through the form model on Save', async () => {
      const state = own()
      const wrapper = mount({ form: state })
      await button(wrapper, 'Edit')!.trigger('click')

      await wrapper.find('#file-sources-paths').setValue('/home\n/var/www')
      await wrapper.find('#file-sources-excludes').setValue('pp:__pycache__')
      await wrapper.find('#file-sources-includes').setValue('/home/keep')
      await wrapper
        .findComponent({ name: 'FileChangePatternsEditor' })
        .vm.$emit('update:modelValue', '**/*.log')
      const commands = wrapper.findAllComponents({ name: 'CommandListEditor' })
      await commands[0].vm.$emit('update:modelValue', [
        { command: 'systemctl stop app', timeout_seconds: null },
      ])
      await commands[1].vm.$emit('update:modelValue', [
        { command: 'systemctl start app', timeout_seconds: 30 },
      ])

      // Nothing reaches the form before Save: Cancel has to be able to drop it.
      expect(state.exclude_patterns).toBe('*.cache')

      await button(wrapper, 'Save')!.trigger('click')
      await flushPromises()

      expect(state).toMatchObject({
        backup_sources: '/home\n/var/www',
        exclude_patterns: 'pp:__pycache__',
        include_patterns: '/home/keep',
        file_change_patterns: '**/*.log',
        pre_backup_commands: [{ command: 'systemctl stop app', timeout_seconds: null }],
        post_backup_commands: [{ command: 'systemctl start app', timeout_seconds: 30 }],
      })
      // With nothing to commit (the wizard), Save just closes the form.
      expect(wrapper.find('#file-sources-paths').exists()).toBe(false)
    })

    it('leaves the form alone on Cancel', async () => {
      const state = own()
      const wrapper = mount({ form: state })
      await button(wrapper, 'Edit')!.trigger('click')
      await wrapper.find('#file-sources-excludes').setValue('*.tmp')
      await button(wrapper, 'Cancel')!.trigger('click')

      expect(state.exclude_patterns).toBe('*.cache')
      expect(wrapper.find('#file-sources-excludes').exists()).toBe(false)
    })

    it('commits the schedule after writing the values, and closes once that worked', async () => {
      const state = own()
      const commit = vi.fn(() => {
        // The values are in the form by the time the save is asked for.
        expect(state.exclude_patterns).toBe('*.tmp')
        return Promise.resolve(null)
      })
      const wrapper = mount({ form: state, commit })
      await button(wrapper, 'Edit')!.trigger('click')
      await wrapper.find('#file-sources-excludes').setValue('*.tmp')
      await button(wrapper, 'Save')!.trigger('click')
      await flushPromises()

      expect(commit).toHaveBeenCalledTimes(1)
      expect(wrapper.find('#file-sources-excludes').exists()).toBe(false)
    })

    it('keeps the form open with the error when the commit fails', async () => {
      const commit = vi.fn().mockResolvedValue('Failed to save schedule')
      const wrapper = mount({ form: own(), commit })
      await button(wrapper, 'Edit')!.trigger('click')
      await button(wrapper, 'Save')!.trigger('click')
      await flushPromises()

      expect(wrapper.find('#file-sources-excludes').exists()).toBe(true)
      expect(wrapper.find('.form-error').text()).toBe('Failed to save schedule')
    })

    it('offers to split the paths per host only on a multi-host schedule', () => {
      expect(toggle(mount({ form: own() }), 'Different paths per host')).toBeDefined()
      expect(
        toggle(mount({ form: own(), agentIds: [1] }), 'Different paths per host'),
      ).toBeUndefined()
      expect(toggle(mount(), 'Different paths per host')).toBeUndefined()
    })

    it('discloses what splitting the paths per host does', async () => {
      const wrapper = mount({ form: own() })
      await wrapper.find('[aria-label="Help: splitting the paths by host"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toBe(
        'Give each host its own paths. Excludes, patterns and commands stay shared.',
      )
    })

    it('writes the per-host switch back through its model', async () => {
      const wrapper = mount({ form: own() })
      await toggle(wrapper, 'Different paths per host')!.vm.$emit('update:modelValue', true)
      expect(wrapper.emitted('update:usePerHostPaths')?.at(-1)?.[0]).toBe(true)
    })

    it('shows the paths of each host once they are split', () => {
      const wrapper = mount({
        form: own(),
        usePerHostPaths: true,
        perHostSources: { 1: '/home\n/etc', 2: '' },
      })
      const grid = wrapper.find('.info-grid')
      expect(grid.findAll('dt').map((d) => d.text())).toEqual(['host-01', 'host-02'])
      expect(grid.findAll('dd').map((d) => d.text())).toEqual(['/home /etc', '-'])
    })

    it('edits a paths field per host once they are split, routing each to its host', async () => {
      const perHostSources: Record<number, string> = {}
      const wrapper = mount({ form: own(), usePerHostPaths: true, perHostSources })
      await button(wrapper, 'Edit')!.trigger('click')

      expect(wrapper.find('#file-sources-paths').exists()).toBe(false)
      expect(wrapper.findComponent({ name: 'PerAgentFields' }).text()).toContain('host-02')
      const fields = wrapper.findAll('textarea[placeholder="Directories to back up, one per line"]')
      expect(fields).toHaveLength(2)
      await fields[1].setValue('/opt/data')
      expect(perHostSources[2]).toBe('/opt/data')
    })
  })

  // The per-agent switches for excludes, includes, file change patterns and
  // commands are gone; only the paths split per host. A schedule that still
  // carries per-host values from before is told so, with a way back to the
  // shared ones, instead of having them act unseen.
  describe('per-host values', () => {
    const own = form({ files_mode: 'own' })

    it('says nothing while every host uses the shared values', () => {
      expect(mount({ form: own }).find('[data-testid="per-host-file-values"]').exists()).toBe(false)
    })

    it('names each host and the values it keeps for itself', () => {
      const wrapper = mount({
        form: own,
        overrides: agentOverrides({
          usePerHostExcludes: true,
          perHostExcludes: { 1: '/var/cache' },
          usePerHostIncludes: true,
          usePerHostFileChangePatterns: true,
          usePerAgentCmds: true,
        }),
      })
      const notice = wrapper.find('[data-testid="per-host-file-values"]')
      expect(notice.text()).toContain('Some hosts use their own values')
      expect(notice.text().replace(/\s+/g, ' ')).toContain(
        'host-01 (excludes, include patterns, file change patterns, commands); host-02 (excludes, include patterns, file change patterns, commands).',
      )
    })

    it('lists only the kinds of value that are split', () => {
      const notice = mount({
        form: own,
        overrides: agentOverrides({ usePerAgentCmds: true }),
      }).find('[data-testid="per-host-file-values"]')
      expect(notice.text().replace(/\s+/g, ' ')).toContain(
        'host-01 (commands); host-02 (commands).',
      )
    })

    it('clears them all and marks them for deletion on "Use the shared values"', async () => {
      const wrapper = mount({
        form: own,
        overrides: agentOverrides({
          usePerHostExcludes: true,
          perHostExcludes: { 1: '/var/cache' },
          usePerHostIncludes: true,
          perHostIncludes: { 2: '/keep' },
          usePerHostFileChangePatterns: true,
          perHostFileChangePatterns: { 1: '**/*.log:ignore' },
          usePerAgentCmds: true,
          perAgentPreCmds: { 1: [{ command: 'stop', timeout_seconds: null }] },
          perAgentPostCmds: { 2: [{ command: 'start', timeout_seconds: null }] },
        }),
      })
      await button(wrapper, 'Use the shared values')!.trigger('click')

      expect(wrapper.emitted('update:overrides')?.at(-1)?.[0]).toEqual({
        ...agentOverrides(),
        clearPerHost: true,
      })
    })
  })

  describe('virtual machines and databases', () => {
    it.each([
      ['Virtual machines', 'vm_snapshot_enabled'],
      ['Databases', 'database_backup_enabled'],
    ] as const)('flips the %s switch onto form.%s', async (label, key) => {
      const state = form()
      const wrapper = mount({ form: state })
      await toggle(wrapper, label)!.vm.$emit('update:modelValue', true)
      expect(state[key]).toBe(true)
    })

    it('discloses the VM snapshot hint with its explainer text', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: VM snapshots"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain('Snapshots the libvirt domains')
    })

    it('discloses the database dumps hint with its explainer text', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: database dumps"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain(
        'Streams a dump of the database sources each host includes',
      )
    })

    it('names the target hosts that have none set up, only for what the schedule takes', async () => {
      hostSources({ 1: { vms: 0, databases: 2 }, 2: { vms: 3, databases: 0 } })
      const wrapper = mount({
        form: form({ vm_snapshot_enabled: true, database_backup_enabled: false }),
      })
      await flushPromises()

      const vms = wrapper.find('[data-testid="source-not-set-up-vms"]')
      expect(vms.text()).toContain('No virtual machines will be backed up on 1 host')
      expect(vms.findAll('a').map((a) => a.text())).toEqual(['host-01'])
      // host-02 has no databases either, but this schedule does not take them.
      expect(wrapper.find('[data-testid="source-not-set-up-databases"]').exists()).toBe(false)

      await wrapper.setProps({
        form: form({ vm_snapshot_enabled: true, database_backup_enabled: true }),
      })
      const databases = wrapper.find('[data-testid="source-not-set-up-databases"]')
      expect(databases.findAll('a').map((a) => a.text())).toEqual(['host-02'])
    })

    it('only asks about the hosts this schedule targets', async () => {
      const wrapper = mount({ agentIds: [2] })
      await flushPromises()
      expect(vi.mocked(getAgentVms).mock.calls.map((call) => call[0])).toEqual(['host-02'])
      expect(vi.mocked(getAgentDatabases).mock.calls.map((call) => call[0])).toEqual(['host-02'])
      expect(previewRows(wrapper).map((row) => row[0])).toEqual(['host-02'])
    })

    it('does not report a host whose lists could not be read as having none', async () => {
      vi.mocked(getAgentVms).mockRejectedValue(new Error('offline'))
      const wrapper = mount({
        form: form({ vm_snapshot_enabled: true, database_backup_enabled: true }),
      })
      await flushPromises()
      expect(wrapper.find('[data-testid="source-not-set-up-vms"]').exists()).toBe(false)
      expect(wrapper.find('[data-testid="source-not-set-up-databases"]').exists()).toBe(false)
    })
  })

  describe('what each host will back up', () => {
    it("lists each host's own paths and commands while taking each host's files", async () => {
      const wrapper = mount({
        form: form({ vm_snapshot_enabled: true, database_backup_enabled: false }),
      })
      await flushPromises()
      expect(previewRows(wrapper)).toEqual([
        ['host-01', '/etc /home', '1 pre, 0 post', '2 VMs', 'Off'],
        ['host-02', '/etc /home', '1 pre, 0 post', '3 VMs', 'Off'],
      ])
      expect(wrapper.find('a[href="/agents/host-01?tab=sources"]').exists()).toBe(true)
    })

    it('says the commands are skipped when the schedule turns them off', () => {
      const rows = previewRows(mount({ form: form({ run_host_commands: false }) }))
      expect(rows.map((row) => row[2])).toEqual(['Skipped', 'Skipped'])
    })

    it("says what the schedule's own paths replace", () => {
      const rows = previewRows(
        mount({
          form: form({
            files_mode: 'own',
            backup_sources: '/srv',
            post_backup_commands: [{ command: 'start', timeout_seconds: null }],
          }),
        }),
      )
      expect(rows[0].slice(1, 3)).toEqual(["/srv (replaces the host's 2)", '0 pre, 1 post'])
    })

    it('shows each host its own paths once they are split', () => {
      const rows = previewRows(
        mount({
          form: form({ files_mode: 'own' }),
          usePerHostPaths: true,
          perHostSources: { 1: '/data' },
        }),
      )
      expect(rows.map((row) => row[1])).toEqual([
        "/data (replaces the host's 2)",
        'No paths given for it',
      ])
    })

    it('reads no files and runs no commands while files are off', () => {
      const rows = previewRows(mount({ form: form({ files_mode: 'off' }) }))
      expect(rows.map((row) => row.slice(1, 3))).toEqual([
        ['Off', 'None'],
        ['Off', 'None'],
      ])
    })

    it('marks a host that has nothing set up for what the schedule takes', async () => {
      hostSources({ 1: { vms: 0, databases: 0 }, 2: { vms: 1, databases: 1 } })
      const wrapper = mount({ form: form({ database_backup_enabled: true }) })
      await flushPromises()
      const cell = wrapper.findAll('tbody tr')[0].findAll('td')[4]
      expect(cell.text()).toBe('None set up')
      expect(cell.classes()).toContain('source-missing')
    })

    it('flags a host with no paths of its own', () => {
      const wrapper = mount({
        agents: [agent(1, { default_backup_paths: [] }), agent(2)],
      })
      expect(previewRows(wrapper)[0][1]).toBe('No paths set on this host')
    })
  })
})
