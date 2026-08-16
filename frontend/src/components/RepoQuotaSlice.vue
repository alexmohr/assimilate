<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import type { RepoQuotaSummaryResponse } from '../types/generated'
import { formatBytes } from '../utils/format'
import { actionForHealth, actionLabel, computeSliceGeometry, quotaCeiling } from '../utils/quota'

const props = defineProps<{
  quota: RepoQuotaSummaryResponse | null
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
    quota: props.quota,
  }),
)

const ownCeilingBytes = computed(() => (props.quota?.enabled ? quotaCeiling(props.quota) : null))

// The portion of the fill within the own ceiling; the rest (if any) is drawn as the "over" segment.
const normalFillWidthPercent = computed(() => {
  const g = geometry.value
  return g.hasOwnQuota ? Math.min(g.fillWidthPercent, g.bracketWidthPercent) : g.fillWidthPercent
})

const overFillLeftPercent = computed(
  () => geometry.value.leftPercent + normalFillWidthPercent.value,
)

const overFillWidthPercent = computed(() =>
  geometry.value.pastOwnLimit
    ? Math.max(0, geometry.value.fillWidthPercent - normalFillWidthPercent.value)
    : 0,
)

const action = computed(() => {
  if (!props.quota) return null
  return actionForHealth(
    geometry.value.ownHealth,
    props.quota.warn_action,
    props.quota.critical_action,
  )
})

const chipLabel = computed(() => {
  const g = geometry.value
  if (g.hasOwnQuota && ownCeilingBytes.value !== null && ownCeilingBytes.value > 0) {
    const pct = Math.round((props.usageBytes / ownCeilingBytes.value) * 100)
    return `${pct}% of own`
  }
  if (props.boxMaxBytes > 0) {
    return `${Math.round((props.usageBytes / props.boxMaxBytes) * 100)}% of box`
  }
  return '—'
})

const labelLine = computed(() => {
  const g = geometry.value
  if (!g.hasOwnQuota) {
    return props.boxMaxBytes > 0
      ? formatBytes(props.usageBytes)
      : `${formatBytes(props.usageBytes)} · no host quota`
  }
  if (g.pastOwnLimit) {
    const actionText = action.value ? actionLabel(action.value) : 'over'
    return `${formatBytes(props.usageBytes)} over by ${formatBytes(g.overOwnBytes)} · ${actionText}`
  }
  return `${formatBytes(props.usageBytes)} · ${formatBytes(g.headroomBytes ?? 0)} headroom`
})
</script>

<template>
  <div class="slice">
    <div
      class="slice-track"
      role="img"
      :aria-label="`${formatBytes(usageBytes)} used${geometry.hasOwnQuota ? `, own quota ${chipLabel}` : ''}`"
    >
      <span
        class="slice-fill"
        :class="`slice-fill-step-${colorStep % 2}`"
        :style="{ left: `${geometry.leftPercent}%`, width: `${normalFillWidthPercent}%` }"
      ></span>
      <span
        v-if="overFillWidthPercent > 0"
        class="slice-fill-over"
        :style="{ left: `${overFillLeftPercent}%`, width: `${overFillWidthPercent}%` }"
      ></span>
      <span
        v-if="geometry.hasOwnQuota"
        class="own-bracket"
        :class="{ 'own-bracket-overcommit': geometry.bracketOvercommit }"
        :style="{ left: `${geometry.leftPercent}%`, width: `${geometry.bracketWidthPercent}%` }"
      ></span>
      <span
        v-if="geometry.tickPercent !== null"
        class="own-tick"
        :style="{ left: `${geometry.tickPercent}%` }"
      ></span>
    </div>
    <div class="slice-row">
      <span class="slice-label">{{ labelLine }}</span>
      <span
        class="slice-chip"
        :class="`slice-chip-${geometry.hasOwnQuota ? geometry.ownHealth : 'neutral'}`"
      >
        {{ chipLabel }}
      </span>
    </div>
  </div>
</template>

<style scoped>
.slice {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
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

.slice-fill-over {
  position: absolute;
  top: 0;
  height: 100%;
  border-radius: 0 var(--radius-pill) var(--radius-pill) 0;
  background: repeating-linear-gradient(
    -45deg,
    var(--danger) 0 5px,
    color-mix(in oklab, var(--danger) 40%, var(--bg-card)) 5px 9px
  );
}

.own-bracket {
  position: absolute;
  top: -4px;
  height: 14px;
  border: 1.5px solid var(--text-secondary);
  border-radius: var(--radius-pill);
  pointer-events: none;
}

.own-bracket-overcommit {
  border-right: 1.5px dashed var(--danger);
  border-top-right-radius: 0;
  border-bottom-right-radius: 0;
}

.own-tick {
  position: absolute;
  top: -1px;
  height: 8px;
  width: 2px;
  border-radius: var(--radius-pill);
  background: var(--text-muted);
}

.slice-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.5rem;
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

.slice-chip-ok {
  color: var(--success);
}

.slice-chip-warning {
  color: var(--warning);
}

.slice-chip-critical {
  color: var(--danger);
}
</style>
