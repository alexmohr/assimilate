<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'

interface ClientRow {
  id: number
  hostname: string
  display_name: string | null
}

interface MergeResult {
  merged: boolean
}

const props = defineProps<{
  source: ClientRow
  allClients: ClientRow[]
}>()

const emit = defineEmits<{
  merged: []
  cancel: []
}>()

const targetHostname = ref('')
const savePattern = ref(true)
const patternValue = ref(`${props.source.hostname}*`)
const mergeLoading = ref(false)
const mergeError = ref<string | null>(null)

const realClients = computed<ClientRow[]>(() =>
  props.allClients.filter(
    (c) => c.id !== props.source.id && !(c.display_name?.endsWith('(imported)') ?? false),
  ),
)

async function confirmMerge(): Promise<void> {
  if (!targetHostname.value) return
  mergeLoading.value = true
  mergeError.value = null
  try {
    const body: Record<string, string> = {}
    if (savePattern.value && patternValue.value.trim()) {
      body.create_pattern = patternValue.value.trim()
    }
    await apiClient.post<MergeResult>(
      `/clients/${targetHostname.value}/merge-from/${props.source.id}`,
      body,
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
  <div
    class="overlay"
    @click.self="emit('cancel')"
  >
    <div class="dialog dialog-sm">
      <div class="dialog-header">
        <h2 class="dialog-title">Merge Client</h2>
        <button
          class="close-btn"
          @click="emit('cancel')"
        >
          &times;
        </button>
      </div>
      <div class="dialog-body">
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
            v-model="targetHostname"
            class="input"
          >
            <option value="">Select target client...</option>
            <option
              v-for="c in realClients"
              :key="c.id"
              :value="c.hostname"
            >
              {{ c.hostname }}{{ c.display_name ? ` — ${c.display_name}` : '' }}
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
            This pattern will be added to the target client's hostname aliases.
          </span>
        </div>
        <div
          v-if="mergeError"
          class="form-error"
        >
          {{ mergeError }}
        </div>
      </div>
      <div class="dialog-footer">
        <button
          class="btn btn-ghost"
          @click="emit('cancel')"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="mergeLoading || !targetHostname"
          @click="confirmMerge"
        >
          {{ mergeLoading ? 'Merging...' : 'Merge' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
  margin-top: 0.5rem;
}

.toggle-row-label {
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.checkbox {
  width: 16px;
  height: 16px;
  cursor: pointer;
}
</style>
