<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * A caption over a run of `BrowserRow`s - "Today", "3 matches in this
 * archive", "2 added · 1 changed" - and the hairline above it that splits it
 * from the rows before.
 */
defineProps<{ label: string }>()

defineSlots<{
  /** Something at the caption's right end, e.g. a group's size. */
  aside?: () => unknown
}>()
</script>

<template>
  <div class="row-group">
    <span class="row-group-label">{{ label }}</span>
    <span
      v-if="$slots.aside"
      class="row-group-aside"
    >
      <slot name="aside" />
    </span>
  </div>
</template>

<style scoped>
.row-group {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-4);
  padding: var(--space-5) var(--space-7) var(--space-2);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.browser-row + .row-group,
.row-group + .row-group {
  border-top: 1px solid var(--border-subtle);
}

.row-group-label {
  flex: 1 1 12rem;
  min-width: 0;
}

.row-group-aside {
  flex: none;
  font-variant-numeric: tabular-nums;
}
</style>
