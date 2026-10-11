<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import type { RouteLocationRaw } from 'vue-router'
import { getMachine, type Machine } from '../api/machines'
import { useWebSocket } from '../composables/useWebSocket'
import { domainParams } from '../utils/agent'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import BaseSpinner from './BaseSpinner.vue'
import HelpHint from './HelpHint.vue'
import MachineAvailabilitySection from './MachineAvailabilitySection.vue'
import MachinePowerSection from './MachinePowerSection.vue'

/**
 * The Power & dependencies pane: one machine's wake, shutdown and
 * availability settings, the same on every page that shows one of its roles -
 * an agent, a repository host, a dependency. What only one role has (the
 * agent process, the dependencies a host requires) goes in the default slot,
 * below the machine's own sections.
 */
const props = defineProps<{
  machineId: number
  canEdit: boolean
}>()

const emit = defineEmits<{ changed: [machine: Machine] }>()

const machine = ref<Machine | null>(null)
const loadError = ref<string | null>(null)

async function load(): Promise<void> {
  // A reply for the machine shown a moment ago is dropped rather than shown
  // under the one shown now.
  const id = props.machineId
  try {
    const loaded = await getMachine(id)
    if (props.machineId !== id) return
    machine.value = loaded
    loadError.value = null
  } catch (e: unknown) {
    if (props.machineId !== id) return
    if (machine.value === null) loadError.value = extractError(e)
    else logger.error('background machine refresh failed', e)
  }
}

onMounted(load)
watch(
  () => props.machineId,
  () => {
    machine.value = null
    void load()
  },
)

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => {
  void load()
})

function saved(updated: Machine): void {
  machine.value = updated
  emit('changed', updated)
}

async function checked(): Promise<void> {
  await load()
  if (machine.value) emit('changed', machine.value)
}

interface RoleLink {
  key: string
  kind: string
  name: string
  to: RouteLocationRaw
}

/** Every role the machine plays, linked, so a shared setting says where else it applies. */
const roles = computed<RoleLink[]>(() => {
  const m = machine.value
  if (!m) return []
  const links: RoleLink[] = []
  const agent = m.roles.agent
  if (agent) {
    links.push({
      key: `agent-${agent.id}`,
      kind: 'agent',
      name: agent.hostname,
      to: {
        path: `/agents/${encodeURIComponent(agent.hostname)}`,
        query: { ...domainParams(agent.domain), tab: 'settings', section: 'power' },
      },
    })
  }
  const host = m.roles.repo_host
  if (host) {
    links.push({
      key: `repo-host-${host.id}`,
      kind: 'repository host',
      name: host.ssh_host,
      to: { path: `/repo-hosts/${host.id}`, query: { section: 'power' } },
    })
  }
  for (const dependency of m.roles.dependencies) {
    links.push({
      key: `dependency-${dependency.id}`,
      kind: 'dependency',
      name: dependency.name,
      to: { path: `/dependency-hosts/${dependency.id}`, query: { section: 'power' } },
    })
  }
  return links
})
</script>

<template>
  <div
    v-if="loadError"
    class="error-banner"
  >
    {{ loadError }}
  </div>
  <div
    v-else-if="!machine"
    class="loading-row"
  >
    <BaseSpinner size="sm" />
  </div>
  <template v-else>
    <div
      v-if="roles.length > 1"
      class="machine-roles"
    >
      <span class="group-label">Same machine</span>
      <HelpHint label="settings shared by one machine">
        These run on one machine, so they share how it is woken, whether it is shut down and whether
        it is always online. A change here applies to all of them.
      </HelpHint>
      <span
        v-for="role in roles"
        :key="role.key"
        class="meta-pill"
      >
        {{ role.kind }}
        <RouterLink
          class="mono"
          :to="role.to"
        >
          {{ role.name }}
        </RouterLink>
      </span>
    </div>

    <MachinePowerSection
      :machine="machine"
      :can-edit="canEdit"
      @saved="saved"
    />
    <MachineAvailabilitySection
      :machine="machine"
      :can-edit="canEdit"
      @saved="saved"
      @checked="checked"
    />
    <slot />
  </template>
</template>

<style scoped>
.machine-roles {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}
</style>
