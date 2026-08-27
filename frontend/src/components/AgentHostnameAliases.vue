<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { X } from '@lucide/vue'
import {
  listAgentHostnamePatterns,
  createAgentHostnamePattern,
  deleteAgentHostnamePattern,
} from '../api/agents'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import type { AgentHostnamePattern } from '../types/agent'

/**
 * Glob patterns that map archive hostnames onto this agent during import.
 */
const props = defineProps<{
  hostname: string
  /** Disambiguates `hostname` when it is shared by more than one agent. */
  domain?: string | null
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
    patterns.value = await listAgentHostnamePatterns(h, props.domain)
  } catch (e: unknown) {
    logger.error('loadHostnamePatterns failed', e)
  }
}

async function addPattern(): Promise<void> {
  if (!newPattern.value.trim()) return
  addLoading.value = true
  error.value = null
  try {
    const res = await createAgentHostnamePattern(
      props.hostname,
      newPattern.value.trim(),
      props.domain,
    )
    patterns.value = [...patterns.value, res]
    newPattern.value = ''
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    addLoading.value = false
  }
}

async function deletePattern(id: number): Promise<void> {
  try {
    await deleteAgentHostnamePattern(props.hostname, id, props.domain)
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
  <p class="pane-lede">
    Glob patterns that match archive hostnames to this agent during repository import. Only affects
    future discoveries — existing imported agents are not retroactively reassigned, so use "Merge
    into" on one to move its historical archives. <code>*</code> matches any characters,
    <code>?</code> a single one.
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
      {{ addLoading ? 'Adding...' : 'Add pattern' }}
    </button>
  </div>
</template>

<style scoped>
.pattern-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.pattern-delete {
  font-size: var(--fs-lg);
  flex-shrink: 0;
}

.pattern-add-row {
  display: flex;
  gap: var(--space-4);
  align-items: center;
  flex-wrap: wrap;
}

.input-sm {
  min-width: 140px;
}
</style>
