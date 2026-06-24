<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { relativeTime, formatDuration } from '../utils/format'
import { logger } from '../utils/logger'

interface ActivityEntry {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
  repo_id: number | null
  archive_name: string | null
  error_message: string | null
}

const items = ref<ActivityEntry[]>([])
const loading = ref(true)
const router = useRouter()
const now = ref(Date.now())
const expandedId = ref<number | null>(null)
let refreshTimer: ReturnType<typeof setInterval> | null = null

async function fetchActivity(): Promise<void> {
  try {
    const response = await apiClient.get<ActivityEntry[]>('/stats/activity?limit=5')
    items.value = response.data
    now.value = Date.now()
  } finally {
    loading.value = false
  }
}

function onItemClick(item: ActivityEntry): void {
  if (item.status === 'success' && item.repo_id) {
    const query: Record<string, string> = { tab: 'archives' }
    if (item.archive_name) {
      query.archive = item.archive_name
    }
    router.push({ path: `/repos/${item.repo_id}`, query })
  } else if (item.error_message) {
    expandedId.value = expandedId.value === item.id ? null : item.id
  }
}

onMounted(() => {
  fetchActivity().catch(logger.error)
  refreshTimer = setInterval(() => {
    now.value = Date.now()
    fetchActivity().catch(logger.error)
  }, 30_000)
})

onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer)
})

function statusColor(status: string): string {
  if (status === 'success') return 'var(--success)'
  if (status === 'warning') return 'var(--warning)'
  if (status === 'started') return 'var(--info)'
  return 'var(--danger)'
}

function liveRelativeTime(iso: string): string {
  void now.value
  return relativeTime(iso)
}
</script>

<template>
  <section class="panel">
    <h2 class="panel-title">Recent Activity</h2>
    <div
      v-if="loading"
      class="state-msg"
    >
      Loading…
    </div>
    <div
      v-else-if="items.length === 0"
      class="state-msg"
    >
      No recent activity.
    </div>
    <div
      v-else
      class="activity-list"
    >
      <div
        v-for="item in items"
        :key="item.id"
        class="activity-item activity-item-clickable"
        @click="onItemClick(item)"
      >
        <span
          class="activity-dot"
          :style="{ background: statusColor(item.status) }"
        />
        <div class="activity-info">
          <span class="activity-host">{{ item.hostname }}</span>
          <span class="activity-target">{{ item.target_name }}</span>
          <pre
            v-if="expandedId === item.id && item.error_message"
            class="activity-error"
            >{{ item.error_message }}</pre
          >
        </div>
        <div class="activity-meta">
          <span class="activity-time">{{ liveRelativeTime(item.started_at) }}</span>
          <span class="activity-duration">{{ formatDuration(item.duration_secs) }}</span>
        </div>
      </div>
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
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 0.75rem;
}

.state-msg {
  color: var(--text-muted);
  font-size: 0.875rem;
  padding: 0.5rem 0;
}

.activity-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  overflow-y: auto;
  min-height: 0;
}

.activity-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0;
  border-bottom: 1px solid var(--border);
}

.activity-item-clickable {
  cursor: pointer;
  padding: 0.35rem 0.5rem;
  border-radius: var(--radius-sm);
}

.activity-item-clickable:hover {
  background: var(--bg-hover);
}

.activity-item:last-child {
  border-bottom: none;
}

.activity-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.activity-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.activity-host {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.activity-target {
  font-size: 0.7rem;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.activity-meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 0.1rem;
  flex-shrink: 0;
}

.activity-time {
  font-size: 0.7rem;
  color: var(--text-muted);
}

.activity-duration {
  font-size: 0.65rem;
  color: var(--text-muted);
  font-family: var(--mono);
}

.activity-error {
  font-size: 0.65rem;
  color: var(--danger);
  background: var(--bg-code, var(--bg-hover));
  border-radius: var(--radius-sm);
  padding: 0.4rem;
  margin-top: 0.25rem;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 6rem;
  overflow-y: auto;
}
</style>
