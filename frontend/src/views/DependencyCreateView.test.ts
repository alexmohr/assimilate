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
import { routes } from '../router'
import DependencyCreateView from './DependencyCreateView.vue'

type Wrapper = ReturnType<typeof renderWithPlugins>

const REPO_HOSTS = [
  { id: 9, ssh_host: 'nas-01' },
  { id: 12, ssh_host: 'backup-box' },
]

async function render(): Promise<Wrapper> {
  const wrapper = renderWithPlugins(DependencyCreateView, { routeOverrides: '/dependencies/new' })
  await wrapper.vm.$router.push('/dependencies/new')
  await flushPromises()
  return wrapper
}

function button(wrapper: Wrapper, label: string) {
  const match = wrapper.findAll('button').find((b) => b.text().trim() === label)
  if (!match) throw new Error(`no button "${label}"`)
  return match
}

/** Jumps straight to a step from the wizard's rail. */
async function step(wrapper: Wrapper, label: string): Promise<void> {
  const item = wrapper
    .findAll('.wizard-step')
    .find((s) => s.get('.wizard-step-text > span').text() === label)
  if (!item) throw new Error(`no step "${label}"`)
  await item.trigger('click')
  await flushPromises()
}

async function fillIn(wrapper: Wrapper): Promise<void> {
  await setFieldByLabel(wrapper, 'Name', ' nas-media ')
  await setFieldByLabel(wrapper, 'Address', 'nas-media.lan')
}

/** Fills in the host step, then walks to the review step with Continue. */
async function fillInAndReview(wrapper: Wrapper): Promise<void> {
  await fillIn(wrapper)
  for (let i = 0; i < 3; i++) {
    await button(wrapper, 'Continue').trigger('click')
    await flushPromises()
  }
}

async function create(wrapper: Wrapper): Promise<void> {
  await button(wrapper, 'Create dependency').trigger('click')
  await flushPromises()
}

function path(wrapper: Wrapper): string {
  return wrapper.vm.$router.currentRoute.value.path
}

