// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  MachineResponse,
  MachineWaitResponse,
  ScheduleMachineResponse,
} from '../types/generated'

/** A repository host's machine that wakes and shuts down, and is always online. */
export function machine(overrides: Partial<MachineResponse> = {}): MachineResponse {
  return {
    id: 20,
    label: 'nas.lan',
    roles: { agent: null, repo_host: { id: 5, ssh_host: 'nas.lan' }, dependencies: [] },
    power: {
      wake_enabled: true,
      wake_mac_address: '9C:B6:D0:1A:44:7F',
      wake_broadcast_address: null,
      wake_timeout_seconds: 240,
      shutdown_after_backup: true,
    },
    can_shut_down: true,
    availability: {
      intermittent: false,
      catch_up_recheck_minutes: 15,
      catch_up_give_up_minutes: 0,
    },
    last_checked_at: null,
    last_check_reachable: null,
    waiting: [],
    schedule_overrides: [],
    ...overrides,
  }
}

/** A run waiting on a repository on the machine. */
export function machineWait(overrides: Partial<MachineWaitResponse> = {}): MachineWaitResponse {
  return {
    role: 'repo_host',
    schedule_id: 7,
    schedule_name: 'Nightly servers',
    hostname: null,
    repo_id: 12,
    repo_name: 'server-daily',
    dependency_name: null,
    pending_for: '2026-09-26T02:00:00Z',
    last_probe_at: null,
    next_probe_at: '2026-09-26T02:30:00Z',
    give_up_at: null,
    ...overrides,
  }
}

/** A repository host a schedule writes to, following the schedule's own setting. */
export function scheduleMachine(
  overrides: Partial<ScheduleMachineResponse> = {},
): ScheduleMachineResponse {
  return {
    machine_id: 20,
    label: 'nas.lan',
    roles: ['repo_host'],
    power: { wake_enabled: true, has_mac_address: true, shutdown_after_backup: false },
    intermittent: false,
    mode: null,
    can_wait: true,
    ...overrides,
  }
}
