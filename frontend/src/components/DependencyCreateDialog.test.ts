// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRead, mockErrorUtilsPassthrough } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRead())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())

import { fieldByLabel, renderWithPlugins, setFieldByLabel } from '../test-utils'
import { dependencyHost } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import DependencyCreateDialog from './DependencyCreateDialog.vue'

const REPO_HOSTS = [
  { id: 9, ssh_host: 'nas-01' },
  { id: 12, ssh_host: 'backup-box' },
]

async function render(): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(DependencyCreateDialog, { props: { open: true } })
  await flushPromises()
  return wrapper
}

function button(wrapper: ReturnType<typeof renderWithPlugins>, label: string) {
  const match = wrapper.findAll('button').find((b) => b.text().trim() === label)
  if (!match) throw new Error(`no button "${label}"`)
  return match
}

async function fillIn(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
  await setFieldByLabel(wrapper, 'Name', ' nas-media ')
  await setFieldByLabel(wrapper, 'Address', 'nas-media.lan')
}

describe('DependencyCreateDialog', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: REPO_HOSTS })
  })

  it('offers the repository hosts it can be the same machine as', async () => {
    const wrapper = await render()
    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts')
    const options = fieldByLabel(wrapper, 'Same machine as')
      .findAll('option')
      .map((o) => o.text())
    expect(options).toEqual([
      'Not a repository host',
      'Repository host nas-01',
      'Repository host backup-box',
    ])
  })

  it('sets the port from a preset, and asks for one under Other port', async () => {
    const wrapper = await render()
    expect(wrapper.find('input[type="number"]').exists()).toBe(false)

    await button(wrapper, 'NFS').trigger('click')
    await button(wrapper, 'Other port').trigger('click')
    const port = wrapper.get('input[type="number"]')
    expect((port.element as HTMLInputElement).value).toBe('2049')

    await port.setValue('8443')
    await fillIn(wrapper)
    vi.mocked(apiClient.post).mockResolvedValue({ data: dependencyHost() })
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(vi.mocked(apiClient.post).mock.calls[0]?.[1]).toMatchObject({ port: 8443 })
  })

  it('creates the dependency with no repository host by default', async () => {
    const created = dependencyHost()
    vi.mocked(apiClient.post).mockResolvedValue({ data: created })
    const wrapper = await render()
    await fillIn(wrapper)
    await button(wrapper, 'SSH').trigger('click')

    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts', {
      name: 'nas-media',
      address: 'nas-media.lan',
      port: 22,
      description: '',
      repo_host_id: null,
    })
    expect(wrapper.emitted('created')).toEqual([[created]])
  })

  it('sends the repository host it shares when one is picked', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: dependencyHost() })
    const wrapper = await render()
    await fillIn(wrapper)
    await fieldByLabel(wrapper, 'Same machine as').findAll('option')[2]!.setSelected()
    await setFieldByLabel(wrapper, 'Description', 'Media share')

    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(vi.mocked(apiClient.post).mock.calls[0]?.[1]).toEqual({
      name: 'nas-media',
      address: 'nas-media.lan',
      port: 445,
      description: 'Media share',
      repo_host_id: 12,
    })
  })

  it('shows why creating failed and stays open', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('name already in use'))
    const wrapper = await render()
    await fillIn(wrapper)
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(wrapper.get('.form-error').text()).toContain('name already in use')
    expect(wrapper.emitted('created')).toBeUndefined()
  })

  it('reports a connection test that answered', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { reachable: true, address: 'nas-media.lan', port: 445 },
    })
    const wrapper = await render()
    await fillIn(wrapper)
    await button(wrapper, 'Test connection').trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts/test', {
      address: 'nas-media.lan',
      port: 445,
    })
    expect(wrapper.get('.form-success').text()).toBe('nas-media.lan answered on port 445')
  })

  it('reports a connection test that did not answer, without blocking the create', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { reachable: false, address: 'nas-media.lan', port: 445 },
    })
    const wrapper = await render()
    await fillIn(wrapper)
    await button(wrapper, 'Test connection').trigger('click')
    await flushPromises()

    expect(wrapper.get('.state-warning').text()).toBe(
      'nas-media.lan did not answer on port 445 within 5 seconds. You can still add it.',
    )
    const submit = button(wrapper, 'Create dependency')
    expect(submit.attributes('disabled')).toBeUndefined()

    // A result belongs to the address it was asked for.
    await setFieldByLabel(wrapper, 'Address', 'other.lan')
    expect(wrapper.find('.state-warning').exists()).toBe(false)
  })

  it('shows a connection test that could not run', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('address is invalid'))
    const wrapper = await render()
    await fillIn(wrapper)
    await button(wrapper, 'Test connection').trigger('click')
    await flushPromises()
    expect(wrapper.get('.form-error').text()).toBe('address is invalid')
  })

  it('needs a name and an address before it can create', async () => {
    const wrapper = await render()
    expect(button(wrapper, 'Create dependency').attributes('disabled')).toBeDefined()
    expect(button(wrapper, 'Test connection').attributes('disabled')).toBeDefined()
  })
})
