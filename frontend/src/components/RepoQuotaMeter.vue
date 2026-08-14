<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import type { RepoQuotaSummaryResponse } from '../types/generated'
import { formatBytes } from '../utils/format'
import { actionLabel, quotaCeiling, quotaHealth, actionForHealth } from '../utils/quota'

const props = defineProps<{
  quota: RepoQuotaSummaryResponse | null
  usageBytes: number
}>()

const ceiling = computed(() => quotaCeiling(props.quota))
const health = computed(() => quotaHealth(props.quota, props.usageBytes))

const visible = computed(
  () => !!props.quota?.enabled && ceiling.value !== null && ceiling.value > 0,
)

const fillPercent = computed(() => {
  if (ceiling.value === null || ceiling.value <= 0) return 0
  return Math.min(100, (props.usageBytes / ceiling.value) * 100)
})

const tickPercent = computed(() => {
  const warnBytes = props.quota?.warn_bytes ?? null
  if (warnBytes === null || warnBytes <= 0 || ceiling.value === null) return null
  if (warnBytes >= ceiling.value) return null
  return (warnBytes / ceiling.value) * 100
})

const action = computed(() => {
  if (!props.quota) return null
  return actionForHealth(health.value, props.quota.warn_action, props.quota.critical_action)
})

const statusLabel = computed(() => {
  if (health.value === 'critical')
    return `Over · ${action.value ? actionLabel(action.value) : 'Critical'}`
  if (health.value === 'warning') return `${Math.round(fillPercent.value)}% · Warning`
  return `${Math.round(fillPercent.value)}% · Healthy`
})
</script>

<template>
  <div
    v-if="visible"
    class="quota-meter"
  >
    <div
      class="quota-track"
      :class="{ 'quota-track-over': health === 'critical' }"
    >
      <div
        class="quota-fill"
        :class="`quota-fill-${health}`"
        :style="{ width: `${fillPercent}%` }"
      ></div>
      <span
        v-if="tickPercent !== null"
        class="quota-tick"
        :style="{ left: `${tickPercent}%` }"
      ></span>
    </div>
    <div class="quota-row">
      <span class="quota-usage"
        >{{ formatBytes(usageBytes) }} of {{ formatBytes(ceiling ?? 0) }}</span
      >
      <span
        class="quota-status"
        :class="`quota-status-${health}`"
      >
        {{ statusLabel }}
      </span>
    </div>
  </div>
</template>

<style scoped>
.quota-meter {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.quota-track {
  position: relative;
  height: 6px;
  border-radius: 3px;
  background: var(--border);
}

.quota-fill {
  height: 100%;
  border-radius: 3px;
  transition: width 0.3s ease;
}

.quota-fill-ok {
  background: var(--success);
}

.quota-fill-warning {
  background: var(--warning);
}

.quota-fill-critical,
.quota-track-over .quota-fill {
  background: repeating-linear-gradient(
    -45deg,
    var(--danger) 0 5px,
    color-mix(in oklab, var(--danger) 40%, var(--bg-card)) 5px 9px
  );
}

.quota-tick {
  position: absolute;
  top: -3px;
  bottom: -3px;
  width: 2px;
  border-radius: 1px;
  background: var(--text-muted);
  box-shadow: 0 0 0 2px var(--bg-card);
}

.quota-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.5rem;
  font-size: 0.7rem;
}

.quota-usage {
  font-family: var(--mono);
  color: var(--text-secondary);
}

.quota-status {
  font-weight: 600;
  white-space: nowrap;
}

.quota-status-ok {
  color: var(--success);
}

.quota-status-warning {
  color: var(--warning);
}

.quota-status-critical {
  color: var(--danger);
}
</style>
