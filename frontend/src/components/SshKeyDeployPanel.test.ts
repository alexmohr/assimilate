// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SshKeyDeployPanel from './SshKeyDeployPanel.vue'

const { mockPost } = vi.hoisted(() => ({ mockPost: vi.fn() }))

vi.mock('../api/client', () => ({
  apiClient: { post: mockPost },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown): string => 'API error',
  extractBlobError: async (_e: unknown): Promise<string> => 'API error',
}))

function mountPanel(props: Record<string, unknown> = {}): ReturnType<typeof mount> {
  return mount(SshKeyDeployPanel, {
    props: { sshHost: 'host.example.com', ...props },
    attachTo: document.body,
  })
}

function deployBtn(wrapper: ReturnType<typeof mount>): ReturnType<typeof mount.prototype.find> {
  return (
    wrapper.findAll('button').find((b) => b.text().includes('Deploy')) ?? wrapper.find('button')
  )
}

describe('SshKeyDeployPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the hint text', () => {
    const wrapper = mountPanel()
    expect(wrapper.text()).toContain("Deploy the server's SSH public key")
  })

  it('deploy button is disabled when password is empty', () => {
    const wrapper = mountPanel()
    expect(deployBtn(wrapper).attributes('disabled')).toBeDefined()
  })

  it('deploy button is enabled once password is filled', async () => {
    const wrapper = mountPanel()
    await wrapper.find('input[type="password"]').setValue('secret')
    expect(deployBtn(wrapper).attributes('disabled')).toBeUndefined()
  })

  it('calls the deploy-key endpoint with correct params', async () => {
    mockPost.mockResolvedValueOnce({ data: { success: true, already_deployed: false } })
    const wrapper = mountPanel()
    await wrapper.find('input[type="password"]').setValue('secret')
    await deployBtn(wrapper).trigger('click')
    expect(mockPost).toHaveBeenCalledWith(
      '/ssh/deploy-key',
      expect.objectContaining({
        ssh_host: 'host.example.com',
        password: 'secret',
        use_sftp: true,
      }),
    )
  })

  it('shows success message after a successful deploy', async () => {
    mockPost.mockResolvedValueOnce({ data: { success: true, already_deployed: false } })
    const wrapper = mountPanel()
    await wrapper.find('input[type="password"]').setValue('secret')
    await deployBtn(wrapper).trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Key deployed successfully')
  })

  it('shows already-deployed message when key exists', async () => {
    mockPost.mockResolvedValueOnce({ data: { success: false, already_deployed: true } })
    const wrapper = mountPanel()
    await wrapper.find('input[type="password"]').setValue('secret')
    await deployBtn(wrapper).trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Key already deployed')
  })

  it('shows error message on API failure', async () => {
    mockPost.mockRejectedValueOnce(new Error('network'))
    const wrapper = mountPanel()
    await wrapper.find('input[type="password"]').setValue('secret')
    await deployBtn(wrapper).trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('API error')
  })

  it('hides SSH credential fields by default', () => {
    const wrapper = mountPanel()
    const inputs = wrapper.findAll('input')
    expect(inputs).toHaveLength(1)
  })

  it('shows SSH host, user, and port fields when showCredentials is true', () => {
    const wrapper = mountPanel({ showCredentials: true })
    const inputs = wrapper.findAll('input')
    expect(inputs.length).toBeGreaterThan(1)
  })

  it('disables deploy when showCredentials=true and host is blank', async () => {
    const wrapper = mountPanel({ sshHost: '', showCredentials: true })
    await wrapper.find('input[type="password"]').setValue('secret')
    expect(deployBtn(wrapper).attributes('disabled')).toBeDefined()
  })

  it('uses current sshHost prop (not local copy) when showCredentials=false', async () => {
    mockPost.mockResolvedValueOnce({ data: { success: true, already_deployed: false } })
    const wrapper = mountPanel({ sshHost: 'initial.host' })
    await wrapper.setProps({ sshHost: 'updated.host' })
    await wrapper.find('input[type="password"]').setValue('secret')
    await deployBtn(wrapper).trigger('click')
    expect(mockPost).toHaveBeenCalledWith(
      '/ssh/deploy-key',
      expect.objectContaining({ ssh_host: 'updated.host' }),
    )
  })
})
