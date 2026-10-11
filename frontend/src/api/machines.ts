// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  MachineCheckResponse,
  MachineResponse,
  ScheduleMachineInput,
  ScheduleMachinesResponse,
  UpdateMachineAvailabilityRequest,
  UpdateMachinePowerRequest,
} from '../types/generated'

/**
 * A machine: an agent's host, a repository host and any dependencies on it
 * share one, and with it one set of wake, shutdown and availability settings.
 */
export type Machine = MachineResponse

export async function getMachine(id: number): Promise<Machine> {
  const response = await apiClient.get<Machine>(`/machines/${id}`)
  return response.data
}

export async function updateMachinePower(
  id: number,
  data: UpdateMachinePowerRequest,
): Promise<Machine> {
  const response = await apiClient.put<Machine>(`/machines/${id}/power`, data)
  return response.data
}

export async function updateMachineAvailability(
  id: number,
  data: UpdateMachineAvailabilityRequest,
): Promise<Machine> {
  const response = await apiClient.put<Machine>(`/machines/${id}/availability`, data)
  return response.data
}

/** Asks the machine whether it is back, and catches up every run waiting on it if so. */
export async function checkMachine(id: number): Promise<MachineCheckResponse> {
  const response = await apiClient.post<MachineCheckResponse>(`/machines/${id}/check`)
  return response.data
}

/** Every machine a schedule needs, with the schedule's own choice for each. */
export async function getScheduleMachines(scheduleId: number): Promise<ScheduleMachinesResponse> {
  const response = await apiClient.get<ScheduleMachinesResponse>(
    `/schedules/${scheduleId}/machines`,
  )
  return response.data
}

/** Replaces the schedule's per-machine choices; a machine left out follows the schedule. */
export async function updateScheduleMachines(
  scheduleId: number,
  machines: ScheduleMachineInput[],
): Promise<ScheduleMachinesResponse> {
  const response = await apiClient.put<ScheduleMachinesResponse>(
    `/schedules/${scheduleId}/machines`,
    { machines },
  )
  return response.data
}
