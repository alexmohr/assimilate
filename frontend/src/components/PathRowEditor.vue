<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    modelValue: string[]
    placeholder?: string
    addLabel?: string
  }>(),
  {
    placeholder: 'Path',
    addLabel: 'Add Path',
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: string[]]
}>()

function updatePath(index: number, value: string): void {
  const paths = [...props.modelValue]
  paths[index] = value
  emit('update:modelValue', paths)
}

function addPath(): void {
  emit('update:modelValue', [...props.modelValue, ''])
}

function removePath(index: number): void {
  emit(
    'update:modelValue',
    props.modelValue.filter((_, i) => i !== index),
  )
}

function normalizePaths(): void {
  emit(
    'update:modelValue',
    props.modelValue.map((path) => path.trim()).filter((path) => path.length > 0),
  )
}
</script>

<template>
  <div class="path-row-editor">
    <div
      v-if="modelValue.length > 0"
      class="path-row-list"
    >
      <div
        v-for="(path, index) in modelValue"
        :key="index"
        class="path-row"
      >
        <input
          class="path-input"
          :value="path"
          :placeholder="placeholder"
          spellcheck="false"
          @blur="normalizePaths"
          @input="updatePath(index, ($event.target as HTMLInputElement).value)"
        />
        <button
          type="button"
          class="path-remove"
          title="Remove path"
          @click="removePath(index)"
        >
          &times;
        </button>
      </div>
    </div>
    <button
      type="button"
      class="path-add"
      @click="addPath"
    >
      {{ addLabel }}
    </button>
  </div>
</template>

<style scoped>
.path-row-editor {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.path-row-list {
  display: flex;
  flex-direction: column;
  gap: 0.45rem;
}

.path-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 2rem;
  gap: 0.45rem;
  align-items: center;
}

.path-input {
  width: 100%;
  min-width: 0;
  padding: 0.5rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-family: var(--mono);
  font-size: 0.82rem;
  line-height: 1.4;
  outline: none;
  box-sizing: border-box;
}

.path-input:focus {
  border-color: var(--accent);
}

.path-remove,
.path-add {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-weight: 600;
  cursor: pointer;
  transition:
    background 0.15s,
    color 0.15s,
    border-color 0.15s;
}

.path-remove {
  width: 2rem;
  height: 2rem;
  line-height: 1;
  font-size: 1.1rem;
}

.path-add {
  align-self: flex-start;
  padding: 0.35rem 0.65rem;
  font-size: 0.78rem;
}

.path-remove:hover,
.path-add:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
  border-color: var(--accent);
}
</style>
