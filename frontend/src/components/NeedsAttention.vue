<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { RouteLocationRaw } from 'vue-router'
import { RouterLink } from 'vue-router'
import { X } from '@lucide/vue'
import type { DashboardFinding } from '../types/dashboard'
import { relativeTime } from '../utils/format'
import { dismissFinding } from '../api/stats'
import { logger } from '../utils/logger'

defineProps<{ findings: DashboardFinding[] }>()
const emit = defineEmits<{ dismissed: [] }>()

function destinationRoute(finding: DashboardFinding): RouteLocationRaw {
  const dest = finding.destination
  switch (dest.kind) {
    case 'host':
      return `/agents/${encodeURIComponent(dest.hostname)}`
    case 'schedule':
      return `/schedules/${dest.schedule_id}`
    case 'repository':
      return `/repos/${dest.repo_id}`
    case 'activity': {
      const query: Record<string, string> = { category: 'backup' }
      if (finding.kind === 'backup_failed') query.status = 'failed'
      else if (finding.kind === 'backup_warning') query.status = 'warning'
      if (finding.schedule_id !== null) query.schedule_id = String(finding.schedule_id)
      return { path: '/activity', query }
    }
  }
}

function findingLabel(finding: DashboardFinding): string {
  return finding.hostname ?? finding.schedule_name ?? finding.repo_name ?? 'Backup system'
}

async function dismiss(finding: DashboardFinding): Promise<void> {
  try {
    await dismissFinding(finding.id)
    emit('dismissed')
  } catch (e: unknown) {
    logger.error('Failed to dismiss finding', e)
  }
}
</script>

<template>
  <section
    id="needs-attention"
    class="panel attention-panel"
  >
    <div class="panel-heading">
      <div>
        <h2>Needs attention</h2>
        <p>Current actionable findings, ordered by severity.</p>
      </div>
      <span class="finding-count">{{ findings.length }} findings</span>
    </div>
    <div
      v-if="findings.length > 0"
      class="finding-list"
    >
      <div
        v-for="finding in findings"
        :key="finding.id"
        class="finding-row"
      >
        <span
          class="severity-mark"
          :class="`severity-${finding.severity}`"
        />
        <span class="finding-body">
          <strong>{{ findingLabel(finding) }}</strong>
          <span class="finding-context">
            <template v-if="finding.schedule_name">{{ finding.schedule_name }}</template>
            <template v-if="finding.schedule_name && finding.repo_name"> · </template>
            <template v-if="finding.repo_name">{{ finding.repo_name }}</template>
          </span>
          <span
            class="finding-reason"
            :title="finding.reason"
            >{{ finding.reason }}</span
          >
        </span>
        <span class="finding-time">
          <template v-if="finding.deadline">Due {{ relativeTime(finding.deadline) }}</template>
          <template v-else-if="finding.occurred_at">
            {{ relativeTime(finding.occurred_at) }}
          </template>
        </span>
        <div class="finding-actions">
          <RouterLink
            :to="destinationRoute(finding)"
            class="finding-action"
          >
            Open
          </RouterLink>
          <button
            class="dismiss-btn"
            type="button"
            title="Dismiss"
            aria-label="Dismiss"
            @click="dismiss(finding)"
          >
            <X :size="12" />
          </button>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.attention-panel {
  border-top: 3px solid var(--warning);
}

.panel-heading {
  display: flex;
  justify-content: space-between;
  gap: var(--space-6);
  align-items: start;
}

h2 {
  margin: 0;
  font-size: var(--fs-base);
}

p {
  margin: var(--space-2) 0 var(--space-6);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.finding-count {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  white-space: nowrap;
}

.finding-list {
  display: flex;
  flex-direction: column;
}

.finding-row {
  display: grid;
  grid-template-columns: 8px minmax(0, 1fr) auto auto;
  gap: var(--space-5);
  align-items: center;
  padding: var(--space-5) 0;
  border-top: 1px solid var(--border);
}

.severity-mark {
  width: 8px;
  height: 32px;
  border-radius: var(--radius-pill);
}

.severity-critical {
  background: var(--danger);
}

.severity-warning {
  background: var(--warning);
}

.severity-info {
  background: var(--accent);
}

.finding-body {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-4);
  min-width: 0;
}

.finding-context,
.finding-reason,
.finding-time {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

/* A finding reason carries agent-supplied text (borg stderr, import errors).
   The server caps its length, but a narrow column can still turn that into a
   tall block, so the row keeps a hard two-line ceiling and hands the full text
   to the tooltip and the linked activity record. */
.finding-reason {
  flex-basis: 100%;
  min-width: 0;
  overflow-wrap: anywhere;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
}

.finding-actions {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.finding-action {
  font-weight: 600;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  text-decoration: none;
  transition: color var(--duration-base);
}

.finding-action:hover {
  color: var(--accent);
}

.dismiss-btn {
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--text-muted);
  font-size: var(--fs-2xs);
  padding: var(--space-1) var(--space-3);
  border-radius: var(--radius-sm);
  line-height: 1;
  transition:
    color var(--duration-base),
    background var(--duration-base);
}

.dismiss-btn:hover {
  color: var(--danger);
  background: var(--danger-subtle);
}

@media (max-width: 768px) {
  .finding-row {
    grid-template-columns: 8px minmax(0, 1fr) auto;
  }

  .finding-time {
    display: none;
  }
}
</style>
