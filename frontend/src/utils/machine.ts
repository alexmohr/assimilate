// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'
import type {
  MachineCheckResponse,
  MachineRoleKind,
  MachineScheduleOverrideResponse,
  MachineWakeMode,
  ScheduleMachineResponse,
  ScheduleWakeOverride,
} from '../types/generated'

/** A role as a reader names it. */
export function machineRoleLabel(role: MachineRoleKind): string {
  switch (role) {
    case 'agent':
      return 'agent'
    case 'repo_host':
      return 'repository host'
    case 'dependency':
      return 'dependency'
  }
}

/**
 * The per-machine choice a schedule makes, plus "no choice" - the schedule's
 * own setting applies. `follow` never reaches the server: a machine without a
 * choice is simply left out of the request.
 */
export type MachineChoice = MachineWakeMode | 'follow'

export function choiceOf(mode: MachineWakeMode | null): MachineChoice {
  return mode ?? 'follow'
}

export function modeOf(choice: MachineChoice): MachineWakeMode | null {
  return choice === 'follow' ? null : choice
}

/**
 * Mirrors `ScheduleWakeOverride::for_machine` and `resolve` in
 * `crates/shared/src/types.rs`: an explicit Wake always wakes, Don't wake and
 * Wait never do, and no choice leaves the schedule-wide setting in charge.
 */
export function resolvesToWake(
  wakeOverride: ScheduleWakeOverride,
  mode: MachineWakeMode | null,
  machineWakes: boolean,
): boolean {
  switch (mode) {
    case 'wake':
      return true
    case 'no_wake':
    case 'wait':
      return false
    case null:
      break
  }
  switch (wakeOverride) {
    case 'host_default':
      return machineWakes
    case 'enabled':
      return true
    case 'disabled':
      return false
  }
}

/** What a schedule's choices resolve to for one machine, as a badge and a sentence. */
export interface MachineEffect {
  label: string
  tone: BadgeTone
  reason: string
}

/**
 * Mirrors `MachinePolicy::resolve`: whether the run wakes the machine, and
 * whether it waits for it - because the machine is not always online, or
 * because this schedule chose to wait.
 */
export function scheduleMachineEffect(
  machine: ScheduleMachineResponse,
  wakeOverride: ScheduleWakeOverride,
  mode: MachineWakeMode | null,
): MachineEffect {
  const wakes = resolvesToWake(wakeOverride, mode, machine.power.wake_enabled)
  const waits = machine.intermittent || mode === 'wait'
  if (wakes && !machine.power.has_mac_address) {
    return {
      label: 'Cannot wake',
      tone: 'warning',
      reason: 'No MAC address on this machine, so there is nothing to send a wake packet to.',
    }
  }
  if (wakes) {
    const after = machine.power.shutdown_after_backup
      ? 'Shut down afterwards, because this run is what woke it.'
      : 'Left running afterwards.'
    const missing = waits
      ? 'If it still does not answer, the run is skipped and caught up once it does.'
      : 'If it still does not answer, the run fails.'
    const woken = machine.power.wake_enabled
      ? 'Woken if it does not answer.'
      : 'Woken for this job only - the machine itself does not wake by default.'
    return { label: 'Wakes', tone: 'success', reason: `${woken} ${missing} ${after}` }
  }
  if (waits) {
    return {
      label: 'Waits',
      tone: 'info',
      reason: machine.intermittent
        ? 'Not woken. It is not always online, so a run that finds it down is skipped and caught up once it answers.'
        : 'Not woken. If it is down, its targets run last and are skipped and caught up once it answers.',
    }
  }
  return {
    label: 'Not woken',
    tone: 'neutral',
    reason: 'Not woken. It has to be up already, or the run fails.',
  }
}

/** One schedule's exception to a machine's own setting, in words. */
export function scheduleChoiceText(entry: MachineScheduleOverrideResponse): string {
  switch (entry.mode) {
    case 'wake':
      return 'always wakes it'
    case 'no_wake':
      return 'never wakes it'
    case 'wait':
      return 'waits for it without waking it'
    case null:
      break
  }
  switch (entry.wake_override) {
    case 'enabled':
      return 'wakes every machine it needs'
    case 'disabled':
      return 'never wakes a machine'
    case 'host_default':
      return 'follows this machine'
  }
}

function runs(count: number): string {
  return count === 1 ? '1 run' : `${count} runs`
}

/**
 * What Check now found, because "done" leaves the one question worth
 * answering - is it back? - unanswered.
 */
export function machineCheckText(outcome: MachineCheckResponse): string {
  if (outcome.started > 0) return `The machine is back - catching up ${runs(outcome.started)}`
  if (outcome.abandoned > 0) {
    return `Stopped waiting - ${runs(outcome.abandoned)} past the window, reported as failed`
  }
  if (outcome.reachable) {
    return outcome.dropped > 0
      ? 'The machine is back, but each schedule runs again soon enough on its own'
      : 'The machine is back'
  }
  return 'The machine is still not answering'
}
