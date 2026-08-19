<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
export interface EntityIssue {
  key: string
  label: string
  severity: 'danger' | 'warning'
  onClick: () => void
  /** Optional detail shown as a native tooltip on hover, e.g. which hosts are overdue. */
  title?: string
}

defineProps<{
  notable?: boolean
  notableLabel?: string
  running?: boolean
  runningLabel?: string
  issues?: EntityIssue[]
}>()
</script>

<template>
  <div
    v-if="running || notable || (issues && issues.length > 0)"
    class="entity-badge-row"
  >
    <span
      v-if="running"
      class="entity-running-pill"
      :title="runningLabel"
    >
      <span class="pulse-dot pulse-dot--accent" />
      {{ runningLabel ?? 'Running' }}
    </span>
    <button
      v-for="issue in issues"
      :key="issue.key"
      type="button"
      class="entity-issue-chip"
      :class="`sev-${issue.severity}`"
      :title="issue.title"
      @click.stop="issue.onClick()"
    >
      {{ issue.label }}
    </button>
    <span
      v-if="notable"
      class="entity-status-pill"
    >
      {{ notableLabel }}
    </span>
  </div>
</template>

<style scoped>
.entity-badge-row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
  align-items: center;
}

.entity-issue-chip {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-4);
  border-radius: var(--radius-pill);
  font-size: var(--fs-2xs);
  font-weight: 600;
  letter-spacing: 0.02em;
  border: none;
  font-family: inherit;
  white-space: nowrap;
  cursor: pointer;
  transition: opacity var(--duration-base);
}

.entity-issue-chip:hover {
  opacity: 0.85;
}

.entity-issue-chip.sev-danger {
  background: var(--danger-subtle);
  color: var(--danger);
}

.entity-issue-chip.sev-warning {
  background: var(--warning-subtle);
  color: var(--warning);
}

.entity-status-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-4);
  border-radius: var(--radius-pill);
  font-size: var(--fs-2xs);
  font-weight: 600;
  letter-spacing: 0.03em;
  text-transform: uppercase;
  background: var(--bg-card);
  color: var(--text-secondary);
}

.entity-running-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-1) var(--space-4);
  border-radius: var(--radius-pill);
  font-size: var(--fs-2xs);
  font-weight: 600;
  background: var(--accent-subtle);
  color: var(--accent);
  max-width: 220px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
</style>
