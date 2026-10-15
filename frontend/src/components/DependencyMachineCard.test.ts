// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
import { dependencyHost } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import DependencyMachineCard from './DependencyMachineCard.vue'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn(), get: vi.fn() },
}))

const OWN = dependencyHost()
const SHARED = dependencyHost({ machine_id: 20, repo_host: { id: 9, ssh_host: 'nas-01' } })

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(DependencyMachineCard, {
    props: { host: OWN, isAdmin: true, ...props },
  })
}

async function choose(wrapper: ReturnType<typeof mount>, source: 'own' | 'shared') {
  await wrapper.findComponent({ name: 'BaseSegmented' }).vm.$emit('update:modelValue', source)
  await flushPromises()
}

describe('DependencyMachineCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockResolvedValue({
      data: [
        { id: 9, ssh_host: 'nas-01' },
        { id: 11, ssh_host: 'backup-02' },
      ],
    } as never)
    vi.mocked(apiClient.put).mockResolvedValue({ data: SHARED } as never)
  })

  it('says a dependency is on its own machine', () => {
    expect(mount().text()).toContain('Its own machine')
  })

  it('names and links the repository host whose machine it shares', () => {
    const link = mount({ host: SHARED }).get('a')
    expect(link.text()).toBe('nas-01')
    expect(link.attributes('href')).toBe('/repo-hosts/9?section=power')
  })

  it('hides the Edit button from a non-admin', () => {
    expect(
      mount({ isAdmin: false })
        .findAll('button')
        .some((b) => b.text() === 'Edit'),
    ).toBe(false)
  })

  it("moves it onto a repository host's machine", async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await choose(wrapper, 'shared')
    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts')
    await wrapper.get('#dependency-machine-repo-host').setValue('11')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/machine', {
      repo_host_id: 11,
    })
    expect(wrapper.emitted('saved')).toEqual([[SHARED]])
  })

  it('moves it back onto a machine of its own', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: OWN } as never)
    const wrapper = mount({ host: SHARED })
    await startEditingSection(wrapper)
    await choose(wrapper, 'own')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/machine', {
      repo_host_id: null,
    })
  })

  it('asks for a repository host before sharing one, without a request', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await choose(wrapper, 'shared')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).not.toHaveBeenCalled()
    expect(wrapper.find('.form-error').text()).toContain('Pick the repository host')
  })

  // A host the list no longer returns is still the one it is on.
  it('still offers the shared host when the host list fails to load', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    const wrapper = mount({ host: SHARED })
    await startEditingSection(wrapper)
    await flushPromises()

    const options = wrapper.findAll('#dependency-machine-repo-host option').map((o) => o.text())
    expect(options).toContain('nas-01')
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('unknown repository host'))
    await expectSaveErrorKeepsEditing(
      mount({ host: SHARED }),
      'unknown repository host',
      '#dependency-machine-repo-host',
    )
  })

  it('explains what sharing a machine takes and leaves without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('[aria-label="Help: sharing a repository host\'s machine"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('never woken, always online')

    await clickSectionButton(wrapper, 'Cancel')
    expect(wrapper.findComponent({ name: 'BaseSegmented' }).exists()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })
})
