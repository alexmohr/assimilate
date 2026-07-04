<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import {
  FileChangeAction,
  parseFileChangePatterns,
  serializeFileChangePatterns,
  type FileChangePatternRow,
} from '../utils/fileChangePatterns'

const props = defineProps<{
  modelValue: string
  placeholder?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const rows = ref<FileChangePatternRow[]>(parseFileChangePatterns(props.modelValue))

let syncing = false

watch(
  () => props.modelValue,
  (raw) => {
    if (syncing) return
    syncing = true
    rows.value = parseFileChangePatterns(raw)
    syncing = false
  },
)

watch(
  rows,
  (value) => {
    if (syncing) return
    syncing = true
    emit('update:modelValue', serializeFileChangePatterns(value))
    syncing = false
  },
  { deep: true },
)

function addRow(): void {
  rows.value = [...rows.value, { path: '', action: FileChangeAction.Warn }]
}

function removeRow(index: number): void {
  const next = [...rows.value]
  next.splice(index, 1)
  rows.value = next
}
</script>

<template>
  <div class="fcp-editor">
    <div
      v-for="(row, index) in rows"
      :key="index"
      class="fcp-row"
    >
      <input
        v-model="row.path"
        type="text"
        class="fcp-input"
        :placeholder="props.placeholder ?? 'Glob against warning text, e.g. */etc/config*'"
        spellcheck="false"
      />
      <select
        v-model="row.action"
        class="fcp-input fcp-select"
      >
        <option :value="FileChangeAction.Warn">warn</option>
        <option :value="FileChangeAction.Ignore">ignore</option>
        <option :value="FileChangeAction.Fatal">fatal</option>
      </select>
      <button
        class="btn btn-sm btn-danger"
        title="Remove"
        @click="removeRow(index)"
      >
        &times;
      </button>
    </div>
    <button
      class="btn btn-sm btn-ghost"
      @click="addRow()"
    >
      + Add pattern
    </button>
    <span class="field-hint">
      <slot name="hint">
        Glob patterns matched against the full warning message, with actions:
        <code>ignore</code> (no warning), <code>warn</code> (default, current behavior),
        <code>fatal</code> (fail backup). A bare path will not match - wrap it in <code>*</code>,
        e.g. <code>*/etc/config*</code>. Unconfigured files still produce warnings.
      </slot>
    </span>
  </div>
</template>

<style scoped>
.fcp-editor {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.fcp-row {
  display: flex;
  gap: 0.5rem;
  align-items: center;
}

.fcp-input {
  padding: 0.5rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
  outline: none;
  transition: border-color 0.15s;
  box-sizing: border-box;
}

.fcp-input:focus {
  border-color: var(--accent);
}

.fcp-row .fcp-input:first-child {
  flex: 1;
  width: auto;
}

.fcp-select {
  width: auto;
  min-width: 8rem;
  flex-shrink: 0;
}

.fcp-row .btn {
  flex-shrink: 0;
}
</style>
