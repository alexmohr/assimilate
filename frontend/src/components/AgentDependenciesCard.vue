<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { getAgentDependencies, updateAgentDependencies } from '../api/dependencyHosts'
import RequiredDependenciesCard from './RequiredDependenciesCard.vue'
import type { AgentRow } from '../types/agent'

/**
 * The dependencies every backup of this agent needs, whichever schedule runs
 * it - for when the agent's own pre-backup commands mount the share, so every
 * schedule here has to wait for the machine behind it.
 */
const props = defineProps<{
  agent: AgentRow
  /** Admins only, and never an imported host, which has no agent to run commands. */
  canEdit: boolean
}>()
</script>

<template>
  <RequiredDependenciesCard
    :subject="`${agent.hostname}@${agent.domain ?? ''}`"
    :load="() => getAgentDependencies(props.agent.hostname, props.agent.domain)"
    :save="(ids) => updateAgentDependencies(props.agent.hostname, ids, props.agent.domain)"
    :can-edit="canEdit"
    lede="Machines every backup of this agent needs. Use this when the agent's own pre-backup
      commands mount a share, so every schedule here waits for it."
    scope-hint="Applies to every schedule on this agent."
  />
</template>
