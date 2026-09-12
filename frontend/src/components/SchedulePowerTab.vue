<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import HelpHint from './HelpHint.vue'
import { badgeClass, type BadgeTone } from '../utils/badge'
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

/** What the chosen override resolves to for one host. */
interface HostEffect {
  key: string
  hostname: string
  role: string
  label: string
  tone: BadgeTone
  reason: string
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
): HostEffect {
  if (!resolve(wakeEnabled)) {
    return {
      key,
      hostname,
      role,
      label: 'Not woken',
      tone: 'neutral',
      reason:
        wakeOverride.value === 'disabled' && wakeEnabled
          ? 'This job overrides the host, which would otherwise be woken. It has to be up already.'
          : 'The host does not wake by default, and this job does not ask it to.',
    }
  }
  if (props.canSeeWakeDetails && mac === null) {
    return {
      key,
      hostname,
      role,
      label: 'Cannot wake',
      tone: 'warning',
      reason:
        'No MAC address on this host, so there is nothing to send a magic packet to. Add one under its own power settings.',
    }
  }
  const woken = wakeEnabled
    ? 'Woken if it is not already reachable.'
    : 'Woken for this job only - the host itself does not wake by default.'
  return {
    key,
    hostname,
    role,
    label: 'Wakes',
    tone: 'success',
    reason: shutdownAfterBackup
      ? `${woken} Shut down afterwards, because this run is what woke it.`
      : `${woken} Left running afterwards, since the host has no shutdown configured.`,
  }
}

const effects = computed<HostEffect[]>(() => {
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

  <div class="field">
    <label class="field-label">Wake hosts</label>
    <BaseSegmented
      v-model="wakeOverride"
      :options="OPTIONS"
      label="Wake hosts"
    />
    <p class="field-hint">{{ HINTS[wakeOverride] }}</p>
  </div>

  <section class="pane-section">
    <span class="group-label group-label--lg">Effect on this job's hosts</span>
    <p
      v-if="effects.length === 0"
      class="field-hint"
    >
      Pick this job's agents and repository first, under Targets.
    </p>
    <dl
      v-else
      class="info-grid"
    >
      <template
        v-for="effect in effects"
        :key="effect.key"
      >
        <dt>
          <span class="mono">{{ effect.hostname }}</span>
          <span class="host-role muted">{{ effect.role }}</span>
        </dt>
        <dd class="host-effect">
          <span
            class="badge"
            :class="badgeClass(effect.tone)"
            >{{ effect.label }}</span
          >
          <span class="host-reason muted">{{ effect.reason }}</span>
        </dd>
      </template>
    </dl>
  </section>
</template>

<style scoped>
.host-role {
  margin-left: var(--space-4);
}

.host-effect {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-3);
}

.host-reason {
  line-height: 1.5;
  max-width: 52ch;
}
</style>
