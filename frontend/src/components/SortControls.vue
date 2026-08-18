<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts" generic="F extends string">
import type { SortDir } from '../composables/useListSort'

/**
 * The "Sort: Name ↑ | Size | ..." strip on a list view. The agents,
 * repositories and schedules views each carried the same four-button block
 * with the same arrow expression repeated per button.
 */

defineProps<{
  /** The column currently sorted by. */
  field: F
  direction: SortDir
  options: readonly { field: F; label: string }[]
}>()

defineEmits<{ toggle: [field: F] }>()
</script>

<template>
  <div class="sort-controls">
    <span class="sort-label">Sort:</span>
    <button
      v-for="option in options"
      :key="option.field"
      class="btn btn-sm btn-ghost"
      :class="{ active: field === option.field }"
      @click="$emit('toggle', option.field)"
    >
      {{ option.label }}
      {{ field === option.field ? (direction === 'asc' ? '↑' : '↓') : '' }}
    </button>
  </div>
</template>
