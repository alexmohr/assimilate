<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { getActivity, type ActivityEntry } from '../api/stats'
import { relativeTime, formatDuration } from '../utils/format'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'

const items = ref<ActivityEntry[]>([])
const loading = ref(true)
const router = useRouter()
const now = ref(Date.now())
const expandedId = ref<number | null>(null)
let refreshTimer: ReturnType<typeof setInterval> | null = null

async function fetchActivity(): Promise<void> {
  try {
    items.value = await getActivity({ limit: 5 })
    now.value = Date.now()
  } finally {
    loading.value = false
  }
}

function onItemClick(item: ActivityEntry): void {
  if (normalizeBackupStatus(item.status) === 'success' && item.repo_id) {
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
  switch (normalizeBackupStatus(status)) {
    case 'success':
      return 'var(--success)'
    case 'warning':
      return 'var(--warning)'
    case 'started':
    case 'pending':
      return 'var(--info)'
    case 'failed':
    case 'cancelled':
      return 'var(--danger)'
  }
}

function liveRelativeTime(iso: string): string {
  void now.value
  return relativeTime(iso)
}
</script>

<template>
  <section class="panel">
    <h2 class="panel-title">Recent activity</h2>
    <div
      v-if="loading"
      class="state-msg state-msg--inline"
    >
      Loading...
    </div>
    <div
      v-else-if="items.length === 0"
      class="state-msg state-msg--inline"
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
            :style="{ color: statusColor(item.status) }"
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
.activity-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  overflow-y: auto;
  min-height: 0;
}

.activity-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3) 0;
  border-bottom: 1px solid var(--border);
}

.activity-item-clickable {
  cursor: pointer;
  padding: var(--space-3) var(--space-4);
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
  gap: var(--space-1);
}

.activity-host {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.activity-target {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.activity-meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--space-1);
  flex-shrink: 0;
}

.activity-time {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
}

.activity-duration {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  font-family: var(--mono);
}

.activity-error {
  font-size: var(--fs-2xs);
  background: var(--bg-code);
  border-radius: var(--radius-sm);
  padding: var(--space-3);
  margin-top: var(--space-2);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 6rem;
  overflow-y: auto;
}
</style>
