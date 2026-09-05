// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { apiClient } from '../api/client'
import AgentVmsCard from './AgentVmsCard.vue'
import VmRestoreWizard from './VmRestoreWizard.vue'
import type { AgentRow } from '../types/agent'
import type { AgentVmResponse, AgentVmSnapshotResponse, VmSelectionMode } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), put: vi.fn(), post: vi.fn() },
}))

const GIB = 1024 * 1024 * 1024

const AGENT = { hostname: 'virt-host-01', domain: null } as unknown as AgentRow

function vm(overrides: Partial<AgentVmResponse> = {}): AgentVmResponse {
  return {
    name: 'web01',
    included: true,
    limit_bytes: null,
    effective_limit_bytes: 200 * GIB,
    state: 'running',
    mode: 'incremental',
    disk_count: 1,
    disk_bytes: 40 * GIB,
    staged_bytes: 42 * GIB,
    chain_length: 4,
    last_error: null,
    last_scanned_at: '2026-09-03T02:00:00Z',
    last_staged_at: '2026-09-03T02:04:00Z',
    ...overrides,
  }
}

function response(
  vms: AgentVmResponse[] = [vm()],
  selection: VmSelectionMode = 'all',
): AgentVmSnapshotResponse {
  return {
    settings: {
      enabled: true,
      staging_dir: '/srv/vm-staging',
      full_interval: 7,
      timeout_seconds: 1800,
      default_limit_bytes: 200 * GIB,
      selection,
    },
    vms,
  }
}

