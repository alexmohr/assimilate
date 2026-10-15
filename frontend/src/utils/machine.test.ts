// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  choiceOf,
  machineCheckText,
  machineRoleLabel,
  modeOf,
  resolvesToWake,
  scheduleLabel,
  scheduleChoiceText,
  scheduleMachineEffect,
} from './machine'
import { scheduleMachine } from '../test-utils/machineFixtures'

describe('machine', () => {
  it('names each role', () => {
    expect(machineRoleLabel('agent')).toBe('agent')
    expect(machineRoleLabel('repo_host')).toBe('repository host')
    expect(machineRoleLabel('dependency')).toBe('dependency')
  })

  it('maps no choice to following the schedule, and back', () => {
    expect(choiceOf(null)).toBe('follow')
    expect(choiceOf('wait')).toBe('wait')
    expect(modeOf('follow')).toBeNull()
    expect(modeOf('no_wake')).toBe('no_wake')
  })

  // Mirrors ScheduleWakeOverride::for_machine and resolve.
  it.each([
    ['host_default', null, true, true],
    ['host_default', null, false, false],
    ['enabled', null, false, true],
    ['disabled', null, true, false],
    ['disabled', 'wake', false, true],
    ['enabled', 'no_wake', true, false],
    ['enabled', 'wait', true, false],
  ] as const)('resolves %s with choice %s on a machine waking %s to %s', (o, mode, own, wakes) => {
    expect(resolvesToWake(o, mode, own)).toBe(wakes)
  })

  it('says a machine woken without a MAC address cannot be woken', () => {
    const machine = scheduleMachine({
      power: { wake_enabled: true, has_mac_address: false, shutdown_after_backup: false },
    })
    expect(scheduleMachineEffect(machine, 'host_default', null).label).toBe('Cannot wake')
  })

  it('says what happens after a wake, and if it still does not answer', () => {
    const plain = scheduleMachine({
      power: { wake_enabled: true, has_mac_address: true, shutdown_after_backup: true },
    })
    const woken = scheduleMachineEffect(plain, 'host_default', null)
    expect(woken.label).toBe('Wakes')
    expect(woken.reason).toContain('the run fails')
    expect(woken.reason).toContain('Shut down afterwards')

    const sleeping = scheduleMachineEffect({ ...plain, intermittent: true }, 'host_default', null)
    expect(sleeping.reason).toContain('skipped and caught up')
  })

  it('says a machine is woken for this job only where it would not wake on its own', () => {
    const machine = scheduleMachine({
      power: { wake_enabled: false, has_mac_address: true, shutdown_after_backup: false },
    })
    expect(scheduleMachineEffect(machine, 'enabled', null).reason).toContain(
      'Woken for this job only',
    )
    expect(scheduleMachineEffect(machine, 'host_default', 'wake').reason).toContain(
      'Woken for this job only',
    )
  })

  it('waits for a machine the schedule chose to wait for, without waking it', () => {
    const effect = scheduleMachineEffect(scheduleMachine(), 'enabled', 'wait')
    expect(effect.label).toBe('Waits')
    expect(effect.tone).toBe('info')
    expect(effect.reason).toContain('run last')
  })

  it('waits for a machine that is not always online, even unwoken', () => {
    const effect = scheduleMachineEffect(scheduleMachine({ intermittent: true }), 'disabled', null)
    expect(effect.label).toBe('Waits')
    expect(effect.reason).toContain('It is not always online, so this job waits for it')
  })

  // Either alone would wait; the job's own choice still has to show.
  it("names the job's own wait on a machine that is also not always online", () => {
    const effect = scheduleMachineEffect(scheduleMachine({ intermittent: true }), 'enabled', 'wait')
    expect(effect.label).toBe('Waits')
    expect(effect.reason).toContain(
      'This job waits for it, and it is also marked as not always online',
    )
  })

  // Only a repository host's targets are moved to the end of the run.
  it('says what a run that finds it down does, by what the machine is to the job', () => {
    const host = scheduleMachineEffect(scheduleMachine(), 'host_default', 'wait')
    expect(host.reason).toContain('This job waits for it. If it is down, its targets run last')

    const dependency = scheduleMachine({ roles: ['dependency'] })
    expect(scheduleMachineEffect(dependency, 'host_default', 'wait').reason).toContain(
      'If it is down, the run is skipped and caught up once it answers',
    )
  })

  it('fails a run whose unwoken machine is not up', () => {
    const effect = scheduleMachineEffect(scheduleMachine(), 'host_default', 'no_wake')
    expect(effect.label).toBe('Not woken')
    expect(effect.reason).toContain('the run fails')
  })

  it.each([
    ['wake', 'host_default', 'always wakes it'],
    ['no_wake', 'host_default', 'never wakes it'],
    ['wait', 'enabled', 'waits for it without waking it'],
    [null, 'enabled', 'wakes every machine it needs'],
    [null, 'disabled', 'never wakes a machine'],
    [null, 'host_default', 'follows this machine'],
  ] as const)('describes a schedule choosing %s under %s', (mode, wakeOverride, text) => {
    expect(
      scheduleChoiceText({ schedule_id: 1, schedule_name: 'n', wake_override: wakeOverride, mode }),
    ).toBe(text)
  })

  it.each([
    [
      { reachable: true, started: 1, abandoned: 0, dropped: 0 },
      'The machine is back - catching up 1 run',
    ],
    [
      { reachable: true, started: 3, abandoned: 0, dropped: 0 },
      'The machine is back - catching up 3 runs',
    ],
    [
      { reachable: false, started: 0, abandoned: 2, dropped: 0 },
      'Stopped waiting - 2 runs past the window, reported as failed',
    ],
    [
      { reachable: true, started: 0, abandoned: 0, dropped: 1 },
      'The machine is back, but each schedule runs again soon enough on its own',
    ],
    [{ reachable: true, started: 0, abandoned: 0, dropped: 0 }, 'The machine is back'],
    [
      { reachable: false, started: 0, abandoned: 0, dropped: 0 },
      'The machine is still not answering',
    ],
  ])('says what Check now found', (outcome, text) => {
    expect(machineCheckText(outcome)).toBe(text)
  })

  // An unnamed schedule would otherwise be an empty link.
  it('names a schedule without a name by its id', () => {
    expect(scheduleLabel({ schedule_id: 7, schedule_name: 'Nightly' })).toBe('Nightly')
    expect(scheduleLabel({ schedule_id: 7, schedule_name: '' })).toBe('Schedule #7')
  })
})
