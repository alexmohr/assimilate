<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * The dependencies something requires, read-only: each one a chip linking to
 * its detail page. One the reader cannot remove from here - required by an
 * agent's backup defaults, seen on a schedule - is drawn dashed, with a note
 * saying where it comes from.
 */
export interface RequiredDependencyChip {
  id: number
  name: string
  /** A muted aside after the name, e.g. "SMB · nas-media.lan". */
  detail?: string
  /** Set somewhere else and only shown here. */
  inherited?: boolean
}

defineProps<{
  items: readonly RequiredDependencyChip[]
}>()
</script>

<template>
  <span class="dependency-chips">
    <RouterLink
      v-for="item in items"
      :key="item.id"
      class="dependency-chip"
      :class="{ 'dependency-chip--inherited': item.inherited }"
      :to="`/dependency-hosts/${item.id}`"
      :title="item.inherited ? 'From the agent\'s backup defaults' : undefined"
    >
      {{ item.name }}
      <span
        v-if="item.detail"
        class="muted"
        >· {{ item.detail }}</span
      >
    </RouterLink>
  </span>
</template>

<style scoped>
.dependency-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.dependency-chip {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-base);
  color: var(--text-primary);
  font-size: var(--fs-sm);
  text-decoration: none;
  transition: border-color var(--duration-base);
}

.dependency-chip:hover {
  border-color: var(--accent);
}

/* Dashed: it is listed here, but it is set, and removed, somewhere else. */
.dependency-chip--inherited {
  border-style: dashed;
  color: var(--text-secondary);
}
</style>
