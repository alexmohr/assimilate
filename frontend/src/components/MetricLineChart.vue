<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { Line } from 'vue-chartjs'
import type { ChartData, ChartOptions } from 'chart.js'

defineProps<{
  label: string
  data: ChartData<'line'>
  options: ChartOptions<'line'>
}>()
</script>

<template>
  <div class="chart-cell">
    <span class="metric-label">{{ label }}</span>
    <div class="chart-container">
      <Line
        :data="data"
        :options="options"
      />
    </div>
  </div>
</template>

<style scoped>
.chart-cell {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.metric-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
}

.chart-container {
  height: 220px;
  position: relative;
}
</style>
