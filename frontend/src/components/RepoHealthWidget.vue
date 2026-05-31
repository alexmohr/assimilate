<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { relativeTime } from '../utils/format'

interface HealthEntry {
  repo_id: number
  hostname: string
  target_name: string
  last_status: string | null
  last_backup_at: string | null
  is_overdue: boolean
}

const props = defineProps<{ health: HealthEntry[] }>()
const router = useRouter()

const visibleItems = computed((): HealthEntry[] => props.health.slice(0, 8))
const hasMore = computed((): boolean => props.health.length > 8)

function statusColor(entry: HealthEntry): string {
  if (entry.is_overdue) return 'var(--danger)'
  if (entry.last_status === 'success') return 'var(--success)'
  if (entry.last_status === 'warning') return 'var(--warning)'
  return 'var(--danger)'
}
</script>

<template>
  <section class="panel">
    <h2 class="panel-title">Detailed Status</h2>
    <div
      v-if="props.health.length === 0"
      class="state-msg"
    >
      No repositories configured.
    </div>
    <div
      v-else
      class="health-list"
    >
      <div
        v-for="entry in visibleItems"
        :key="`${entry.hostname}-${entry.target_name}`"
        class="health-item"
        @click="router.push(`/repos/${entry.repo_id}`)"
      >
        <span
          class="health-dot"
          :style="{ background: statusColor(entry) }"
        />
        <div class="health-info">
          <span class="health-name">{{ entry.hostname }} / {{ entry.target_name }}</span>
          <span class="health-time">{{
            entry.last_backup_at ? relativeTime(entry.last_backup_at) : 'Never'
          }}</span>
        </div>
        <span
          v-if="entry.is_overdue"
          class="overdue-badge"
        >
          OVERDUE
        </span>
      </div>
      <button
        v-if="hasMore"
        class="view-all-btn"
        @click="router.push({ name: 'repos' })"
      >
        View All ({{ props.health.length }})
      </button>
    </div>
  </section>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.panel-title {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  margin: 0 0 0.75rem;
}

.state-msg {
  color: var(--text-muted);
  font-size: 0.875rem;
  padding: 0.5rem 0;
}

.health-list {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.health-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.5rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.15s;
}

.health-item:hover {
  background: var(--bg-hover);
}

.health-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
}

.health-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.health-name {
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.health-time {
  font-size: 0.68rem;
  color: var(--text-muted);
}

.overdue-badge {
  background: var(--danger-subtle);
  color: var(--danger);
  padding: 0.1rem 0.35rem;
  border-radius: 0.2rem;
  font-weight: 700;
  font-size: 0.55rem;
  flex-shrink: 0;
}

.view-all-btn {
  margin-top: 0.5rem;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-size: 0.7rem;
  padding: 0.35rem 0.75rem;
  cursor: pointer;
  transition: border-color 0.15s;
}

.view-all-btn:hover {
  border-color: var(--accent);
  color: var(--text-primary);
}
</style>
