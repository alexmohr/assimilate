// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { dependencyHost } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import RepoHostDependenciesCard from './RepoHostDependenciesCard.vue'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn(), get: vi.fn() },
}))

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoHostDependenciesCard, {
    props: { hostId: 5, canEdit: true, ...props },
  })
}

describe('RepoHostDependenciesCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockImplementation(async (url: string) => {
      if (url === '/dependency-hosts') {
        return { data: [dependencyHost(), dependencyHost({ id: 4, name: 'files-01' })] } as never
      }
      return { data: { dependency_host_ids: [3] } } as never
    })
    vi.mocked(apiClient.put).mockResolvedValue({ data: { dependency_host_ids: [3, 4] } } as never)
  })

  it('lists what every backup to this host needs, and where it applies', async () => {
    const wrapper = mount()
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts/5/dependencies')
    expect(wrapper.text()).toContain('nas-media')
    expect(wrapper.text()).not.toContain('files-01')
    expect(wrapper.text()).toContain('every schedule writing to a repository on this host')
  })

  it('saves the picked dependencies to the host', async () => {
    const wrapper = mount()
    await flushPromises()
    await startEditingSection(wrapper)
    await wrapper.findAll('input[type="checkbox"]')[1]!.setValue(true)
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/repo-hosts/5/dependencies', {
      dependency_host_ids: [3, 4],
    })
    expect(wrapper.text()).toContain('files-01')
  })

  it('loads the other host when it is pointed at a different one', async () => {
    const wrapper = mount()
    await flushPromises()
    await wrapper.setProps({ hostId: 6 })
    await flushPromises()
    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts/6/dependencies')
  })

  it('offers no Edit to a viewer who may not change it', async () => {
    const wrapper = mount({ canEdit: false })
    await flushPromises()
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })
})
