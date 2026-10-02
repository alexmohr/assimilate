// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  checkDependencyHostNow,
  createDependencyHost,
  deleteDependencyHost,
  getAgentDependencies,
  getDependencyHost,
  getDependencyHostAvailability,
  getScheduleDependencies,
  listDependencyHostUsage,
  listDependencyHosts,
  testDependencyAddress,
  testDependencyHost,
  updateAgentDependencies,
  updateDependencyHost,
  updateDependencyHostAvailability,
  updateDependencyHostPower,
  updateScheduleDependencies,
} from './dependencyHosts'
import { dependencyAvailability, dependencyHost } from '../test-utils/dependencyFixtures'

const HOST = dependencyHost()

describe('dependency hosts api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists, reads and removes dependencies', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [HOST] })
    await expect(listDependencyHosts()).resolves.toEqual([HOST])
    expect(apiClient.get).toHaveBeenCalledWith('/dependency-hosts')

    vi.mocked(apiClient.get).mockResolvedValue({ data: HOST })
    await expect(getDependencyHost(3)).resolves.toEqual(HOST)
    expect(apiClient.get).toHaveBeenCalledWith('/dependency-hosts/3')

    vi.mocked(apiClient.delete).mockResolvedValue({})
    await deleteDependencyHost(3)
    expect(apiClient.delete).toHaveBeenCalledWith('/dependency-hosts/3')
  })

  it('creates a dependency with the repository host it shares', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: HOST })
    const body = {
      name: 'nas-media',
      address: 'nas-media.lan',
      port: 445,
      description: '',
      repo_host_id: 9,
    }
    await expect(createDependencyHost(body)).resolves.toEqual(HOST)
    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts', body)
  })

  it('updates the connection, the power settings and the availability', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: HOST })
    const connection = { name: 'n', address: 'a', port: 2049, description: 'd' }
    await updateDependencyHost(3, connection)
    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3', connection)

    const power = {
      repo_host_id: null,
      wake_enabled: true,
      wake_mac_address: '9C:B6:D0:1A:44:7F',
      wake_broadcast_address: null,
      wake_timeout_seconds: 120,
    }
    await updateDependencyHostPower(3, power)
    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/power', power)

    const availability = {
      intermittent: true,
      catch_up_recheck_minutes: 30,
      catch_up_give_up_minutes: 0,
    }
    vi.mocked(apiClient.put).mockResolvedValue({ data: dependencyAvailability() })
    await updateDependencyHostAvailability(3, availability)
    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/availability', availability)
  })

  it('reads the availability and what uses a dependency', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: dependencyAvailability() })
    await expect(getDependencyHostAvailability(3)).resolves.toEqual(dependencyAvailability())
    expect(apiClient.get).toHaveBeenCalledWith('/dependency-hosts/3/availability')

    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })
    await listDependencyHostUsage(3)
    expect(apiClient.get).toHaveBeenCalledWith('/dependency-hosts/3/usage')
  })

  it('checks a saved dependency, an unsaved address, and what waits on one', async () => {
    const answer = { reachable: true, address: 'nas-media.lan', port: 445 }
    vi.mocked(apiClient.post).mockResolvedValue({ data: answer })

    await expect(testDependencyHost(3)).resolves.toEqual(answer)
    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts/3/test')

    await testDependencyAddress('nas-media.lan', 445)
    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts/test', {
      address: 'nas-media.lan',
      port: 445,
    })

    const outcome = { probed: 1, reachable: 1, started: 2, abandoned: 0, dropped: 0 }
    vi.mocked(apiClient.post).mockResolvedValue({ data: outcome })
    await expect(checkDependencyHostNow(3)).resolves.toEqual(outcome)
    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts/3/availability/check')
  })

  it('reads and replaces what a schedule needs', async () => {
    const body = { dependencies: [] }
    vi.mocked(apiClient.get).mockResolvedValue({ data: body })
    await getScheduleDependencies(12)
    expect(apiClient.get).toHaveBeenCalledWith('/schedules/12/dependencies')

    vi.mocked(apiClient.put).mockResolvedValue({ data: body })
    const input = [{ agent_id: 7, dependency_host_id: 3 }]
    await updateScheduleDependencies(12, input)
    expect(apiClient.put).toHaveBeenCalledWith('/schedules/12/dependencies', {
      dependencies: input,
    })
  })

  it("passes an agent's domain along with its defaults, and leaves it out when there is none", async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { dependency_host_ids: [3] } })
    await expect(getAgentDependencies('web 01', 'lab.example')).resolves.toEqual([3])
    expect(apiClient.get).toHaveBeenCalledWith('/agents/web%2001/dependencies', {
      params: { domain: 'lab.example' },
    })

    vi.mocked(apiClient.put).mockResolvedValue({ data: { dependency_host_ids: [3, 4] } })
    await expect(updateAgentDependencies('web-01', [3, 4])).resolves.toEqual([3, 4])
    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01/dependencies',
      { dependency_host_ids: [3, 4] },
      { params: {} },
    )
  })
})
