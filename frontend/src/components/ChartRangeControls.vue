<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts" generic="T extends string | number">
import type { Repo } from '../types/repo'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'

defineProps<{
  repos: Repo[]
  options: readonly SegmentedOption<T>[]
  /** Names the range group for assistive tech, e.g. "Storage trend range". */
  label: string
}>()

const repoId = defineModel<number | undefined>('repoId')
const days = defineModel<T>('days', { required: true })
</script>

<template>
  <div class="chart-range-controls">
    <select
      v-model="repoId"
      class="input chart-range-select"
    >
      <option :value="undefined">All Repos</option>
      <option
        v-for="repo in repos"
        :key="repo.id"
        :value="repo.id"
      >
        {{ repo.name }}
      </option>
    </select>
    <BaseSegmented
      v-model="days"
      :options="options"
      :label="label"
    />
  </div>
</template>

<style scoped>
.chart-range-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.chart-range-select {
  width: auto;
  padding: 0.25rem 0.5rem;
  font-size: var(--fs-xs);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
}
</style>
