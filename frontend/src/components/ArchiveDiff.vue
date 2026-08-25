<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { diffArchives } from '../api/archives'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseModal from './BaseModal.vue'
import type { DiffResponse } from '../types/generated'

interface ArchiveEntry {
  name: string
  start: string
  hostname: string
  comment: string
}

interface Props {
  open: boolean
  repoId: number | null
  archives: ArchiveEntry[]
}

const props = defineProps<Props>()

const emit = defineEmits<{
  close: []
}>()

const archive1 = ref<string | null>(null)
const archive2 = ref<string | null>(null)
const { loading, error, run } = useAsyncAction()
const result = ref<DiffResponse | null>(null)

function reset(): void {
  archive1.value = null
  archive2.value = null
  loading.value = false
  error.value = null
  result.value = null
}

function close(): void {
  reset()
  emit('close')
}

async function compare(): Promise<void> {
  const repoId = props.repoId
  const archiveName1 = archive1.value
  const archiveName2 = archive2.value
  if (repoId === null || archiveName1 === null || archiveName2 === null) return
  result.value = null

  await run(async () => {
    result.value = await diffArchives(repoId, {
      archive1: archiveName1,
      archive2: archiveName2,
    })
  })
}
</script>

<template>
  <BaseModal
    :open="open"
    title="Compare archives"
    size="lg"
    @close="close"
  >
    <div class="diff-controls">
      <div class="diff-select-group">
        <label class="field-label">Archive 1</label>
        <select
          v-model="archive1"
          class="input"
        >
          <option
            :value="null"
            disabled
          >
            — select —
          </option>
          <option
            v-for="archive in archives"
            :key="archive.name"
            :value="archive.name"
          >
            {{ archive.name }}
          </option>
        </select>
      </div>
      <div class="diff-select-group">
        <label class="field-label">Archive 2</label>
        <select
          v-model="archive2"
          class="input"
        >
          <option
            :value="null"
            disabled
          >
            — select —
          </option>
          <option
            v-for="archive in archives"
            :key="archive.name"
            :value="archive.name"
          >
            {{ archive.name }}
          </option>
        </select>
      </div>
      <button
        class="btn btn-primary compare-btn"
        :disabled="archive1 === null || archive2 === null || archive1 === archive2 || loading"
        @click="compare"
      >
        {{ loading ? 'Comparing...' : 'Compare' }}
      </button>
    </div>

    <div
      v-if="error"
      class="form-error"
    >
      {{ error }}
    </div>

    <div
      v-if="result"
      class="diff-results"
    >
      <!-- Added -->
      <details
        v-if="result.added.length > 0"
        open
        class="diff-section"
      >
        <summary class="diff-summary diff-added">Added ({{ result.added.length }})</summary>
        <ul class="diff-list">
          <li
            v-for="path in result.added"
            :key="path"
            class="diff-item diff-item-added"
          >
            {{ path }}
          </li>
        </ul>
      </details>

      <!-- Removed -->
      <details
        v-if="result.removed.length > 0"
        open
        class="diff-section"
      >
        <summary class="diff-summary diff-removed">Removed ({{ result.removed.length }})</summary>
        <ul class="diff-list">
          <li
            v-for="path in result.removed"
            :key="path"
            class="diff-item diff-item-removed"
          >
            {{ path }}
          </li>
        </ul>
      </details>

      <!-- Modified -->
      <details
        v-if="result.modified.length > 0"
        open
        class="diff-section"
      >
        <summary class="diff-summary diff-modified">
          Modified ({{ result.modified.length }})
        </summary>
        <ul class="diff-list">
          <li
            v-for="path in result.modified"
            :key="path"
            class="diff-item diff-item-modified"
          >
            {{ path }}
          </li>
        </ul>
      </details>

      <div
        v-if="
          result.added.length === 0 && result.removed.length === 0 && result.modified.length === 0
        "
        class="no-diff"
      >
        No differences found between the two archives.
      </div>
    </div>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="close"
      >
        Close
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.diff-controls {
  display: flex;
  gap: var(--space-6);
  align-items: flex-end;
  margin-bottom: var(--space-7);
}

.diff-select-group {
  flex: 1;
}

.field-label {
  display: block;
  margin-bottom: var(--space-3);
}

.compare-btn {
  white-space: nowrap;
}

.form-error {
  margin-bottom: var(--space-5);
}

.diff-results {
  margin-top: var(--space-4);
}

.diff-section {
  margin-bottom: var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.diff-summary {
  padding: var(--space-4) var(--space-6);
  font-size: var(--fs-sm);
  font-weight: 600;
  cursor: pointer;
  user-select: none;
}

.diff-added {
  background: color-mix(in srgb, var(--success) 10%, transparent);
  color: var(--success);
}

.diff-removed {
  background: color-mix(in srgb, var(--danger) 10%, transparent);
  color: var(--danger);
}

.diff-modified {
  background: color-mix(in srgb, var(--warning) 10%, transparent);
  color: var(--warning);
}

.diff-list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 200px;
  overflow-y: auto;
}

.diff-item {
  padding: var(--space-3) var(--space-6);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  border-top: 1px solid var(--border-subtle);
}

.diff-item-added {
  color: var(--success);
}

.diff-item-removed {
  color: var(--danger);
}

.diff-item-modified {
  color: var(--warning);
}

.no-diff {
  text-align: center;
  padding: var(--space-8);
  color: var(--text-muted);
  font-size: var(--fs-base);
}
</style>
