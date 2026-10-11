<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import HelpHint from './HelpHint.vue'
import PaneRow from './PaneRow.vue'
import MachineEffectList, { type MachineEffectRow } from './MachineEffectList.vue'
import ScheduleMachinesCard from './ScheduleMachinesCard.vue'
import type { ScheduleWakeOverride } from '../types/generated'
import type { AgentRow } from '../types/agent'
import type { Repo } from '../types/repo'

/**
 * Whether this one schedule wakes the hosts it needs, overriding what those
 * hosts default to.
 *
 * A host holds the details a wake needs - MAC address, broadcast address,
 * timeout, SSH destination - and its own `wake_enabled` flag is the default
 * answer for every schedule that touches it. This section is where a single
 * schedule says otherwise, in either direction. Under the control is what
 * that choice resolves to for the hosts this schedule actually uses, so the
 * answer is on screen rather than reconstructed from two other pages.
 */
const props = defineProps<{
  agents: readonly AgentRow[]
  repos: readonly Repo[]
  selectedAgentIds: number[]
  /**
   * Every repository this schedule writes to, in write order. A multi-target
   * schedule wakes each target's host in turn, so the read-out lists them all
   * rather than only the primary.
   */
  selectedRepoIds: readonly number[]
  /**
   * Whether the viewer is allowed to see wake details. The API redacts a
   * host's MAC address for anyone below operator, so without this the
   * read-out would report "cannot wake" for every host merely because the
   * viewer cannot see the address.
   */
  canSeeWakeDetails: boolean
  /**
   * The saved schedule, whose per-machine choices are saved on their own.
   * Without one, only the schedule-wide setting's effect is shown.
   */
  scheduleId?: number | null
  /** Whether the viewer may change the per-machine choices. */
  canEditMachines?: boolean
}>()

/**
 * Just the one field, rather than the whole schedule form: this section
 * neither reads nor writes anything else on it, and a scalar model is what
 * lets the choice travel back to the form as an event.
 */
const wakeOverride = defineModel<ScheduleWakeOverride>('wakeOverride', { required: true })

const OPTIONS: SegmentedOption<ScheduleWakeOverride>[] = [
  { value: 'host_default', label: 'Host default' },
  { value: 'enabled', label: 'Enabled' },
  { value: 'disabled', label: 'Disabled' },
]

const HINTS: Record<ScheduleWakeOverride, string> = {
  host_default:
    'Each host decides for itself, exactly as it does today. Change this only where this job needs to differ from the rest.',
  enabled:
    'Wake every host this job needs, even one whose own setting is off. A host with no MAC address still cannot be woken.',
  disabled:
    'Never send a wake packet for this job, whatever its hosts are set to. Starting the agent process over SSH is unaffected, and a host that stays down fails the run.',
}

function resolve(hostWakeEnabled: boolean): boolean {
  switch (wakeOverride.value) {
    case 'host_default':
      return hostWakeEnabled
    case 'enabled':
      return true
    case 'disabled':
      return false
  }
}

/**
 * Mirrors `ScheduleWakeOverride::resolve` and the MAC guard in
 * `crates/server/src/power.rs`: waking needs both a resolved yes and
 * somewhere to send the magic packet.
 */
function effectFor(
  key: string,
  hostname: string,
  role: string,
  wakeEnabled: boolean,
  mac: string | null,
  shutdownAfterBackup: boolean,
): MachineEffectRow {
  const row = { key, name: hostname, role }
  if (!resolve(wakeEnabled)) {
    return {
      ...row,
      effect: {
        label: 'Not woken',
        tone: 'neutral',
        reason:
          wakeOverride.value === 'disabled' && wakeEnabled
            ? 'This job overrides the host, which would otherwise be woken. It has to be up already.'
            : 'The host does not wake by default, and this job does not ask it to.',
      },
    }
  }
  if (props.canSeeWakeDetails && mac === null) {
    return {
      ...row,
      effect: {
        label: 'Cannot wake',
        tone: 'warning',
        reason:
          'No MAC address on this host, so there is nothing to send a magic packet to. Add one under its own power settings.',
      },
    }
  }
  const woken = wakeEnabled
    ? 'Woken if it is not already reachable.'
    : 'Woken for this job only - the host itself does not wake by default.'
  return {
    ...row,
    effect: {
      label: 'Wakes',
      tone: 'success',
      reason: shutdownAfterBackup
        ? `${woken} Shut down afterwards, because this run is what woke it.`
        : `${woken} Left running afterwards, since the host has no shutdown configured.`,
    },
  }
}

const effects = computed<MachineEffectRow[]>(() => {
  const rows = props.selectedAgentIds
    .map((id) => props.agents.find((a) => a.id === id))
    .filter((agent): agent is AgentRow => agent !== undefined)
    .map((agent) =>
      effectFor(
        `agent-${agent.id}`,
        agent.hostname,
        'source host',
        agent.power.wake.wake_enabled,
        agent.power.wake.wake_mac_address,
        agent.power.wake.shutdown_after_backup,
      ),
    )
  // Two targets can live on one machine, and waking is per host - so the
  // hosts are deduplicated rather than listed once per repository.
  const seenHosts = new Set<string>()
  for (const repoId of props.selectedRepoIds) {
    const repo = props.repos.find((r) => r.id === repoId)
    if (!repo || seenHosts.has(repo.ssh_host)) continue
    seenHosts.add(repo.ssh_host)
    rows.push(
      effectFor(
        `repo-${repo.id}`,
        repo.ssh_host,
        'repository host',
        repo.power.wake_enabled,
        repo.power.wake_mac_address,
        repo.power.shutdown_after_backup,
      ),
    )
  }
  return rows
})
</script>

<template>
  <div class="pane-head pane-head--end">
    <HelpHint
      label="waking hosts"
      align="end"
    >
      Whether this job wakes the hosts it needs before it runs. Shutting a host down afterwards
      follows from it, since only a host this run woke is ever powered off.
    </HelpHint>
  </div>

  <div class="pane-rows">
    <PaneRow
      title="Wake hosts"
      stack
    >
      <BaseSegmented
        v-model="wakeOverride"
        :options="OPTIONS"
        label="Wake hosts"
      />
      <p class="field-hint">{{ HINTS[wakeOverride] }}</p>
    </PaneRow>
  </div>

  <ScheduleMachinesCard
    v-if="scheduleId != null"
    :schedule-id="scheduleId"
    :wake-override="wakeOverride"
    :can-edit="canEditMachines"
  />
  <section
    v-else
    class="pane-section"
  >
    <span class="group-label group-label--lg">Effect on this job's hosts</span>
    <p
      v-if="effects.length === 0"
      class="field-hint"
    >
      Pick this job's agents and repository first, under Targets.
    </p>
    <MachineEffectList
      v-else
      :rows="effects"
    />
    <p class="field-hint">
      Once the job is saved, each machine can also be set on its own here - including waiting for a
      machine without waking it.
    </p>
  </section>
</template>
