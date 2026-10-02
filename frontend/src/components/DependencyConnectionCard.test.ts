// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockErrorUtilsPassthrough } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())

import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
import { dependencyHost, dependencyPower, infoGridRows } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import DependencyConnectionCard from './DependencyConnectionCard.vue'
import type { DependencyHostResponse } from '../types/generated'

async function render(
  host: DependencyHostResponse = dependencyHost(),
  canEdit = true,
): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(DependencyConnectionCard, { props: { host, canEdit } })
  await flushPromises()
  return wrapper
}

describe('DependencyConnectionCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
  })

  it('says where the dependency is checked and on which port', async () => {
    const wrapper = await render()
    expect(infoGridRows(wrapper)).toEqual({
      Name: 'nas-media',
      Address: 'nas-media.lan',
      Check: 'SMB, TCP port 445',
      'Checked from': 'The Assimilate server',
      Description: 'Media share mounted on media-store-01',
    })
    expect(wrapper.text()).toContain('An open port means the machine is up')
  })

  it('names the repository host it is the same machine as, and a bare TCP port', async () => {
    const wrapper = await render(
      dependencyHost({
        port: 8443,
        description: '',
        power: dependencyPower({ repo_host: { id: 9, ssh_host: 'nas-01' } }),
      }),
    )
    const rows = infoGridRows(wrapper)
    expect(rows.Check).toBe('TCP port 8443')
    expect(rows['Same machine as']).toBe('Repository host nas-01')
    expect(rows.Description).toBe('None')
    expect(wrapper.get('a[href="/repo-hosts/9"]').text()).toBe('nas-01')
  })

  it('offers no Edit to a viewer who may not change it', async () => {
    const wrapper = await render(dependencyHost(), false)
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('saves the edited connection and hands the result back', async () => {
    const updated = dependencyHost({ name: 'files-01', port: 2049 })
    vi.mocked(apiClient.put).mockResolvedValue({ data: updated })
    const wrapper = await render()
    await startEditingSection(wrapper)

    await wrapper.get('#dependency-edit-name').setValue(' files-01 ')
    await wrapper.get('#dependency-edit-address').setValue('10.0.20.14')
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'NFS')!
      .trigger('click')
    await wrapper.get('#dependency-edit-description').setValue('')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3', {
      name: 'files-01',
      address: '10.0.20.14',
      port: 2049,
      description: '',
    })
    expect(wrapper.emitted('saved')).toEqual([[updated]])
  })

  it('starts the port picker on Other port for a port no preset has', async () => {
    const wrapper = await render(dependencyHost({ port: 8443 }))
    await startEditingSection(wrapper)
    const port = wrapper.get('#dependency-edit-port')
    expect((port.element as HTMLInputElement).value).toBe('8443')
  })

  it('keeps editing and shows why a save failed', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('address is invalid'))
    const wrapper = await render()
    await expectSaveErrorKeepsEditing(wrapper, 'address is invalid', '#dependency-edit-address')
  })
})
