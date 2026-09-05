// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import VmRestoreWizard from './VmRestoreWizard.vue'
import BaseModal from './BaseModal.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn() },
}))

const AGENT = { hostname: 'virt-host-01', domain: null } as unknown as AgentRow

const REPORTS = [
  {
    id: 2,
    repo_id: 7,
    repo_name: 'nas-daily',
    archive_name: 'virt-host-01-2026-09-03T02:00',
    started_at: '2026-09-03T02:00:00Z',
    status: 'success',
  },
  {
    id: 1,
    repo_id: 7,
    repo_name: 'nas-daily',
    archive_name: null,
    started_at: '2026-09-02T02:00:00Z',
    status: 'failed',
  },
]

const OUTCOME = {
  name: 'web01-restored',
  images: ['/var/lib/libvirt/images/web01-restored-vda.qcow2'],
  merged_increments: 4,
  defined: true,
  started: false,
}

async function mount() {
  const wrapper = renderWithPlugins(VmRestoreWizard, {
    props: {
      open: true,
      agent: AGENT,
      domainName: 'web01',
      stagingDir: '/srv/vm-staging',
    },
  })
  await flushPromises()
  return wrapper
}

function button(wrapper: Awaited<ReturnType<typeof mount>>, label: string) {
  return wrapper.findAll('button').find((b) => b.text() === label)
}

