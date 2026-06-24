<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { RouterLink } from 'vue-router'
import type { DashboardRepositoryCapacity } from '../types/dashboard'
import { formatBytes, relativeTime } from '../utils/format'

defineProps<{ repositories: DashboardRepositoryCapacity[] }>()
</script>

<template>
  <section
    id="repository-capacity"
    class="panel"
  >
    <h2 class="panel-title">Repository Capacity</h2>
    <div
      v-if="repositories.length === 0"
      class="empty-state"
    >
      No enabled repositories
    </div>
    <RouterLink
      v-for="repo in repositories"
      :key="repo.repo_id"
      :to="`/repos/${repo.repo_id}`"
      class="capacity-row"
    >
      <span
        class="quota-dot"
        :class="`quota-${repo.quota_status}`"
      />
      <span>
        <strong>{{ repo.repo_name }}</strong>
        <small>{{ formatBytes(repo.deduplicated_size) }} deduplicated</small>
      </span>
      <span class="quota-value">
        <template v-if="repo.quota_utilization_percent !== null">
          {{ Math.round(repo.quota_utilization_percent) }}% of
          {{ formatBytes(repo.quota_bytes ?? 0) }}
        </template>
        <template v-else>No quota</template>
      </span>
      <span class="history-value">
        {{
          repo.threshold_estimate ? relativeTime(repo.threshold_estimate) : 'Insufficient history'
        }}
      </span>
    </RouterLink>
  </section>
</template>

<style scoped>
.panel-title {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 0.75rem;
}

.capacity-row {
  display: grid;
  grid-template-columns: 10px minmax(0, 1fr) auto auto;
  gap: 0.75rem;
  align-items: center;
  padding: 0.75rem 0;
  border-top: 1px solid var(--border);
  color: inherit;
  text-decoration: none;
}

.capacity-row > span:nth-child(2) {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

small,
.quota-value,
.history-value,
.empty-state {
  color: var(--text-muted);
  font-size: 0.7rem;
}

.quota-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
}

.quota-unconfigured {
  background: var(--text-muted);
}

.quota-healthy {
  background: var(--success);
}

.quota-warning {
  background: var(--warning);
}

.quota-critical {
  background: var(--danger);
}

@media (max-width: 700px) {
  .capacity-row {
    grid-template-columns: 10px minmax(0, 1fr) auto;
  }

  .history-value {
    display: none;
  }
}
</style>