describe('DependencyCreateView', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: REPO_HOSTS })
  })

  it('is routed at /dependencies/new for admins only', () => {
    const route = routes.find((r) => r.path === '/dependencies/new')
    expect(route?.name).toBe('dependency-create')
    expect(route?.meta?.requiresAdmin).toBe(true)
  })

  it('walks through host, check, machine and review, with a crumb back to the list', async () => {
    const wrapper = await render()
    expect(wrapper.get('h1').text()).toBe('New Dependency')
    expect(wrapper.get('.detail-breadcrumb a').attributes('href')).toBe('/dependencies')
    expect(
      wrapper.findAll('.wizard-step').map((s) => s.get('.wizard-step-text > span').text()),
    ).toEqual(['Host', 'Check', 'Machine', 'Review'])
    expect(wrapper.find('#dependency-name').exists()).toBe(true)
  })

  it('offers the repository hosts it can be the same machine as', async () => {
    const wrapper = await render()
    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts')
    await step(wrapper, 'Machine')
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
    await fillIn(wrapper)
    await step(wrapper, 'Check')
    expect(wrapper.find('input[type="number"]').exists()).toBe(false)

    await button(wrapper, 'NFS').trigger('click')
    await button(wrapper, 'Other port').trigger('click')
    const port = wrapper.get('input[type="number"]')
    expect((port.element as HTMLInputElement).value).toBe('2049')

    await port.setValue('8443')
    vi.mocked(apiClient.post).mockResolvedValue({ data: dependencyHost() })
    await step(wrapper, 'Review')
    expect(wrapper.text()).toContain('TCP · port 8443')
    await create(wrapper)
    expect(vi.mocked(apiClient.post).mock.calls[0]?.[1]).toMatchObject({ port: 8443 })
  })

  it('creates the dependency with no repository host by default, then opens it', async () => {
    const created = dependencyHost()
    vi.mocked(apiClient.post).mockResolvedValue({ data: created })
    const wrapper = await render()
    await fillIn(wrapper)
    await step(wrapper, 'Check')
    await button(wrapper, 'SSH').trigger('click')
    await step(wrapper, 'Review')

    expect(wrapper.text()).toContain('Not a repository host')
    expect(wrapper.text()).toContain('Not tested')
    await create(wrapper)

    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts', {
      name: 'nas-media',
      address: 'nas-media.lan',
      port: 22,
      description: '',
      repo_host_id: null,
    })
    expect(path(wrapper)).toBe('/dependency-hosts/3')
  })

  it('sends the repository host it shares when one is picked', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: dependencyHost() })
    const wrapper = await render()
    await fillIn(wrapper)
    await setFieldByLabel(wrapper, 'Description', 'Media share')
    await step(wrapper, 'Machine')
    await fieldByLabel(wrapper, 'Same machine as').findAll('option')[2]!.setSelected()
    await step(wrapper, 'Review')

    expect(wrapper.text()).toContain('Repository host backup-box')
    expect(wrapper.text()).toContain('Media share')
    await create(wrapper)

    expect(vi.mocked(apiClient.post).mock.calls[0]?.[1]).toEqual({
      name: 'nas-media',
      address: 'nas-media.lan',
      port: 445,
      description: 'Media share',
      repo_host_id: 12,
    })
  })

  it('shows why creating failed and stays on the page', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('name already in use'))
    const wrapper = await render()
    await fillInAndReview(wrapper)
    await create(wrapper)

    expect(wrapper.get('.form-error').text()).toContain('name already in use')
    expect(path(wrapper)).toBe('/dependencies/new')
    // It can be tried again.
    expect(button(wrapper, 'Create dependency').attributes('disabled')).toBeUndefined()
  })

  it('reports a connection test that answered', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { reachable: true, address: 'nas-media.lan', port: 445, timeout_seconds: 5 },
    })
    const wrapper = await render()
    await fillIn(wrapper)
    await step(wrapper, 'Check')
    await button(wrapper, 'Test connection').trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts/test', {
      address: 'nas-media.lan',
      port: 445,
    })
    expect(wrapper.get('.form-success').text()).toBe('nas-media.lan answered on port 445')

    // The review repeats what the last test found.
    await step(wrapper, 'Review')
    expect(wrapper.text()).toContain('nas-media.lan answered on port 445')
  })

  it('reports a connection test that did not answer, without blocking the create', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { reachable: false, address: 'nas-media.lan', port: 445, timeout_seconds: 5 },
    })
    const wrapper = await render()
    await fillIn(wrapper)
    await step(wrapper, 'Check')
    await button(wrapper, 'Test connection').trigger('click')
    await flushPromises()

    expect(wrapper.get('.state-warning').text()).toBe(
      'nas-media.lan did not answer on port 445 within 5 seconds. You can still add it.',
    )
    expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
    await step(wrapper, 'Review')
    expect(button(wrapper, 'Create dependency').attributes('disabled')).toBeUndefined()

    // A result belongs to the address it was asked for.
    await step(wrapper, 'Host')
    await setFieldByLabel(wrapper, 'Address', 'other.lan')
    await step(wrapper, 'Check')
    expect(wrapper.find('.state-warning').exists()).toBe(false)
  })

  it('forgets a test result once the port changes', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { reachable: true, address: 'nas-media.lan', port: 445, timeout_seconds: 5 },
    })
    const wrapper = await render()
    await fillIn(wrapper)
    await step(wrapper, 'Check')
    await button(wrapper, 'Test connection').trigger('click')
    await flushPromises()
    expect(wrapper.find('.form-success').exists()).toBe(true)

    await button(wrapper, 'NFS').trigger('click')
    expect(wrapper.find('.form-success').exists()).toBe(false)
  })

  it('shows a connection test that could not run', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('address is invalid'))
    const wrapper = await render()
    await fillIn(wrapper)
    await step(wrapper, 'Check')
    await button(wrapper, 'Test connection').trigger('click')
    await flushPromises()
    expect(wrapper.get('.form-error').text()).toBe('address is invalid')
  })

  it('needs a name and an address before it can create', async () => {
    const wrapper = await render()
    expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
    expect(wrapper.get('.wizard-status').text()).toBe('Needs a name and an address to continue.')

    await step(wrapper, 'Check')
    expect(button(wrapper, 'Test connection').attributes('disabled')).toBeDefined()

    await step(wrapper, 'Review')
    expect(button(wrapper, 'Create dependency').attributes('disabled')).toBeDefined()
    expect(wrapper.get('.wizard-status').text()).toBe(
      'Still needs a name (Host), an address (Host).',
    )
    await create(wrapper)
    expect(apiClient.post).not.toHaveBeenCalled()
  })

  it('takes a name alone as not enough: a blank address still blocks', async () => {
    const wrapper = await render()
    await setFieldByLabel(wrapper, 'Name', 'nas-media')
    await setFieldByLabel(wrapper, 'Address', '   ')
    expect(wrapper.get('.wizard-status').text()).toBe('Needs an address to continue.')
    expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
  })

  it('still offers "Not a repository host" when the repository hosts cannot be listed', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('forbidden'))
    const wrapper = await render()
    await step(wrapper, 'Machine')
    const options = fieldByLabel(wrapper, 'Same machine as')
      .findAll('option')
      .map((o) => o.text())
    expect(options).toEqual(['Not a repository host'])

    vi.mocked(apiClient.post).mockResolvedValue({ data: dependencyHost() })
    await step(wrapper, 'Host')
    await fillInAndReview(wrapper)
    await create(wrapper)
    expect(vi.mocked(apiClient.post).mock.calls[0]?.[1]).toMatchObject({ repo_host_id: null })
  })

  it('jumps back to a step from its review block', async () => {
    const wrapper = await render()
    await fillInAndReview(wrapper)
    expect(wrapper.find('.review-grid').exists()).toBe(true)

    const edits = wrapper.findAll('.review-head button')
    expect(wrapper.findAll('.review-head .group-label').map((l) => l.text())).toEqual([
      'Host',
      'Check',
      'Machine',
    ])
    await edits[1]!.trigger('click')
    await flushPromises()
    expect(wrapper.find('.dependency-test').exists()).toBe(true)
    expect(wrapper.find('.wizard-step--current').text()).toContain('Check')
  })

  it.each([
    ['Host', 0, '#dependency-name'],
    ['Machine', 2, 'select'],
  ])('jumps back to the %s step from its review block Edit button', async (label, index, field) => {
    const wrapper = await render()
    await fillInAndReview(wrapper)

    await wrapper.findAll('.review-head button')[index]!.trigger('click')
    await flushPromises()
    expect(wrapper.find('.review-grid').exists()).toBe(false)
    expect(wrapper.find('.wizard-step--current').text()).toContain(label)
    expect(wrapper.find(field).exists()).toBe(true)
  })

  it('goes back to the list from Cancel without creating anything', async () => {
    const wrapper = await render()
    await fillIn(wrapper)
    await button(wrapper, 'Cancel').trigger('click')
    await flushPromises()

    expect(path(wrapper)).toBe('/dependencies')
    expect(apiClient.post).not.toHaveBeenCalled()
  })
})
