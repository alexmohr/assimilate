<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { RouteLocationRaw } from 'vue-router'
import { RouterLink } from 'vue-router'
import type { DashboardDestination, DashboardFinding } from '../types/dashboard'
import { relativeTime } from '../utils/format'

defineProps<{ findings: DashboardFinding[] }>()

function destinationRoute(destination: DashboardDestination): RouteLocationRaw {
  switch (destination.kind) {
    case 'host':
      return `/agents/${encodeURIComponent(destination.hostname)}`
    case 'schedule':
      return `/schedules/${destination.schedule_id}`
    case 'repository':
      return `/repos/${destination.repo_id}`
    case 'activity':
      return { path: '/activity', query: { report: destination.report_id } }
  }
}

function findingLabel(finding: DashboardFinding): string {
  return finding.hostname ?? finding.schedule_name ?? finding.repo_name ?? 'Backup system'
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
      <RouterLink
        v-for="finding in findings"
        :key="finding.id"
        :to="destinationRoute(finding.destination)"
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
        <span class="finding-action">Open</span>
      </RouterLink>
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
  font-size: 1rem;
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
  color: inherit;
  text-decoration: none;
  border-top: 1px solid var(--border);
}

.finding-row:hover .finding-action {
  color: var(--accent);
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
.finding-time,
.finding-action {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.finding-reason {
  flex-basis: 100%;
}

.finding-action {
  font-weight: 600;
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
