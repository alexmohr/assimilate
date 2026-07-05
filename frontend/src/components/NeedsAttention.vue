<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { RouteLocationRaw } from 'vue-router'
import { RouterLink } from 'vue-router'
import type { DashboardFinding } from '../types/dashboard'
import { relativeTime } from '../utils/format'
import { apiClient } from '../api/client'
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
    await apiClient.post(`/stats/findings/${encodeURIComponent(finding.id)}/dismiss`)
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
        <h2>Needs Attention</h2>
        <p>Current actionable findings, ordered by severity.</p>
      </div>
      <span class="finding-count">{{ findings.length }} findings</span>
    </div>
    <div
      v-if="findings.length === 0"
      class="empty-state"
    >
      No active problems
    </div>
    <div
      v-else
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
          <span class="finding-reason">{{ finding.reason }}</span>
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
            title="Dismiss"
            @click="dismiss(finding)"
          >
            ✕
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
  gap: 1rem;
  align-items: start;
}

h2 {
  margin: 0;
  font-size: 0.875rem;
}

p {
  margin: 0.25rem 0 1rem;
  color: var(--text-muted);
  font-size: 0.75rem;
}

.finding-count {
  color: var(--text-muted);
  font-size: 0.75rem;
  white-space: nowrap;
}

.empty-state {
  padding: 1.5rem;
  border: 1px dashed var(--border);
  border-radius: var(--radius-sm);
  color: var(--success);
  text-align: center;
}

.finding-list {
  display: flex;
  flex-direction: column;
}

.finding-row {
  display: grid;
  grid-template-columns: 8px minmax(0, 1fr) auto auto;
  gap: 0.85rem;
  align-items: center;
  padding: 0.8rem 0;
  border-top: 1px solid var(--border);
}

.severity-mark {
  width: 8px;
  height: 32px;
  border-radius: 99px;
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
  gap: 0.3rem 0.55rem;
  min-width: 0;
}

.finding-context,
.finding-reason,
.finding-time {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.finding-reason {
  flex-basis: 100%;
  min-width: 0;
  overflow-wrap: anywhere;
}

.finding-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.finding-action {
  font-weight: 600;
  color: var(--text-muted);
  font-size: 0.75rem;
  text-decoration: none;
  transition: color 0.15s;
}

.finding-action:hover {
  color: var(--accent);
}

.dismiss-btn {
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--text-muted);
  font-size: 0.7rem;
  padding: 0.15rem 0.35rem;
  border-radius: var(--radius-sm);
  line-height: 1;
  transition:
    color 0.15s,
    background 0.15s;
}

.dismiss-btn:hover {
  color: var(--danger);
  background: var(--danger-subtle);
}

@media (max-width: 700px) {
  .finding-row {
    grid-template-columns: 8px minmax(0, 1fr) auto;
  }

  .finding-time {
    display: none;
  }
}
</style>
