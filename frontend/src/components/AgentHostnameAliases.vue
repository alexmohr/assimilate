<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { X } from '@lucide/vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import type { AgentHostnamePattern } from '../types/agent'

/**
 * Glob patterns that map archive hostnames onto this agent during import.
 * See docs/contributing/ui-design-audit.md (F-24).
 */
const props = defineProps<{
  hostname: string
  /** False for imported hosts, whose patterns are read-only. */
  canEdit: boolean
}>()

const patterns = ref<AgentHostnamePattern[]>([])
const newPattern = ref('')
const addLoading = ref(false)
const error = ref<string | null>(null)

async function load(hostname?: string): Promise<void> {
  const h = hostname ?? props.hostname
  if (!h) return
  try {
    const res = await apiClient.get<AgentHostnamePattern[]>(`/agents/${h}/hostname-patterns`)
    patterns.value = res.data
  } catch (e: unknown) {
    logger.error('loadHostnamePatterns failed', e)
  }
}

async function addPattern(): Promise<void> {
  if (!newPattern.value.trim()) return
  addLoading.value = true
  error.value = null
  try {
    const res = await apiClient.post<AgentHostnamePattern>(
      `/agents/${props.hostname}/hostname-patterns`,
      { pattern: newPattern.value.trim() },
    )
    patterns.value = [...patterns.value, res.data]
    newPattern.value = ''
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    addLoading.value = false
  }
}

async function deletePattern(id: number): Promise<void> {
  try {
    await apiClient.delete(`/agents/${props.hostname}/hostname-patterns/${id}`)
    patterns.value = patterns.value.filter((p) => p.id !== id)
  } catch (e: unknown) {
    error.value = extractError(e)
  }
}

watch(() => props.hostname, load, { immediate: true })

// Renaming a host offers to keep its old name as a pattern; the view drives
// that dialog, so it needs to be able to reload this list afterwards.
defineExpose({ reload: load })
</script>

<template>
  <div class="info-card">
    <h3 class="info-title">Hostname Aliases</h3>
    <p class="field-hint">
      Glob patterns that match archive hostnames to this agent during repository import. Only
      affects future discoveries — existing imported agents are not retroactively reassigned. Use
      "Merge into" on an imported agent to move its historical archives.
    </p>
    <div
      v-if="patterns.length > 0"
      class="paths-list"
    >
      <div
        v-for="p in patterns"
        :key="p.id"
        class="pattern-row"
      >
        <code class="path-item mono">{{ p.pattern }}</code>
        <button
          v-if="canEdit"
          class="tag-remove pattern-delete"
          title="Delete pattern"
          aria-label="Delete hostname pattern"
          @click="deletePattern(p.id)"
        >
          <X :size="12" />
        </button>
      </div>
    </div>
    <span
      v-else
      class="muted"
      >No alias patterns configured.</span
    >
    <p class="field-hint">
      <code>*</code> matches any characters, <code>?</code> matches a single character.
    </p>
    <div
      v-if="error"
      class="form-error"
    >
      {{ error }}
    </div>
    <div
      v-if="canEdit"
      class="pattern-add-row"
    >
      <input
        v-model="newPattern"
        class="input input-sm"
        placeholder="e.g. myhost* or host-??"
        @keyup.enter="addPattern"
      />
      <button
        class="btn btn-sm btn-primary"
        :disabled="addLoading || !newPattern.trim()"
        @click="addPattern"
      >
        {{ addLoading ? 'Adding...' : 'Add Pattern' }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.muted {
  color: var(--text-muted);
  font-size: var(--fs-base);
}

.paths-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  margin-bottom: 0.5rem;
}

.path-item {
  font-size: var(--fs-sm);
  padding: 0.2rem 0.5rem;
  background: var(--bg-input);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
}

.pattern-row {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.pattern-delete {
  font-size: var(--fs-lg);
  flex-shrink: 0;
}

.pattern-add-row {
  display: flex;
  gap: 0.5rem;
  align-items: center;
  margin-top: 0.75rem;
  flex-wrap: wrap;
}

.input-sm {
  padding: 0.35rem 0.55rem;
  font-size: var(--fs-sm);
  width: auto;
  min-width: 140px;
}
</style>
