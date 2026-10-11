// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  checkMachine,
  getMachine,
  getScheduleMachines,
  updateMachineAvailability,
  updateMachinePower,
  updateScheduleMachines,
} from './machines'
import { machine } from '../test-utils/machineFixtures'

const MACHINE = machine()

describe('machines api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
  })

  it('reads a machine', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: MACHINE })
    await expect(getMachine(20)).resolves.toEqual(MACHINE)
    expect(apiClient.get).toHaveBeenCalledWith('/machines/20')
  })

  it('saves how it is woken and whether it is always online', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: MACHINE })
    await expect(updateMachinePower(20, MACHINE.power)).resolves.toEqual(MACHINE)
    expect(apiClient.put).toHaveBeenCalledWith('/machines/20/power', MACHINE.power)

    await updateMachineAvailability(20, MACHINE.availability)
    expect(apiClient.put).toHaveBeenCalledWith('/machines/20/availability', MACHINE.availability)
  })

  it('asks it whether it is back', async () => {
    const outcome = { reachable: true, started: 1, abandoned: 0, dropped: 0 }
    vi.mocked(apiClient.post).mockResolvedValue({ data: outcome })
    await expect(checkMachine(20)).resolves.toEqual(outcome)
    expect(apiClient.post).toHaveBeenCalledWith('/machines/20/check')
  })

  it("reads and replaces a schedule's per-machine choices", async () => {
    const body = { machines: [] }
    vi.mocked(apiClient.get).mockResolvedValue({ data: body })
    await expect(getScheduleMachines(4)).resolves.toEqual(body)
    expect(apiClient.get).toHaveBeenCalledWith('/schedules/4/machines')

    vi.mocked(apiClient.put).mockResolvedValue({ data: body })
    const choices = [{ machine_id: 20, mode: 'wait' as const }]
    await updateScheduleMachines(4, choices)
    expect(apiClient.put).toHaveBeenCalledWith('/schedules/4/machines', { machines: choices })
  })
})