async function mount(props: Record<string, unknown> = {}) {
  const wrapper = renderWithPlugins(AgentVmsCard, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
  await flushPromises()
  return wrapper
}

describe('AgentVmsCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.post).mockReset()
    // The card loads the VM table; the restore wizard it opens loads this
    // host's reports, so the two are answered by URL rather than by one
    // blanket response.
    vi.mocked(apiClient.get).mockImplementation(((url: string) =>
      Promise.resolve({
        data: url.endsWith('/reports') ? [] : response(),
      })) as never)
    vi.mocked(apiClient.put).mockResolvedValue({ data: response() } as never)
    vi.mocked(apiClient.post).mockResolvedValue({ data: response() } as never)
  })

  it('summarizes the host settings it loaded', async () => {
    const text = (await mount()).text()
    expect(text).toContain('/srv/vm-staging')
    expect(text).toContain('7 increments')
    expect(text).toContain('1800 seconds per domain')
    expect(text).toContain('200 GiB')
  })

  it('lists each domain with its state, mode and staged size', async () => {
    const text = (await mount()).text()
    expect(text).toContain('web01')
    expect(text).toContain('Running')
    expect(text).toContain('Incremental')
    expect(text).toContain('full + 4 increments')
    expect(text).toContain('42.0 GB of 200.0 GB')
  })

  it('says a domain inherits the host default until it is overridden', async () => {
    const wrapper = await mount()
    expect(wrapper.text()).toContain('Host default')

    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ limit_bytes: 500 * GIB, effective_limit_bytes: 500 * GIB })]),
    } as never)
    const overridden = await mount()
    expect(overridden.text()).toContain('Overridden')
  })

  it('shows the reason a domain failed its last run', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ last_error: 'a full backup needs about 21.0 GiB' })]),
    } as never)
    expect((await mount()).text()).toContain('a full backup needs about 21.0 GiB')
  })

  it('offers a rescan when the host has never been scanned', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: response([]) } as never)
    const wrapper = await mount()
    expect(wrapper.text()).toContain('No domains reported')
  })

  it('asks the agent to rescan its host', async () => {
    const wrapper = await mount()
    const rescan = wrapper.findAll('button').find((b) => b.text().includes('Rescan host'))
    await rescan?.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/agents/virt-host-01/vms/scan', {}, { params: {} })
  })

  it('reports a scan the agent could not run', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('agent is not connected'))
    const wrapper = await mount()
    const rescan = wrapper.findAll('button').find((b) => b.text().includes('Rescan host'))
    await rescan?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('agent is not connected')
  })

  it('saves the host settings in bytes, from the GiB the operator typed', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)

    await wrapper.find<HTMLInputElement>('#vm-staging-dir').setValue('/data/vm-staging')
    await wrapper.find<HTMLInputElement>('#vm-full-interval').setValue('14')
    await wrapper.find<HTMLInputElement>('#vm-default-limit').setValue('500')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vm-snapshot',
      {
        enabled: true,
        staging_dir: '/data/vm-staging',
        full_interval: 14,
        timeout_seconds: 1800,
        default_limit_bytes: 500 * GIB,
        selection: 'all',
      },
      { params: {} },
    )
  })

  it('saves a per-domain limit as soon as it is entered', async () => {
    const wrapper = await mount()
    const limit = wrapper.find<HTMLInputElement>('input.vm-limit')
    limit.element.value = '50'
    await limit.trigger('change')
    await flushPromises()

    // No `included` in the payload: the flag the table shows for an undecided
    // domain is the host's mode resolved, so sending it back would turn
    // setting a budget into an opt-in.
    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vms/web01',
      { limit_bytes: 50 * GIB },
      { params: {} },
    )
  })

  it('clears a per-domain limit back to the host default', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ limit_bytes: 500 * GIB, effective_limit_bytes: 500 * GIB })]),
    } as never)
    const wrapper = await mount()
    const limit = wrapper.find<HTMLInputElement>('input.vm-limit')
    limit.element.value = ''
    await limit.trigger('change')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vms/web01',
      { limit_bytes: null },
      { params: {} },
    )
  })

  it('leaves an imported host read only', async () => {
    const wrapper = await mount({ canEdit: false })
    expect(wrapper.text()).not.toContain('Rescan host')
    expect(wrapper.find<HTMLInputElement>('input.vm-limit').element.disabled).toBe(true)
  })

  it('says a domain with no budget is unlimited', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ limit_bytes: 0, effective_limit_bytes: 0 })]),
    } as never)
    const wrapper = await mount()

    expect(wrapper.text()).toContain('No limit')
    // Nothing to be a share of, so there is no bar to fill.
    expect(wrapper.find('.progress-track').exists()).toBe(false)
  })

  // The bar is the warning a domain is about to blow its budget, so which
  // tone it takes at which share is the behaviour, not decoration.
  it.each([
    [42, ''],
    [160, 'progress-bar--warning'],
    [195, 'progress-bar--danger'],
  ])('marks a domain staging %i GiB of its 200 GiB budget', async (stagedGib, tone) => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ staged_bytes: stagedGib * GIB })]),
    } as never)
    const wrapper = await mount()

    const bar = wrapper.find('.progress-bar')
    expect(bar.classes().includes('progress-bar--warning')).toBe(tone === 'progress-bar--warning')
    expect(bar.classes().includes('progress-bar--danger')).toBe(tone === 'progress-bar--danger')
  })

  it('reports a host whose settings could not be loaded', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('agent is not connected'))
    const wrapper = await mount()

    expect(wrapper.text()).toContain('agent is not connected')
    // The table is what failed to load, so none of it is claimed to be there.
    expect(wrapper.find('input.vm-limit').exists()).toBe(false)
  })

  it('keeps the operator in the form when the host settings fail to save', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    vi.mocked(apiClient.put).mockRejectedValue(new Error('staging_dir must be absolute'))

    await wrapper.find<HTMLInputElement>('#vm-staging-dir').setValue('relative/path')
    await clickSectionButton(wrapper, 'Save')

    expect(wrapper.text()).toContain('staging_dir must be absolute')
    // Still editing: the value the operator typed is not thrown away.
    expect(wrapper.find<HTMLInputElement>('#vm-staging-dir').element.value).toBe('relative/path')
  })

  it('abandons an edit on cancel, keeping what the host had', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await wrapper.find<HTMLInputElement>('#vm-staging-dir').setValue('/data/vm-staging')
    await clickSectionButton(wrapper, 'Cancel')

    expect(apiClient.put).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('/srv/vm-staging')
  })

  it('saves the staging toggle and the timeout the operator set', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)

    await wrapper.find<HTMLInputElement>('#vm-timeout').setValue('900')
    await wrapper.find('[role="switch"]').trigger('click')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vm-snapshot',
      expect.objectContaining({ enabled: false, timeout_seconds: 900 }),
      { params: {} },
    )
  })

  it('excludes a domain from staging when its include switch is turned off', async () => {
    const wrapper = await mount()
    const rowSwitch = wrapper.find('tbody tr [role="switch"]')
    await rowSwitch.trigger('click')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vms/web01',
      { included: false, limit_bytes: null },
      { params: {} },
    )
  })

  it('sends the include flag only when the operator decided it', async () => {
    const wrapper = await mount()

    const limit = wrapper.find<HTMLInputElement>('input.vm-limit')
    limit.element.value = '50'
    await limit.trigger('change')
    await flushPromises()
    const limitPayload = vi.mocked(apiClient.put).mock.calls[0][1] as Record<string, unknown>
    expect(
      'included' in limitPayload,
      'a budget is a cap, not consent - the flag must be absent',
    ).toBe(false)

    await wrapper.find('tbody tr [role="switch"]').trigger('click')
    await flushPromises()
    const togglePayload = vi.mocked(apiClient.put).mock.calls[1][1] as Record<string, unknown>
    expect(togglePayload).toHaveProperty('included', false)
  })

  it('reports a per-domain change the agent rejected', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('domain is unknown to this host'))
    const wrapper = await mount()
    await wrapper.find('tbody tr [role="switch"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('domain is unknown to this host')
  })

  it('summarizes which domains the host stages', async () => {
    expect((await mount()).text()).toContain('Every domain except the ones excluded below')
  })

  it('summarizes an opt-in host as staging only what was selected', async () => {
    vi.mocked(apiClient.get).mockImplementation(((url: string) =>
      Promise.resolve({
        data: url.endsWith('/reports') ? [] : response([vm()], 'selected'),
      })) as never)

    expect((await mount()).text()).toContain('Only the domains selected below')
  })

  it('tells the operator what the include switch decides, per mode', async () => {
    const including = await mount()
    expect(including.text()).toContain('Turn a domain off to leave it out of the backup')

    vi.mocked(apiClient.get).mockImplementation(((url: string) =>
      Promise.resolve({
        data: url.endsWith('/reports') ? [] : response([vm()], 'selected'),
      })) as never)
    const selecting = await mount()
    expect(selecting.text()).toContain('Turn a domain on to back it up')
  })

  it('switches the host to staging only the domains that were selected', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)

    const onlySelected = wrapper
      .findAll('[role="radio"]')
      .find((option) => option.text() === 'Only selected')
    await onlySelected?.trigger('click')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vm-snapshot',
      expect.objectContaining({ selection: 'selected' }),
      { params: {} },
    )
  })

  it('abandons a change of mode on cancel', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)

    const onlySelected = wrapper
      .findAll('[role="radio"]')
      .find((option) => option.text() === 'Only selected')
    await onlySelected?.trigger('click')
    await clickSectionButton(wrapper, 'Cancel')

    expect(wrapper.text()).toContain('Every domain except the ones excluded below')
    expect(apiClient.put).not.toHaveBeenCalled()
  })

  it('opens the restore wizard for one domain and closes it again', async () => {
    const wrapper = await mount()
    const restore = wrapper.findAll('button').find((b) => b.text() === 'Restore')
    await restore?.trigger('click')
    await flushPromises()

    const wizard = wrapper.findComponent(VmRestoreWizard)
    expect(wizard.exists()).toBe(true)
    expect(wizard.props('domainName')).toBe('web01')

    wizard.vm.$emit('close')
    await flushPromises()
    expect(wrapper.findComponent(VmRestoreWizard).exists()).toBe(false)
  })

  it('reloads the domains once a restore has finished', async () => {
    const wrapper = await mount()
    const restore = wrapper.findAll('button').find((b) => b.text() === 'Restore')
    await restore?.trigger('click')
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    wrapper.findComponent(VmRestoreWizard).vm.$emit('restored')
    await flushPromises()

    // The wizard closes and the table is re-read, so a restored domain's new
    // staged size is what the operator sees.
    expect(wrapper.findComponent(VmRestoreWizard).exists()).toBe(false)
    expect(apiClient.get).toHaveBeenCalled()
  })
})
