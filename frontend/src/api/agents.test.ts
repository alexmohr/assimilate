// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  createAgent,
  createAgentHostnamePattern,
  deleteAgent,
  deleteAgentArchives,
  deleteAgentHostnamePattern,
  deployAgent,
  deployAgentSshKey,
  hideAgent,
  listAgentHostnamePatterns,
  listAgentReports,
  listAgentRepos,
  listAgents,
  mergeAgent,
  previewAgentServiceUnit,
  regenerateAgentToken,
  restartAgent,
  unhideAgent,
  updateAgent,
} from './agents'

describe('agents api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists agents without params', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listAgents()

    expect(apiClient.get).toHaveBeenCalledWith('/agents', { params: undefined })
  })

  it('lists agents including hidden', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listAgents(true)

    expect(apiClient.get).toHaveBeenCalledWith('/agents', { params: { include_hidden: true } })
  })

  it('creates an agent', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { agent: {}, token: 'tok' } })

    await createAgent({ hostname: 'web-01', display_name: 'Web 01' })

    expect(apiClient.post).toHaveBeenCalledWith('/agents', {
      hostname: 'web-01',
      display_name: 'Web 01',
    })
  })

  it('updates an agent', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: {} })

    await updateAgent('web-01', { display_name: 'New name' })

    expect(apiClient.put).toHaveBeenCalledWith('/agents/web-01', { display_name: 'New name' })
  })

  it('deletes an agent', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteAgent('web-01')

    expect(apiClient.delete).toHaveBeenCalledWith('/agents/web-01')
  })

  it('hides an agent', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await hideAgent('legacy-01')

    expect(apiClient.put).toHaveBeenCalledWith('/agents/legacy-01/hide')
  })

  it('unhides an agent', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await unhideAgent('legacy-01')

    expect(apiClient.put).toHaveBeenCalledWith('/agents/legacy-01/unhide')
  })

  it('regenerates an agent token', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { agent: {}, token: 'tok' } })

    await regenerateAgentToken('web-01')

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-01/regenerate-token')
  })

  it('restarts an agent', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await restartAgent('web-01')

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-01/restart')
  })

  it('deletes agent archives', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await deleteAgentArchives('legacy-01')

    expect(apiClient.post).toHaveBeenCalledWith('/agents/legacy-01/delete-archives')
  })

  it('lists hostname patterns', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listAgentHostnamePatterns('web-01')

    expect(apiClient.get).toHaveBeenCalledWith('/agents/web-01/hostname-patterns')
  })

  it('creates a hostname pattern', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await createAgentHostnamePattern('web-01', 'web*')

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-01/hostname-patterns', {
      pattern: 'web*',
    })
  })

  it('deletes a hostname pattern', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteAgentHostnamePattern('web-01', 1)

    expect(apiClient.delete).toHaveBeenCalledWith('/agents/web-01/hostname-patterns/1')
  })

  it('merges an agent with a pattern', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { merged: true } })

    await mergeAgent('web-server-01', 10, 'legacy*')

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-server-01/merge-from/10', {
      create_pattern: 'legacy*',
    })
  })

  it('merges an agent without a pattern', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { merged: true } })

    await mergeAgent('web-server-01', 10)

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-server-01/merge-from/10', {})
  })

  it('previews the service unit', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { content: null } })

    await previewAgentServiceUnit('web-01', {
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
      ssh_password: 'secret',
    })

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-01/service-unit', {
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
      ssh_password: 'secret',
    })
  })

  it('deploys an agent', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { success: true, skipped: false, token: 'tok' },
    })

    await deployAgent('web-01', {
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
      ssh_password: 'secret',
      server_url: 'http://server:8080',
      install_path: '/usr/local/bin/assimilate-agent',
      systemd_service_content: '[Unit]',
    })

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-01/deploy', {
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
      ssh_password: 'secret',
      server_url: 'http://server:8080',
      install_path: '/usr/local/bin/assimilate-agent',
      systemd_service_content: '[Unit]',
    })
  })

  it('deploys the ssh key', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { success: true, already_deployed: false },
    })

    await deployAgentSshKey({
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
      password: 'secret',
      use_sftp: true,
    })

    expect(apiClient.post).toHaveBeenCalledWith('/ssh/deploy-key', {
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
      password: 'secret',
      use_sftp: true,
    })
  })

  it('lists agent repos', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listAgentRepos('web-01')

    expect(apiClient.get).toHaveBeenCalledWith('/agents/web-01/repos')
  })

  it('lists agent reports with and without params', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listAgentReports('web-01')
    await listAgentReports('web-01', { limit: 100, target: 'home' })

    expect(apiClient.get).toHaveBeenNthCalledWith(1, '/agents/web-01/reports', {
      params: undefined,
    })
    expect(apiClient.get).toHaveBeenNthCalledWith(2, '/agents/web-01/reports', {
      params: { limit: 100, target: 'home' },
    })
  })
})
