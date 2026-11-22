// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import AgentInstallCommand from './AgentInstallCommand.vue'
import type { CreateEnrollmentTokenResponse } from '../types/generated'

const { mockCopy, mockKey, mockTags, mockList } = vi.hoisted(() => ({
  mockCopy: vi.fn(),
  mockKey: vi.fn(),
  mockTags: vi.fn(),
  mockList: vi.fn(),
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({ copied: false, copy: mockCopy }),
}))
vi.mock('../api/system', () => ({ getAgentChannelKey: mockKey }))
vi.mock('../api/tags', () => ({ listTags: mockTags }))
vi.mock('../api/enrollment', () => ({
  listEnrollmentTokens: mockList,
  revokeEnrollmentToken: vi.fn(),
  createEnrollmentToken: vi.fn(),
}))

const CREATED: CreateEnrollmentTokenResponse = {
  plaintext: 'enr_secret',
  token: {
    id: 1,
    hint: 'enr_secr',
    created_by: 'admin',
    hostname: 'db-02',
    tag_ids: [],
    status: 'pending',
    created_at: '2026-11-22T10:00:00Z',
    expires_at: new Date(Date.now() + 3_600_000).toISOString(),
    used_by: null,
  },
}

type Wrapper = ReturnType<typeof renderWithPlugins>

async function mountInstall(created: CreateEnrollmentTokenResponse | null = null) {
  const wrapper = renderWithPlugins(AgentInstallCommand, {
    props: {
      created,
      error: null,
      version: 0,
      lifetime: 3600,
      hostname: '',
      tagIds: [],
    },
  })
  await flushPromises()
  return wrapper
}

async function pick(wrapper: Wrapper, label: string): Promise<void> {
  await wrapper
    .findAll('[role="radio"]')
    .find((b) => b.text() === label)
    ?.trigger('click')
}

describe('AgentInstallCommand', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockKey.mockResolvedValue({ public_key: 'SERVERKEY=', fingerprint: 'fp' })
    mockTags.mockResolvedValue([{ id: 3, name: 'production', color: '#f00', scope: 'agent' }])
    mockList.mockResolvedValue({ tokens: [] })
  })

  it('reports the lifetime and hostname that are chosen', async () => {
    const wrapper = await mountInstall()

    await wrapper.get('#enroll-expiry').setValue('900')
    await wrapper.get('#enroll-hostname').setValue('db-02')

    expect(wrapper.emitted('update:lifetime')).toEqual([[900]])
    expect(wrapper.emitted('update:hostname')).toEqual([['db-02']])
  })

  it('asks for the lifetime, hostname and tags before a token exists', async () => {
    const wrapper = await mountInstall()

    expect(wrapper.find('#enroll-expiry').exists()).toBe(true)
    expect(wrapper.find('#enroll-hostname').exists()).toBe(true)
    expect(wrapper.text()).toContain('production')
    expect(wrapper.find('[data-testid="install-command"]').exists()).toBe(false)
    expect(mockTags).toHaveBeenCalledWith('agent')
  })

  it('records the tags that are ticked', async () => {
    const wrapper = await mountInstall()

    await wrapper.get('input[type="checkbox"]').setValue(true)
    expect(wrapper.emitted('update:tagIds')).toEqual([[[3]]])
  })

  it('builds the command from this server, the token and the server key', async () => {
    const wrapper = await mountInstall(CREATED)
    const command = (): string => wrapper.get('[data-testid="install-command"]').text()

    expect(command()).toContain(`${window.location.origin}/install.sh`)
    expect(command()).toContain('--token enr_secret')
    expect(command()).toContain('--server-key SERVERKEY=')

    await pick(wrapper, 'macOS')
    expect(command()).toContain('| sudo sh')
    await pick(wrapper, 'Windows')
    expect(command()).toContain('install.ps1')
    expect(command()).toContain('-Token enr_secret')
  })

  it('shows the time left and who the token is for', async () => {
    const wrapper = await mountInstall(CREATED)

    expect(wrapper.get('[data-testid="install-expiry"]').text()).toMatch(/Expires in \d+:\d{2}/)
    expect(wrapper.text()).toContain('db-02')
  })

  it('says so once the token has run out', async () => {
    const wrapper = await mountInstall({
      ...CREATED,
      token: { ...CREATED.token, expires_at: '2020-01-01T00:00:00Z' },
    })

    expect(wrapper.find('[data-testid="install-expiry"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('Expired')
  })

  it('lists the autostart step for each platform under Install manually', async () => {
    const wrapper = await mountInstall(CREATED)
    const buttons = wrapper.findAll('button')
    await buttons.find((b) => b.text().includes('Install manually'))?.trigger('click')

    expect(wrapper.text()).toContain('systemctl enable --now assimilate-agent')
    await pick(wrapper, 'macOS')
    expect(wrapper.text()).toContain('com.assimilate.agent.plist')
    await pick(wrapper, 'Windows')
    expect(wrapper.text()).toContain('services.msc')
  })

  it('copies the command from its copy button', async () => {
    const wrapper = await mountInstall(CREATED)

    await wrapper.get('.token-box button').trigger('click')

    expect(mockCopy).toHaveBeenCalledWith(expect.stringContaining('--token enr_secret'))
  })

  it('shows why the server key could not be loaded', async () => {
    mockKey.mockRejectedValue(new Error('no key'))
    const wrapper = await mountInstall()

    expect(wrapper.find('.form-error').exists()).toBe(true)
  })

  it('shows the error of a failed create', async () => {
    const wrapper = await mountInstall()
    await wrapper.setProps({ error: 'Hostname must not be blank' })

    expect(wrapper.text()).toContain('Hostname must not be blank')
  })
})
