<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import { mergeAgent } from '../api/agents'
import { extractError } from '../utils/error'
import type { AgentRow } from '../types/agent'
import BaseModal from './BaseModal.vue'

const props = defineProps<{
  source: AgentRow
  allAgents: AgentRow[]
}>()

const emit = defineEmits<{
  merged: []
  cancel: []
}>()

/** Selected by ID, not hostname - two candidate targets can share a hostname. */
const targetAgentId = ref<number | ''>('')
const savePattern = ref(true)
const patternValue = ref(`${props.source.hostname}*`)
const mergeLoading = ref(false)
const mergeError = ref<string | null>(null)

const realAgents = computed<AgentRow[]>(() =>
  props.allAgents.filter((c) => c.id !== props.source.id && !c.is_imported),
)

const targetAgent = computed<AgentRow | undefined>(() =>
  realAgents.value.find((c) => c.id === targetAgentId.value),
)

/** Distinguishes options that share a hostname; appended only when needed. */
function targetLabel(c: AgentRow): string {
  const sharesHostname = realAgents.value.some((o) => o.id !== c.id && o.hostname === c.hostname)
  const domainSuffix = sharesHostname ? ` (${c.domain ?? 'no domain'})` : ''
  return `${c.hostname}${domainSuffix}${c.display_name ? ` — ${c.display_name}` : ''}`
}

async function confirmMerge(): Promise<void> {
  if (!targetAgent.value) return
  mergeLoading.value = true
  mergeError.value = null
  try {
    const createPattern =
      savePattern.value && patternValue.value.trim() ? patternValue.value.trim() : undefined
    await mergeAgent(
      targetAgent.value.hostname,
      props.source.id,
      createPattern,
      targetAgent.value.domain,
    )
    emit('merged')
  } catch (e: unknown) {
    mergeError.value = extractError(e)
  } finally {
    mergeLoading.value = false
  }
}
</script>

<template>
  <BaseModal
    :open="true"
    title="Merge agent"
    size="sm"
    @close="emit('cancel')"
  >
    <div class="field">
      <label class="field-label">Source (imported)</label>
      <input
        class="input mono"
        :value="source.hostname"
        disabled
      />
    </div>
    <div class="field">
      <label class="field-label">Merge into <span class="required">*</span></label>
      <select
        v-model="targetAgentId"
        class="input"
      >
        <option value="">Select target agent...</option>
        <option
          v-for="c in realAgents"
          :key="c.id"
          :value="c.id"
        >
          {{ targetLabel(c) }}
        </option>
      </select>
    </div>
    <div class="field toggle-row">
      <span class="toggle-row-label">Save pattern for future imports</span>
      <input
        v-model="savePattern"
        type="checkbox"
        class="checkbox"
      />
    </div>
    <div
      v-if="savePattern"
      class="field"
    >
      <label class="field-label">Pattern</label>
      <input
        v-model="patternValue"
        class="input mono"
        placeholder="e.g. myhost*"
      />
      <span class="field-hint">
        This pattern will be added to the target agent's hostname aliases.
      </span>
    </div>
    <div
      v-if="mergeError"
      class="form-error"
    >
      {{ mergeError }}
    </div>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="emit('cancel')"
      >
        Cancel
      </button>
      <button
        class="btn btn-primary"
        :disabled="mergeLoading || !targetAgent"
        @click="confirmMerge"
      >
        {{ mergeLoading ? 'Merging...' : 'Merge' }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.checkbox {
  width: 16px;
  height: 16px;
  cursor: pointer;
}
</style>
