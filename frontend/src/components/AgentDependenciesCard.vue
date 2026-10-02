<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  getAgentDependencies,
  listDependencyHosts,
  updateAgentDependencies,
  type DependencyHost,
} from '../api/dependencyHosts'
import { extractError } from '../utils/error'
import { protocolLabel } from '../utils/dependencyHost'
import BaseSpinner from './BaseSpinner.vue'
import EditableSection from './EditableSection.vue'
import RequiredDependencyChips, { type RequiredDependencyChip } from './RequiredDependencyChips.vue'
import RequiredDependencyPicker from './RequiredDependencyPicker.vue'
import type { AgentRow } from '../types/agent'

/**
 * The dependencies every backup of this agent needs, whichever schedule runs
 * it - for when the agent's own pre-backup commands mount the share, so every
 * schedule here has to wait for the machine behind it.
 *
 * Its own section with its own save, below the backup defaults card: it is a
 * separate resource on the server, not a field of the agent.
 */
const props = defineProps<{
  agent: AgentRow
  /** Admins only, and never an imported host, which has no agent to run commands. */
  canEdit: boolean
}>()

const hosts = ref<DependencyHost[]>([])
const required = ref<number[]>([])
const loading = ref(false)
const loadError = ref<string | null>(null)

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const draft = ref<number[]>([])

async function load(): Promise<void> {
  loading.value = true
  loadError.value = null
  try {
    const [hostRows, ids] = await Promise.all([
      listDependencyHosts(),
      getAgentDependencies(props.agent.hostname, props.agent.domain),
    ])
    hosts.value = hostRows
    // An answer without the list - a server that predates dependencies - reads
    // as one that requires none, rather than taking the whole pane down.
    required.value = Array.isArray(ids) ? ids : []
  } catch (e: unknown) {
    loadError.value = extractError(e, 'Failed to load required dependencies')
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.agent.hostname, props.agent.domain],
  () => {
    editing.value = false
    void load()
  },
  { immediate: true },
)

/** In the order they were listed; one deleted since is shown by its id. */
const chips = computed<RequiredDependencyChip[]>(() =>
  required.value.map((id) => {
    const host = hosts.value.find((h) => h.id === id)
    return host
      ? { id, name: host.name, detail: `${protocolLabel(host.port)} · ${host.address}` }
      : { id, name: `#${id}` }
  }),
)

function startEdit(): void {
  draft.value = [...required.value]
  error.value = null
  editing.value = true
}

/** Holds the form open if the request fails, so nothing ticked is lost. */
async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    required.value = await updateAgentDependencies(
      props.agent.hostname,
      draft.value,
      props.agent.domain,
    )
    editing.value = false
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <section class="pane-section">
    <div
      v-if="loading && hosts.length === 0"
      class="loading-row"
    >
      <BaseSpinner size="sm" />
    </div>
    <div
      v-else-if="loadError"
      class="error-banner"
    >
      {{ loadError }}
    </div>
    <EditableSection
      v-else
      label="Required dependencies"
      lede="Machines every backup of this agent needs. Use this when the agent's own pre-backup
        commands mount a share, so every schedule here waits for it."
      lede-label="required dependencies"
      :editing="editing"
      :can-edit="canEdit && hosts.length > 0"
      :saving="saving"
      :error="error"
      @edit="startEdit"
      @cancel="editing = false"
      @save="save"
    >
      <template #view>
        <RequiredDependencyChips
          v-if="chips.length > 0"
          :items="chips"
        />
        <span
          v-else
          class="muted"
          >None configured.</span
        >
        <p
          v-if="hosts.length === 0"
          class="field-hint"
        >
          No dependencies yet. Add one with New on the Agents page's
          <RouterLink to="/agents?tab=dependencies">Dependencies</RouterLink> tab.
        </p>
        <p
          v-else
          class="field-hint"
        >
          Applies to every schedule on this agent.
        </p>
      </template>

      <template #edit>
        <RequiredDependencyPicker
          v-model="draft"
          :options="hosts"
          label="Required dependencies"
        />
        <p class="field-hint">Applies to every schedule on this agent.</p>
      </template>
    </EditableSection>
  </section>
</template>
