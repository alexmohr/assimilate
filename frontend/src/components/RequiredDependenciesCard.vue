<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { listDependencyHosts, type DependencyHost } from '../api/dependencyHosts'
import { extractError } from '../utils/error'
import { protocolLabel } from '../utils/dependencyHost'
import BaseSpinner from './BaseSpinner.vue'
import EditableSection from './EditableSection.vue'
import RequiredDependencyChips, { type RequiredDependencyChip } from './RequiredDependencyChips.vue'
import RequiredDependencyPicker from './RequiredDependencyPicker.vue'

/**
 * The dependencies something requires of every backup that involves it - an
 * agent's defaults, or a repository host's. Its own section with its own
 * save: a separate resource on the server, not a field of its owner.
 */
const props = defineProps<{
  /**
   * Which owner the calls below are bound to. The functions themselves are
   * rebuilt on every render of the page, so this - not their identity - says
   * the card now shows another owner.
   */
  subject: string
  load: () => Promise<number[]>
  save: (ids: number[]) => Promise<number[]>
  canEdit: boolean
  lede: string
  /** Where the dependencies apply, under the list. */
  scopeHint: string
}>()

const hosts = ref<DependencyHost[]>([])
const required = ref<number[]>([])
const loading = ref(false)
const loadError = ref<string | null>(null)

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const draft = ref<number[]>([])

async function loadAll(): Promise<void> {
  const subject = props.subject
  loading.value = true
  loadError.value = null
  try {
    const [hostRows, ids] = await Promise.all([listDependencyHosts(), props.load()])
    if (props.subject !== subject) return
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
  () => props.subject,
  () => {
    editing.value = false
    void loadAll()
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
async function saveDraft(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    required.value = await props.save(draft.value)
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
      :lede="lede"
      lede-label="required dependencies"
      :editing="editing"
      :can-edit="canEdit && hosts.length > 0"
      :saving="saving"
      :error="error"
      @edit="startEdit"
      @cancel="editing = false"
      @save="saveDraft"
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
          {{ scopeHint }}
        </p>
      </template>

      <template #edit>
        <RequiredDependencyPicker
          v-model="draft"
          :options="hosts"
          label="Required dependencies"
        />
        <p class="field-hint">{{ scopeHint }}</p>
      </template>
    </EditableSection>
  </section>
</template>