describe('VmRestoreWizard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: REPORTS } as never)
    vi.mocked(apiClient.post).mockImplementation((url: string) =>
      Promise.resolve({
        data: url.endsWith('/build') ? OUTCOME : { success: true, files_restored: 5 },
      } as never),
    )
  })

  it('offers only the runs that produced an archive', async () => {
    const wrapper = await mount()
    expect(wrapper.text()).toContain('virt-host-01-2026-09-03T02:00')
    expect(wrapper.findAll('input[name="vm-restore-archive"]')).toHaveLength(1)
  })

  it('says where stage one will leave the domain', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')

    expect(wrapper.text()).toContain('/var/tmp/assimilate-restore/srv/vm-staging/web01')
  })

  it('defaults the restored domain to a new name', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')

    expect(wrapper.find<HTMLInputElement>('#vm-restore-name').element.value).toBe('web01-restored')
  })

  it('restores the files and then builds the domain', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    // Stage one: the staged domain's path out of the archive, into the working
    // directory, on the host that owns it.
    expect(apiClient.post).toHaveBeenNthCalledWith(
      1,
      '/repos/7/archives/virt-host-01-2026-09-03T02%3A00/restore',
      {
        paths: ['srv/vm-staging/web01'],
        target_path: '/var/tmp/assimilate-restore',
        hostname: 'virt-host-01',
      },
    )

    // Stage two: build from exactly where stage one put it.
    expect(apiClient.post).toHaveBeenNthCalledWith(
      2,
      '/agents/virt-host-01/vms/build',
      {
        source_dir: '/var/tmp/assimilate-restore/srv/vm-staging/web01',
        name: 'web01-restored',
        image_dir: '/var/lib/libvirt/images',
        action: 'define',
      },
      { params: {} },
    )

    expect(wrapper.text()).toContain('4')
    expect(wrapper.text()).toContain('Defined, shut off')
    expect(wrapper.emitted('restored')).toBeTruthy()
  })

  it('skips borg when the files are already on disk', async () => {
    const wrapper = await mount()
    await wrapper.find('button[role="switch"]').trigger('click')
    await button(wrapper, 'Next')?.trigger('click')
    await wrapper.find<HTMLInputElement>('#vm-restore-source').setValue('/data/restored/web01')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledTimes(1)
    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/virt-host-01/vms/build',
      expect.objectContaining({ source_dir: '/data/restored/web01' }),
      { params: {} },
    )
    expect(wrapper.text()).toContain('Skipped, the files are already on disk')
  })

  it('stops at the failed stage and says why', async () => {
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: { success: false, error_message: 'no such path in archive' },
    } as never)
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('no such path in archive')
    // The build never ran, so nothing was defined.
    expect(apiClient.post).toHaveBeenCalledTimes(1)
    expect(wrapper.emitted('restored')).toBeFalsy()
  })

  it('reports a build the agent refused', async () => {
    vi.mocked(apiClient.post).mockImplementation((url: string) =>
      url.endsWith('/build')
        ? Promise.reject(new Error('the chain of vda is incomplete'))
        : Promise.resolve({ data: { success: true } } as never),
    )
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('the chain of vda is incomplete')
  })

  it('will not proceed without an archive to restore from', async () => {
    const wrapper = await mount()
    expect(button(wrapper, 'Next')?.attributes('disabled')).toBeDefined()
  })

  it('asks the server for nothing while it is closed', async () => {
    renderWithPlugins(VmRestoreWizard, {
      props: { open: false, agent: AGENT, domainName: 'web01', stagingDir: '/srv/vm-staging' },
    })
    await flushPromises()

    expect(apiClient.get).not.toHaveBeenCalled()
  })

  it('says why the archives could not be listed', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('repository is unreachable'))
    const wrapper = await mount()

    expect(wrapper.text()).toContain('repository is unreachable')
    expect(wrapper.findAll('input[name="vm-restore-archive"]')).toHaveLength(0)
  })

  it('steps back to the archive it came from', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')
    expect(wrapper.find('#vm-restore-name').exists()).toBe(true)

    await button(wrapper, 'Back')?.trigger('click')
    await button(wrapper, 'Back')?.trigger('click')

    // Back at stage one, with the chosen archive still chosen.
    expect(wrapper.find<HTMLInputElement>('input[name="vm-restore-archive"]').element.checked).toBe(
      true,
    )
    expect(button(wrapper, 'Back')?.attributes('disabled')).toBeDefined()
  })

  it('builds into the directories and under the name the operator chose', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await wrapper.find<HTMLInputElement>('#vm-restore-workdir').setValue('/data/staging')
    await button(wrapper, 'Next')?.trigger('click')

    await wrapper.find<HTMLInputElement>('#vm-restore-name').setValue('web01-clone')
    await wrapper.find<HTMLInputElement>('#vm-restore-image-dir').setValue('/pool/images')
    await wrapper.find('input[type="radio"][value="define_and_start"]').setValue()
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenNthCalledWith(
      2,
      '/agents/virt-host-01/vms/build',
      {
        source_dir: '/data/staging/srv/vm-staging/web01',
        name: 'web01-clone',
        image_dir: '/pool/images',
        action: 'define_and_start',
      },
      { params: {} },
    )
  })

  it('can be put back to defining the domain after another option', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')

    await wrapper.find('input[type="radio"][value="files_only"]').setValue()
    await wrapper.find('input[type="radio"][value="define"]').setValue()
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenNthCalledWith(
      2,
      '/agents/virt-host-01/vms/build',
      expect.objectContaining({ action: 'define' }),
      { params: {} },
    )
  })

  it('leaves the images alone when that is all that was asked for', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')
    await wrapper.find('input[type="radio"][value="files_only"]').setValue()
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenNthCalledWith(
      2,
      '/agents/virt-host-01/vms/build',
      expect.objectContaining({ action: 'files_only' }),
      { params: {} },
    )
  })

  it('closes once the operator is done with the outcome', async () => {
    const wrapper = await mount()
    await wrapper.find('input[name="vm-restore-archive"]').trigger('change')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Next')?.trigger('click')
    await button(wrapper, 'Restore')?.trigger('click')
    await flushPromises()

    await button(wrapper, 'Done')?.trigger('click')
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('closes when the dialog itself is dismissed', async () => {
    const wrapper = await mount()
    wrapper.findComponent(BaseModal).vm.$emit('close')
    await flushPromises()

    expect(wrapper.emitted('close')).toBeTruthy()
  })
})
