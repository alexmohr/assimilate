<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
export interface EntityCardStat {
  value: string | number
  label: string
  mono?: boolean
}

withDefaults(
  defineProps<{
    title: string
    subtitle?: string | null
    stats?: EntityCardStat[]
  }>(),
  {
    subtitle: null,
    stats: () => [],
  },
)
</script>

<template>
  <div class="entity-card">
    <div class="entity-card-top">
      <div class="entity-card-info">
        <span class="entity-card-title">{{ title }}</span>
        <span
          v-if="subtitle"
          class="entity-card-subtitle"
          >{{ subtitle }}</span
        >
      </div>
      <div
        v-if="$slots['top-badges']"
        class="entity-card-top-badges"
      >
        <slot name="top-badges" />
      </div>
    </div>

    <slot name="extra" />

    <slot name="status" />

    <div
      v-if="$slots.meta"
      class="entity-card-meta"
    >
      <slot name="meta" />
    </div>

    <div
      v-if="stats.length > 0"
      class="entity-card-stats"
    >
      <div
        v-for="stat in stats"
        :key="stat.label"
        class="entity-card-stat"
      >
        <span
          class="entity-card-stat-value"
          :class="{ mono: stat.mono }"
          >{{ stat.value }}</span
        >
        <span class="entity-card-stat-label">{{ stat.label }}</span>
      </div>
    </div>

    <div
      v-if="$slots.actions"
      class="entity-card-actions"
      @click.stop
    >
      <slot name="actions" />
    </div>
  </div>
</template>

<style scoped>
.entity-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  cursor: pointer;
  transition:
    box-shadow 0.15s,
    border-color 0.15s;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.entity-card:hover {
  border-color: var(--accent);
  box-shadow: var(--shadow);
}

.entity-card-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
}

.entity-card-info {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 0;
}

.entity-card-title {
  font-weight: 600;
  font-family: var(--mono);
  font-size: 0.9rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.entity-card-subtitle {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.entity-card-top-badges {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-shrink: 0;
}

.entity-card-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
}

.entity-card-stats {
  display: flex;
  gap: 1.25rem;
}

.entity-card-stat {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.entity-card-stat-value {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-primary);
}

.entity-card-stat-label {
  font-size: 0.7rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.entity-card-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.25rem;
  margin-top: auto;
}
</style>
