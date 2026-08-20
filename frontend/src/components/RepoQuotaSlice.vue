<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes } from '../utils/format'
import { computeSliceGeometry } from '../utils/quota'

const props = defineProps<{
  usageBytes: number
  offsetBytes: number
  boxMaxBytes: number
  /** Cycles a small set of fill tones so stacked slices in one group read as one sequence. */
  colorStep: number
}>()

const geometry = computed(() =>
  computeSliceGeometry({
    offsetBytes: props.offsetBytes,
    usageBytes: props.usageBytes,
    boxMaxBytes: props.boxMaxBytes,
  }),
)

const chipLabel = computed(() =>
  props.boxMaxBytes > 0
    ? `${Math.round((props.usageBytes / props.boxMaxBytes) * 100)}% of box`
    : '—',
)
</script>

<template>
  <div class="slice">
    <div
      class="slice-track"
      role="img"
      :aria-label="`${formatBytes(usageBytes)} used`"
    >
      <span
        class="slice-fill"
        :class="`slice-fill-step-${colorStep % 2}`"
        :style="{ left: `${geometry.leftPercent}%`, width: `${geometry.fillWidthPercent}%` }"
      ></span>
    </div>
    <div class="slice-row">
      <span class="slice-label">{{ formatBytes(usageBytes) }}</span>
      <span class="slice-chip slice-chip-neutral">{{ chipLabel }}</span>
    </div>
  </div>
</template>

<style scoped>
.slice {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.slice-track {
  position: relative;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--border);
  overflow: visible;
}

.slice-fill {
  position: absolute;
  top: 0;
  height: 100%;
  border-radius: var(--radius-pill);
}

.slice-fill-step-0 {
  background: var(--warning);
}

.slice-fill-step-1 {
  background: color-mix(in oklab, var(--warning) 62%, var(--bg-card));
}

.slice-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-4);
}

.slice-label,
.slice-chip {
  font-size: var(--fs-2xs);
  font-family: var(--mono);
  white-space: nowrap;
}

.slice-label {
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
}

.slice-chip {
  font-weight: 600;
  flex-shrink: 0;
}

.slice-chip-neutral {
  color: var(--text-muted);
}
</style>
