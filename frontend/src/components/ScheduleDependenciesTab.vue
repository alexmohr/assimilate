<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  getScheduleDependencies,
  listDependencyHosts,
  updateScheduleDependencies,
  type DependencyHost,
} from '../api/dependencyHosts'
import { extractError } from '../utils/error'
import { normalizeScheduleDependencies } from '../utils/scheduleDependencies'
import { domainParams } from '../utils/agent'
import BaseSpinner from './BaseSpinner.vue'
import EditableSection from './EditableSection.vue'
import PerAgentFields from './PerAgentFields.vue'
import RequiredDependencyChips, { type RequiredDependencyChip } from './RequiredDependencyChips.vue'
import RequiredDependencyPicker, { type LockedDependency } from './RequiredDependencyPicker.vue'
import type { RouteLocationRaw } from 'vue-router'
import type { AgentRow } from '../types/agent'
import type { ScheduleDependencyInput, ScheduleDependencyResponse } from '../types/generated'

/**
 * The machines a backup needs besides its agents and repositories - an SMB
 * server whose share a pre-backup command mounts - per agent, since each agent
 * mounts its own shares.
 *
 * Saved on its own, apart from the schedule form's Save changes: it is a
 * separate resource on the server, and its Edit/Save sits beside it the way
 * every other independently saved section's does.
 */
const props = defineProps<{
  scheduleId: number
  /** The schedule's agents, in execution order. */
  agentIds: readonly number[]
  agents: readonly AgentRow[]
  agentLabel: (id: number) => string
}>()

const emit = defineEmits<{ saved: [] }>()

const HINT =
  "Per agent, because each agent mounts its own shares. Dependencies set in an agent's backup defaults apply to every schedule on that agent and can't be removed here."

const hosts = ref<DependencyHost[]>([])
const dependencies = ref<ScheduleDependencyResponse[]>([])
const loading = ref(false)
const loadError = ref<string | null>(null)

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
/** What the schedule itself requires, per agent, while the form is open. */
const draft = ref<Record<number, number[]>>({})

async function load(): Promise<void> {
  loading.value = true
  loadError.value = null
  try {
    const [hostRows, response] = await Promise.all([
      listDependencyHosts(),
      getScheduleDependencies(props.scheduleId),
    ])
    hosts.value = hostRows
    dependencies.value = normalizeScheduleDependencies(response).dependencies
  } catch (e: unknown) {
    loadError.value = extractError(e, 'Failed to load dependencies')
  } finally {
    loading.value = false
  }
}

watch(
  () => props.scheduleId,
  () => {
    editing.value = false
    void load()
  },
  { immediate: true },
)

function forAgent(agentId: number): ScheduleDependencyResponse[] {
  return dependencies.value.filter((d) => d.agent_id === agentId)
}

function chipsFor(agentId: number): RequiredDependencyChip[] {
  return forAgent(agentId).map((d) => ({
    id: d.dependency_host_id,
    name: d.dependency_name,
    inherited: d.source === 'agent_default',
    detail: d.source === 'agent_default' ? 'from agent defaults' : undefined,
  }))
}

/** Where an agent's backup defaults are edited, for the locked rows' link. */
function agentDefaultsLink(agentId: number): RouteLocationRaw {
  const agent = props.agents.find((a) => a.id === agentId)
  if (!agent) return '/agents'
  return {
    path: `/agents/${encodeURIComponent(agent.hostname)}`,
    query: { ...domainParams(agent.domain), tab: 'settings', section: 'defaults' },
  }
}

/** The ones an agent's backup defaults require: shown ticked, and not ours to untick. */
function lockedFor(agentId: number): ReadonlyMap<number, LockedDependency> {
  const locked = new Map<number, LockedDependency>()
  for (const d of forAgent(agentId)) {
    if (d.source !== 'agent_default') continue
    locked.set(d.dependency_host_id, {
      text: `${props.agentLabel(agentId)}'s backup defaults`,
      to: agentDefaultsLink(agentId),
    })
  }
  return locked
}

function startEdit(): void {
  const next: Record<number, number[]> = {}
  for (const agentId of props.agentIds) {
    next[agentId] = forAgent(agentId)
      .filter((d) => d.source === 'schedule')
      .map((d) => d.dependency_host_id)
  }
  draft.value = next
  error.value = null
  editing.value = true
}

/**
 * Only what the schedule sets itself goes back: the agent defaults apply on
 * top of whatever is saved here, and sending them would copy them onto the
 * schedule - where they would outlive their removal from the agent.
 */
const payload = computed<ScheduleDependencyInput[]>(() =>
  props.agentIds.flatMap((agentId) => {
    const locked = lockedFor(agentId)
    return (draft.value[agentId] ?? [])
      .filter((id) => !locked.has(id))
      .map((id) => ({ agent_id: agentId, dependency_host_id: id }))
  }),
)

/** Holds the form open if the request fails, so nothing ticked is lost. */
async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const response = await updateScheduleDependencies(props.scheduleId, payload.value)
    dependencies.value = normalizeScheduleDependencies(response).dependencies
    editing.value = false
    emit('saved')
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div
    v-if="loading && hosts.length === 0"
    class="loading-row"
  >
    <BaseSpinner size="md" />
  </div>
  <div
    v-else-if="loadError"
    class="error-banner"
  >
    {{ loadError }}
  </div>
  <EditableSection
    v-else
    lede="Machines this backup needs besides its agents and repositories. Each one is checked, and
      woken if it is set up for that, before the pre-backup commands run. If one does not answer,
      the run is skipped and caught up once it does when the dependency is marked as not always
      online, and fails otherwise."
    lede-label="dependencies"
    hint-label="per-agent dependencies"
    :editing="editing"
    :can-edit="hosts.length > 0 && agentIds.length > 0"
    :saving="saving"
    :error="error"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #view>
      <p
        v-if="hosts.length === 0"
        class="field-hint"
      >
        No dependencies yet. Add one with New on the Agents page's
        <RouterLink to="/agents?tab=dependencies">Dependencies</RouterLink> tab.
      </p>
      <p
        v-else-if="agentIds.length === 0"
        class="field-hint"
      >
        This schedule has no agents yet - pick them under Targets first.
      </p>
      <dl
        v-else
        class="info-grid"
      >
        <template
          v-for="agentId in agentIds"
          :key="agentId"
        >
          <dt class="mono">{{ agentLabel(agentId) }}</dt>
          <dd>
            <RequiredDependencyChips
              v-if="forAgent(agentId).length > 0"
              :items="chipsFor(agentId)"
            />
            <span
              v-else
              class="muted"
              >None</span
            >
          </dd>
        </template>
      </dl>
    </template>

    <template #hint>{{ HINT }}</template>

    <template #edit>
      <PerAgentFields
        :agent-ids="[...agentIds]"
        :agent-label="agentLabel"
      >
        <template #default="{ agentId }">
          <RequiredDependencyPicker
            :model-value="draft[agentId] ?? []"
            :options="hosts"
            :locked="lockedFor(agentId)"
            :label="`Dependencies for ${agentLabel(agentId)}`"
            @update:model-value="draft[agentId] = $event"
          />
        </template>
        <template #hint>{{ HINT }}</template>
      </PerAgentFields>
    </template>
  </EditableSection>
</template>
